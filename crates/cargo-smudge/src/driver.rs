//! The `dev` session: fat-builds the app, runs it against the wire format,
//! watches its sources and patches it on every save.

use std::collections::{BTreeMap, HashMap};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::dwarf::Signatures;
use crate::elf::{self, Symbols};
use crate::patch::{self, Outcome, Plan};
use crate::record::{self, Recorded};
use crate::wire::{self, HELLO};

/// The variable that names the socket, as the platform crate documents it.
const SOCKET_ENVIRONMENT: &str = "SMUDGE_HOT_SOCKET";

/// The variable that carries the build id, as the platform crate documents
/// it.
const BUILD_ENVIRONMENT: &str = "SMUDGE_HOT_BUILD";

/// How long the builder waits for the app to connect.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the builder waits between polls of the sources.
const POLL: Duration = Duration::from_millis(100);

/// The shape of the symbol cache, bumped when what it stores changes.
const CACHE_SHAPE: u32 = 4;

/// The Unix socket path limit, which counts the terminating NUL.
const SOCKET_PATH_LIMIT: usize = 108;

/// What the user asked the `dev` session to build and run.
#[derive(Default)]
pub(crate) struct Options {
    /// The bin target, or none for the package's only one.
    pub bin: Option<String>,
    /// The example target, instead of a bin.
    pub example: Option<String>,
    /// The package to build, in a workspace.
    pub package: Option<String>,
    /// The features to build with, one entry per `--features`.
    pub features: Vec<String>,
    /// The profile to build with.
    pub profile: Option<String>,
}

/// What one session ended with, and what the outer loop does next.
enum Control {
    /// The app exited, so the session ends.
    Exited,
    /// The build inputs changed, so the whole session restarts.
    Restart,
    /// The user asked to quit.
    Quit,
}

/// The original binary a patch is built against, read once per fat build.
struct Original {
    /// Every symbol the binary defines or imports.
    symbols: Symbols,
    /// The signature of every function the DWARF describes.
    signatures: Signatures,
    /// The address `main` was linked at.
    main: u64,
    /// The initialization image of the binary's thread-locals, with its
    /// alignment and the relocations its pointer slots live in.
    tls: elf::TlsTemplate,
    /// The binary's thread-locals that the standard library defines.
    sysroot_tls: std::collections::BTreeSet<String>,
}

/// One recorded unit the fat build compiled: one invocation of one
/// crate under one feature set. A build can make several units of one
/// crate, which each keep their own record.
struct UnitRecord {
    /// The crate name the invocation declares.
    name: String,
    /// The stem of the unit's artifacts, which its dep-info file and
    /// its consumers' artifacts carry.
    stem: String,
    /// Whether the unit compiles as a procedural macro.
    proc_macro: bool,
    /// The crate root the invocation compiles, absolute.
    root: PathBuf,
    /// When the record file was written, which tells a recording this
    /// build made from one an earlier request left behind.
    recorded: Option<u128>,
    /// The recorded invocation.
    record: Recorded,
}

/// One workspace crate the patch builder knows about: the units the
/// build named for this crate, collapsed into one record for the replay
/// chain.
struct CrateRecord {
    /// The crate name the invocation declares.
    name: String,
    /// Whether the crate compiles as a procedural macro, which produces
    /// only the shared library its consumers read at their own compile
    /// time.
    proc_macro: bool,
    /// Whether the crate's objects are linked into the running app, which
    /// a crate is only when no procedural macro sits between it and the
    /// app crate.
    runtime: bool,
    /// Whether a unit of the build names this crate, which a record a
    /// session with other cargo features left behind does not.
    trusted: bool,
    /// Whether the build made the crate as more than one unit, which one
    /// recording cannot replay.
    ambiguous: bool,
    /// The source files the build's units compiled, from their
    /// dep-info, which decides what an edited file belongs to.
    sources: Vec<PathBuf>,
    /// The crate root the record's invocation compiles, absolute.
    root: PathBuf,
    /// The dependencies of every unit of the build, as the alias the
    /// consumer uses and the real crate name, which a renamed dependency
    /// keeps. Only the union holds a chain together when the units of a
    /// crate carry different dependencies.
    edges: Vec<(String, String)>,
    /// The recorded invocation of the unit a patch replays.
    record: Recorded,
}

/// A build script whose run a patch replays against: the package it
/// belongs to, the crate names its expansion reads, the source files it
/// itself compiled from, the files and directories its run declared
/// through `cargo:rerun-if-changed`, and whether this request's own
/// build ran it.
struct ScriptGuard {
    /// The script's package directory.
    package: PathBuf,
    /// The crate names the script's run reads, which a replay of one of
    /// them would leave the generated files old for.
    reads: Vec<String>,
    /// The source files the script compiles from, from its dep-info,
    /// which a stale replay would leave unrerun as well.
    inputs: Vec<PathBuf>,
    /// The directories the run declared, whose whole content a stale
    /// replay would leave unrerun with them.
    dirs: Vec<PathBuf>,
    /// The build request the recording names, which tells the scripts
    /// one request's build ran from the ones other requests' feature
    /// selections left behind.
    request: Option<String>,
    /// Whether the recording predates this request's marker, which a
    /// recording without a request stamp falls back to.
    fresh: bool,
}

/// One session: one fat build, one app, one connection.
struct Session {
    /// The directory the records, the cache and the libraries go into.
    dir: PathBuf,
    /// The app crate's recorded invocation.
    rustc: Recorded,
    /// Every other workspace crate the fat build compiled, with its own
    /// recorded invocation.
    crates: Vec<CrateRecord>,
    /// The linker cargo configured, or `cc`.
    linker: String,
    /// The original binary.
    original: Original,
    /// The running app.
    child: Child,
    /// The builder's end of the connection, until the app closes it.
    stream: Option<UnixStream>,
    /// The address `main` runs at.
    runtime_main: u64,
    /// The linked address of the view, when the hello carried it.
    view: Option<u64>,
    /// Whether a patch was already refused over a signature change, which
    /// the later patches omit the changed functions for instead of
    /// refusing again.
    signature_refused: bool,
    /// Whether an edit a build script compiles from was saved, which no
    /// patch of this session can act on: the script cannot rerun, so
    /// every later attempt would replay against the expansion its last
    /// run generated.
    scripts_stale: bool,
    /// The build scripts of the packages this build compiled, which a
    /// patch can never rerun.
    build_scripts: Vec<ScriptGuard>,
    /// The directories whose `.rs` files are watched: the app crate's and
    /// every recorded workspace crate's.
    watch: Vec<PathBuf>,
    /// The dep-info sources outside those directories, watched as files
    /// of their own.
    watch_files: Vec<PathBuf>,
    /// The directories the build scripts declared, each with the newest
    /// time their content answered at bootstrap, whose movement a patch
    /// cannot regenerate.
    script_dirs: Vec<(PathBuf, Option<BTreeMap<PathBuf, u128>>)>,
    /// The script-declared inputs that existed at bootstrap, whose
    /// disappearance the watcher's scans cannot report.
    script_inputs_present: std::collections::BTreeSet<PathBuf>,
    /// The watched sources that existed at bootstrap, whose
    /// disappearance the scans cannot report either: a library input
    /// that is gone cannot compile again, so its owners must hear of
    /// the loss.
    watched_present: std::collections::BTreeSet<PathBuf>,
    /// The modification time each watched path carried when it was
    /// last observed, whatever direction the next save moves it in: a
    /// save can land on a file whose stamp stands ahead of the
    /// session, and the fall back down is the change.
    source_times: std::collections::HashMap<PathBuf, SystemTime>,
    /// The manifests and their modification times, watched for restarts.
    manifests: Vec<(PathBuf, SystemTime)>,
    /// The build script path of every workspace package that may have one,
    /// and whether it existed when the session started: a script that
    /// appears or goes without a manifest edit changes the build as much
    /// as a manifest edit does, and a manifest edit restarts the session,
    /// which decides the paths again.
    scripts_present: Vec<(PathBuf, bool)>,
    /// The configuration files cargo reads, with the content stamps the
    /// session saw at its start, whose change a restart follows.
    config_stamps: Vec<(PathBuf, u64)>,
    /// How many patches this session has sent.
    generation: u32,
    /// The app crate's source root, which belongs to no recorded crate.
    tip_root: PathBuf,
    /// The source files the app crate compiled, from its dep-info.
    tip_sources: Vec<PathBuf>,
    /// The crates whose replayed code is installed in the running app,
    /// because every patch must carry the library edits of the patches
    /// before it, and a restart of the app drops them.
    live: std::collections::BTreeSet<String>,
    /// The app crate's statics the session has already said start fresh
    /// in the patched code, by symbol name, so each is named once per
    /// session.
    told_fresh: std::collections::BTreeSet<String>,
    /// The crates whose edits a patch attempt took over without yet
    /// installing, because a failed or refused attempt must not forget
    /// them the way a forgotten scan would.
    pending: std::collections::BTreeSet<String>,
}

/// Runs the `dev` session for the app in the current directory.
pub(crate) fn run(options: Options) -> ExitCode {
    let app = match std::env::current_dir()
        .map_err(|e| e.to_string())
        .and_then(|dir| dir.canonicalize().map_err(|e| e.to_string()))
    {
        Ok(app) => app,
        Err(error) => return fail(&format!("the app directory cannot be used: {error}")),
    };
    let (sender, commands) = mpsc::channel();
    if std::thread::Builder::new()
        .name(String::from("smudge-dev-stdin"))
        .spawn(move || commands_thread(sender))
        .is_err()
    {
        return fail("the session could not start its reader for the standard input");
    }
    let mut started = false;
    let edit_root = workspace_root_of(&app).unwrap_or_else(|| app.clone());
    loop {
        // The build writes its output while it fails, which must not read
        // as an edit, wherever the target directory sits and whatever its
        // name. It is asked for again each time, since a manifest edit can
        // move it.
        let output = target_dir_of(&app)
            .ok()
            .map(|dir| dir.canonicalize().unwrap_or(dir))
            .unwrap_or_else(|| edit_root.join("target"));
        // An edit that lands while the build runs must count as one that
        // follows it, so the snapshot predates the build.
        let edited = started.then(|| edit_snapshot(&edit_root, &output));
        let mut session = match bootstrap(&app, &options) {
            Ok(session) => session,
            Err(error) if !started => return fail(&error),
            Err(error) => {
                // A rebuild that fails, such as on a compile error, keeps
                // the session, which builds again after the next edit.
                eprintln!("cargo smudge: {error}");
                println!("waiting for an edit to build again");
                match wait_for_edit(&edit_root, &output, edited.unwrap_or_default(), &commands) {
                    Control::Restart => continue,
                    Control::Exited | Control::Quit => return ExitCode::SUCCESS,
                }
            }
        };
        started = true;
        match supervise(&mut session, &commands) {
            Control::Exited | Control::Quit => return ExitCode::SUCCESS,
            Control::Restart => continue,
        }
    }
}

/// Waits until a file under `root`, outside `output`, differs from
/// `before`, or the user asks to restart, which both answer
/// [`Control::Restart`], or to quit.
fn wait_for_edit(
    root: &Path,
    output: &Path,
    before: BTreeMap<PathBuf, SystemTime>,
    commands: &mpsc::Receiver<String>,
) -> Control {
    loop {
        match commands.try_recv() {
            Ok(command) if command == "quit" => return Control::Quit,
            Ok(_) => return Control::Restart,
            Err(_) => {}
        }
        std::thread::sleep(POLL * 5);
        if edit_snapshot(root, output) != before {
            return Control::Restart;
        }
    }
}

/// The modification time of every file under `dir`, leaving out the
/// build's `output`, every `target` directory beside a manifest, which holds
/// some build's output, and the hidden directories a version control tool
/// writes, but not cargo's own configuration directory. Every file counts on
/// its own, so a file whose time stands in the future hides no other edit.
fn edit_snapshot(dir: &Path, output: &Path) -> BTreeMap<PathBuf, SystemTime> {
    let mut times = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let hidden = name.to_string_lossy().starts_with('.') && name != ".cargo";
            // A package's default output sits beside its manifest; a
            // `target` directory anywhere else, such as a module's, is
            // source.
            let output_beside_manifest = name == "target" && dir.join("Cargo.toml").exists();
            if hidden || output_beside_manifest || entry.path() == output {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                stack.push(entry.path());
            } else if let Ok(modified) = metadata.modified() {
                times.insert(entry.path(), modified);
            }
        }
    }
    times
}

