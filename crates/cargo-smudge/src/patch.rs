//! The patch step: replays the chain of crates an edit touched, verifies
//! the signatures of every function it would map, refuses the crates it
//! must not rebuild, builds the stub, links the patch library and sends
//! the table.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::dwarf::{Described, Signatures};
use crate::elf::{StaticFacts, Sym, Symbols, TlsTemplate};
use crate::record::Recorded;
use crate::stub;
use crate::wire::{self, APPLIED, FAILED, PATCH};

/// What one patch step ended with.
pub(crate) enum Outcome {
    /// The app applied the patch.
    Applied {
        /// How long the replayed compilation took.
        compile: Duration,
        /// How long everything took, compilation included.
        total: Duration,
        /// The entries the table holds.
        entries: usize,
        /// Functions the table maps that the original binary has not.
        added: usize,
        /// Functions left out because their signature changed, apart from
        /// the ones `changed_captures` counts.
        changed_signature: usize,
        /// Functions left out because they name a closure whose captures
        /// changed.
        changed_captures: usize,
        /// The closures whose captures changed, by path.
        recaptured: Vec<String>,
        /// The layouts the app's live values of each probe's type hold now
        /// that the patch is applied, for the probes it changed.
        type_layouts: Vec<LiveLayout>,
        /// Functions left out because they name a closure of a function
        /// whose closures the edit renumbered.
        renumbered: usize,
        /// The writable statics of the app crate the patched code starts
        /// fresh copies of, because they cannot be shared with the running
        /// app, by symbol name and source name.
        fresh_statics: Vec<(String, String)>,
        /// Functions left out whose signature could not be verified.
        unverified: usize,
        /// Imports the stub resolved to the running binary's code.
        trampolines: usize,
        /// Imports the stub resolved to the running binary's data.
        absolutes: usize,
        /// Thread-locals of the standard library the stub gave copies of
        /// the binary's initialization image.
        tls_copies: usize,
        /// Imports the binary itself does not define, which the dynamic
        /// linker resolves when the library loads.
        missing: usize,
        /// The crates the step replayed other than the app crate, in the
        /// order they were replayed.
        replayed: Vec<String>,
        /// The crates whose replay dep-info listed sources the fat build
        /// never compiled, with the sources, so the session learns what a
        /// module created after the build belongs to.
        discovered: Vec<(String, Vec<PathBuf>)>,
    },
    /// The replayed compilation failed, which the app does not see.
    CompileFailed {
        /// The compiler's output.
        log: String,
        /// The dep-info the steps that did compile wrote, which holds
        /// even when a later step failed, though a failed compile may
        /// list less than a full one.
        discovered: Vec<(String, Vec<PathBuf>)>,
    },
    /// The patch could not be built, which the app does not see. The
    /// steps that did compile still taught the session their sources.
    BuildFailed {
        /// Why.
        reason: String,
        /// What the replay learned about its sources.
        discovered: Vec<(String, Vec<PathBuf>)>,
    },
    /// Nothing was sent, because the change needs a restart.
    RestartNeeded {
        /// Why the change cannot patch.
        reason: String,
        /// The crate the change was in, when the refusal is about one
        /// crate's content and the crate can never patch this session.
        refused: Option<String>,
        /// What the replay learned about sources before the refusal.
        discovered: Vec<(String, Vec<PathBuf>)>,
    },
    /// The patch was sent and the app refused it.
    AppRefused {
        /// The app's reason.
        reason: String,
        /// What the replay learned about its sources.
        discovered: Vec<(String, Vec<PathBuf>)>,
    },
    /// The dry replay that warms the caches ran as far as it goes, and
    /// nothing was sent.
    Warmed,
    /// The connection broke while one frame went over it, so nothing can
    /// be trusted over it anymore and the session drops it.
    ConnectionLost {
        /// Why the connection broke.
        reason: String,
        /// What the replay learned about its sources.
        discovered: Vec<(String, Vec<PathBuf>)>,
    },
}

/// One crate of the replay chain: its name, what its replay contributes,
/// and the invocation to replay.
pub(crate) struct Step<'a> {
    /// The crate's name, which its replayed library and the patch line
    /// name.
    pub(crate) name: String,
    /// Whether the crate compiles as a procedural macro, which produces
    /// only the shared library its consumers read at their own compile
    /// time.
    pub(crate) proc_macro: bool,
    /// Whether the crate's objects are linked into the running app, which
    /// a crate is only when no procedural macro sits between it and the
    /// app crate.
    pub(crate) runtime: bool,
    /// Whether this step is the app crate itself, whose replay answers
    /// the objects a patch links and the link arguments it needs.
    pub(crate) tip: bool,
    /// The recorded invocation.
    pub(crate) record: &'a Recorded,
}

/// Everything one patch step needs from its session.
pub(crate) struct Plan<'a> {
    /// The replay chain, dependencies first, with the app crate itself
    /// last.
    pub chain: &'a [Step<'a>],
    /// The directory the records, the stub and the libraries go into.
    pub dir: &'a Path,
    /// The symbols of the original binary.
    pub symbols: &'a Symbols,
    /// The signatures of the original binary.
    pub signatures: &'a mut Signatures,
    /// The names the original binary defines at each function address.
    pub originals: &'a BTreeMap<u64, Vec<String>>,
    /// What the replays of this session learned from their objects.
    pub objects: &'a mut ObjectCache,
    /// The address `main` was linked at, which the table is built against.
    pub main: u64,
    /// The address `main` runs at, which slides the stub's targets.
    pub runtime_main: u64,
    /// The linked address of the view the app announced, when the hello
    /// carried it.
    pub view: Option<u64>,
    /// The linked address of the state probe the app announced, whose
    /// type parameter is the state type, when the hello carried it.
    pub state_probe: Option<u64>,
    /// Every instance of the state probe's generic function, each a probe
    /// of the type it is instantiated with, which the app records for every
    /// value it carries across a remount, by linked address and mangled
    /// name.
    pub type_probes: &'a [(u64, String)],
    /// The layout the app's live values of each probe's type hold, by the
    /// probe, where a patch changed it: none when the layout is unknown. A
    /// probe missing here holds the original binary's layout.
    pub type_layouts: &'a HashMap<ProbeId, Option<String>>,
    /// Whether the type of each layout digest reaches no runtime handle,
    /// for the layouts a patch already answered.
    pub handle_free: &'a mut HashMap<String, bool>,
    /// Whether a patch was already refused over a signature change, which
    /// the later patches omit the changed functions for instead of
    /// refusing again.
    pub signature_refused: &'a mut bool,
    /// The linker to link the library with.
    pub linker: &'a str,
    /// The initialization image of the binary's thread-locals, with its
    /// alignment and the relocations its pointer slots live in, which the
    /// stub copies for the standard library thread-locals the patch
    /// imports.
    pub tls: &'a TlsTemplate,
    /// The thread-locals of the binary the standard library defines, whose
    /// copies the stub may hand a patch.
    pub sysroot_tls: &'a std::collections::BTreeSet<String>,
    /// The builder's end of the connection.
    pub stream: &'a mut std::os::unix::net::UnixStream,
}

/// Runs one patch step for `generation`.
///
/// # Errors
///
/// Returns why the step could not run at all, which is a session error
/// rather than an outcome.
pub(crate) fn run(plan: &mut Plan<'_>, generation: u32) -> Outcome {
    replay(plan, generation, false)
}

/// Replays and links the chain without sending anything, and answers how
/// the step ended, a refusal included, so the first patch of a session
/// finds the compiler's incremental cache for the
/// replay directory, the objects' facts and the binary's signatures warm,
/// and the linker's inputs read once.
pub(crate) fn warm(plan: &mut Plan<'_>) -> Outcome {
    replay(plan, 0, true)
}

