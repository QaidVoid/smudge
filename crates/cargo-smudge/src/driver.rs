//! The `dev` session: fat-builds the app, runs it against the wire format,
//! watches its sources and patches it on every save.

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
const CACHE_SHAPE: u32 = 2;

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
}

/// One session: one fat build, one app, one connection.
struct Session {
    /// The app's directory, which cargo runs in.
    app: PathBuf,
    /// The directory the records, the cache and the libraries go into.
    dir: PathBuf,
    /// The app crate's recorded invocation.
    rustc: Recorded,
    /// The linker cargo configured, or `cc`.
    linker: String,
    /// The original binary.
    original: Original,
    /// The executable the session runs.
    exe: PathBuf,
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
    /// The directory whose `.rs` files are watched.
    watch: PathBuf,
    /// The newest source modification time already seen.
    seen: SystemTime,
    /// The manifests and their modification times, watched for restarts.
    manifests: Vec<(PathBuf, SystemTime)>,
    /// The socket path, which a restarted app reconnects to.
    socket: PathBuf,
    /// The build id, which stays valid until the next fat build.
    build: String,
    /// How many patches this session has sent.
    generation: u32,
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
    loop {
        let mut session = match bootstrap(&app, &options) {
            Ok(session) => session,
            Err(error) => return fail(&error),
        };
        match supervise(&mut session, &commands) {
            Control::Exited | Control::Quit => return ExitCode::SUCCESS,
            Control::Restart => continue,
        }
    }
}

/// Reads the standard input for commands until it closes.
fn commands_thread(sender: mpsc::Sender<String>) {
    let mut line = String::new();
    loop {
        line.clear();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => break,
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
    let dir = app.join("target").join("smudge-hot");
    let patches = dir.join("patches");
    std::fs::create_dir_all(&patches).map_err(|e| format!("{}: {e}", patches.display()))?;
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| app.join("target"));
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let started = SystemTime::now();
    let status = Command::new("cargo")
        .current_dir(app)
        .args(cargo_args(options))
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", &me)
        .env(crate::roles::DIR_ENVIRONMENT, &dir)
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
    let watch = source
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| String::from("the crate root has no directory"))?;
    let fat = record::RecordedLink::load(&dir.join("link-fat.json"))?;
    let exe = output_of(&fat)?;
    let linker = crate::roles::real_linker(&dir);
    let original = read_original(&dir, &exe, &fat, &target)?;

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
    let seen = newest_source(&watch);
    let manifests = manifest_states(app, &watch);
    Ok(Session {
        app: app.to_path_buf(),
        dir,
        rustc,
        linker,
        original,
        exe,
        child,
        stream: stream.map(|(stream, _)| stream),
        runtime_main,
        view,
        signature_refused: false,
        watch,
        seen,
        manifests,
        socket,
        build,
        generation: 1,
    })
}

/// The arguments one `cargo rustc` invocation needs, with the rustc
/// arguments the build cannot do without.
fn cargo_args(options: &Options) -> Vec<String> {
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
        (None, None) => {}
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
    Ok(Original {
        symbols,
        signatures,
        main,
        tls: elf::tdata_template(&bytes),
    })
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

/// Reads the hello, checks it names the app this session started and
/// answers what the builder needs from it.
/// How long the app may take to answer one frame: its hello and its
/// acknowledgement of a patch. A slower app than this is treated as one
/// that failed the step, so the session keeps taking commands.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

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
            Ok(_) => restart_app(session),
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {}
        }
        if !manifests_unchanged(session) {
            println!("the build inputs changed; restarting the build");
            let _ = session.child.kill();
            let _ = session.child.wait();
            return Control::Restart;
        }
        let newest = newest_source(&session.watch);
        if newest != session.seen {
            session.seen = newest;
            // An editor writes a save in steps, so the last state wins.
            std::thread::sleep(Duration::from_millis(150));
            session.seen = newest_source(&session.watch);
            apply_patch(session);
        }
        std::thread::sleep(POLL);
    }
}

/// Stops the app, starts it again and performs the handshake, keeping the
/// original binary as the patch target.
fn restart_app(session: &mut Session) {
    println!("restarting the app");
    let _ = session.child.kill();
    let _ = session.child.wait();
    let _ = record::remove_file(&session.socket);
    let listener = match UnixListener::bind(&session.socket) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("{}: {error}", session.socket.display());
            session.stream = None;
            return;
        }
    };
    let build = format!(
        "smudge-build-{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    session.build = build.clone();
    let spawned = Command::new(&session.exe)
        .current_dir(&session.app)
        .env(SOCKET_ENVIRONMENT, &session.socket)
        .env(BUILD_ENVIRONMENT, &build)
        .spawn();
    // The session owns the child from the spawn onward, so no path
    // through the accept or the handshake can leave it unwatched.
    session.child = match spawned {
        Ok(child) => child,
        Err(error) => {
            eprintln!("{}: {error}", session.exe.display());
            session.stream = None;
            return;
        }
    };
    match accept(&listener, &mut session.child) {
        Ok(Some(stream)) => {
            match handshake(stream, &mut session.child, &session.original, &build) {
                Ok((stream, hand)) => {
                    session.stream = Some(stream);
                    session.runtime_main = hand.runtime_main;
                    session.view = hand.view;
                }
                Err(error) => {
                    // The spawned app of a failed handshake must not
                    // outlive the session as an orphan nobody watches.
                    let _ = session.child.kill();
                    let _ = session.child.wait();
                    eprintln!("the restarted app failed its handshake: {error}");
                    session.stream = None;
                }
            }
        }
        Ok(None) => {
            session.stream = None;
        }
        Err(error) => {
            let _ = session.child.kill();
            let _ = session.child.wait();
            eprintln!("{error}");
            session.stream = None;
        }
    }
}