/// Reads the standard input for commands until it closes.
fn commands_thread(sender: mpsc::Sender<String>) {
    let mut line = String::new();
    loop {
        line.clear();
        match std::io::stdin().read_line(&mut line) {
            // A closed input, as under a task runner, leaves the session
            // running until the app exits or the process is stopped.
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        match line.trim() {
            "r" | "restart" => {
                if sender.send(String::from("restart")).is_err() {
                    break;
                }
            }
            "q" | "quit" | "exit" => {
                let _ = sender.send(String::from("quit"));
                break;
            }
            _ => {}
        }
    }
    let _ = sender.send(String::from("quit"));
}

/// Fat-builds the app, reads what the build recorded, starts the app and
/// performs the handshake.
fn bootstrap(app: &Path, options: &Options) -> Result<Session, String> {
    let target = target_dir_of(app)?;
    let dir = target.join("smudge-hot");
    let patches = dir.join("patches");
    std::fs::create_dir_all(&patches).map_err(|e| format!("{}: {e}", patches.display()))?;
    // A build request the recordings did not come from has to rebuild the
    // workspace's own crates from clean: cargo's cache would otherwise
    // serve the old units without running the wrapper, and the session
    // would replay recordings of another build. The target directory, the
    // app it serves and everything that selects cargo's cached units
    // belong to the request, because the recordings sit outside them and
    // cargo's cache is per directory. The flags cargo reads live in the
    // environment under several names, which the scan catches whole.
    let mut request_args = cargo_args(app, options);
    request_args.push(app.display().to_string());
    request_args.push(target.display().to_string());
    if let Ok(real) = target.canonicalize() {
        request_args.push(real.display().to_string());
    }
    let root = workspace_root_of(app);
    request_args.push(rustc_identity(app, root.as_deref()));
    let mut flag_variables: Vec<String> = std::env::vars()
        .filter(|(key, _)| build_variable(key))
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    flag_variables.sort();
    request_args.extend(flag_variables);
    // The manifests and the configuration files decide which units the
    // build makes: a feature moved into a default, a build script
    // gained or dropped, a compiler moved between entries. Their
    // content joins the request, so a change renews it and the cached
    // units of the old selection rebuild from clean.
    for path in request_inputs(app) {
        request_args.push(format!("{}={:016x}", path.display(), file_stamp(&path)));
    }
    let request = serde_json::to_string(&request_args).map_err(|e| e.to_string())?;
    let known = std::fs::read_to_string(dir.join("build-request.json")).ok();
    if known.as_deref() != Some(request.as_str()) {
        // An unknown request is as good as a changed one: the cache of
        // an earlier session, whatever wrote it, carries units no
        // recording describes, and only a clean makes the build rerun
        // the wrapper for every crate this request answers for.
        if known.is_some() {
            println!("the build request changed, so the workspace's own crates rebuild from clean");
        } else {
            println!(
                "the build request is unknown, so the workspace's own crates rebuild from clean"
            );
        }
        for package in workspace_packages(app) {
            let mut clean = Command::new("cargo");
            clean.current_dir(app).args(["clean", "-p", &package]);
            // The clean must answer to the same profile the build
            // names: cargo's package clean removes the artifacts of
            // one profile, and a custom profile's cache would
            // otherwise survive the clean and serve its units without
            // running the wrapper again.
            if let Some(profile) = &options.profile {
                clean.args(["--profile", profile]);
            }
            clean.env("CARGO_TARGET_DIR", &target);
            let status = clean
                .status()
                .map_err(|e| format!("cargo clean could not run: {e}"))?;
            if !status.success() {
                return Err(format!(
                    "cargo clean -p {package} failed; cargo's output is above"
                ));
            }
        }
        record::write_bytes(request.as_bytes(), &dir.join("build-request.json"))?;
    }
    // Every request also keeps a marker of its own, written the first
    // time the target sees it: the recordings one request's build made
    // stay fresh for it while other requests' sessions come and go, for
    // a build-only package's script survives the clean of the workspace
    // members and its recording answers this request as long as this
    // request's marker predates it.
    let request_stamp = request_key(&request);
    let request_path = dir.join(format!("build-request-{request_stamp}.json"));
    if !request_path.exists() {
        record::write_bytes(request.as_bytes(), &request_path)?;
    }
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let started = SystemTime::now();
    let status = Command::new("cargo")
        .current_dir(app)
        .args(cargo_args(app, options))
        .env("CARGO_TARGET_DIR", &target)
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", &me)
        .env(crate::roles::DIR_ENVIRONMENT, &dir)
        .env(crate::roles::REQUEST_ENVIRONMENT, &request_stamp)
        .env(crate::roles::LINK_ENVIRONMENT, "fat")
        .env(crate::roles::TARGET_ENVIRONMENT, &target)
        .status()
        .map_err(|e| format!("cargo could not run: {e}"))?;
    if !status.success() {
        return Err(String::from(
            "the fat build failed; cargo's output is above",
        ));
    }
    println!("fat build in {:?}", elapsed(started));

    let rustc = Recorded::load(&dir.join("rustc.json"))?;
    // The recording the tip's compile left names the request it came
    // from. One that names another is a cache the clean could not
    // reach, and replaying it would serve another build's arguments,
    // so the session refuses rather than start on the wrong footing.
    if rustc.request.as_deref() != Some(request_stamp.as_str()) {
        return Err(format!(
            "the recording of the app describes another build request; \
             remove {} and start the session again",
            dir.join("rustc.json").display()
        ));
    }
    let units = crate_records(&dir)?;
    let tip_name = String::from(rustc.crate_name().unwrap_or_default());
    // A unit belongs to the build the app links when the chain of
    // artifacts names it: the tip's own externs seed the set, and the
    // externs of every named unit grow it. A record a session with
    // other cargo features left behind is never named, so it cannot
    // make this build look multi-unit or trusted.
    let mut referenced: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (_, path) in rustc.externs() {
        if let Some(stem) = crate::record::stem_of_artifact(&path) {
            referenced.insert(stem);
        }
    }
    loop {
        let mut grew = false;
        for unit in &units {
            if !referenced.contains(&unit.stem) {
                continue;
            }
            for (_, path) in unit.record.externs() {
                if let Some(stem) = crate::record::stem_of_artifact(&path)
                    && referenced.insert(stem)
                {
                    grew = true;
                }
            }
        }
        if !grew {
            break;
        }
    }
    // A recording only replays when it describes the build the app
    // links, which a recording a session with other cargo features left
    // does not: cargo's fingerprints then reuse the artifact this build
    // links without running the wrapper again. And a crate the build
    // made as two units, which different features for a normal and a
    // macro consumer do, cannot replay from one recording at all.
    let mut units_of: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for stem in &referenced {
        if let Some(name) = crate::record::crate_of_stem(stem) {
            *units_of.entry(name).or_default() += 1;
        }
    }
    // The build scripts the units describe, with the crate names their
    // expansions read, before the collapse takes the units apart. A
    // recording this request's own build wrote is kept beside them: its
    // package can be a build-only dependency, which no library record
    // of the app's extern graph names. The marker of this request sets
    // the epoch and the recording's directory keeps a script another
    // fixture's build left in the shared target out.
    let request_mark = modified_nanos(&request_path);
    let home = root
        .and_then(|root| std::fs::canonicalize(root).ok())
        .or_else(|| std::fs::canonicalize(app).ok())
        .unwrap_or_else(|| app.to_path_buf());
    // The build directories whose runs this build answers for: the
    // compile's tree carries every script's binary, and an explicit
    // target moves the runs of the units the app's kind consumes to a
    // tree of their own, whose build directories sit beside the
    // recorded ones.
    let active_build = rustc
        .out_dir()
        .and_then(|out| Path::new(out).parent())
        .map(|profile| profile.join("build"));
    // The files and directories the script runs declared, per package,
    // which resolver two's feature variants make several of: one build
    // can run a package's script once per unit it makes of it, and each
    // run declares its own inputs, so every output whose window this
    // request's recordings name joins the guard. The oldest recording
    // of the package sets the window's floor, since a run follows its
    // own compile and the outputs an earlier request's runs wrote
    // predate this build's recordings.
    let mut stamp_bounds: std::collections::BTreeMap<PathBuf, SystemTime> = Default::default();
    for unit in units.iter().filter(|unit| is_script_unit(unit)) {
        let Some(nanos) = unit.recorded else {
            continue;
        };
        if unit.record.request.as_deref() != Some(request_stamp.as_str()) {
            continue;
        }
        let Some(package) = unit.record.env.get("CARGO_MANIFEST_DIR").map(PathBuf::from) else {
            continue;
        };
        let bound = time_of(nanos);
        stamp_bounds
            .entry(package)
            .and_modify(|kept| {
                if *kept > bound {
                    *kept = bound;
                }
            })
            .or_insert(bound);
    }
    let mut declared_of: std::collections::BTreeMap<PathBuf, (Vec<PathBuf>, Vec<PathBuf>)> =
        Default::default();
    let mut build_scripts: Vec<ScriptGuard> = units
        .iter()
        .filter(|unit| is_script_unit(unit))
        .filter_map(|script| {
            let package = script
                .record
                .env
                .get("CARGO_MANIFEST_DIR")
                .map(PathBuf::from)
                .or_else(|| script.root.parent().map(Path::to_path_buf))?;
            let here = std::fs::canonicalize(&script.record.cwd)
                .unwrap_or_else(|_| script.record.cwd.clone());
            let fresh = script.recorded > request_mark && here.starts_with(&home);
            // The files and directories the script's run declared are
            // compile-time content like the sources it compiles from,
            // wherever a library reads them too or the script alone
            // does. So are the files the libraries its compile links
            // compile from: those are its build dependencies, which
            // the app's library graph never reaches, and what they
            // carry bakes into what the script generates. The walk
            // follows the artifact stems the script's recording names,
            // not the crate names: a recording of another feature
            // selection under the same name describes a dependency
            // this script does not read, and its edges would refuse
            // patches this session could rightly send.
            let mut inputs = unit_sources(&script.record, &script.stem);
            let mut reads: std::collections::BTreeSet<String> = Default::default();
            let mut visited: std::collections::BTreeSet<String> = Default::default();
            let mut build_only: Vec<String> = script
                .record
                .externs()
                .into_iter()
                .filter_map(|(_, path)| crate::record::stem_of_artifact(&path))
                .collect();
            while let Some(stem) = build_only.pop() {
                if !visited.insert(stem.clone()) {
                    continue;
                }
                for unit in units.iter().filter(|unit| unit.stem == stem) {
                    inputs.extend(unit_sources(&unit.record, &unit.stem));
                    reads.insert(unit.name.clone());
                    build_only.extend(
                        unit.record
                            .externs()
                            .into_iter()
                            .filter_map(|(_, path)| crate::record::stem_of_artifact(&path)),
                    );
                }
            }
            let declared = match stamp_bounds.get(&package) {
                Some(bound) => declared_of
                    .entry(package.clone())
                    .or_insert_with(|| {
                        script_declared_inputs(
                            &script.record,
                            &package,
                            Some(*bound),
                            active_build.as_deref(),
                        )
                    })
                    .clone(),
                None => {
                    script_declared_inputs(&script.record, &package, None, active_build.as_deref())
                }
            };
            inputs.extend(declared.0);
            Some(ScriptGuard {
                package,
                reads: reads.into_iter().collect(),
                inputs,
                dirs: declared.1,
                request: script.record.request.clone(),
                fresh,
            })
        })
        .collect();
    // The units of one name collapse into one record for the chain,
    // whose edges are the union of the units the build named: a
    // dependency only one unit carries still holds the chain together,
    // whatever the last recorded invocation happened to be.
    let mut by_name: std::collections::BTreeMap<String, Vec<UnitRecord>> =
        std::collections::BTreeMap::new();
    for unit in units {
        // A build script never replays, so it holds no place in the
        // chain; its recording serves the build-script refusals only.
        if is_script_unit(&unit) {
            continue;
        }
        by_name.entry(unit.name.clone()).or_default().push(unit);
    }
    let mut crates: Vec<CrateRecord> = Vec::new();
    for (name, mut group) in by_name {
        group.sort_by(|a, b| a.stem.cmp(&b.stem));
        let mut edges = std::collections::BTreeSet::new();
        let mut sources: Vec<PathBuf> = Vec::new();
        for unit in &group {
            if referenced.contains(&unit.stem) {
                edges.extend(unit.record.extern_crates());
                sources.extend(unit_sources(&unit.record, &unit.stem));
            }
        }
        let at = group
            .iter()
            .position(|unit| referenced.contains(&unit.stem))
            .unwrap_or(0);
        let trusted = group.iter().any(|unit| referenced.contains(&unit.stem));
        let chosen = group.swap_remove(at);
        let sources = sources
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let ambiguous = units_of.get(&name).is_some_and(|units| *units > 1);
        crates.push(CrateRecord {
            edges: edges.into_iter().collect(),
            ambiguous,
            trusted,
            sources,
            name,
            proc_macro: chosen.proc_macro,
            runtime: false,
            root: chosen.root,
            record: chosen.record,
        });
    }
    // The objects a patch carries are the ones no procedural macro sits
    // between: the macro's own helpers serve its expansion only.
    let mut visited = std::collections::BTreeSet::from([tip_name.clone()]);
    let mut queue = vec![tip_name.clone()];
    while let Some(name) = queue.pop() {
        let edges = if name == tip_name {
            rustc.extern_crates()
        } else {
            crates
                .iter()
                .find(|record| record.name == name)
                .map(|record| record.edges.clone())
                .unwrap_or_default()
        };
        for (_, real) in edges {
            if visited.insert(real.clone())
                && let Some(record) = crates.iter_mut().find(|record| record.name == real)
                && !record.proc_macro
            {
                record.runtime = true;
                queue.push(real);
            }
        }
    }
    for record in &crates {
        if record.ambiguous {
            println!(
                "the build made {} as more than one unit; edits in {} will \
                 not patch this session",
                record.name, record.name
            );
        } else if !record.trusted {
            println!(
                "the recording of {} describes another build of it than the \
                 app links; edits in {} will not patch until the session \
                 restarts and cargo rebuilds it",
                record.name, record.name
            );
        }
    }
    let Some(crate_root) = rustc.last_arg(".rs") else {
        return Err(String::from("the recorded invocation names no crate root"));
    };
    // Cargo names the crate root relative to the working directory it runs
    // rustc in, which is the workspace root, not the directory this
    // session runs in.
    let root = Path::new(&crate_root);
    let source = if root.is_absolute() {
        root.to_path_buf()
    } else {
        Path::new(&rustc.cwd).join(root)
    };
    let tip_watch = source
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| String::from("the crate root has no directory"))?;
    let mut watch = vec![tip_watch];
    let tip_sources = unit_sources(&rustc, &rustc.artifact_stem().unwrap_or_default());
    for crate_record in &crates {
        let Some(dir) = crate_record.root.parent() else {
            return Err(format!(
                "the crate root of {} has no directory",
                crate_record.name
            ));
        };
        let dir = dir.to_path_buf();
        if !watch.contains(&dir) {
            watch.push(dir);
        }
    }
    watch.sort();
    let mut sources = tip_sources.clone();
    sources.extend(
        crates
            .iter()
            .flat_map(|record| record.sources.iter().cloned()),
    );
    // The inputs a build script compiles from are compile-time content
    // of the app as much as any library source is, so they join the
    // watched files whether a library reads them too or the script
    // alone does.
    sources.extend(
        build_scripts
            .iter()
            .flat_map(|script| script.inputs.iter().cloned()),
    );
    let watch_files = watch_files_of(&watch, &sources);
    // The sources that stand now are the ones whose going the scans
    // must still report: a walk answers the files that exist and the
    // name checks answer the times of paths that are gone, so the
    // bootstrap remembers which watched paths existed.
    let watched_present = sources
        .iter()
        .filter(|path| std::fs::metadata(path).is_ok())
        .cloned()
        .collect();
    // Only the build scripts of packages this build compiled can describe
    // this build, so a recording another session with other features left
    // behind must not refuse a patch of this one. The manifest directory
    // names the package, the tip's included, whatever layout its sources
    // sit in.
    let mut packages: Vec<PathBuf> = crates
        .iter()
        .filter(|record| record.trusted)
        .filter_map(|record| {
            record
                .record
                .env
                .get("CARGO_MANIFEST_DIR")
                .map(PathBuf::from)
        })
        .collect();
    if let Some(manifest_dir) = rustc.env.get("CARGO_MANIFEST_DIR") {
        packages.push(PathBuf::from(manifest_dir));
    }
    // A script recording this request's build wrote belongs to the
    // session even when its package is only a build dependency, which
    // the library graph of the app never names. The recording names its
    // request, so the scripts this request's own builds ran are told
    // from the ones other requests' feature selections left behind in
    // the same directory: a script one feature set compiled can read a
    // dependency the current one does not, whose movement would refuse
    // patches this session could rightly send. A recording without a
    // stamp, from before the field existed, falls back to its age
    // against this request's marker, and a stamped recording of another
    // request stays only when no recording of the same package matches
    // this one, this build's libraries name the package and the package
    // still carries a build script, which is all a cached build can
    // tell.
    let matched: std::collections::BTreeSet<PathBuf> = build_scripts
        .iter()
        .filter(|script| script.request.as_deref() == Some(request_stamp.as_str()))
        .map(|script| script.package.clone())
        .collect();
    build_scripts.retain(|script| {
        has_build_script(&script.package)
            && (script.request.as_deref() == Some(request_stamp.as_str())
                || (script.request.is_none() && script.fresh)
                || (!matched.contains(&script.package) && packages.contains(&script.package)))
    });
    // The script guards' baselines stand before the app starts, beside
    // the other watermarks: an edit a script's declared directory or
    // input takes during the connection would otherwise fold into the
    // baseline and stay unseen forever.
    let script_dirs = build_scripts
        .iter()
        .flat_map(|script| script.dirs.iter().cloned())
        .map(|dir| {
            let contents = dir_snapshot(&dir);
            (dir, contents)
        })
        .collect();
    let script_inputs_present = build_scripts
        .iter()
        .flat_map(|script| script.inputs.iter().chain(script.dirs.iter()).cloned())
        .filter(|input| std::fs::metadata(input).is_ok())
        .collect();
    // The watermarks of every watched path, the manifests and the
    // configuration files stand before the app starts: an edit the
    // watcher must answer lands anywhere after this moment, and one
    // captured beside the handshake would swallow an edit that landed
    // while the app connected, whose time the watermark would carry as
    // already seen.
    let source_times = observe(&watch, &watch_files);
    let manifests = manifest_states(app, &watch);
    let scripts_present = script_presence(app);
    let config_stamps: Vec<(PathBuf, u64)> = config_inputs(app)
        .into_iter()
        .map(|path| {
            let stamp = file_stamp(&path);
            (path, stamp)
        })
        .collect();
    let fat = record::RecordedLink::load(&dir.join("link-fat.json"))?;
    let exe = output_of(&fat)?;
    let linker = crate::roles::real_linker(&dir);
    let original = read_original(&dir, &exe, &fat, &target, sysroot_of(&rustc).as_deref())?;

    let socket = socket_path(&dir)?;
    let _ = record::remove_file(&socket);
    let listener = UnixListener::bind(&socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    let build = format!(
        "smudge-build-{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    );
    println!("starting {}", exe.display());
    let mut child = Command::new(&exe)
        .current_dir(app)
        .env(SOCKET_ENVIRONMENT, &socket)
        .env(BUILD_ENVIRONMENT, &build)
        .spawn()
        .map_err(|e| format!("{}: {e}", exe.display()))?;
    let accepted = match accept(&listener, &mut child) {
        Ok(accepted) => accepted,
        Err(error) => {
            // The app of a failed accept would otherwise keep running
            // with nobody connected to it.
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let stream = match accepted {
        Some(stream) => match handshake(stream, &mut child, &original, &build) {
            Ok(hand) => Some(hand),
            Err(error) => {
                // The app of a failed handshake would otherwise keep
                // running with nobody connected to it.
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        },
        None => {
            eprintln!(
                "no app connected within {:?}; the app must enable the \
                 hot-reload feature and start through run_view or run_view_with",
                CONNECT_TIMEOUT
            );
            None
        }
    };
    let (runtime_main, view) = match &stream {
        Some((_, hand)) => (hand.runtime_main, hand.view),
        None => (0, None),
    };
    Ok(Session {
        dir,
        rustc,
        crates,
        linker,
        original,
        child,
        stream: stream.map(|(stream, _)| stream),
        runtime_main,
        view,
        signature_refused: false,
        scripts_stale: false,
        script_dirs,
        script_inputs_present,
        watched_present,
        build_scripts,
        watch,
        watch_files,
        source_times,
        manifests,
        scripts_present,
        config_stamps,
        generation: 1,
        tip_root: source,
        tip_sources,
        live: std::collections::BTreeSet::new(),
        told_fresh: std::collections::BTreeSet::new(),
        pending: std::collections::BTreeSet::new(),
    })
}

/// The arguments one `cargo rustc` invocation needs, with the rustc
/// arguments the build cannot do without.
fn cargo_args(app: &Path, options: &Options) -> Vec<String> {
    let mut args = vec![String::from("rustc")];
    if let Some(package) = &options.package {
        args.push(String::from("-p"));
        args.push(package.clone());
    }
    match (&options.bin, &options.example) {
        (Some(bin), _) => {
            args.push(String::from("--bin"));
            args.push(bin.clone());
        }
        (None, Some(example)) => {
            args.push(String::from("--example"));
            args.push(example.clone());
        }
        // A package that builds more than its binary, such as one with a
        // library of its own, needs its bin named, or cargo rustc
        // refuses the ambiguity. The package the request names is the
        // one whose bin answers, which the current directory only
        // carries when no package was named.
        (None, None) => {
            if let Some(bin) = default_bin(app, options.package.as_deref()) {
                args.push(String::from("--bin"));
                args.push(bin);
            }
        }
    }
    for features in &options.features {
        args.push(String::from("--features"));
        args.push(features.clone());
    }
    if let Some(profile) = &options.profile {
        args.push(String::from("--profile"));
        args.push(profile.clone());
    }
    let me = std::env::current_exe()
        .map(|exe| exe.display().to_string())
        .unwrap_or_default();
    args.push(String::from("--"));
    args.push(String::from("-Csave-temps=true"));
    args.push(String::from("-Clink-dead-code"));
    args.push(format!("-Clinker={me}"));
    args
}

/// The one bin target of the package the request names, from cargo's
/// own answer about the workspace, when it has exactly one: the
/// package named in `selected`, or the nearest package manifest the
/// app directory or an ancestor of it carries, which is the package
/// cargo itself answers for from where the session runs. The selection
/// may be a package specification with a version, which names the same
/// package as its name alone.
fn default_bin(app: &Path, selected: Option<&str>) -> Option<String> {
    let metadata = workspace_metadata(app)?;
    let selected_name = selected.map(package_spec_name);
    let packages = metadata.get("packages")?.as_array()?;
    let package = if let Some(name) = selected_name {
        packages
            .iter()
            .find(|package| package.get("name").and_then(|name| name.as_str()) == Some(name))?
    } else {
        // The package the working directory belongs to is the nearest
        // ancestor that carries a manifest, since a session started in
        // a package's subdirectory belongs to that package the way
        // cargo answers for it, not to the first package in the list.
        let mut dir = std::fs::canonicalize(app).ok()?;
        loop {
            let found = packages.iter().find(|package| {
                package
                    .get("manifest_path")
                    .and_then(|path| path.as_str())
                    .map(|path| {
                        std::fs::canonicalize(path)
                            .ok()
                            .and_then(|path| path.parent().map(Path::to_path_buf))
                            .is_some_and(|parent| parent == dir)
                    })
                    .unwrap_or(false)
            });
            if let Some(package) = found {
                break package;
            }
            if !dir.pop() {
                return None;
            }
        }
    };
    let targets = package.get("targets")?.as_array()?;
    let bins: Vec<&str> = targets
        .iter()
        .filter(|target| {
            target
                .get("kind")
                .and_then(|kind| kind.as_array())
                .is_some_and(|kinds| kinds.iter().any(|kind| kind.as_str() == Some("bin")))
        })
        .filter_map(|target| target.get("name").and_then(|name| name.as_str()))
        .collect();
    if bins.len() == 1 {
        bins.first().map(|name| String::from(*name))
    } else {
        None
    }
}

/// The package name a `-p` specification carries, without a version
/// qualifier: `app@0.1.0` names `app`, as does `app:0.1.0`.
fn package_spec_name(spec: &str) -> &str {
    let end = spec.find(['@', ':']).unwrap_or(spec.len());
    &spec[..end]
}

/// The names of every package of the workspace `app` belongs to, from
/// cargo's own answer about it. These are the crates the wrapper
/// records, so a changed build request cleans them to keep the
/// recordings of the build it serves.
fn workspace_packages(app: &Path) -> Vec<String> {
    let Some(metadata) = workspace_metadata(app) else {
        return Vec::new();
    };
    metadata
        .get("packages")
        .and_then(|packages| packages.as_array())
        .map(|packages| {
            packages
                .iter()
                .filter_map(|package| {
                    package
                        .get("name")
                        .and_then(|name| name.as_str())
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Cargo's own answer about the workspace `app` belongs to, without its
/// dependencies.
fn workspace_metadata(app: &Path) -> Option<serde_json::Value> {
    let output = Command::new("cargo")
        .current_dir(app)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .ok()?;
    serde_json::from_slice(&output.stdout).ok()
}

/// The target directory cargo builds the workspace `app` belongs to
/// into, which `CARGO_TARGET_DIR` and the config's `build.target-dir`
/// decide and which a member borrows from its workspace root. Asking
/// cargo itself keeps the clean, the build, the linker shim and the
/// extraction in one directory.
fn target_dir_of(app: &Path) -> Result<PathBuf, String> {
    let metadata = workspace_metadata(app)
        .ok_or_else(|| String::from("cargo metadata could not answer for the app directory"))?;
    metadata
        .get("target_directory")
        .and_then(|value| value.as_str())
        .map(PathBuf::from)
        .ok_or_else(|| String::from("cargo metadata named no target directory"))
}

/// The workspace the app belongs to, whose directory cargo runs its
/// compiler invocations in, so a recording of this workspace carries it
/// as its working directory.
fn workspace_root_of(app: &Path) -> Option<PathBuf> {
    workspace_metadata(app)
        .and_then(|metadata| {
            metadata
                .get("workspace_root")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .map(PathBuf::from)
}

/// What the compiler reports about itself, which tells a toolchain
/// change apart from a cached rebuild the wrapper would never rerun:
/// cargo would reuse the old artifacts and the session would replay
/// the recordings of the build they came from.
fn rustc_identity(app: &Path, root: Option<&Path>) -> String {
    // Cargo picks the compiler from RUSTC, then from CARGO_BUILD_RUSTC
    // and the config files' `build.rustc`, then from the search path;
    // whichever it picks, a request recorded for one compiler must
    // never serve another, so the identity carries the path it is
    // invoked by, the file that path names, and what that invocation
    // says about itself. A relative config path stands for where the
    // config file was found, which cargo resolves it against, and a
    // program may choose its behavior by the name it is called by, so
    // the version comes from the invocation path, not the file behind
    // it.
    let selected = std::env::var_os("RUSTC")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("CARGO_BUILD_RUSTC").map(PathBuf::from))
        .or_else(|| {
            config_rustc(app).map(|(value, base)| {
                let value = PathBuf::from(value);
                // A bare name is a search-path lookup for cargo, not a
                // file beside the config, so only a relative path with
                // a separator joins the config's base.
                if value.is_absolute() || !value.to_string_lossy().contains('/') {
                    value
                } else {
                    base.join(value)
                }
            })
        })
        .unwrap_or_else(|| PathBuf::from("rustc"));
    let search = effective_search_path(app);
    let (invocation, resolved) = resolve_program(app, root, &selected, &search);
    let mut identity = format!("rustc {} -> {}", invocation.display(), resolved.display());
    if let Ok(out) = Command::new(&invocation)
        .arg("-vV")
        .env("PATH", &search)
        .output()
    {
        identity.push(' ');
        identity.push_str(String::from_utf8_lossy(&out.stdout).trim());
    }
    identity
}

/// One stable name of a build request, which names the file its marker
/// lives in. A hash of the process's own making would do for one run
/// and betray the next, so the rounds mix the bytes the plain way.
fn request_key(request: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in request.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// One stable stamp of a file's content, mixed the way the request key
/// mixes its bytes. The file's presence joins the mix before its bytes,
/// so a file that appears or vanishes moves the stamp even beside one
/// that answers empty, and a file that cannot be read answers as if
/// empty.
fn file_stamp(path: &Path) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let present = std::fs::metadata(path).is_ok();
    hash ^= u64::from(present);
    hash = hash.wrapping_mul(0x0100_0000_01b3);
    for byte in std::fs::read(path).unwrap_or_default() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// The environment variables whose values reach the build the request
/// names: the flags under their several names, the compiler and the
/// bootstrap switch, the target and the profile overrides, and the
/// incremental switch. The scheduling, presentation and networking
/// variables stay out, which a change of theirs must not rebuild the
/// workspace over.
fn build_variable(key: &str) -> bool {
    key.contains("RUSTFLAGS")
        || matches!(
            key,
            "RUSTC" | "RUSTC_BOOTSTRAP" | "CARGO_INCREMENTAL" | "CARGO_HOME"
        )
        || key.starts_with("CARGO_PROFILE_")
        || (key.starts_with("CARGO_BUILD_") && key != "CARGO_BUILD_JOBS")
}

/// The files whose content joins the build request: the workspace's
/// manifest and lock, every package manifest cargo's own answer names,
/// and the configuration files cargo reads, each directory's legacy
/// `config` counting over its `config.toml` when both stand.
fn request_inputs(app: &Path) -> Vec<PathBuf> {
    let mut inputs = std::collections::BTreeSet::new();
    let root = workspace_root_of(app).unwrap_or_else(|| app.to_path_buf());
    inputs.insert(root.join("Cargo.toml"));
    inputs.insert(root.join("Cargo.lock"));
    if let Some(packages) = workspace_metadata(app)
        .and_then(|metadata| metadata.get("packages").cloned())
        .and_then(|packages| packages.as_array().cloned())
    {
        for package in packages {
            if let Some(manifest) = package
                .get("manifest_path")
                .and_then(|value| value.as_str())
            {
                inputs.insert(PathBuf::from(manifest));
            }
        }
    }
    for (dir, _) in config_dirs(app) {
        inputs.extend(config_inputs_of(&dir));
    }
    inputs.into_iter().collect()
}

/// The configuration files one `.cargo` directory keeps, both spellings
/// of the name, since a session can meet either: cargo reads the legacy
/// `config` over the `config.toml` when both stand, but a file that
/// appears under either name mid-session changes what the build reads.
fn config_inputs_of(dir: &Path) -> [PathBuf; 2] {
    [dir.join("config"), dir.join("config.toml")]
}

/// Every configuration file cargo reads for `app`, both spellings per
/// directory, whose stamps a session watches for restarts.
fn config_inputs(app: &Path) -> Vec<PathBuf> {
    let mut inputs = std::collections::BTreeSet::new();
    for (dir, _) in config_dirs(app) {
        inputs.extend(config_inputs_of(&dir));
    }
    inputs.into_iter().collect()
}

/// The compiler the cargo config files select, `build.rustc`, read from
/// every `.cargo` configuration directory from the app directory up,
/// and from the cargo home last, which the environment names or the
/// user's home directory keeps. Each directory's legacy `config` file
/// counts over its `config.toml` when both stand, the way cargo reads
/// them, and only the file that wins is read. Answers the value and the
/// directory a relative path in it resolves against: the directory the
/// config's `.cargo` sits in, or the cargo home's parent. Answers none
/// when every file is missing or silent.
fn config_rustc(app: &Path) -> Option<(String, PathBuf)> {
    for (dir, base) in config_dirs(app) {
        let legacy = dir.join("config");
        let path = if legacy.exists() {
            legacy
        } else {
            dir.join("config.toml")
        };
        if let Some(value) = config_string(&path, "build", "rustc") {
            return Some((value, base));
        }
    }
    None
}

/// The `.cargo` configuration directories for `app`, nearest first, each
/// with the directory a relative path inside it resolves against, and
/// the cargo home last. A relative path in the home's configuration
/// resolves against the home's parent, the way cargo reads them, and a
/// relative `CARGO_HOME` itself stands against the working directory.
fn config_dirs(app: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut dirs: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut ancestor = app.to_path_buf();
    loop {
        dirs.push((ancestor.join(".cargo"), ancestor.clone()));
        if !ancestor.pop() {
            break;
        }
    }
    let home = match std::env::var_os("CARGO_HOME") {
        Some(home) => {
            let home = PathBuf::from(home);
            if home.is_absolute() {
                home
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(home)
            }
        }
        None => match std::env::home_dir() {
            Some(home) => home.join(".cargo"),
            None => return dirs,
        },
    };
    let base = home.parent().unwrap_or(&home).to_path_buf();
    dirs.push((home, base));
    dirs
}

/// The search path cargo's own `[env]` table gives the processes it
/// runs, which a bare compiler name resolves against at its exec: an
/// `[env.PATH]` table's `value`, whose `relative` joins the
/// configuration's `.cargo` parent and whose `force` overrides the
/// environment's own PATH, or a plain `PATH` key under `[env]`, which
/// cargo applies only while the variable is unset. The tables cargo
/// merges across its configuration files merge here too, each field
/// the nearest file sets winning over the parents' answer, and the
/// value's own file deciding what a relative entry resolves against.
fn effective_search_path(app: &Path) -> std::ffi::OsString {
    let mut value: Option<(String, PathBuf)> = None;
    let mut force: Option<bool> = None;
    let mut relative: Option<bool> = None;
    for (dir, base) in config_dirs(app) {
        let legacy = dir.join("config");
        let path = if legacy.exists() {
            legacy
        } else {
            dir.join("config.toml")
        };
        let Some(found) = config_env_path(&path) else {
            continue;
        };
        if value.is_none() {
            value = found.value.map(|named| (named, base.clone()));
        }
        if force.is_none() {
            force = found.force;
        }
        if relative.is_none() {
            relative = found.relative;
        }
    }
    let set = std::env::var_os("PATH");
    if !(force == Some(true) || set.is_none()) {
        return set.unwrap_or_default();
    }
    let Some((named, base)) = value else {
        return set.unwrap_or_default();
    };
    if relative == Some(true) {
        base.join(named).into_os_string()
    } else {
        named.into()
    }
}

/// The `PATH` one configuration file sets for cargo's child processes,
/// with each field kept apart for the merge across files: the
/// `[env.PATH]` table's `value`, `force` and `relative`, and a bare
/// `PATH` key under `[env]`, which fills the value alone. Answers none
/// when the file sets no PATH at all.
#[derive(Default)]
struct EnvPathConfig {
    value: Option<String>,
    force: Option<bool>,
    relative: Option<bool>,
}

fn config_env_path(path: &Path) -> Option<EnvPathConfig> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut found = EnvPathConfig::default();
    let mut inside: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = strip_comment(line).trim();
        if let Some(header) = line.strip_prefix('[') {
            inside = dotted_parts(header.trim_end_matches(']').trim());
            continue;
        }
        let Some((name, line)) = line.split_once('=') else {
            continue;
        };
        let mut full = inside.clone();
        full.extend(dotted_parts(name.trim()));
        let names: Vec<&str> = full.iter().map(String::as_str).collect();
        let line = line.trim();
        let quoted = line.starts_with('"') || line.starts_with('\'');
        match names.as_slice() {
            ["env", "PATH", "value"] if quoted => found.value = toml_string(line),
            ["env", "PATH", "force"] => found.force = Some(line == "true"),
            ["env", "PATH", "relative"] => found.relative = Some(line == "true"),
            ["env", "PATH"] if quoted => found.value = toml_string(line),
            _ => {}
        }
    }
    (found.value.is_some() || found.force.is_some() || found.relative.is_some()).then_some(found)
}

/// The string one TOML file gives the complete `table.key` path, which
/// reads cargo's own settings: a header opens its table, and a key's
/// own dots name subtables of the table it stands in, the way TOML
/// scopes them, so `build . rustc` at the top level and `rustc` under
/// `[build]` both read, while `build.rustc` under `[build]` names
/// `build.build.rustc`, which cargo leaves unused. A value ends at an
/// unquoted `#` or the line's end, and only a quoted one answers, since
/// cargo takes its string settings from quoted values alone.
fn config_string(path: &Path, table: &str, key: &str) -> Option<String> {
    let mut wanted = dotted_parts(table);
    wanted.push(key.to_string());
    config_value(path, &wanted, true)
}

/// The boolean one TOML file gives the complete `table.key` path: a
/// bare token answers as it stands, since the booleans cargo reads are
/// never quoted.
fn config_bool(path: &Path, table: &str, key: &str) -> Option<bool> {
    let mut wanted = dotted_parts(table);
    wanted.push(key.to_string());
    config_value(path, &wanted, false).map(|value| value == "true")
}

/// The value one TOML file gives the complete path `wanted`, quoted
/// strings decoded and bare tokens carried as they are written; with
/// `quoted` only the quoted ones answer.
fn config_value(path: &Path, wanted: &[String], quoted: bool) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut inside: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = strip_comment(line).trim();
        if let Some(header) = line.strip_prefix('[') {
            inside = dotted_parts(header.trim_end_matches(']').trim());
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let mut full = inside.clone();
        full.extend(dotted_parts(name.trim()));
        if full != wanted {
            continue;
        }
        let value = value.trim();
        let is_quoted = value.starts_with('"') || value.starts_with('\'');
        if quoted && !is_quoted {
            continue;
        }
        return if is_quoted {
            toml_string(value)
        } else {
            Some(value.to_string())
        };
    }
    None
}

/// The parts a dotted key or header names, with the whitespace a
/// component may carry around its separators taken out, so
/// `build . rustc` reads as `build.rustc`, and a quoted component
/// decoded the way TOML decodes it, so `"build" = "x"` names `build`
/// like a bare one does. A separator inside quotes does not split.
fn dotted_parts(name: &str) -> Vec<String> {
    let mut raw: Vec<&str> = Vec::new();
    let mut start = None;
    let mut quote = None;
    let mut escaped = false;
    for (at, ch) in name.char_indices() {
        if let Some(open) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' && open == '"' {
                escaped = true;
            } else if ch == open {
                quote = None;
            }
        } else if ch == '.' {
            raw.push(&name[start.unwrap_or(0)..at]);
            start = None;
        } else if start.is_none() && !ch.is_whitespace() {
            start = Some(at);
            if ch == '"' || ch == '\'' {
                quote = Some(ch);
            }
        }
    }
    raw.push(&name[start.unwrap_or(name.len())..]);
    raw.into_iter()
        .map(|part| match part.trim() {
            quoted if quoted.starts_with('"') || quoted.starts_with('\'') => {
                toml_string(quoted).unwrap_or_else(|| quoted.to_string())
            }
            bare => bare.to_string(),
        })
        .collect()
}

/// The TOML string `value` carries: a basic string in double quotes,
/// whose escapes are undone, or a literal string in single quotes,
/// whose characters stand as they are written.
fn toml_string(value: &str) -> Option<String> {
    if value.starts_with('\'') {
        return value
            .strip_prefix('\'')?
            .strip_suffix('\'')
            .map(String::from);
    }
    basic_string(value)
}

/// The line without a trailing comment, which a `#` outside either
/// kind of string starts.
fn strip_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (at, ch) in line.char_indices() {
        if let Some(open) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' && open == '"' {
                escaped = true;
            } else if ch == open {
                quote = None;
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch == '#' {
            return &line[..at];
        }
    }
    line
}

/// The basic string `value` carries, with the escapes cargo's TOML
/// accepts undone: the one-letter controls and the escape character,
/// the quote and the backslash, the hexadecimal escapes of two hex
/// digits, and the Unicode escapes of four and eight. Any other escape
/// answers nothing, which the file as a whole would not survive under
/// cargo either.
fn basic_string(value: &str) -> Option<String> {
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next()? {
            'b' => out.push('\u{0008}'),
            't' => out.push('\t'),
            'n' => out.push('\n'),
            'f' => out.push('\u{000c}'),
            'r' => out.push('\r'),
            'e' => out.push('\u{001b}'),
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            'x' => out.push(hex_escape(&mut chars, 2)?),
            'u' => out.push(hex_escape(&mut chars, 4)?),
            'U' => out.push(hex_escape(&mut chars, 8)?),
            _ => return None,
        }
    }
    Some(out)
}

/// The character one escape of `width` hex digits carries.
fn hex_escape(chars: &mut impl Iterator<Item = char>, width: usize) -> Option<char> {
    let mut value: u32 = 0;
    for _ in 0..width {
        value = value * 16 + chars.next()?.to_digit(16)?;
    }
    char::from_u32(value)
}

/// Where the selected compiler program lives, resolved the way cargo
/// runs it: a path stands relative to the app directory, which is cargo's
/// own starting directory, and a bare name resolves through the search
/// path `search` names, whose relative entries the compiler invocation's
/// working directory, the workspace root, decides, so two directories
/// that carry different compilers under the same name tell apart. Answers
/// the invocation path cargo would use and the file that path names, kept
/// apart because a program may choose its behavior by the name it is
/// called by, which canonicalizing alone would erase.
fn resolve_program(
    app: &Path,
    root: Option<&Path>,
    program: &Path,
    search: &std::ffi::OsStr,
) -> (PathBuf, PathBuf) {
    let spelled = program.to_string_lossy();
    if spelled.contains('/') {
        let joined = if program.is_absolute() {
            program.to_path_buf()
        } else {
            app.join(program)
        };
        let named = std::fs::canonicalize(&joined).unwrap_or_else(|_| joined.clone());
        return (joined, named);
    }
    let base = root.unwrap_or(app).to_path_buf();
    for dir in std::env::split_paths(search) {
        let candidate = if dir.is_absolute() {
            dir.join(program)
        } else {
            base.join(dir).join(program)
        };
        let runs = std::fs::metadata(&candidate)
            .map(|meta| meta.is_file() && executable(&candidate))
            .unwrap_or(false);
        if runs {
            let named = std::fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
            return (candidate, named);
        }
    }
    let bare = program.to_path_buf();
    (bare.clone(), bare)
}

/// Whether this process can execute `path`, the check the search-path
/// lookup itself makes: the execute permission of the class that owns
/// the file decides, not the presence of any execute bit at all.
fn executable(path: &Path) -> bool {
    let Ok(name) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    // SAFETY: `access` reads the name and touches no memory the caller
    // keeps, and `name` outlives the call.
    unsafe { libc::access(name.as_ptr(), libc::X_OK) == 0 }
}

/// The executable a fat link produced, from its `-o` argument.
fn output_of(fat: &[String]) -> Result<PathBuf, String> {
    let at = fat
        .iter()
        .position(|arg| arg == "-o")
        .ok_or_else(|| String::from("the recorded link names no output"))?;
    let exe = fat
        .get(at + 1)
        .map(PathBuf::from)
        .ok_or_else(|| String::from("the recorded link names no output path"))?;
    exe.canonicalize()
        .map_err(|e| format!("{}: {e}", exe.display()))
}

/// Reads the original binary, using the cache when the binary has not
/// changed since the last session.
fn read_original(
    dir: &Path,
    exe: &Path,
    fat: &[String],
    target: &Path,
    sysroot: Option<&Path>,
) -> Result<Original, String> {
    let cache = dir.join("cache");
    std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let bytes = std::fs::read(exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    let symbols = match cached_symbols(&cache, exe, &bytes) {
        Some(symbols) => symbols,
        None => {
            let extracted = dir.join("extracted");
            let _ = std::fs::remove_dir_all(&extracted);
            std::fs::create_dir_all(&extracted)
                .map_err(|e| format!("{}: {e}", extracted.display()))?;
            elf::extract_rlib_objects(fat, target, &extracted)?;
            let symbols = Symbols::read(&bytes)?;
            write_symbols(&cache, exe, &bytes, &symbols)?;
            symbols
        }
    };
    let signatures = Signatures::read(&bytes)?;
    let main = symbols.main_address()?;
    let sysroot_tls = sysroot
        .map(|sysroot| elf::sysroot_symbols(fat, sysroot))
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .filter(|name| {
            symbols
                .get(name)
                .is_some_and(|sym| sym.kind == object::SymbolKind::Tls)
        })
        .collect();
    Ok(Original {
        symbols,
        signatures,
        main,
        tls: elf::tdata_template(&bytes),
        sysroot_tls,
    })
}

/// The sysroot the recorded compiler builds against: the one a
/// `--sysroot` flag names, or the one the compiler reports. A compiler that
/// cannot answer gives none, and then every thread-local stays refused.
fn sysroot_of(rustc: &Recorded) -> Option<PathBuf> {
    if let Some(named) = record::values_in(&rustc.args, "--sysroot").next() {
        return Some(rustc.cwd.join(named));
    }
    let named = rustc.args.first().and_then(|program| {
        Command::new(program)
            .args(["--print", "sysroot"])
            .current_dir(&rustc.cwd)
            .env_clear()
            .envs(rustc.env.clone())
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
            .filter(|sysroot| sysroot.is_absolute())
    });
    if named.is_none() {
        eprintln!(
            "cargo smudge: the compiler did not name its sysroot, so every \
             patch that uses a thread-local of the standard library is refused"
        );
    }
    named
}

/// One cached symbol, with the kind reduced to what the builder reads.
#[derive(Serialize, Deserialize)]
struct CachedSym {
    /// The symbol's name.
    name: String,
    /// Its address in the binary.
    address: u64,
    /// Its size.
    size: u64,
    /// Its kind, as [`kind_code`] encodes it.
    kind: u8,
    /// Whether the binary defines it.
    defined: bool,
    /// Whether its binding is weak.
    weak: bool,
    /// Whether the symbol lives in a section the program writes.
    writable: bool,
}

/// The marker a cache is valid for: which binary it was read from, and
/// which shape of the cached symbols it holds.
#[derive(Serialize, Deserialize, PartialEq)]
struct Marker {
    /// The cache shape, bumped when what is cached changes.
    shape: u32,
    /// The binary's path.
    exe: String,
    /// Its modification time in nanoseconds.
    modified: u128,
    /// Its size in bytes.
    size: u64,
}

/// Maps a symbol kind onto a small code, for the cache.
fn kind_code(kind: object::SymbolKind) -> u8 {
    match kind {
        object::SymbolKind::Text => 1,
        object::SymbolKind::Data => 2,
        object::SymbolKind::Tls => 3,
        _ => 0,
    }
}

/// Maps a cache code back onto a symbol kind.
fn kind_of(code: u8) -> object::SymbolKind {
    match code {
        1 => object::SymbolKind::Text,
        2 => object::SymbolKind::Data,
        3 => object::SymbolKind::Tls,
        _ => object::SymbolKind::Unknown,
    }
}

/// The symbols in the cache, when the cache was read from this binary.
fn cached_symbols(cache: &Path, exe: &Path, bytes: &[u8]) -> Option<Symbols> {
    let marker = Marker {
        shape: CACHE_SHAPE,
        exe: exe.display().to_string(),
        modified: modified_nanos(exe)?,
        size: bytes.len() as u64,
    };
    let saved: Marker =
        serde_json::from_str(&std::fs::read_to_string(cache.join("marker.json")).ok()?).ok()?;
    if saved != marker {
        return None;
    }
    let cached: Vec<CachedSym> =
        serde_json::from_str(&std::fs::read_to_string(cache.join("symbols.json")).ok()?).ok()?;
    let mut map = std::collections::BTreeMap::new();
    for entry in cached {
        map.insert(
            entry.name.clone(),
            elf::Sym {
                address: entry.address,
                size: entry.size,
                kind: kind_of(entry.kind),
                defined: entry.defined,
                weak: entry.weak,
                writable: entry.writable,
            },
        );
    }
    println!("reusing the cached symbol table of {}", exe.display());
    Some(Symbols { map })
}

/// Writes the symbols and their marker into the cache.
///
/// # Errors
///
/// Returns why the cache could not be written, which keeps the session
/// working without one.
fn write_symbols(cache: &Path, exe: &Path, bytes: &[u8], symbols: &Symbols) -> Result<(), String> {
    let cached: Vec<CachedSym> = symbols
        .map
        .iter()
        .map(|(name, sym)| CachedSym {
            name: name.clone(),
            address: sym.address,
            size: sym.size,
            kind: kind_code(sym.kind),
            defined: sym.defined,
            weak: sym.weak,
            writable: sym.writable,
        })
        .collect();
    let json = serde_json::to_string(&cached).map_err(|e| e.to_string())?;
    record::write_bytes(json.as_bytes(), &cache.join("symbols.json"))?;
    let marker = Marker {
        shape: CACHE_SHAPE,
        exe: exe.display().to_string(),
        modified: modified_nanos(exe).unwrap_or(0),
        size: bytes.len() as u64,
    };
    let json = serde_json::to_string(&marker).map_err(|e| e.to_string())?;
    record::write_bytes(json.as_bytes(), &cache.join("marker.json"))
}

/// The modification time of `path` in nanoseconds, when it can be read.
fn modified_nanos(path: &Path) -> Option<u128> {
    path.metadata()
        .and_then(|meta| meta.modified())
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|age| age.as_nanos())
}

/// The time `nanos` since the epoch names, or now when it cannot be built.
fn time_of(nanos: u128) -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_nanos(nanos.min(u64::MAX as u128) as u64))
        .unwrap_or_else(SystemTime::now)
}

/// How long `started` is ago, or zero when the clock cannot tell.
fn elapsed(started: SystemTime) -> Duration {
    started.elapsed().unwrap_or_default()
}

/// The socket path under `dir`, or under the runtime directory when the
/// path would outgrow the socket limit.
fn socket_path(dir: &Path) -> Result<PathBuf, String> {
    let path = dir.join("hot.sock");
    if path.as_os_str().as_encoded_bytes().len() < SOCKET_PATH_LIMIT {
        return Ok(path);
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| {
            format!(
                "{} is longer than the socket path limit of {} bytes and no \
                 XDG_RUNTIME_DIR names a shorter place",
                path.display(),
                SOCKET_PATH_LIMIT - 1
            )
        })?;
    Ok(runtime.join(format!("smudge-hot-{}.sock", std::process::id())))
}

/// Waits for the app to connect, answering its stream when it does and
/// `None` once the timeout passes or the app exits first.
fn accept(listener: &UnixListener, child: &mut Child) -> Result<Option<UnixStream>, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let started = SystemTime::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).map_err(|e| e.to_string())?;
                return Ok(Some(stream));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.to_string()),
        }
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Ok(None);
        }
        if elapsed(started) > CONNECT_TIMEOUT {
            return Ok(None);
        }
        std::thread::sleep(POLL);
    }
}