/// Runs one patch step, or only its dry part when `dry` is set.
fn replay(plan: &mut Plan<'_>, generation: u32, dry: bool) -> Outcome {
    // Every replay writes into a directory of its own, so cargo's
    // artifacts keep the fingerprints of the build they came from, and
    // the directory is emptied before the step, which a killed session
    // can have left it full for, and removed after it, either way. Its
    // name stays the same from patch to patch: the compiler reuses its
    // incremental cache only for the output directory it last wrote.
    let replay_dir = plan.dir.join("replay");
    let _ = std::fs::remove_dir_all(&replay_dir);
    let outcome = step(plan, &replay_dir, generation, dry);
    let _ = std::fs::remove_dir_all(&replay_dir);
    outcome
}

/// What compiling one replayed crate ended with.
enum Compiled {
    /// How long the compilation took.
    Done(Duration),
    /// The compiler's output.
    Failed(String),
}

/// What the replays of a session learned from their objects, by file.
///
/// Incremental compilation hands back the codegen units an edit did not
/// touch as links to the files it kept, so such an object is the same
/// file as in the last replay, and a patch parses only the objects that
/// changed. The compiler never rewrites an object in place: a unit that
/// changed is a new file, written after the one it replaces.
pub(crate) type ObjectCache = HashMap<FileId, ObjectFacts>;

/// The identity of one file and of the content it held when it was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FileId {
    /// The device the file lives on.
    device: u64,
    /// The file's inode on that device.
    inode: u64,
    /// The file's length.
    size: u64,
    /// The file's modification time, in seconds and nanoseconds.
    modified: (i64, i64),
}

impl FileId {
    /// The identity of the file at `path`, when it can stand for the
    /// file's content: the file must be a link the compiler kept, and its
    /// modification time must be finer than a second. A new file can take
    /// over the inode of a removed one with the same size, which only its
    /// later modification time tells apart, and a whole second can hold
    /// both. A file of one link, such as an object extracted from a
    /// library's archive, is new each replay and parses each time.
    ///
    /// # Errors
    ///
    /// Returns why the file's metadata could not be read.
    fn of(path: &Path) -> Result<Option<Self>, String> {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.nlink() < 2 || meta.mtime_nsec() == 0 {
            return Ok(None);
        }
        Ok(Some(Self {
            device: meta.dev(),
            inode: meta.ino(),
            size: meta.size(),
            modified: (meta.mtime(), meta.mtime_nsec()),
        }))
    }
}

/// What one object file answers, which an identical file answers again.
pub(crate) struct ObjectFacts {
    /// Every symbol the object names.
    symbols: Symbols,
    /// The signatures its debug information describes.
    signatures: Signatures,
    /// The signature of every function the object defines, by symbol name.
    answered: HashMap<String, Option<String>>,
    /// What the object says about sharing statics with the running binary.
    statics: StaticFacts,
}

/// One object a patch links, with what its symbols and its debug
/// information answer.
struct ObjectInfo {
    /// Where the object sits, for the link line.
    path: PathBuf,
    /// The crate the object belongs to.
    krate: String,
    /// The object file's identity, which keys its facts in the cache,
    /// when it has one.
    id: Option<FileId>,
    /// What the object answers.
    facts: ObjectFacts,
}

/// Replays the chain, collects what it produced, and sends the table.
/// A failure after some steps compiled keeps what those steps learned
/// about their sources, so the session can still watch them.
fn step(plan: &mut Plan<'_>, replay_dir: &Path, generation: u32, dry: bool) -> Outcome {
    let mut discovered: Vec<(String, Vec<PathBuf>)> = Vec::new();
    let mut objects: Vec<ObjectInfo> = Vec::new();
    let outcome = match step_inner(
        plan,
        replay_dir,
        generation,
        dry,
        &mut discovered,
        &mut objects,
    ) {
        Ok(outcome) => outcome,
        Err(reason) => Outcome::BuildFailed { reason, discovered },
    };
    // Every later patch replays at least the crates this one did, so the
    // cache keeps this replay's objects alone, and an edited object's
    // earlier content leaves it.
    plan.objects.clear();
    for info in objects {
        if let Some(id) = info.id {
            plan.objects.insert(id, info.facts);
        }
    }
    outcome
}

/// Reads the objects at `paths`, which belong to `krate`, taking the facts
/// of every one an earlier replay read out of `cache` and parsing the
/// others in parallel.
fn load_objects(
    cache: &mut ObjectCache,
    paths: &[PathBuf],
    krate: &str,
    binary: &Symbols,
) -> Result<Vec<ObjectInfo>, String> {
    let mut loaded: Vec<Option<ObjectInfo>> = Vec::with_capacity(paths.len());
    let mut fresh: Vec<(usize, Option<FileId>)> = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let id = FileId::of(path)?;
        match id.and_then(|id| cache.remove(&id)) {
            Some(facts) => loaded.push(Some(ObjectInfo {
                path: path.clone(),
                krate: krate.to_string(),
                id,
                facts,
            })),
            None => {
                loaded.push(None);
                fresh.push((index, id));
            }
        }
    }
    // The objects share nothing, so they parse in parallel. Eight workers
    // at most: more would only add threads to wait on. The signature walk
    // recurses as deep as the types nest, which the default stack of a
    // worker is too narrow for.
    let workers = fresh.len().clamp(1, 8);
    let chunk = fresh.len().div_ceil(workers).max(1);
    let mut parsed: Vec<Option<ObjectFacts>> = (0..fresh.len()).map(|_| None).collect();
    std::thread::scope(|scope| -> Result<(), String> {
        let mut handles = Vec::new();
        for (group, slots) in fresh.chunks(chunk).zip(parsed.chunks_mut(chunk)) {
            handles.push(
                std::thread::Builder::new()
                    .stack_size(64 * 1024 * 1024)
                    .spawn_scoped(scope, move || -> Result<(), String> {
                        for ((index, _), slot) in group.iter().zip(slots) {
                            let path = &paths[*index];
                            let bytes = std::fs::read(path)
                                .map_err(|e| format!("{}: {e}", path.display()))?;
                            *slot = Some(object_facts(&bytes, binary)?);
                        }
                        Ok(())
                    })
                    .map_err(|error| format!("an object reader could not start: {error}"))?,
            );
        }
        for handle in handles {
            handle
                .join()
                .map_err(|_| String::from("an object reader stopped"))??;
        }
        Ok(())
    })?;
    for ((index, id), facts) in fresh.into_iter().zip(parsed) {
        let Some(facts) = facts else {
            return Err(String::from("an object reader skipped an object"));
        };
        loaded[index] = Some(ObjectInfo {
            path: paths[index].clone(),
            krate: krate.to_string(),
            id,
            facts,
        });
    }
    Ok(loaded.into_iter().flatten().collect())
}

/// Parses what the object in `bytes` answers, the signature of every
/// function it defines included.
fn object_facts(bytes: &[u8], binary: &Symbols) -> Result<ObjectFacts, String> {
    let symbols = Symbols::read(bytes)?;
    let mut signatures = Signatures::read(bytes)?;
    let mut answered = HashMap::new();
    for (name, sym) in &symbols.map {
        if sym.defined && sym.kind == object::SymbolKind::Text {
            answered.insert(name.clone(), signatures.signature_by_name(name)?);
        }
    }
    let statics = crate::elf::static_facts(bytes, binary)?;
    Ok(ObjectFacts {
        symbols,
        signatures,
        answered,
        statics,
    })
}