/// Runs one patch step and reports what it ended with.
fn apply_patch(session: &mut Session) {
    // Every attempt counts, so a refusal or a failure still advances the
    // numbering and the next save patches under the next number.
    let generation = session.generation;
    session.generation += 1;
    let Some(stream) = session.stream.as_mut() else {
        println!("patch {generation}: skipped, because the app is not connected");
        return;
    };
    let mut plan = Plan {
        rustc: &session.rustc,
        dir: &session.dir,
        symbols: &session.original.symbols,
        signatures: &mut session.original.signatures,
        main: session.original.main,
        runtime_main: session.runtime_main,
        view: session.view,
        signature_refused: &mut session.signature_refused,
        linker: &session.linker,
        tls: &session.original.tls,
        stream,
    };
    let outcome = match patch::run(&mut plan, generation) {
        Ok(outcome) => outcome,
        Err(error) => Outcome::BuildFailed { reason: error },
    };
    match outcome {
        Outcome::Applied {
            compile,
            total,
            entries,
            added,
            changed_signature,
            unverified,
            trampolines,
            absolutes,
            tls_copies,
            missing,
        } => {
            println!(
                "patch {generation}: applied; compile {compile:?}, total {total:?}, \
                 {entries} entries, {added} new, {changed_signature} omitted for a \
                 changed signature, {unverified} unverifiable, {trampolines} \
                 trampolines, {absolutes} absolutes, {tls_copies} tls, {missing} missing"
            );
        }
        Outcome::CompileFailed { log } => {
            println!("patch {generation}: compile failed; the app keeps running");
            eprintln!("{}", tail(&log, 30));
        }
        Outcome::BuildFailed { reason } => {
            println!("patch {generation}: not built; the app keeps running");
            eprintln!("{reason}");
        }
        Outcome::RestartNeeded { reason } => {
            println!("patch {generation}: not sent; {reason}");
        }
        Outcome::AppRefused { reason } => {
            println!(
                "patch {generation}: the app refused the patch: {reason}; \
                 restarting the app"
            );
            restart_app(session);
        }
        Outcome::ConnectionLost { reason } => {
            println!("patch {generation}: the connection to the app broke; the app keeps running");
            eprintln!("{reason}");
            // The framing of a connection that timed out or broke cannot
            // be trusted, so a late answer never reaches a later patch.
            session.stream = None;
        }
    }
}

/// The last `lines` lines of a log, which keeps a compiler error readable.
fn tail(log: &str, lines: usize) -> String {
    let collected: Vec<&str> = log.lines().collect();
    let at = collected.len().saturating_sub(lines);
    collected[at..].join("\n")
}

/// The newest modification time of any `.rs` file under `dir`.
fn newest_source(dir: &Path) -> SystemTime {
    let mut newest = SystemTime::UNIX_EPOCH;
    walk(dir, &mut |path| {
        if path.extension().is_some_and(|ext| ext == "rs")
            && let Some(nanos) = modified_nanos(path)
        {
            let at = time_of(nanos);
            if at > newest {
                newest = at;
            }
        }
    });
    newest
}

/// Calls `visit` for every file under `dir`.
fn walk(dir: &Path, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, visit);
        } else {
            visit(&path);
        }
    }
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
/// crate root and every manifest above it, which a workspace build reads.
fn manifest_states(app: &Path, watch: &Path) -> Vec<(PathBuf, SystemTime)> {
    let mut found = Vec::new();
    push_manifest(app.join("Cargo.toml"), &mut found);
    push_manifest(app.join("Cargo.lock"), &mut found);
    let mut ancestor = watch.to_path_buf();
    loop {
        push_manifest(ancestor.join("Cargo.toml"), &mut found);
        if !ancestor.pop() {
            break;
        }
    }
    found
}

/// Whether every watched manifest still has the modification time the
/// session saw at its start.
fn manifests_unchanged(session: &Session) -> bool {
    session.manifests.iter().all(|(path, seen)| {
        modified_nanos(path)
            .map(time_of)
            .map(|at| at == *seen)
            .unwrap_or(true)
    })
}

/// Reports `message` on standard error and answers failure.
fn fail(message: &str) -> ExitCode {
    eprintln!("cargo smudge: {message}");
    ExitCode::FAILURE
}