/// What one handshake produced: the addresses the app announced.
struct Handshake {
    /// The address `main` runs at, from the hello.
    runtime_main: u64,
    /// The linked address of the view, when the hello carried it.
    view: Option<u64>,
}

/// How long the app may take to answer one frame: its hello and its
/// acknowledgement of a patch. A slower app than this is treated as one
/// that failed the step, so the session keeps taking commands.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

/// Reads the hello, checks it names the app this session started and
/// answers what the builder needs from it.
fn handshake(
    mut stream: UnixStream,
    child: &mut Child,
    original: &Original,
    build: &str,
) -> Result<(UnixStream, Handshake), String> {
    stream
        .set_read_timeout(Some(ANSWER_TIMEOUT))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(ANSWER_TIMEOUT))
        .map_err(|e| e.to_string())?;
    let (kind, hello) = wire::read_frame(&mut stream)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| String::from("the app closed the socket before its hello"))?;
    if kind != HELLO {
        return Err(format!(
            "the app's first frame was of kind {kind}, not a hello"
        ));
    }
    let err = |why: &str| format!("the app's hello is broken: {why}");
    let version = u32::from_le_bytes(
        hello
            .first_chunk::<4>()
            .ok_or_else(|| err("it names no version"))?
            .to_owned(),
    );
    let pid = u32::from_le_bytes(
        hello
            .get(4..8)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| err("it names no process"))?,
    );
    let runtime_main = u64::from_le_bytes(
        hello
            .get(8..16)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| err("it names no address"))?,
    );
    if pid != child.id() {
        return Err(format!(
            "the app that connected runs as pid {pid}, but this session \
             started pid {}",
            child.id()
        ));
    }
    let (view, announced) = match version {
        2 => {
            let runtime_view = u64::from_le_bytes(
                hello
                    .get(16..24)
                    .and_then(|bytes| bytes.try_into().ok())
                    .ok_or_else(|| err("version 2 carries no view address"))?,
            );
            // The view may sit below `main` in the binary, so the rebase
            // goes through signed arithmetic.
            let view = (i128::from(runtime_view)
                .checked_sub(i128::from(runtime_main))
                .and_then(|offset| offset.checked_add(i128::from(original.main))))
            .and_then(|view| u64::try_from(view).ok())
            .ok_or_else(|| err("it names addresses the session cannot rebase"))?;
            (Some(view), hello.get(24..).unwrap_or_default())
        }
        1 => (None, hello.get(16..).unwrap_or_default()),
        other => return Err(err(&format!("it names the unknown version {other}"))),
    };
    if announced != build.as_bytes() {
        return Err(format!(
            "the app announced the build {:?}, but this session started the \
             build {build:?}",
            String::from_utf8_lossy(announced)
        ));
    }
    println!("app running as pid {}", child.id());
    Ok((stream, Handshake { runtime_main, view }))
}