/// The body of [`step`], which hands its discoveries back through
/// `discovered` and the objects it read through `objects`, even when it
/// fails. A `dry` step stops once the caches are warm.
fn step_inner(
    plan: &mut Plan<'_>,
    replay_dir: &Path,
    generation: u32,
    dry: bool,
    discovered: &mut Vec<(String, Vec<PathBuf>)>,
    objects: &mut Vec<ObjectInfo>,
) -> Result<Outcome, String> {
    let started = Instant::now();
    std::fs::create_dir_all(replay_dir).map_err(|e| format!("{}: {e}", replay_dir.display()))?;
    // The replayed library of each crate, which the `--extern` arguments
    // of the crates after it point at.
    let mut outputs: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut thin: Vec<String> = Vec::new();
    let mut compile = Duration::ZERO;
    let mut replayed: Vec<String> = Vec::new();
    for step in plan.chain {
        let args = rewritten(step.record, &outputs, replay_dir);
        match compile_step(step.record, &args)? {
            Compiled::Done(elapsed) => compile += elapsed,
            Compiled::Failed(log) => {
                // A compile that fails can still have written its
                // dep-info, which names the modules it got to, and one
                // that fails before it does still names the files it
                // read in its diagnostic spans; the session needs both
                // to see later edits in them.
                let dep_info = replay_dir.join(format!("{}.d", step.name));
                let mut sources: std::collections::BTreeSet<std::path::PathBuf> =
                    crate::record::dep_info_sources(&dep_info, &step.record.cwd)
                        .into_iter()
                        .collect();
                sources.extend(crate::record::span_sources(
                    &log,
                    &step.record.cwd,
                    &crate::record::unremap_of(&step.record.args),
                ));
                if !sources.is_empty() {
                    discovered.push((step.name.clone(), sources.into_iter().collect()));
                }
                return Ok(Outcome::CompileFailed {
                    log,
                    discovered: std::mem::take(discovered),
                });
            }
        }
        if step.tip {
            // The app crate's replay dep-info decides what belongs to it
            // now, which a module the build never saw can have changed.
            let dep_info = replay_dir.join(format!("{}.d", step.name));
            let sources = crate::record::dep_info_sources(&dep_info, &step.record.cwd);
            if !sources.is_empty() {
                discovered.push((step.name.clone(), sources));
            }
            let thin_args = crate::record::RecordedLink::load(&plan.dir.join("link-thin.json"))?;
            let paths: Vec<PathBuf> = thin_args
                .iter()
                .filter(|arg| arg.ends_with(".rcgu.o"))
                .map(PathBuf::from)
                .collect();
            if paths.is_empty() {
                return Err(String::from(
                    "the replay recorded no objects, which means the recorded \
                     invocation no longer matches the app",
                ));
            }
            let mut objects_read = load_objects(plan.objects, &paths, &step.name, plan.symbols)?;
            objects.append(&mut objects_read);
            thin = thin_args;
        } else {
            replayed.push(step.name.clone());
            // The replay may have compiled modules the fat build never
            // saw, which its dep-info lists and later ownership
            // decisions need, even across an app restart.
            let dep_info = replay_dir.join(format!("{}.d", step.name));
            let sources = crate::record::dep_info_sources(&dep_info, &step.record.cwd);
            if !sources.is_empty() {
                discovered.push((step.name.clone(), sources));
            }
            // A procedural macro links as a shared library its consumers
            // read at their own compile time; every other crate produces
            // the rlib its consumers name. A crate whose recorded
            // invocation names no rlib at all, a `dylib`-only library,
            // produces none, and no artifact path can paper over that.
            let artifact = if step.proc_macro {
                replay_dir.join(format!("lib{}.so", step.name))
            } else {
                let kinds = step.record.crate_types();
                if !kinds.iter().any(|kind| *kind == "lib" || *kind == "rlib") {
                    let named = if kinds.is_empty() {
                        String::from("a dynamic library")
                    } else {
                        kinds.join(" and ")
                    };
                    return Ok(Outcome::RestartNeeded {
                        reason: format!(
                            "the crate {} builds as {}, which the replay \
                             cannot serve; restart the app",
                            step.name, named
                        ),
                        refused: None,
                        discovered: std::mem::take(discovered),
                    });
                }
                replay_dir.join(format!("lib{}.rlib", step.name))
            };
            // Only a crate the running app links contributes objects, so
            // a library a macro reads at expansion time stays out of the
            // patch and its thread-locals never reach the runtime rule.
            if step.runtime {
                let dir = replay_dir.join("objects").join(&step.name);
                std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                let extracted = crate::elf::extract_rlib_objects(
                    &[artifact.display().to_string()],
                    replay_dir,
                    &dir,
                )?;
                let mut infos = load_objects(plan.objects, &extracted, &step.name, plan.symbols)?;
                let tls = infos.iter().any(|info| {
                    info.facts
                        .symbols
                        .map
                        .values()
                        .any(|sym| sym.kind == object::SymbolKind::Tls)
                });
                if tls {
                    objects.append(&mut infos);
                    return Ok(Outcome::RestartNeeded {
                        reason: format!(
                            "the crate {} uses thread-locals, which a patch \
                             would give empty copies of; restart the app",
                            step.name
                        ),
                        refused: Some(step.name.clone()),
                        discovered: std::mem::take(discovered),
                    });
                }
                objects.append(&mut infos);
            }
            outputs.insert(step.name.clone(), artifact);
        }
    }
    // A static every reference in the patch reaches through a full
    // address is the running app's own in the patched code. The decision
    // weighs every object the patch links, since a consumer's reference
    // by a 32-bit distance could not reach the running app's copy.
    let shared = crate::elf::shared_statics(objects.iter().map(|info| &info.facts.statics));
    let shared = same_static_types(shared, objects, plan.signatures)?;
    share_statics(objects, &shared, &replay_dir.join("shared"))?;
    // A static the program writes is state the running app builds since
    // it started, or since the patch that introduced it: a patch that
    // carries its own fresh copy of it resets that state behind the
    // app's back, so a library crate with one leaves the replay to the
    // restart. Any symbol in a writable section counts, whatever its
    // type: storage defined in assembly is as much state as a Rust
    // static. The app crate's ones start fresh in the patched code, and
    // the app hears of them, since refusing would block every app whose
    // macros keep writable statics, such as logging call sites.
    let tip_name = plan
        .chain
        .iter()
        .find(|step| step.tip)
        .map(|step| step.name.clone())
        .unwrap_or_default();
    for krate in &replayed {
        let fresh: Vec<String> =
            fresh_statics(objects.iter().filter(|info| &info.krate == krate), &shared)
                .iter()
                .map(|name| static_name(objects, name))
                .collect();
        if !fresh.is_empty() {
            return Ok(Outcome::RestartNeeded {
                reason: format!(
                    "the crate {krate} defines writable statics the patch cannot \
                     share with the running app ({}), which it would give fresh \
                     copies of; restart the app",
                    fresh.join(", ")
                ),
                refused: Some(krate.clone()),
                discovered: std::mem::take(discovered),
            });
        }
    }
    let app_fresh: Vec<(String, String)> = fresh_statics(
        objects.iter().filter(|info| info.krate == tip_name),
        &shared,
    )
    .into_iter()
    .map(|name| {
        let shown = static_name(objects, &name);
        (name, shown)
    })
    .collect();

    // Every object of the replay keeps its own signatures, because an
    // address is relative to its own object: the same number names a
    // different function in each of them.
    let mut defined: BTreeMap<String, (Sym, usize)> = BTreeMap::new();
    let mut imported: BTreeMap<String, ()> = BTreeMap::new();
    for (index, object) in objects.iter().enumerate() {
        for (name, sym) in &object.facts.symbols.map {
            // A shared static is the running binary's, which the patch
            // imports.
            if sym.defined && !shared.contains(name) {
                defined.insert(name.clone(), (sym.clone(), index));
            } else {
                imported.insert(name.clone(), ());
            }
        }
    }
    let imports: Vec<String> = imported
        .into_keys()
        .filter(|name| !defined.contains_key(name))
        .collect();
    // A loaded library cannot reach the binary's own thread-locals: the
    // binary keeps them out of its dynamic symbol table, and the stub's
    // copy gives the patched code an instance apart from the one the
    // running code uses. That is harmless for the per-thread caches of the
    // standard library, which inlined code of every crate reaches, and
    // wrong for state another crate keeps, which refuses.
    let thread_locals: Vec<&str> = imports
        .iter()
        .filter(|name| {
            plan.symbols
                .get(name)
                .is_some_and(|sym| sym.defined && sym.kind == object::SymbolKind::Tls)
        })
        .filter(|name| !plan.sysroot_tls.contains(*name))
        .map(String::as_str)
        .collect();
    if let Some(first) = thread_locals.first() {
        let more = match thread_locals.len() {
            1 => String::new(),
            n => format!(" and {} more", n - 1),
        };
        return Ok(Outcome::RestartNeeded {
            reason: format!(
                "the app's code uses the thread-local {first}{more} of another \
                 crate through a generic or inlined function, which a patch cannot \
                 share; patches stay refused while the code uses it, and a restart \
                 does not change that"
            ),
            refused: None,
            discovered: std::mem::take(discovered),
        });
    }
    let missing = imports
        .iter()
        .filter(|name| plan.symbols.get(name).is_none_or(|sym| !sym.defined))
        .count();
    let trampolines = imports
        .iter()
        .filter(|name| {
            plan.symbols
                .get(name)
                .is_some_and(|sym| sym.defined && sym.kind == object::SymbolKind::Text)
        })
        .count();
    let absolutes = imports.len() - missing - trampolines;

    // The binary's own signatures answer on a worker of the same
    // breadth the objects get: the walk is the same recursion, and the
    // main thread's stack is no wider than a worker's default one.
    let mut old_signatures: HashMap<String, Option<String>> = HashMap::new();
    let wanted: Vec<(String, u64)> = defined
        .iter()
        .filter(|(_, (sym, _))| sym.kind == object::SymbolKind::Text)
        .filter_map(|(name, _)| {
            plan.symbols
                .get(name)
                .filter(|old| old.defined && old.kind == object::SymbolKind::Text)
                .map(|old| (name.clone(), old.address))
        })
        .collect();
    {
        let answers = &mut old_signatures;
        let signatures = &mut *plan.signatures;
        on_wide_stack(move || {
            // A folding link can give one address to several functions, so
            // each is read by its own name, and by its address only where
            // the debug information names it nowhere.
            for (name, address) in &wanted {
                let signature = match signatures.signature_by_name(name)? {
                    Some(signature) => Some(signature),
                    None => signatures.signature(*address)?,
                };
                answers.insert(name.clone(), signature);
            }
            Ok(())
        })?;
    }

    // A function whose closures changed in number renumbers the ones
    // after the change, so a closure's name can now belong to another
    // closure of the same shape; whatever names such a closure, its body
    // or a function built for its type, stays out of the table.
    // Only the replayed crates' own functions compare: a function of a
    // crate the patch did not rebuild shows only the closures whose
    // types reached the replayed code, never all of them.
    let rebuilt: Vec<&str> = plan
        .chain
        .iter()
        .filter(|step| step.tip || replayed.contains(&step.name))
        .map(|step| step.name.as_str())
        .collect();
    let mut new_closures = crate::dwarf::ClosureIndices::new();
    for object in objects.iter() {
        for (parent, numbers) in object.facts.signatures.closures() {
            new_closures
                .entry(parent.clone())
                .or_default()
                .extend(numbers);
        }
    }
    let old_closures = plan.signatures.closures();
    let renumbered_parents: Vec<String> = new_closures
        .iter()
        .filter(|(parent, _)| {
            parent
                .split("::")
                .next()
                .is_some_and(|krate| rebuilt.contains(&krate))
        })
        .filter(|(parent, numbers)| old_closures.get(*parent) != Some(*numbers))
        .map(|(parent, _)| parent.clone())
        .collect();
    let mut renumbered = 0;

    // Every function the patch rebuilt that the binary also has: a change
    // to its parameter types or its return type leaves it out of the table.
    let mut tabled: Vec<(String, u64)> = Vec::new();
    let mut added = 0;
    let mut changed: Vec<(String, u64)> = Vec::new();
    let mut unverified = 0;
    for (name, (sym, index)) in &defined {
        if sym.kind != object::SymbolKind::Text {
            continue;
        }
        let Some(old) = plan.symbols.get(name) else {
            added += 1;
            continue;
        };
        if !old.defined || old.kind != object::SymbolKind::Text {
            continue;
        }
        let old_signature = old_signatures.get(name).and_then(|entry| entry.as_deref());
        // The object's addresses cannot be compared against the binary's,
        // so its side of the pairing goes by the symbol's name.
        let Some(new_signature) = objects[*index].facts.answered.get(name) else {
            continue;
        };
        // A type probe takes nothing and answers the same tuple whatever its
        // type, so it maps even when its type changed: the app finds the
        // original of a probe the patched code reaches through the table,
        // and the kept types say what may cross.
        let probe_only = |old_sig: &str, new_sig: &str| {
            plan.type_probes
                .iter()
                .any(|(address, probe)| *address == old.address && probe == name)
                && without_type_parameters(old_sig) == without_type_parameters(new_sig)
        };
        match (old_signature, new_signature.as_deref()) {
            (Some(old_sig), Some(new_sig))
                if old_sig == new_sig || probe_only(old_sig, new_sig) =>
            {
                if names_a_closure_of(new_sig, &renumbered_parents) {
                    renumbered += 1;
                } else {
                    tabled.push((name.clone(), old.address));
                }
            }
            (Some(_), Some(_)) => changed.push((name.clone(), old.address)),
            _ => unverified += 1,
        }
    }
    // Of the functions omitted for a changed signature, the ones that name
    // a closure whose captures changed are counted apart, so the patch line
    // can say why.
    let mut old_envs: BTreeMap<&str, &str> = BTreeMap::new();
    let mut new_envs: BTreeMap<&str, &str> = BTreeMap::new();
    let mut new_signatures: Vec<&str> = Vec::new();
    for (name, _) in &changed {
        if let Some(Some(old_sig)) = old_signatures.get(name) {
            old_envs.extend(closure_envs(old_sig));
        }
        if let Some((_, index)) = defined.get(name)
            && let Some(Some(new_sig)) = objects[*index].facts.answered.get(name)
        {
            new_envs.extend(closure_envs(new_sig));
            new_signatures.push(new_sig);
        }
    }
    // Only the replayed crates' own closures count: an environment the
    // standard library makes around one of them changes with it, and the
    // closure it wraps is the one to name.
    let recaptured: Vec<&str> = old_envs
        .iter()
        .filter(|(path, digest)| new_envs.get(*path).is_some_and(|new| new != *digest))
        .filter(|(path, _)| {
            crate::dwarf::closure_of(path).is_some_and(|(parent, _)| {
                !renumbered_parents.contains(&parent)
                    && parent
                        .split("::")
                        .next()
                        .is_some_and(|krate| rebuilt.contains(&krate))
            })
        })
        .map(|(path, _)| *path)
        .collect();
    let changed_captures = new_signatures
        .iter()
        .filter(|signature| closure_envs(signature).any(|(path, _)| recaptured.contains(&path)))
        .count();
    let recaptured: Vec<String> = recaptured
        .into_iter()
        .map(|path| path.replacen("_env#", "#", 1))
        .collect();

    // The running app holds its state in the old layout until it restarts,
    // so a patch that changed it is refused every time, not once.
    if !dry
        && let Some(probe) = plan.state_probe
        && let Some(reason) = state_change(plan, objects, &defined, probe)?
    {
        return Ok(Outcome::RestartNeeded {
            reason,
            refused: None,
            discovered: std::mem::take(discovered),
        });
    }

    // A signature change the app announced no view for, or one that lands
    // on the view, is refused once, so the app learns it must rebuild; the
    // later patches omit those functions and keep going.
    if !changed.is_empty() && !dry {
        let attributed = match plan.view {
            Some(view) => changed.iter().any(|(_, address)| *address == view),
            None => true,
        };
        let view_changed = plan
            .view
            .and_then(|view| changed.iter().find(|(_, address)| *address == view));
        if attributed && !*plan.signature_refused {
            *plan.signature_refused = true;
            if let Some((name, _)) = view_changed {
                return Ok(Outcome::RestartNeeded {
                    reason: format!(
                        "the view `{name}` changed its signature, which cannot be \
                         patched; restart the app"
                    ),
                    refused: None,
                    discovered: std::mem::take(discovered),
                });
            }
            let names: Vec<&str> = changed.iter().map(|(name, _)| name.as_str()).collect();
            return Ok(Outcome::RestartNeeded {
                reason: format!(
                    "the signature of {} changed, and this app's hello does not \
                     announce which function is the view, so the change cannot be \
                     attributed; restart the app",
                    names.join(", ")
                ),
                refused: None,
                discovered: std::mem::take(discovered),
            });
        }
    }

    let library = plan.dir.join(format!("patch-{generation}.so"));
    let slide = plan.runtime_main - plan.main;
    let (stub_object, tls_copies) = stub::build(
        &imports,
        plan.symbols,
        slide,
        plan.tls,
        plan.dir,
        generation,
    )?;
    let paths: Vec<PathBuf> = objects.iter().map(|object| object.path.clone()).collect();
    link(plan, &paths, &stub_object, &thin, &library)?;
    if dry {
        let _ = std::fs::remove_file(&library);
        return Ok(Outcome::Warmed);
    }
    let lib_bytes = std::fs::read(&library).map_err(|e| format!("{}: {e}", library.display()))?;
    let lib_symbols = Symbols::read(&lib_bytes)?;
    let Some(new_main) = lib_symbols
        .get("main")
        .filter(|sym| sym.defined && sym.kind == object::SymbolKind::Text)
        .map(|sym| sym.address)
    else {
        return Err(String::from("the patch library exports no main"));
    };
    // An optimized link can fold two functions into one address, and
    // a name the replay drops leaves the address behind: the app's
    // callers hold whatever address they captured, whatever happened
    // to the name, so a mapping only serves an address while every
    // name the original defined on it stayed in the replay and
    // answers to the same replacement.
    let mut grouped: BTreeMap<u64, Vec<(&str, u64)>> = BTreeMap::new();
    for (name, old) in &tabled {
        let Some(new) = lib_symbols
            .get(name)
            .filter(|sym| sym.defined && sym.kind == object::SymbolKind::Text)
            .map(|sym| sym.address)
            .filter(|new| *new != 0)
        else {
            continue;
        };
        grouped.entry(*old).or_default().push((name.as_str(), new));
    }
    let mut table: BTreeMap<u64, u64> = BTreeMap::new();
    let mut shared: Vec<&str> = Vec::new();
    for (old, entries) in &grouped {
        let Some(names) = plan.originals.get(old) else {
            continue;
        };
        let agreed = entries.windows(2).all(|pair| pair[0].1 == pair[1].1);
        let complete = names.len() == entries.len()
            && entries
                .iter()
                .all(|(name, _)| names.iter().any(|known| known == name));
        if agreed && complete {
            table.insert(*old, entries[0].1);
        } else {
            shared.extend(names.iter().map(String::as_str));
        }
    }
    if !shared.is_empty() && !*plan.signature_refused {
        *plan.signature_refused = true;
        return Ok(Outcome::RestartNeeded {
            reason: format!(
                "{} share one original address, and the replay cannot serve \
                 every name on it; restart the app",
                shared.join(", ")
            ),
            refused: None,
            discovered: std::mem::take(discovered),
        });
    }
    // The app runs a patched view only beside its patched state probe, so a
    // patch without the probe leaves the view out, and the app keeps the
    // view it has instead of refusing the patch.
    if let (Some(view), Some(probe)) = (plan.view, plan.state_probe)
        && !table.contains_key(&probe)
    {
        table.remove(&view);
    }
    let entries = table.len();
    let (mut kept_probes, mut type_layouts) = kept_types(plan, objects, &defined)?;
    // Without the view the remount runs the original one, whose values
    // hold the original layouts while the closures it stores run the
    // patch's code, so no layout can be vouched for, and none crosses.
    if plan.view.is_some_and(|view| !table.contains_key(&view)) {
        kept_probes.clear();
        for (_, layout) in &mut type_layouts {
            *layout = None;
        }
    }
    let mut payload = Vec::new();
    payload.extend_from_slice(&plan.main.to_le_bytes());
    payload.extend_from_slice(&new_main.to_le_bytes());
    payload.extend_from_slice(&(entries as u64).to_le_bytes());
    for (old, new) in &table {
        payload.extend_from_slice(&old.to_le_bytes());
        payload.extend_from_slice(&new.to_le_bytes());
    }
    payload.extend_from_slice(&(kept_probes.len() as u64).to_le_bytes());
    for probe in &kept_probes {
        payload.extend_from_slice(&probe.to_le_bytes());
    }
    payload.extend_from_slice(library.as_os_str().as_encoded_bytes());
    if let Err(error) = wire::write_frame(plan.stream, PATCH, &payload) {
        return Ok(Outcome::ConnectionLost {
            reason: format!("the patch could not be sent: {error}"),
            discovered: std::mem::take(discovered),
        });
    }
    let outcome = match wire::read_frame(plan.stream) {
        Ok(Some((APPLIED, _))) => Outcome::Applied {
            compile,
            total: started.elapsed(),
            entries,
            added,
            changed_signature: changed.len() - changed_captures,
            changed_captures,
            recaptured,
            type_layouts,
            renumbered,
            fresh_statics: app_fresh,
            unverified,
            trampolines,
            absolutes,
            tls_copies,
            missing,
            replayed,
            discovered: std::mem::take(discovered),
        },
        Ok(Some((FAILED, reason))) => Outcome::AppRefused {
            reason: String::from_utf8_lossy(&reason).into_owned(),
            discovered: std::mem::take(discovered),
        },
        Ok(Some((kind, _))) => Outcome::AppRefused {
            reason: format!("the app answered with the unknown frame kind {kind}"),
            discovered: std::mem::take(discovered),
        },
        // A closed connection is not a refusal: the app may have died,
        // and supervision notices the child's exit and ends the session
        // if it did, so the caller drops the stream rather than reading
        // a late answer into a later patch.
        Ok(None) => Outcome::ConnectionLost {
            reason: String::from("the app closed the connection"),
            discovered: std::mem::take(discovered),
        },
        // A frame that never arrives, or one that arrives broken, leaves
        // the connection's framing untrustworthy, so the caller drops it
        // rather than reading a late answer into a later patch.
        Err(error) => Outcome::ConnectionLost {
            reason: format!("the app did not answer the patch: {error}"),
            discovered: std::mem::take(discovered),
        },
    };
    Ok(outcome)
}

/// Runs one recorded invocation with `args`, answering how long it took,
/// or the compiler log of a failed build.
///
/// # Errors
///
/// Returns why the replay could not run at all.
fn compile_step(record: &Recorded, args: &[String]) -> Result<Compiled, String> {
    let started = Instant::now();
    let output = Command::new(&args[0])
        .args(&args[1..])
        .current_dir(&record.cwd)
        .env_clear()
        .envs(record.env.clone())
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("CARGO_MAKEFLAGS")
        .env(crate::roles::LINK_ENVIRONMENT, "thin")
        .output()
        .map_err(|e| e.to_string())?;
    let log = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        return Ok(Compiled::Failed(log));
    }
    Ok(Compiled::Done(started.elapsed()))
}

/// The arguments one replayed crate runs with: its output goes to the
/// replay directory under a fixed name, and the libraries the crates
/// before it replayed are named where they now sit.
fn rewritten(
    record: &Recorded,
    outputs: &BTreeMap<String, PathBuf>,
    out_dir: &Path,
) -> Vec<String> {
    let mut args = Vec::with_capacity(record.args.len() + 2);
    if record.args.is_empty() {
        return args;
    }
    args.push(record.args[0].clone());
    // A crate the tip names only through another crate's metadata resolves
    // through the search path, so the replayed libraries must shadow the
    // originals.
    args.push(String::from("-L"));
    args.push(format!("dependency={}", out_dir.display()));
    // A token another option consumed as its operand answers for
    // nothing: dropping or rewriting it would hand the option a
    // different value than the recorded compile read.
    let mask = crate::roles::operand_mask(&record.args);
    let mut index = 1;
    while index < record.args.len() {
        let arg = &record.args[index];
        if mask[index] {
            args.push(arg.clone());
            index += 1;
            continue;
        }
        if arg == "--out-dir" && record.args.get(index + 1).is_some() {
            args.push(String::from("--out-dir"));
            args.push(out_dir.display().to_string());
            index += 2;
            continue;
        }
        if arg.starts_with("--out-dir=") {
            args.push(format!("--out-dir={}", out_dir.display()));
            index += 1;
            continue;
        }
        // The fixed output name of a replay needs no per-build file name.
        if arg == "-C"
            && record
                .args
                .get(index + 1)
                .is_some_and(|next| next.starts_with("extra-filename="))
        {
            index += 2;
            continue;
        }
        if arg.starts_with("-Cextra-filename=") {
            index += 1;
            continue;
        }
        if arg == "--extern"
            && let Some(next) = record.args.get(index + 1)
        {
            args.push(String::from("--extern"));
            args.push(redirected(next, outputs));
            index += 2;
            continue;
        }
        if let Some(value) = arg.strip_prefix("--extern=") {
            args.push(format!("--extern={}", redirected(value, outputs)));
            index += 1;
            continue;
        }
        args.push(arg.clone());
        index += 1;
    }
    args
}