/// Watches the sources and patches the app on every save, until the app
/// exits, the inputs change or the user asks to quit.
fn supervise(session: &mut Session, commands: &mpsc::Receiver<String>) -> Control {
    loop {
        match session.child.try_wait() {
            Ok(Some(_)) => {
                println!("the app exited");
                return Control::Exited;
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!("the app could not be polled: {error}");
                return Control::Exited;
            }
        }
        match commands.try_recv() {
            Ok(command) if command == "quit" => {
                let _ = session.child.kill();
                let _ = session.child.wait();
                return Control::Quit;
            }
            Ok(_) => {
                println!("rebuilding and restarting the app");
                let _ = session.child.kill();
                let _ = session.child.wait();
                return Control::Restart;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {}
        }
        if !manifests_unchanged(session)
            || !configs_unchanged(session)
            || !scripts_unchanged(session)
        {
            println!("the build inputs changed; restarting the build");
            let _ = session.child.kill();
            let _ = session.child.wait();
            return Control::Restart;
        }
        note_script_input_moves(session);
        // The losses are caught once and ride both scans: a file that
        // is gone leaves the remembered set at the first scan, and the
        // settle's second scan must not forget what the first saw.
        let gone = vanished_files(session);
        let (mut fresh, _) = scan_sources(&session.watch, &session.source_times);
        let (file_fresh, _) = scan_files(&session.watch_files, &session.source_times);
        fresh.extend(file_fresh);
        fresh.extend(gone.iter().cloned());
        fresh.sort();
        if !fresh.is_empty() {
            // An editor writes a save in steps, so the last state wins,
            // and saves that landed in other crates of the same window
            // ride along in the same patch. Both scans compare against
            // the state the last patch left behind, so the settle's
            // second scan still answers what the first saw.
            std::thread::sleep(Duration::from_millis(150));
            let (mut fresh, sources_now) = scan_sources(&session.watch, &session.source_times);
            let (file_fresh, files_now) = scan_files(&session.watch_files, &session.source_times);
            fresh.extend(file_fresh);
            fresh.extend(gone);
            fresh.sort();
            fresh.dedup_by(|a, b| a.1 == b.1);
            // Each scan answers only the paths it visited, so both
            // observation sets land on the remembered states and the
            // entries of paths the other scan reached survive.
            for (path, time) in sources_now {
                session.source_times.insert(path, time);
            }
            for (path, time) in files_now {
                session.source_times.insert(path, time);
            }
            // The guards re-read after the settle: an input or a
            // declared directory that moved while the first scan
            // settled must refuse this attempt, not only the next one.
            note_script_input_moves(session);
            let mut changed: Vec<String> = Vec::new();
            for (_, path) in &fresh {
                for record in owners_of(
                    &session.crates,
                    &session.tip_root,
                    &session.tip_sources,
                    path,
                ) {
                    if !changed.contains(&record.name) {
                        changed.push(record.name.clone());
                    }
                }
            }
            apply_patch(session, &changed, &fresh);
        }
        std::thread::sleep(POLL);
    }
}

/// Runs one patch step and reports what it ended with. `changed` names
/// the library crates the watched edits landed in, empty for an edit in
/// the app crate itself.
fn apply_patch(session: &mut Session, changed: &[String], saved: &[(SystemTime, PathBuf)]) {
    // Every attempt counts, so a refusal or a failure still advances the
    // numbering and the next save patches under the next number.
    let generation = session.generation;
    session.generation += 1;
    // The edits of this attempt are taken over before anything can skip
    // or refuse them, so no path forgets what the seen time consumed.
    session.pending.extend(changed.iter().cloned());
    // A build script whose input moved this attempt cannot rerun, and
    // the refusal must outlive the attempt: a later save that only
    // touches the app must not consume the event and replay libraries
    // against the expansion the script's last run generated. The scan
    // runs before any early return can take the attempt apart.
    if script_input_saved(&session.build_scripts, saved) {
        session.scripts_stale = true;
    }
    let Some(stream) = session.stream.as_mut() else {
        println!("patch {generation}: skipped, because the app is not connected");
        return;
    };
    let tip_name = session.rustc.crate_name().unwrap_or_default().to_string();
    // An edit in a crate that shares the app crate's name, in one whose
    // recording describes another build, or in one the build made as two
    // units, can never patch this session. A refusal of one of this
    // attempt's own edits takes the whole attempt over into pending; a
    // leftover of an earlier attempt retires itself, so it cannot
    // refuse every later attempt forever.
    for name in changed {
        if let Some(reason) = refusal_of(&session.crates, &tip_name, name) {
            println!("patch {generation}: not sent; {reason}");
            session.pending.remove(name);
            return;
        }
    }
    for name in session.pending.clone() {
        if changed.contains(&name) {
            continue;
        }
        if let Some(reason) = refusal_of(&session.crates, &tip_name, &name) {
            println!("patch {generation}: {name} stays unpatched; {reason}");
            session.pending.remove(&name);
        }
    }
    // A patch must carry every library edit the app runs with: the ones
    // of this attempt, the ones earlier patches installed, and the ones
    // an earlier attempt took over without installing.
    let mut names: Vec<String> = changed.to_vec();
    for name in session.live.iter().chain(session.pending.iter()) {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    let references: Vec<&str> = names.iter().map(String::as_str).collect();
    let steps = match chain_of(&session.crates, (&tip_name, &session.rustc), &references) {
        Ok(steps) => steps,
        Err(error) => {
            if !error.preserve {
                session.pending.remove(&error.name);
            }
            println!("patch {generation}: not sent; {}", error.reason);
            return;
        }
    };
    // A build script reads its dependencies and its own sources while
    // cargo runs it, and a patch replays a crate against the files that
    // script generated, so a chain that replays a crate the script reads,
    // or an edit to a file the script compiles from, would leave those
    // files and that expansion old. The session cannot rerun the script,
    // so the whole attempt refuses and its edits stay pending for the
    // next session; once stale, it stays stale to the session's end.
    let replays: Vec<&str> = steps.iter().map(|step| step.name.as_str()).collect();
    let stale = session.scripts_stale
        || script_input_saved(&session.build_scripts, saved)
        || script_refusal(&session.build_scripts, &replays, saved).is_some();
    if stale {
        let reason = script_refusal(&session.build_scripts, &replays, saved).unwrap_or_else(|| {
            let script = session
                .build_scripts
                .first()
                .map(|script| {
                    script
                        .package
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("a package")
                        .to_string()
                })
                .unwrap_or_else(|| String::from("a build"));
            format!(
                "the build script of {script} runs against inputs that \
                     moved, which a patch cannot rerun; restart the session"
            )
        });
        println!("patch {generation}: not sent; {reason}");
        return;
    }
    let mut plan = Plan {
        chain: &steps,
        dir: &session.dir,
        symbols: &session.original.symbols,
        signatures: &mut session.original.signatures,
        main: session.original.main,
        runtime_main: session.runtime_main,
        view: session.view,
        signature_refused: &mut session.signature_refused,
        linker: &session.linker,
        tls: &session.original.tls,
        sysroot_tls: &session.original.sysroot_tls,
        stream,
    };
    let outcome = patch::run(&mut plan, generation);
    match outcome {
        Outcome::Applied {
            compile,
            total,
            entries,
            added,
            changed_signature,
            renumbered,
            fresh_statics,
            unverified,
            trampolines,
            absolutes,
            tls_copies,
            missing,
            replayed,
            discovered,
        } => {
            let mut line = format!(
                "patch {generation}: applied; compile {compile:?}, total {total:?}, \
                 {entries} entries, {added} new, {changed_signature} omitted for a \
                 changed signature, {renumbered} omitted for renumbered closures, \
                 {unverified} unverifiable, {trampolines} \
                 trampolines, {absolutes} absolutes, {tls_copies} tls, {missing} missing"
            );
            if !replayed.is_empty() {
                line.push_str(&format!(", replayed {}", replayed.join(", ")));
            }
            println!("{line}");
            let untold: Vec<String> = fresh_statics
                .into_iter()
                .filter(|(symbol, _)| session.told_fresh.insert(symbol.clone()))
                .map(|(_, shown)| shown)
                .collect();
            if !untold.is_empty() {
                println!(
                    "note: the patched code starts fresh copies of the app's statics {}, \
                     which only a restart keeps the state of",
                    untold.join(", ")
                );
            }
            session.live.extend(replayed);
            // What the replays learned about their sources holds for the
            // rest of the session, an app restart included: the app
            // crate's own sources answer for it, every library's for its
            // crate.
            learn(session, tip_name, discovered, true);
            // Everything the patch carried is installed now.
            session.pending.clear();
        }
        Outcome::CompileFailed { log, discovered } => {
            println!("patch {generation}: compile failed; the app keeps running");
            eprintln!("{}", tail(&log, 30));
            // A compile that failed may have listed less than a full
            // dep-info, so its discoveries only ever join the membership,
            // never replace it.
            learn(session, tip_name, discovered, false);
        }
        Outcome::BuildFailed { reason, discovered } => {
            println!("patch {generation}: not built; the app keeps running");
            eprintln!("{reason}");
            // A build that failed may have learned from only part of the
            // chain, so its discoveries only ever join the membership.
            learn(session, tip_name, discovered, false);
        }
        Outcome::RestartNeeded {
            reason,
            refused,
            discovered,
        } => {
            // The refused crate can never patch this session, but the
            // other edits of the attempt stay uninstalled, so they must
            // ride along in the next patch.
            if let Some(name) = refused {
                session.pending.remove(&name);
            }
            println!("patch {generation}: not sent; {reason}");
            learn(session, tip_name, discovered, true);
        }
        Outcome::AppRefused { reason, discovered } => {
            // The app keeps running: with the table it had when the
            // library did not load, or with this one but its earlier view
            // when the state type changed, which needs the rebuild. The
            // edits of this patch stay pending for the next one.
            println!(
                "patch {generation}: the app refused the patch: {reason}; the app \
                 keeps running, and `r` rebuilds and restarts it"
            );
            learn(session, tip_name, discovered, true);
        }
        Outcome::ConnectionLost { reason, discovered } => {
            println!("patch {generation}: the connection to the app broke; the app keeps running");
            eprintln!("{reason}");
            // The framing of a connection that timed out or broke cannot
            // be trusted, so a late answer never reaches a later patch.
            session.stream = None;
            learn(session, tip_name, discovered, true);
        }
    }
}

/// Folds what a replay learned about its sources into the session's
/// membership, and rebuilds the watched files from it, so an edit in a
/// source the replay discovered reaches the next patch even when the
/// patch itself never installed. With `replace` the sources answer for
/// the crate whole, which a finished compile may; without it they only
/// join, which a failed compile's partial dep-info needs.
fn learn(
    session: &mut Session,
    tip_name: String,
    discovered: Vec<(String, Vec<PathBuf>)>,
    replace: bool,
) {
    if discovered.is_empty() {
        return;
    }
    for (name, sources) in discovered {
        if name == tip_name {
            if replace {
                session.tip_sources = sources;
            } else {
                session.tip_sources.extend(sources);
                session.tip_sources.sort();
                session.tip_sources.dedup();
            }
        } else if let Some(record) = session.crates.iter_mut().find(|record| record.name == name) {
            if replace {
                record.sources = sources;
            } else {
                record.sources.extend(sources);
                record.sources.sort();
                record.sources.dedup();
            }
        }
    }
    let mut sources = session.tip_sources.clone();
    sources.extend(
        session
            .crates
            .iter()
            .flat_map(|record| record.sources.iter().cloned()),
    );
    sources.extend(
        session
            .build_scripts
            .iter()
            .flat_map(|script| script.inputs.iter().cloned()),
    );
    session.watch_files = watch_files_of(&session.watch, &sources);
    // The membership the sources name now is the one whose presence the
    // watcher answers for, so a source a patch retired, say an include
    // a library stopped reading, leaves the set: its later deletion
    // would otherwise report a loss nothing reads anymore and patch
    // against a replay that does not compile it. A member that stood at
    // bootstrap stays watched until its loss is actually reported, even
    // when this very moment finds it already gone, and a member that
    // exists joins: only the retirement takes a watched path out.
    let membership: std::collections::BTreeSet<PathBuf> = sources.iter().cloned().collect();
    session
        .watched_present
        .retain(|path| membership.contains(path));
    for path in sources {
        if session.watched_present.contains(&path) {
            continue;
        }
        if std::fs::metadata(&path).is_ok() {
            session.watched_present.insert(path);
        }
    }
}

/// Why an edit in `name` can never patch this session, no matter how
/// often the user saves.
fn refusal_of(crates: &[CrateRecord], tip_name: &str, name: &str) -> Option<String> {
    if name == tip_name {
        return Some(format!(
            "the package {name} builds the app and a library of the same \
             name, which one patch chain cannot replay separately; restart \
             the session"
        ));
    }
    let record = crates.iter().find(|record| record.name == name)?;
    if !record.trusted {
        return Some(format!(
            "the recording of {name} describes another build of it than \
             the app links; restart the session so cargo records this build"
        ));
    }
    if record.ambiguous {
        return Some(format!(
            "the build made {name} as more than one unit, which one \
             recording cannot replay; restart the session"
        ));
    }
    None
}

/// The recorded crates whose sources `path` belongs to. The dep-info of
/// each recorded unit decides first, so a source two units compile lands
/// with every unit a patch can replay, and only where a unit's dep-info
/// is missing does the directory the source sits in decide. The app
/// crate's own sources answer none.
fn owners_of<'a>(
    crates: &'a [CrateRecord],
    tip_root: &Path,
    tip_sources: &[PathBuf],
    path: &Path,
) -> Vec<&'a CrateRecord> {
    let owned: Vec<&CrateRecord> = crates
        .iter()
        .filter(|record| record.sources.iter().any(|source| source == path))
        .collect();
    if !owned.is_empty() {
        return owned;
    }
    if tip_sources.iter().any(|source| source == path) || path == tip_root {
        return Vec::new();
    }
    crates
        .iter()
        .filter(|record| record.sources.is_empty())
        .filter(|record| {
            record
                .root
                .parent()
                .is_some_and(|dir| path.starts_with(dir))
        })
        .max_by_key(|record| {
            record
                .root
                .parent()
                .map(|dir| dir.components().count())
                .unwrap_or(0)
        })
        .into_iter()
        .collect()
}

/// The replay chain's key for the app crate itself, which stays apart
/// from the library of the same name its package may also build.
const TIP_NODE: &str = "\u{0}tip";

/// Why a replay chain was refused before a patch was built.
struct ChainError {
    /// The crate the chain refused.
    name: String,
    /// The sentence the session prints about it.
    reason: String,
    /// Whether the refused crate's edit stays pending: a crate the
    /// chain cannot reach was never installable, while one the patch
    /// must carry may replay after a session restart.
    preserve: bool,
}

/// The replay order for edits in the crates `changed` name and for the
/// crates of the earlier patches: everything the app crate reaches that
/// must replay, dependencies first, with the app crate itself last.
/// Answers an error when a name cannot join the chain.
fn chain_of<'a>(
    crates: &'a [CrateRecord],
    tip: (&str, &'a Recorded),
    changed: &[&str],
) -> Result<Vec<patch::Step<'a>>, ChainError> {
    let (tip_name, tip_record) = tip;
    let mut graph: std::collections::BTreeMap<&str, Vec<String>> =
        std::collections::BTreeMap::new();
    for record in crates {
        graph.insert(
            &record.name,
            record
                .edges
                .iter()
                .map(|(_, real)| real.clone())
                .filter(|dependency| *dependency != record.name.as_str())
                .collect(),
        );
    }
    // The app crate keeps a key of its own, so the library of the same
    // name its package may also build keeps its edges, and a chain that
    // would have to recompile that library is seen.
    graph.insert(
        TIP_NODE,
        tip_record
            .extern_crates()
            .into_iter()
            .map(|(_, real)| real)
            .filter(|dependency| *dependency != tip_name)
            .collect(),
    );
    // What the app crate reaches, which is what a patch can affect.
    let mut descendants = std::collections::BTreeSet::from([TIP_NODE]);
    let mut growing = vec![TIP_NODE];
    while let Some(name) = growing.pop() {
        for dependency in graph.get(name).into_iter().flatten() {
            if descendants.insert(dependency.as_str()) {
                growing.push(dependency);
            }
        }
    }
    for name in changed {
        if !descendants.contains(name) {
            return Err(ChainError {
                name: String::from(*name),
                reason: format!("{name} is not in the dependency chain of the app crate"),
                preserve: false,
            });
        }
    }
    // What depends on any changed crate, transitively.
    let mut members = std::collections::BTreeSet::from([TIP_NODE]);
    for name in changed {
        members.insert(name);
        let mut growing = vec![*name];
        while let Some(name) = growing.pop() {
            for (dependent, dependencies) in &graph {
                if dependencies.iter().any(|dependency| dependency == name)
                    && members.insert(dependent)
                {
                    growing.push(dependent);
                }
            }
        }
    }
    // The library of the app's own package never replays, so a chain
    // that would change what its own code baked in refuses instead of
    // silently leaving that copy old. Only a procedural macro the
    // library itself names bakes expansion output into its content; a
    // plain dependency change merely re-links it, which its old code
    // survives through the remapped addresses of the patch.
    let bakes = crates
        .iter()
        .find(|record| record.name == tip_name)
        .is_some_and(|record| {
            record.edges.iter().any(|(_, dependency)| {
                crates
                    .iter()
                    .find(|other| other.name == *dependency)
                    .is_some_and(|other| other.proc_macro && members.contains(other.name.as_str()))
            })
        });
    if bakes {
        return Err(ChainError {
            name: tip_name.to_string(),
            reason: format!(
                "the package {tip_name} builds the app and a library of the \
                 same name, which one patch chain cannot replay separately; \
                 restart the session"
            ),
            preserve: true,
        });
    }
    members.retain(|name| descendants.contains(*name));
    // A crate whose recording describes another build, or one the build
    // made as two units, never replays. For a crate the patch must
    // carry, dropping it would compile its consumers against the
    // artifact this build links and bake the stale expansion in, so
    // the whole attempt refuses and its edits stay pending.
    for name in &members {
        if *name == TIP_NODE {
            continue;
        }
        let Some(record) = crates.iter().find(|record| record.name == *name) else {
            return Err(ChainError {
                name: String::from(*name),
                reason: format!("{} is not in the dependency chain of the app crate", *name),
                preserve: false,
            });
        };
        if !record.trusted {
            return Err(ChainError {
                name: record.name.clone(),
                reason: format!(
                    "{} must replay for this patch, but the recording of {} \
                     describes another build of it than the app links; restart \
                     the session so cargo records this build",
                    record.name, record.name
                ),
                preserve: true,
            });
        }
        if record.ambiguous {
            return Err(ChainError {
                name: record.name.clone(),
                reason: format!(
                    "{} must replay for this patch, but the build made {} as \
                     more than one unit, which one recording cannot replay; \
                     restart the session",
                    record.name, record.name
                ),
                preserve: true,
            });
        }
    }
    // Dependencies before dependents, always emitting a crate whose
    // replayed dependencies within the chain are all emitted.
    let mut order = Vec::with_capacity(members.len());
    let mut pending: std::collections::BTreeSet<&str> = members;
    while !pending.is_empty() {
        let mut progressed = false;
        for name in pending.clone() {
            let ready = graph
                .get(name)
                .into_iter()
                .flatten()
                .all(|dependency| !pending.contains(dependency.as_str()));
            if ready {
                order.push(name);
                pending.remove(name);
                progressed = true;
            }
        }
        if !progressed {
            let name = pending.iter().next().copied().unwrap_or(TIP_NODE);
            return Err(ChainError {
                name: String::from(name),
                reason: format!("{name} is not in the dependency chain of the app crate"),
                preserve: false,
            });
        }
    }
    Ok(order
        .into_iter()
        .map(|name| {
            if name == TIP_NODE {
                patch::Step {
                    name: String::from(tip_name),
                    proc_macro: false,
                    runtime: true,
                    tip: true,
                    record: tip_record,
                }
            } else {
                let record = crates
                    .iter()
                    .find(|entry| entry.name == name)
                    .expect("the chain names a recorded crate");
                patch::Step {
                    name: record.name.clone(),
                    proc_macro: record.proc_macro,
                    runtime: record.runtime,
                    tip: false,
                    record: &record.record,
                }
            }
        })
        .collect())
}