/// One `--extern` argument value with the libraries the chain already
/// replayed named where they now sit. The lookup goes by the crate name
/// the artifact carries, so a dependency renamed in a manifest keeps its
/// consumer's alias.
fn redirected(value: &str, outputs: &BTreeMap<String, PathBuf>) -> String {
    let Some((alias, path)) = value.split_once('=') else {
        return String::from(value);
    };
    let real = crate::record::crate_of_artifact(path).unwrap_or_else(|| String::from(alias));
    match outputs.get(&real) {
        Some(output) => format!("{alias}={}", output.display()),
        None => String::from(value),
    }
}

/// Links the patch library from the objects, the stub and the recorded
/// arguments.
///
/// # Errors
///
/// Returns why the step could not run at all.
fn link(
    plan: &Plan<'_>,
    objects: &[PathBuf],
    stub_object: &Path,
    thin: &[String],
    library: &Path,
) -> Result<(), String> {
    let mut args: Vec<String> = objects
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    args.push(stub_object.display().to_string());
    args.extend(
        [
            "-shared",
            // The patched code was compiled for an executable, which
            // reaches its own exported symbols directly. A shared library
            // could have them overridden, so the linker refuses those
            // references unless the library binds its definitions to
            // itself, which a patch wants anyway.
            "-Wl,-Bsymbolic",
            "-Wl,--eh-frame-hdr",
            "-Wl,-z,noexecstack",
            "-Wl,-z,relro,-z,now",
            "-nodefaultlibs",
            // Nothing reads the patch's debug information, and copying it
            // is most of the link's work.
            "-Wl,--strip-debug",
            "-Wl,-Bdynamic",
        ]
        .map(String::from),
    );
    args.extend(passthrough(thin));
    args.push(String::from("-o"));
    args.push(library.display().to_string());
    // The link answers to the directory and the environment the original
    // compile ran in, like the replayed compiles do: the search paths
    // the recorded link carried are relative to that directory, and the
    // variables the environment gave the linker selection stand only
    // there.
    let tip = plan.chain.iter().find(|step| step.tip);
    let mut command = Command::new(plan.linker);
    command.args(&args);
    if let Some(step) = tip {
        let record = &step.record;
        command
            .current_dir(&record.cwd)
            .env_clear()
            .envs(record.env.clone());
    }
    let output = command.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "linking the patch library failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// The link arguments of the recorded link that a patch library needs too:
/// search paths, libraries and linker selection.
fn passthrough(thin: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < thin.len() {
        let arg = &thin[index];
        if arg == "-L"
            && let Some(next) = thin.get(index + 1)
        {
            out.push(arg.clone());
            out.push(next.clone());
            index += 2;
            continue;
        }
        if arg.starts_with("-l")
            || arg.starts_with("-B")
            || arg.starts_with("-fuse-ld")
            || arg.contains("-ld-path")
        {
            out.push(arg.clone());
        }
        // The static or dynamic selection a native library answers to
        // rides as a linker argument: a `-l` between `-Wl,-Bstatic`
        // and `-Wl,-Bdynamic` names the archive, and the selection
        // must cross into the patch link or the shared library of the
        // same name is picked instead.
        if arg == "-Wl,-Bstatic" || arg == "-Wl,-Bdynamic" {
            out.push(arg.clone());
        }
        index += 1;
    }
    out
}

/// Makes the statics `shared` imports of the objects among `infos` that
/// define them, writing the changed objects into `dir`, so the stub's
/// symbols at the running binary's copies answer every reference to them.
///
/// # Errors
///
/// Returns why an object could not be rewritten.
fn share_statics(
    infos: &mut [ObjectInfo],
    shared: &std::collections::BTreeSet<String>,
    dir: &Path,
) -> Result<(), String> {
    if shared.is_empty() {
        return Ok(());
    }
    for info in infos {
        let dir = dir.join(&info.krate);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let defines = shared
            .iter()
            .any(|name| info.facts.symbols.get(name).is_some_and(|sym| sym.defined));
        if !defines {
            continue;
        }
        let Some(file_name) = info.path.file_name() else {
            return Err(format!(
                "{}: the object has no file name",
                info.path.display()
            ));
        };
        let out = dir.join(file_name);
        crate::elf::import_statics(&info.path, shared, &out)?;
        info.path = out;
    }
    Ok(())
}

/// The statics among `shared` whose type the replayed objects describe with
/// the same layout as the running binary: a static whose type changed,
/// even at the same size, holds the running app's data in the old layout,
/// which the patched code would read in the new one.
///
/// # Errors
///
/// Returns why a type could not be serialized.
fn same_static_types(
    shared: std::collections::BTreeSet<String>,
    infos: &mut [ObjectInfo],
    binary: &mut Signatures,
) -> Result<std::collections::BTreeSet<String>, String> {
    let mut kept = std::collections::BTreeSet::new();
    for name in shared {
        let Some(running) = binary.static_type(&name)? else {
            continue;
        };
        let mut replayed = None;
        for info in infos.iter_mut() {
            if let Some(found) = info.facts.signatures.static_type(&name)? {
                replayed = Some(found);
                break;
            }
        }
        if replayed.as_deref() == Some(running.as_str()) {
            kept.insert(name);
        }
    }
    Ok(kept)
}

/// The source name the objects' debug information gives the static
/// `name`, or the symbol name when they give none.
fn static_name(infos: &[ObjectInfo], name: &str) -> String {
    infos
        .iter()
        .find_map(|info| info.facts.signatures.static_name(name))
        .unwrap_or_else(|| String::from(name))
}

/// The writable statics `infos` define apart from the `shared` ones,
/// whose state the patched code would not see: any symbol in a writable
/// section, whatever its type, except the personality cell the ABI
/// defines under `DW.ref.`, which the program never writes.
fn fresh_statics<'a>(
    infos: impl IntoIterator<Item = &'a ObjectInfo>,
    shared: &std::collections::BTreeSet<String>,
) -> Vec<String> {
    let mut names: Vec<String> = infos
        .into_iter()
        .flat_map(|info| info.facts.symbols.map.iter())
        .filter(|(name, sym)| {
            sym.defined && sym.writable && !name.starts_with("DW.ref.") && !shared.contains(*name)
        })
        .map(|(name, _)| name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Runs `work` on a thread whose stack is as wide as the signature
/// workers' are, since the type walks recurse as deep as the types nest.
///
/// # Errors
///
/// Returns the error `work` returns, or why the thread failed.
fn on_wide_stack<T: Send>(work: impl FnOnce() -> Result<T, String> + Send) -> Result<T, String> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn_scoped(scope, work)
            .map_err(|error| format!("a type reader could not start: {error}"))?
            .join()
            .map_err(|_| String::from("a type reader stopped"))?
    })
}

/// How the type a probe names compares between the original binary and a
/// patch.
enum ProbeChange {
    /// The patch did not rebuild the probe's crate, so the type keeps the
    /// layout the original binary describes.
    NotRebuilt(Described),
    /// The patch rebuilt the probe's crate without the probe, so the type
    /// was renamed, moved or replaced.
    Gone(Described),
    /// The patch rebuilt the probe. Its description sits in the object at
    /// `index`.
    Rebuilt {
        /// The original binary's description.
        old: Described,
        /// The patch's description.
        new: Described,
        /// The object whose debug information holds `new`.
        index: usize,
    },
    /// The debug information does not describe the probe's type parameter.
    Unreadable,
    /// The type reaches two types the debug information cannot tell apart,
    /// as the error says, in the original binary when `running` is set and
    /// in the patch otherwise.
    Ambiguous {
        /// What the reader reported.
        error: String,
        /// Whether the original binary's description is the ambiguous one.
        running: bool,
    },
}

/// Compares the type parameter of the probe at `probe`, whose original
/// names are `names`, between the original binary and the patch, whose
/// objects come from the crate roots `rebuilt`.
///
/// The probe's symbol names its type, so the patch defines it under the
/// same name while the type keeps its name, and its type parameter there
/// is the new layout. A folding link can give the probe's address to other
/// functions too, so the probe is the name whose description carries a type
/// parameter, and the patch is read under that same name. A patch that did
/// not rebuild the probe's crate rebuilt nothing the type depends on.
///
/// # Errors
///
/// Returns why a type could not be read.
fn compare_probe(
    running: &mut Signatures,
    objects: &mut [ObjectInfo],
    defined: &BTreeMap<String, (Sym, usize)>,
    rebuilt: &std::collections::BTreeSet<String>,
    names: &[String],
    probe: u64,
) -> Result<ProbeChange, String> {
    let ambiguous = |error: String, running: bool| {
        if error.starts_with(crate::dwarf::AMBIGUOUS) {
            Ok(ProbeChange::Ambiguous { error, running })
        } else {
            Err(error)
        }
    };
    let mut found = None;
    for name in names {
        match running.first_type_parameter_by_name(name) {
            // Two generic functions on the probe's address cannot be told
            // apart, so the comparison fails closed.
            Ok(Some(_)) if found.is_some() => {
                return Ok(ProbeChange::Ambiguous {
                    error: String::from("several generic functions share the probe's address"),
                    running: true,
                });
            }
            Ok(Some(old)) => found = Some((Some(name), old)),
            Ok(None) => {}
            Err(error) => return ambiguous(error, true),
        }
    }
    if found.is_none() {
        match running.first_type_parameter(probe) {
            Ok(Some(old)) => found = Some((None, old)),
            Ok(None) => return Ok(ProbeChange::Unreadable),
            Err(error) => return ambiguous(error, true),
        }
    }
    let Some((probe_name, old)) = found else {
        return Ok(ProbeChange::Unreadable);
    };
    let in_patch = match probe_name {
        Some(name) => defined.get(name).map(|(_, index)| (name, *index)),
        None => names
            .iter()
            .find_map(|name| defined.get(name).map(|(_, index)| (name, *index))),
    };
    let Some((name, index)) = in_patch else {
        let root = match probe_name {
            Some(name) => running.crate_root_of_name(name)?,
            None => running.crate_root_of(probe)?,
        };
        let Some(root) = root else {
            return Ok(ProbeChange::Unreadable);
        };
        return Ok(if rebuilt.contains(&root) {
            ProbeChange::Gone(old)
        } else {
            ProbeChange::NotRebuilt(old)
        });
    };
    let new = match objects[index]
        .facts
        .signatures
        .first_type_parameter_by_name(name)
    {
        Ok(Some(new)) => new,
        Ok(None) => return Ok(ProbeChange::Unreadable),
        Err(error) => return ambiguous(error, false),
    };
    Ok(ProbeChange::Rebuilt { old, new, index })
}