/// Whether a recorded unit is a build script's compile: the crate name
/// cargo gives scripts, on an invocation that compiles a binary into a
/// build directory, which cargo gives the scripts alone; an ordinary
/// library or binary of that name compiles into the deps directory.
fn is_script_unit(unit: &UnitRecord) -> bool {
    let rest = &unit.record.args[1..];
    unit.name.starts_with("build_script_")
        && crate::record::values_in(rest, "--crate-type").any(|kind| kind == "bin")
        && crate::record::values_in(rest, "--out-dir").any(|dir| {
            Path::new(dir)
                .parent()
                .is_some_and(|parent| parent.file_name().is_some_and(|name| name == "build"))
        })
}

/// Reads the recorded invocation of every unit the fat build compiled,
/// other than the app crate itself. The records of a previous session
/// stay: a build that cargo serves from its cache runs the wrapper for
/// none of its crates, and the recordings of those crates still describe
/// what cargo would run.
fn crate_records(dir: &Path) -> Result<Vec<UnitRecord>, String> {
    let mut units = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(units);
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("rustc-unit-") && name.ends_with(".json")) {
            continue;
        }
        let recorded = modified_nanos(&entry.path());
        let record = Recorded::load(&entry.path())?;
        let Some(crate_name) = record.crate_name().map(String::from) else {
            continue;
        };
        let Some(crate_root) = record.last_arg(".rs") else {
            continue;
        };
        let root = if crate_root.is_absolute() {
            crate_root
        } else {
            Path::new(&record.cwd).join(crate_root)
        };
        let proc_macro = record.crate_types().contains(&"proc-macro");
        let stem = record.artifact_stem().unwrap_or_else(|| crate_name.clone());
        units.push(UnitRecord {
            name: crate_name,
            stem,
            proc_macro,
            root,
            recorded,
            record,
        });
    }
    units.sort_by(|a, b| a.stem.cmp(&b.stem));
    Ok(units)
}