/// The crate roots the patch's objects were compiled from.
///
/// # Errors
///
/// Returns why a unit could not be read.
fn rebuilt_roots(objects: &[ObjectInfo]) -> Result<std::collections::BTreeSet<String>, String> {
    let mut roots = std::collections::BTreeSet::new();
    for object in objects {
        roots.extend(object.facts.signatures.crate_roots()?);
    }
    Ok(roots)
}

/// Why the state the running app holds no longer fits the patched code, or
/// none when the state type kept its layout.
///
/// The state type is the type parameter of the state probe at `probe`, as
/// [`compare_probe`] reads it, and every type it reaches counts, since the
/// app's value holds them all in the original layout. A renamed, moved or
/// replaced state type is refused too, and so is a probe or a state type
/// the debug information does not spell, since nothing could show the
/// layout unchanged.
///
/// # Errors
///
/// Returns why a type could not be read.
fn state_change(
    plan: &mut Plan<'_>,
    objects: &mut [ObjectInfo],
    defined: &BTreeMap<String, (Sym, usize)>,
    probe: u64,
) -> Result<Option<String>, String> {
    let names = plan.originals.get(&probe).cloned().unwrap_or_default();
    let running = &mut *plan.signatures;
    on_wide_stack(move || {
        let restart = "and the running app holds its state in the old one; restart the app";
        let rebuilt = rebuilt_roots(objects)?;
        let change = compare_probe(running, objects, defined, &rebuilt, &names, probe)?;
        let (old, new, index) = match change {
            ProbeChange::NotRebuilt(_) => return Ok(None),
            ProbeChange::Rebuilt { old, new, .. } if new.digest == old.digest => return Ok(None),
            ProbeChange::Rebuilt { old, new, index } => (old, new, index),
            ProbeChange::Gone(old) => {
                return Ok(Some(format!(
                    "the state type `{}` was renamed, moved or replaced, {restart}",
                    old.name
                )));
            }
            ProbeChange::Unreadable => {
                return Ok(Some(String::from(
                    "patching is off, because the debug information of the crate that \
                     calls run_view does not describe the state probe's type parameter, \
                     so the state's layout cannot be checked; build that crate with full \
                     debug information, `debug = true` in the dev profile",
                )));
            }
            ProbeChange::Ambiguous {
                error,
                running: true,
            } => {
                return Ok(Some(format!(
                    "the state type reaches two types the debug information cannot tell \
                     apart ({error}), so no patch can be checked against it, and patching \
                     stays off until the state's types differ in name"
                )));
            }
            ProbeChange::Ambiguous {
                error,
                running: false,
            } => {
                return Ok(Some(format!(
                    "the layout of the state type cannot be told from the patch's debug \
                     information ({error}), {restart}"
                )));
            }
        };
        let before = running.shapes(&old)?;
        let after = objects[index].facts.signatures.shapes(&new)?;
        let through: Vec<String> = before
            .iter()
            .filter(|(name, shape)| {
                **name != old.name && after.get(*name).is_some_and(|new| new != *shape)
            })
            .map(|(name, _)| format!("`{name}`"))
            .collect();
        let through = if through.is_empty() {
            String::new()
        } else {
            format!(" through {}", through.join(", "))
        };
        Ok(Some(format!(
            "the state type `{}` changed its layout{through}, {restart}",
            old.name
        )))
    })
}