/// Whether one of this attempt's saved files is a source a build script
/// compiles from, which marks the session's scripts stale for the rest
/// of the session, whatever else the attempt does with its edits.
fn script_input_saved(scripts: &[ScriptGuard], saved: &[(SystemTime, PathBuf)]) -> bool {
    saved.iter().any(|(_, path)| {
        let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        scripts.iter().any(|script| {
            script.inputs.iter().any(|input| {
                let spelled = std::fs::canonicalize(input).unwrap_or_else(|_| input.clone());
                *input == real || *input == *path || spelled == real
            })
        })
    })
}

/// The files and directories a build script's run declared through
/// `cargo:rerun-if-changed` lines, resolved against the package, read
/// from the newest `output` file the target's build directory holds for
/// the script's package. Cargo reruns the script when any of them
/// moves, and the expansion it baked from them is compile-time content
/// a patch cannot regenerate. Answers two lists, the files and the
/// directories, since a directory's content a single watched path
/// cannot follow.
fn script_declared_inputs(
    record: &Recorded,
    package: &Path,
    bound: Option<SystemTime>,
    active_build: Option<&Path>,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    // The compile's out directory is one build directory's entry of its
    // package: `<target>/<profile>/build/<pkg>-<hash>`; the runs'
    // outputs live in the build directories of their own kind, whose
    // hashes are their own: beside the compile for the host's units,
    // and in the target's own tree when the build names one, which an
    // explicit target makes for every package the app's units consume.
    // Each run declares its own inputs, so every output in the window
    // joins the guard, and the newest one alone answers when the window
    // catches nothing.
    let Some(out_dir) = record.out_dir() else {
        return (Vec::new(), Vec::new());
    };
    let Some(host) = Path::new(out_dir).parent() else {
        return (Vec::new(), Vec::new());
    };
    let Some(named) = Path::new(out_dir)
        .file_name()
        .and_then(|name| name.to_str())
    else {
        return (Vec::new(), Vec::new());
    };
    let Some(package_name) = package_prefix(named) else {
        return (Vec::new(), Vec::new());
    };
    let mut trees = vec![host.to_path_buf()];
    if let Some(active) = active_build
        && active != host
    {
        trees.push(active.to_path_buf());
    }
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    let mut windowed: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for build in &trees {
        let Ok(entries) = std::fs::read_dir(build) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !one_package_directory(name, package_name) {
                continue;
            }
            let output = entry.path().join("output");
            let Ok(modified) = std::fs::metadata(&output).and_then(|meta| meta.modified()) else {
                continue;
            };
            if bound.is_none_or(|floor| modified >= floor) {
                windowed.push((modified, output.clone()));
            }
            if newest.as_ref().is_some_and(|(kept, _)| modified <= *kept) {
                continue;
            }
            newest = Some((modified, output));
        }
    }
    let outputs: Vec<PathBuf> = if windowed.is_empty() {
        newest.into_iter().map(|(_, output)| output).collect()
    } else {
        windowed.into_iter().map(|(_, output)| output).collect()
    };
    let mut files = std::collections::BTreeSet::new();
    let mut dirs = std::collections::BTreeSet::new();
    for output in outputs {
        let Ok(bytes) = std::fs::read(&output) else {
            continue;
        };
        // The run's own chatter may carry bytes no encoding names, which
        // cargo reads past; the directives beside them survive the lossy
        // reading.
        let text = String::from_utf8_lossy(&bytes);
        let (output_files, output_dirs) = declared_inputs_of(&text, package);
        files.extend(output_files);
        dirs.extend(output_dirs);
    }
    (files.into_iter().collect(), dirs.into_iter().collect())
}

/// The inputs one run's output declares: every `rerun-if-changed`
/// directive, in either of the spellings cargo takes, resolved against
/// the package.
fn declared_inputs_of(text: &str, package: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for line in text.lines() {
        // Cargo reads the line after trimming it, so a directive an
        // indent carries answers all the same, while the value keeps
        // whatever follows the `=` as it is written.
        let line = line.trim();
        let Some(named) = line
            .strip_prefix("cargo:rerun-if-changed=")
            .or_else(|| line.strip_prefix("cargo::rerun-if-changed="))
        else {
            continue;
        };
        if named.is_empty() {
            continue;
        }
        let path = if Path::new(named).is_absolute() {
            PathBuf::from(named)
        } else {
            package.join(named)
        };
        if std::fs::metadata(&path).is_ok_and(|meta| meta.is_dir()) {
            dirs.push(path);
        } else {
            files.push(path);
        }
    }
    (files, dirs)
}

/// The package prefix a build directory name carries: the name without
/// its trailing metadata hash, which the compile and the run of one
/// package hash on their own.
fn package_prefix(name: &str) -> Option<&str> {
    let (prefix, hash) = name.rsplit_once('-')?;
    (!prefix.is_empty() && !hash.is_empty() && hash.chars().all(|c| c.is_ascii_hexdigit()))
        .then_some(prefix)
}

/// Whether the directory name `name` is one of the build directory's
/// entries of the package `package` names: the same prefix with a
/// metadata hash of its own.
fn one_package_directory(name: &str, package: &str) -> bool {
    name.strip_prefix(package)
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|hash| !hash.is_empty() && hash.chars().all(|c| c.is_ascii_hexdigit()))
}

/// The observed state of every entry under `dir`, the directory
/// itself included: each path with the fold of its own states. A
/// declared directory answers by whole-map comparison, so no file
/// stamped ahead of the session can mask an edit, its own included:
/// a rewrite that keeps the stamp still moves the change time the
/// fold carries.
fn dir_snapshot(dir: &Path) -> Option<BTreeMap<PathBuf, u128>> {
    let mut snapshot = BTreeMap::new();
    snapshot.insert(dir.to_path_buf(), entry_nanos(dir)?);
    walk_all(dir, &mut |path| {
        if let Some(nanos) = entry_nanos(path) {
            snapshot.insert(path.to_path_buf(), nanos);
        }
    });
    Some(snapshot)
}

/// The fold of a path's own states, the spell a link at the end of it
/// answers beside the target it names, without following a link on the
/// way in: a replaced file or a retargeted link moves the answer even
/// when the target stands still, and so does a rewrite of a file whose
/// modification time someone holds ahead of the session. The fold is
/// compared for equality, not order, and collapses to sixty-four bits
/// like the source stamps do.
fn entry_nanos(path: &Path) -> Option<u128> {
    let states: Vec<(i128, i128)> = [false, true]
        .into_iter()
        .filter_map(|follow| times_of_path(path, follow))
        .collect();
    let mut fold = 0u128;
    let mut seen: Vec<(i128, i128)> = Vec::new();
    for state in states {
        if !seen.contains(&state) {
            fold ^= state_fold(state.0, state.1);
            seen.push(state);
        }
    }
    Some(u128::from(fold as u64))
}

/// Marks the session's guards stale when a script input moved without a
/// save the watcher could report: a declared input that stood at
/// bootstrap is gone, or a declared directory's content moved. The flag
/// is sticky for the session, since a patch cannot rerun the script
/// whatever a later save touches.
fn note_script_input_moves(session: &mut Session) {
    if session.scripts_stale {
        return;
    }
    let gone = session
        .script_inputs_present
        .iter()
        .any(|input| std::fs::metadata(input).is_err());
    let moved = session
        .script_dirs
        .iter()
        .any(|(dir, then)| dir_snapshot(dir) != *then);
    session.scripts_stale = gone || moved;
}

/// The watched sources that stood at bootstrap and are gone, each with
/// the time of the noticing: a library whose input cannot compile again
/// must replay and fail loudly rather than leave the app running the
/// content it already carries. A gone path leaves the set, so the loss
/// reports once; a file made again afterwards answers the scans that
/// see its new times.
fn vanished_files(session: &mut Session) -> Vec<(SystemTime, PathBuf)> {
    let now = SystemTime::now();
    let mut gone = Vec::new();
    session.watched_present.retain(|path| {
        if std::fs::metadata(path).is_ok() {
            true
        } else {
            gone.push((now, path.clone()));
            false
        }
    });
    gone.sort();
    gone
}

/// The reason a patch cannot be sent because a build script's inputs
/// moved: a replayed crate the script's expansion reads, or a saved
/// file the script itself compiles from, would leave the generated
/// files or the expansion old, and the session cannot rerun the script.
fn script_refusal(
    scripts: &[ScriptGuard],
    replays: &[&str],
    saved: &[(SystemTime, PathBuf)],
) -> Option<String> {
    for script in scripts {
        let package = script
            .package
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("a package");
        for read in &script.reads {
            if replays.contains(&read.as_str()) {
                return Some(format!(
                    "the build script of {package} reads {read}, \
                     which a patch cannot rerun; restart the session"
                ));
            }
        }
        for (_, path) in saved {
            let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
            if script.inputs.iter().any(|input| {
                let spelled = std::fs::canonicalize(input).unwrap_or_else(|_| input.clone());
                *input == real || *input == *path || spelled == real
            }) {
                return Some(format!(
                    "the build script of {package} reads {}, \
                     which a patch cannot rerun; restart the session",
                    path.display()
                ));
            }
        }
    }
    None
}

/// The dep-info sources the watch does not already cover: the files
/// outside the watched directories, and the files inside them that the
/// `.rs` walk cannot promise, such as what an `include_str!` pulls in
/// or a path that goes through a link, with or without an extension. A
/// spelling that carries a `..` component never counts as covered,
/// since `src/../config.rs` answers outside `src`, where no walk of
/// `src` reaches it, and a link can carry a spelling anywhere, so only
/// one whose resolution still stands inside the directory counts as
/// covered.
fn watch_files_of(watch: &[PathBuf], sources: &[PathBuf]) -> Vec<PathBuf> {
    let homes: Vec<PathBuf> = watch
        .iter()
        .map(|dir| std::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone()))
        .collect();
    let mut files: Vec<PathBuf> = sources
        .iter()
        .filter(|source| {
            source
                .components()
                .any(|part| part == std::path::Component::ParentDir)
                || !watch.iter().any(|dir| source.starts_with(dir))
                || !source.extension().is_some_and(|ext| ext == "rs")
            // A `.rs` spelling under a watched directory is the walk's
            // to answer only while it stays there: a link inside the
            // spelling can carry it anywhere, so a source whose
            // resolution leaves the directory stays explicitly watched.
                || watch.iter().zip(&homes).any(|(dir, home)| {
                    source.starts_with(dir)
                        && !std::fs::canonicalize(source)
                            .unwrap_or_else(|_| (**source).clone())
                            .starts_with(home)
                })
            // A spelling whose resolution descends through a link back
            // to an ancestor stays explicitly watched as well: the walk
            // refuses to re-enter the ancestor, so the retargeting of
            // the link moves no watched file, and only the spelling's
            // own resolution carries the link whose stamp a retarget
            // moves.
                || resolution_reenters_an_ancestor(source)
        })
        .cloned()
        .collect();
    files.sort();
    files.dedup();
    files
}

/// Whether `source`'s resolution descends through a directory link whose
/// target stands at or above the link's own directory, which the walk
/// refuses to re-enter and whose retarget moves no watched file.
fn resolution_reenters_an_ancestor(source: &Path) -> bool {
    let mut walked = PathBuf::new();
    for component in source.components() {
        walked.push(component.as_os_str());
        let Ok(meta) = std::fs::symlink_metadata(&walked) else {
            return false;
        };
        if !meta.is_symlink() {
            continue;
        }
        let Ok(resolved) = std::fs::canonicalize(&walked) else {
            continue;
        };
        let Some(parent) = walked.parent() else {
            continue;
        };
        if std::fs::canonicalize(parent).is_ok_and(|parent_real| parent_real.starts_with(&resolved))
        {
            return true;
        }
    }
    false
}

/// The source files one recorded unit compiled, from the dep-info file
/// cargo wrote next to the unit's artifacts. Answers an empty list when
/// the file is missing, in which case ownership falls back to the source
/// directories.
fn unit_sources(record: &Recorded, stem: &str) -> Vec<PathBuf> {
    let Some(out_dir) = record.out_dir() else {
        return Vec::new();
    };
    let path = Path::new(out_dir).join(format!("{stem}.d"));
    crate::record::dep_info_sources(&path, &record.cwd)
}

/// The last `lines` lines of a log, which keeps a compiler error readable.
fn tail(log: &str, lines: usize) -> String {
    let collected: Vec<&str> = log.lines().collect();
    let at = collected.len().saturating_sub(lines);
    collected[at..].join("\n")
}