/// One type probe: its address as linked and its mangled name, since a
/// folding link can give two probes one address.
pub(crate) type ProbeId = (u64, String);

/// The layout the app's live values of one probe's type hold: the probe,
/// and the layout's digest, or none when it is unknown.
pub(crate) type LiveLayout = (ProbeId, Option<String>);

/// The paths of the core's modules whose types are handles into the
/// runtime of the build that made them: a value that holds one of them
/// would bring the old build's disposed signals, owners, effects, timers
/// or channels into the new build, which notify nothing.
const HANDLE_MODULES: [&str; 3] = [
    "smudge_core::reactive::",
    "smudge_core::timer::",
    "smudge_core::channel::",
];

/// Which type probes the patch keeps in the layout the app's live values
/// hold, by their addresses as linked, and the layouts the live values hold
/// once the patch is applied. A type that reaches a runtime handle or a
/// trait object is never kept.
///
/// The live layout of a probe's type is the original binary's until a
/// patch rebuilds it, and the patch's from then on, since the remount after
/// each applied patch remakes the build's values with the patch's code. A
/// type is kept only when the patch spells it in the live layout and none of
/// the types it reaches is a runtime handle. A probe the patch cannot
/// compare is not kept, and its live layout becomes unknown, so its values
/// start fresh until a patch spells the type again.
///
/// # Errors
///
/// Returns why the patch's crate roots could not be read.
fn kept_types(
    plan: &mut Plan<'_>,
    objects: &mut [ObjectInfo],
    defined: &BTreeMap<String, (Sym, usize)>,
) -> Result<(Vec<u64>, Vec<LiveLayout>), String> {
    let probes: Vec<(u64, Vec<String>)> = plan
        .type_probes
        .iter()
        .map(|(probe, name)| (*probe, vec![name.clone()]))
        .collect();
    let running = &mut *plan.signatures;
    let live = plan.type_layouts;
    let handle_free = &mut *plan.handle_free;
    on_wide_stack(move || {
        let rebuilt = rebuilt_roots(objects)?;
        let mut kept = Vec::new();
        let mut layouts = Vec::new();
        for (probe, names) in &probes {
            // A type the reader cannot answer for is not carried, and the
            // other probes still are.
            let change = compare_probe(running, objects, defined, &rebuilt, names, *probe)
                .unwrap_or(ProbeChange::Unreadable);
            let (original, spelled, index) = match change {
                ProbeChange::NotRebuilt(old) => (old.digest.clone(), old, None),
                ProbeChange::Rebuilt { old, new, index } => (old.digest, new, Some(index)),
                ProbeChange::Gone(_) | ProbeChange::Unreadable | ProbeChange::Ambiguous { .. } => {
                    layouts.push(((*probe, names[0].clone()), None));
                    continue;
                }
            };
            let holds = match live.get(&(*probe, names[0].clone())) {
                Some(layout) => layout.as_deref(),
                None => Some(original.as_str()),
            };
            let same = holds == Some(spelled.digest.as_str());
            if index.is_some() {
                layouts.push(((*probe, names[0].clone()), Some(spelled.digest.clone())));
            }
            if !same {
                continue;
            }
            let free = match handle_free.get(&spelled.digest) {
                Some(free) => *free,
                None => {
                    let shapes = match index {
                        Some(index) => objects[index].facts.signatures.shapes(&spelled),
                        None => running.shapes(&spelled),
                    };
                    // A trait object hides the type of what it holds, whose
                    // layout and handles no digest can answer for.
                    let free = shapes.is_ok_and(|shapes| {
                        shapes.iter().all(|(name, shape)| {
                            !HANDLE_MODULES.iter().any(|module| name.starts_with(module))
                                && !name.contains("dyn ")
                                && !shape.contains("dyn ")
                        })
                    });
                    handle_free.insert(spelled.digest.clone(), free);
                    free
                }
            };
            if free {
                kept.push(*probe);
            }
        }
        // The app keeps types by the probe's address, so an address two
        // folded probes share is kept only when both are.
        let mut counts: HashMap<u64, (usize, usize)> = HashMap::new();
        for (probe, _) in &probes {
            counts.entry(*probe).or_default().0 += 1;
        }
        for probe in &kept {
            counts.entry(*probe).or_default().1 += 1;
        }
        let mut listed: Vec<u64> = counts
            .into_iter()
            .filter(|(_, (all, kept))| all == kept)
            .map(|(probe, _)| probe)
            .collect();
        listed.sort_unstable();
        Ok((listed, layouts))
    })
}

/// The parts of `signature` that are not a type parameter's.
fn without_type_parameters(signature: &str) -> Vec<&str> {
    signature
        .split('|')
        .filter(|part| !part.starts_with("T:"))
        .collect()
}

/// The closure environments `signature` names among its parts, a type
/// parameter's included, with their digests.
fn closure_envs(signature: &str) -> impl Iterator<Item = (&str, &str)> {
    signature.split('|').filter_map(|part| {
        let part = part.strip_prefix("T:").unwrap_or(part);
        let (path, digest) = part.rsplit_once('#')?;
        crate::dwarf::closure_of(path).map(|_| (path, digest))
    })
}

/// Whether `signature` names a type the compiler made inside one of the
/// functions `parents` names: a closure's or an async block's environment,
/// `parent::{closure_env#N}`, or anything nested in one of them,
/// `parent::{async_block#N}::…`, and not inside a function whose path
/// merely ends like one of them.
fn names_a_closure_of(signature: &str, parents: &[String]) -> bool {
    parents.iter().any(|parent| {
        let needle = format!("{parent}::{{");
        signature.match_indices(&needle).any(|(at, _)| {
            let before = &signature[..at];
            // A path starts the signature, a part of it, or a type
            // parameter's entry, and nothing that continues another path.
            before.is_empty()
                || before.ends_with("|T:")
                || before
                    .chars()
                    .last()
                    .is_some_and(|c| !(c.is_alphanumeric() || c == '_' || c == ':'))
        })
    })
}

#[cfg(test)]
#[path = "patch_tests.rs"]
mod tests;