/// The fold of the times a watched path can change by: its own, and
/// every link its resolution goes through, whose targets join the
/// remaining components and carry the walk on through them. The fold
/// carries every state the resolution stands on instead of naming
/// their newest, which a stamp ahead of everything else would hold
/// still: a rename that moves a prepared file or link over a watched
/// name keeps the replacement's modification time and only moves its
/// change time, and a retarget of an inner link moves its own times
/// while both ends stand still, and both moves answer under a fold
/// that no single state can mask. The plain directories on the way
/// keep their times out, which a new file beside the watched one must
/// not answer for, and one file's states fold once, whatever number
/// of spellings the walk repeats it under.
fn changed_nanos(path: &Path) -> Option<u128> {
    let mut fold = 0u128;
    let mut seen: Vec<(u64, u64)> = Vec::new();
    if let Some((modified, changed)) = times_of_path(path, false)
        && let Some(id) = identity_of(path)
    {
        fold ^= state_fold(modified, changed);
        seen.push(id);
    } else {
        return None;
    }
    let mut current = if path.is_absolute() {
        PathBuf::from("/")
    } else {
        std::env::current_dir().unwrap_or_default()
    };
    // The components wait in a queue, so the walk takes the first one;
    // a link splices its target's components ahead of the rest.
    let mut pending: std::collections::VecDeque<std::ffi::OsString> = path
        .components()
        .filter(|part| {
            !matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Prefix(_)
            )
        })
        .map(|part| part.as_os_str().to_owned())
        .collect();
    let mut links = 0;
    while let Some(named) = pending.pop_front() {
        if named == std::ffi::OsStr::new(".") {
            continue;
        }
        if named == std::ffi::OsStr::new("..") {
            current.pop();
            continue;
        }
        current.push(&named);
        let Some((modified, changed)) = times_of_path(&current, false) else {
            break;
        };
        if !std::fs::symlink_metadata(&current).is_ok_and(|meta| meta.file_type().is_symlink()) {
            continue;
        }
        if let Some(id) = identity_of(&current)
            && !seen.contains(&id)
        {
            fold ^= state_fold(modified, changed);
            seen.push(id);
        }
        links += 1;
        if links > 32 {
            break;
        }
        let Ok(target) = std::fs::read_link(&current) else {
            break;
        };
        if target.is_absolute() {
            current = PathBuf::from("/");
        } else {
            current.pop();
        }
        let spliced: Vec<std::ffi::OsString> = target
            .components()
            .filter(|part| {
                !matches!(
                    part,
                    std::path::Component::RootDir | std::path::Component::Prefix(_)
                )
            })
            .map(|part| part.as_os_str().to_owned())
            .collect();
        for part in spliced.into_iter().rev() {
            pending.push_front(part);
        }
    }
    // The file the walk ends at carries its own times in, which the
    // spelling's last link hides: a link named as the watched file
    // stands still while the content behind it moves.
    if let Some((modified, changed)) = times_of_path(&current, false)
        && let Some(id) = identity_of(&current)
        && !seen.contains(&id)
    {
        fold ^= state_fold(modified, changed);
    }
    // The fold names a state, not a time: collapsed to sixty-four
    // bits so the states the session remembers keep comparing as
    // times do, since the fold itself spans more than any clock.
    Some(u128::from(fold as u64))
}

/// One resolved state's fold entry: its modification and change times
/// mixed together, which a pair of equal times folds to a value of its
/// own instead of cancelling the entry away.
fn state_fold(modified: i128, changed: i128) -> u128 {
    const SALT: u128 = 0x9E37_79B9_7F4A_7C15_9E37_79B9_7F4A_7C15;
    let mut mixed = u128::from_ne_bytes(modified.to_ne_bytes()).wrapping_mul(SALT);
    mixed ^= u128::from_ne_bytes(changed.to_ne_bytes());
    mixed = mixed.wrapping_mul(SALT);
    mixed ^ (mixed >> 1)
}

/// The device and inode `path` names itself, which two spellings of
/// one file answer the same, so a fold counts a file's states once.
fn identity_of(path: &Path) -> Option<(u64, u64)> {
    let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
    // SAFETY: `status` is a plain out-parameter of the call and `name`
    // outlives it; no memory the caller keeps is touched otherwise.
    let mut status: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: same as above; the call only fills `status` from `name`.
    if unsafe { libc::lstat(name.as_ptr(), &mut status) } != 0 {
        return None;
    }
    Some((status.st_dev, status.st_ino))
}

/// The modification and change times of `path` itself, links and all:
/// the standard library's metadata carries no change time, which a
/// rename over a watched name needs, since it keeps the replacement's
/// own modification time.
fn times_of_path(path: &Path, follow: bool) -> Option<(i128, i128)> {
    let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
    // SAFETY: `status` is a plain out-parameter of the call and `name`
    // outlives it; no memory the caller keeps is touched otherwise.
    let mut status: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: same as above; both calls only fill `status` from `name`.
    let done = unsafe {
        if follow {
            libc::stat(name.as_ptr(), &mut status)
        } else {
            libc::lstat(name.as_ptr(), &mut status)
        }
    };
    if done != 0 {
        return None;
    }
    #[cfg(target_os = "linux")]
    let (modified, changed) = (
        i128::from(status.st_mtime) * 1_000_000_000 + i128::from(status.st_mtime_nsec),
        i128::from(status.st_ctime) * 1_000_000_000 + i128::from(status.st_ctime_nsec),
    );
    #[cfg(target_os = "macos")]
    let (modified, changed) = (
        i128::from(status.st_mtimespec.tv_sec) * 1_000_000_000
            + i128::from(status.st_mtimespec.tv_nsec),
        i128::from(status.st_ctimespec.tv_sec) * 1_000_000_000
            + i128::from(status.st_ctimespec.tv_nsec),
    );
    Some((modified, changed))
}

/// The modification time every watched path carries now, which a
/// scan's comparison starts from at bootstrap and the patch loop
/// renews with after each patch it prepares.
fn observe(watch: &[PathBuf], watch_files: &[PathBuf]) -> HashMap<PathBuf, SystemTime> {
    let empty = HashMap::new();
    let (_, source_times) = scan_sources(watch, &empty);
    let (_, mut times) = scan_files(watch_files, &empty);
    for (path, time) in source_times {
        times.insert(path, time);
    }
    times
}

/// The `.rs` files under the watched directories whose observed state
/// differs from `times`, oldest first, with the state each visited file
/// carries now. A save that moves a stamp backward is as much a change
/// as one that moves it forward, since a file stamped ahead of the
/// session falls back when it is really saved. The map answers only the
/// files the walk visited, so a caller can merge it onto the remembered
/// states without reviving entries of paths this scan never reached.
fn scan_sources(
    dirs: &[PathBuf],
    times: &HashMap<PathBuf, SystemTime>,
) -> (Vec<(SystemTime, PathBuf)>, HashMap<PathBuf, SystemTime>) {
    let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
    let mut now = HashMap::new();
    for dir in dirs {
        walk(dir, &mut |path| {
            if path.extension().is_some_and(|ext| ext == "rs")
                && let Some(nanos) = changed_nanos(path)
            {
                let at = time_of(nanos);
                now.insert(path.to_path_buf(), at);
                if times.get(path) != Some(&at) {
                    found.push((at, path.to_path_buf()));
                }
            }
        });
    }
    found.sort_by_key(|(time, _)| *time);
    (found, now)
}

/// The watched files whose observed state differs from `times`, oldest
/// first, with the state each visited file carries now. The map answers
/// only the files this scan visited, so a caller can merge it onto the
/// remembered states without reviving entries of paths this scan never
/// reached.
fn scan_files(
    paths: &[PathBuf],
    times: &HashMap<PathBuf, SystemTime>,
) -> (Vec<(SystemTime, PathBuf)>, HashMap<PathBuf, SystemTime>) {
    let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
    let mut now = HashMap::new();
    for path in paths {
        if let Some(nanos) = changed_nanos(path) {
            let at = time_of(nanos);
            now.insert(path.clone(), at);
            if times.get(path) != Some(&at) {
                found.push((at, path.clone()));
            }
        }
    }
    found.sort_by_key(|(time, _)| *time);
    (found, now)
}

/// Calls `visit` for every file under `dir`. A symlinked entry is
/// visited under its own name before whatever it points at is
/// followed: a link that retargets, say from a file to a directory,
/// moves the link's own times, which the walk must report even when
/// the entry stops naming a file. A plain directory is visited too
/// when its name names a source, since a file replaced by a directory
/// of the same name stops being an entry any walk below it answers.
/// The descent knows the directories the current path stands inside,
/// so a link loop cannot walk it forever, while an alias of a
/// directory visited elsewhere is a spelling the walk must take: its
/// files answer under their own names, whatever another spelling saw.
fn walk(dir: &Path, visit: &mut impl FnMut(&Path)) {
    let mut stack = Vec::new();
    if let Some(id) = dir_identity(dir) {
        stack.push(id);
    }
    walk_dir(dir, &mut stack, visit);
}

fn walk_dir(dir: &Path, stack: &mut Vec<(u64, u64)>, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let link = std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_symlink());
        let names_a_source =
            path.extension().is_some_and(|ext| ext == "rs") || (!link && !path.is_dir());
        if link || names_a_source {
            visit(&path);
        }
        if path.is_dir()
            && let Some(id) = dir_identity(&path)
            && !stack.contains(&id)
        {
            stack.push(id);
            walk_dir(&path, stack, visit);
            stack.pop();
        }
    }
}

/// Calls `visit` for every entry under `dir`, directories included,
/// which a removal inside the tree answers through: a removed file
/// moves only the time of the directory it sat in. A symlinked
/// directory descends into, and the descent knows the directories the
/// current path stands inside, so a link loop cannot walk it forever
/// while an alias of a directory visited elsewhere is a spelling the
/// walk must take.
fn walk_all(dir: &Path, visit: &mut impl FnMut(&Path)) {
    let mut stack = Vec::new();
    if let Some(id) = dir_identity(dir) {
        stack.push(id);
    }
    walk_all_dir(dir, &mut stack, visit);
}

fn walk_all_dir(dir: &Path, stack: &mut Vec<(u64, u64)>, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        visit(&path);
        if path.is_dir()
            && let Some(id) = dir_identity(&path)
            && !stack.contains(&id)
        {
            stack.push(id);
            walk_all_dir(&path, stack, visit);
            stack.pop();
        }
    }
}

/// The device and inode the path resolves to, which a descent against
/// a link loop and against an alias it is already inside answers by.
fn dir_identity(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((
        std::os::unix::fs::MetadataExt::dev(&meta),
        std::os::unix::fs::MetadataExt::ino(&meta),
    ))
}

/// Adds `path` to `found` with its modification time, when it exists.
fn push_manifest(path: PathBuf, found: &mut Vec<(PathBuf, SystemTime)>) {
    if found.iter().any(|(seen, _)| *seen == path) {
        return;
    }
    if let Some(nanos) = modified_nanos(&path) {
        found.push((path, time_of(nanos)));
    }
}

/// The manifests of the app and when they last changed: the ones at the
/// crate root, every manifest above it, which a workspace build reads,
/// and the lockfile of every directory above, which the workspace's
/// own carries when the app is a member, and the package manifests
/// cargo's own answer names, since a package's crate root can sit
/// outside its package directory, where no ancestor of a watched
/// source directory reaches it.
fn manifest_states(app: &Path, watch: &[PathBuf]) -> Vec<(PathBuf, SystemTime)> {
    let mut found = Vec::new();
    push_manifest(app.join("Cargo.toml"), &mut found);
    push_manifest(app.join("Cargo.lock"), &mut found);
    if let Some(packages) = workspace_metadata(app)
        .and_then(|metadata| metadata.get("packages").cloned())
        .and_then(|packages| packages.as_array().cloned())
    {
        for package in packages {
            if let Some(manifest) = package
                .get("manifest_path")
                .and_then(|value| value.as_str())
            {
                push_manifest(PathBuf::from(manifest), &mut found);
            }
        }
    }
    for dir in watch {
        let mut ancestor = dir.clone();
        loop {
            push_manifest(ancestor.join("Cargo.toml"), &mut found);
            push_manifest(ancestor.join("Cargo.lock"), &mut found);
            if !ancestor.pop() {
                break;
            }
        }
    }
    found
}

/// Whether every watched manifest still has the modification time the
/// session saw at its start. A manifest that cannot be read anymore is
/// a changed input, not an unchanged one: the build the next save
/// replays reads a different workspace, and the restart that follows
/// re-records what stands.
fn manifests_unchanged(session: &Session) -> bool {
    session
        .manifests
        .iter()
        .all(|(path, seen)| modified_nanos(path).map(time_of) == Some(*seen))
}

/// Whether every configuration file still carries the content stamp the
/// session saw at its start: a file that appeared, vanished or changed
/// under either of its spellings moves what the next build reads.
fn configs_unchanged(session: &Session) -> bool {
    session
        .config_stamps
        .iter()
        .all(|(path, seen)| file_stamp(path) == *seen)
}

/// The build script path of every package of the app's workspace that does
/// not turn scripts off, with whether the file exists now.
fn script_presence(app: &Path) -> Vec<(PathBuf, bool)> {
    let Some(packages) = workspace_metadata(app)
        .and_then(|metadata| metadata.get("packages").cloned())
        .and_then(|packages| packages.as_array().cloned())
    else {
        return Vec::new();
    };
    packages
        .iter()
        .filter_map(|package| package.get("manifest_path")?.as_str())
        .filter_map(|manifest| Path::new(manifest).parent().map(Path::to_path_buf))
        .filter_map(|package| script_path(&package))
        .map(|script| {
            let present = script.exists();
            (script, present)
        })
        .collect()
}

/// Whether every build script path still exists exactly when it did at the
/// session's start.
fn scripts_unchanged(session: &Session) -> bool {
    session
        .scripts_present
        .iter()
        .all(|(script, present)| script.exists() == *present)
}

/// Whether the package still carries a build script, which a kept
/// recording of another request must answer for: the manifest names one
/// or the automatic `build.rs` stands, and a package that dropped its
/// script keeps no guard whose inputs nothing reads anymore. The
/// manifest's `build` key reads through the same complete-path matching
/// the configuration does, so a dotted `package.build` counts too, and
/// its bare `false` keeps cargo from treating any `build.rs` as a
/// script at all.
fn has_build_script(package: &Path) -> bool {
    script_path(package).is_some_and(|script| script.exists())
}

/// The path the package's build script has when it has one: the one the
/// manifest's `build` key names, none when the key is `false`, and the
/// automatic `build.rs` otherwise.
fn script_path(package: &Path) -> Option<PathBuf> {
    let manifest = package.join("Cargo.toml");
    if let Some(named) = config_string(&manifest, "package", "build") {
        return Some(package.join(named));
    }
    if config_bool(&manifest, "package", "build") == Some(false) {
        return None;
    }
    Some(package.join("build.rs"))
}

/// Reports `message` on standard error and answers failure.
fn fail(message: &str) -> ExitCode {
    eprintln!("cargo smudge: {message}");
    ExitCode::FAILURE
}

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
