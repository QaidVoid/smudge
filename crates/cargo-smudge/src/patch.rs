//! The patch step: replays the chain of crates an edit touched, verifies
//! the signatures of every function it would map, refuses the crates it
//! must not rebuild, builds the stub, links the patch library and sends
//! the table.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::dwarf::Signatures;
use crate::elf::{Sym, Symbols, TlsTemplate};
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
        /// Functions left out because their signature changed.
        changed_signature: usize,
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
    /// The address `main` was linked at, which the table is built against.
    pub main: u64,
    /// The address `main` runs at, which slides the stub's targets.
    pub runtime_main: u64,
    /// The linked address of the view the app announced, when the hello
    /// carried it.
    pub view: Option<u64>,
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
    // Every replay writes into a directory of its own, so cargo's
    // artifacts keep the fingerprints of the build they came from, and
    // the directory is emptied before the step, which a killed session
    // can have left it full for, and removed after it, either way. Its
    // name stays the same from patch to patch: the compiler reuses its
    // incremental cache only for the output directory it last wrote, and
    // a new one each time made every replay compile from scratch, 0.84 to
    // 0.97 s against 0.19 s for an edit of the `hot` example's view.
    let replay_dir = plan.dir.join("replay");
    let _ = std::fs::remove_dir_all(&replay_dir);
    let outcome = step(plan, &replay_dir, generation);
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

/// One object a patch links, with what its symbols and its debug
/// information answer.
struct ObjectInfo {
    /// Where the object sits, for the link line.
    path: PathBuf,
    /// The crate the object belongs to.
    krate: String,
    /// Every symbol the object names.
    symbols: Symbols,
    /// The signatures its debug information describes.
    signatures: Signatures,
}

/// Replays the chain, collects what it produced, and sends the table.
/// A failure after some steps compiled keeps what those steps learned
/// about their sources, so the session can still watch them.
fn step(plan: &mut Plan<'_>, replay_dir: &Path, generation: u32) -> Outcome {
    let mut discovered: Vec<(String, Vec<PathBuf>)> = Vec::new();
    match step_inner(plan, replay_dir, generation, &mut discovered) {
        Ok(outcome) => outcome,
        Err(reason) => Outcome::BuildFailed { reason, discovered },
    }
}

/// The body of [`step`], which hands its discoveries back through
/// `discovered` even when it fails.
fn step_inner(
    plan: &mut Plan<'_>,
    replay_dir: &Path,
    generation: u32,
    discovered: &mut Vec<(String, Vec<PathBuf>)>,
) -> Result<Outcome, String> {
    let started = Instant::now();
    std::fs::create_dir_all(replay_dir).map_err(|e| format!("{}: {e}", replay_dir.display()))?;
    // The replayed library of each crate, which the `--extern` arguments
    // of the crates after it point at.
    let mut outputs: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut objects: Vec<ObjectInfo> = Vec::new();
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
            // The objects share nothing, so their reads run in parallel;
            // the results land in the paths' order either way. Eight
            // workers at most: more would only add threads to wait on.
            let workers = paths.len().clamp(1, 8);
            let chunk = paths.len().div_ceil(workers).max(1);
            let mut objects_read = Vec::with_capacity(paths.len());
            std::thread::scope(|scope| -> Result<(), String> {
                let mut handles = Vec::new();
                for group in paths.chunks(chunk) {
                    handles.push(
                        std::thread::Builder::new()
                            .spawn_scoped(scope, move || {
                                group
                                    .iter()
                                    .map(|path| read_object(path))
                                    .collect::<Result<Vec<_>, String>>()
                            })
                            .map_err(|error| {
                                format!("an object reader could not start: {error}")
                            })?,
                    );
                }
                for handle in handles {
                    let mut group = handle
                        .join()
                        .map_err(|_| String::from("an object reader stopped"))??;
                    objects_read.append(&mut group);
                }
                Ok(())
            })?;
            for info in &mut objects_read {
                info.krate = step.name.clone();
            }
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
                let mut infos = Vec::with_capacity(extracted.len());
                for path in &extracted {
                    let info = read_object(path)?;
                    let tls = info
                        .symbols
                        .map
                        .values()
                        .any(|sym| sym.kind == object::SymbolKind::Tls);
                    if tls {
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
                    infos.push(info);
                }
                // A static every reference reaches through a full address
                // is the running app's own in the patch; the others the
                // patch would carry fresh copies of.
                for info in &mut infos {
                    info.krate = step.name.clone();
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
    let paths: Vec<PathBuf> = objects.iter().map(|info| info.path.clone()).collect();
    let shared = crate::elf::shared_statics(&paths, plan.symbols)?;
    let shared = same_static_types(shared, &mut objects, plan.signatures)?;
    share_statics(&mut objects, &shared, &replay_dir.join("shared"))?;
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
        let fresh: Vec<String> = fresh_statics(objects.iter().filter(|info| &info.krate == krate))
            .iter()
            .map(|name| static_name(&objects, name))
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
    let app_fresh: Vec<(String, String)> =
        fresh_statics(objects.iter().filter(|info| info.krate == tip_name))
            .into_iter()
            .map(|name| {
                let shown = static_name(&objects, &name);
                (name, shown)
            })
            .collect();

    // Every object of the replay keeps its own signatures, because an
    // address is relative to its own object: the same number names a
    // different function in each of them.
    let mut defined: BTreeMap<String, (Sym, usize)> = BTreeMap::new();
    let mut imported: BTreeMap<String, ()> = BTreeMap::new();
    for (index, object) in objects.iter().enumerate() {
        for (name, sym) in &object.symbols.map {
            if sym.defined {
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

    // Every replayed object answers the signatures of its own text
    // symbols before the pairing walks them, so the per-object work,
    // which shares nothing between objects, runs in parallel and the
    // walk against the binary's signatures stays serial.
    let mut per_object: Vec<Vec<String>> = vec![Vec::new(); objects.len()];
    for (name, (_, index)) in &defined {
        per_object[*index].push(name.clone());
    }
    let workers = objects.len().clamp(1, 8);
    let chunk = objects.len().div_ceil(workers).max(1);
    let mut new_signatures: Vec<HashMap<String, Option<String>>> =
        (0..objects.len()).map(|_| HashMap::new()).collect();
    std::thread::scope(|scope| -> Result<(), String> {
        let mut handles = Vec::new();
        for (offset, group) in objects.chunks_mut(chunk).enumerate() {
            let names = &per_object[offset * chunk..];
            handles.push(
                std::thread::Builder::new()
                    .stack_size(64 * 1024 * 1024)
                    .spawn_scoped(scope, move || -> Result<AnsweredSignatures, String> {
                        let mut answered = Vec::with_capacity(group.len());
                        for (local, object) in group.iter_mut().enumerate() {
                            let mut map = HashMap::new();
                            for name in &names[local] {
                                let signature = object.signatures.signature_by_name(name)?;
                                map.insert(name.clone(), signature);
                            }
                            answered.push((offset * chunk + local, map));
                        }
                        Ok(answered)
                    })
                    .map_err(|error| format!("a signature reader could not start: {error}"))?,
            );
        }
        let mut answers = Vec::new();
        for handle in handles {
            answers.push(
                handle
                    .join()
                    .map_err(|_| String::from("a signature reader stopped"))??,
            );
        }
        for group in answers {
            for (index, map) in group {
                new_signatures[index] = map;
            }
        }
        Ok(())
    })?;

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
    std::thread::scope(|scope| -> Result<(), String> {
        let answers = &mut old_signatures;
        let signatures = &mut *plan.signatures;
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn_scoped(scope, move || -> Result<(), String> {
                for (name, address) in &wanted {
                    let signature = signatures.signature(*address)?;
                    answers.insert(name.clone(), signature);
                }
                Ok(())
            })
            .map_err(|error| format!("a signature reader could not start: {error}"))?
            .join()
            .map_err(|_| String::from("a signature reader stopped"))??;
        Ok(())
    })?;

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
    for object in &objects {
        for (parent, numbers) in object.signatures.closures() {
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
        let Some(new_signature) = new_signatures[*index].get(name) else {
            continue;
        };
        match (old_signature, new_signature.as_deref()) {
            (Some(old_sig), Some(new_sig)) if old_sig == new_sig => {
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
    // A signature change the app announced no view for, or one that lands
    // on the view, is refused once, so the app learns it must rebuild; the
    // later patches omit those functions and keep going.
    if !changed.is_empty() {
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
    let mut originals: BTreeMap<u64, Vec<&str>> = BTreeMap::new();
    for (name, sym) in &plan.symbols.map {
        if sym.defined && sym.kind == object::SymbolKind::Text {
            originals
                .entry(sym.address)
                .or_default()
                .push(name.as_str());
        }
    }
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
        let Some(names) = originals.get(old) else {
            continue;
        };
        let agreed = entries.windows(2).all(|pair| pair[0].1 == pair[1].1);
        let complete =
            names.len() == entries.len() && entries.iter().all(|(name, _)| names.contains(name));
        if agreed && complete {
            table.insert(*old, entries[0].1);
        } else {
            shared.extend_from_slice(names);
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
    let entries = table.len();
    let mut payload = Vec::new();
    payload.extend_from_slice(&plan.main.to_le_bytes());
    payload.extend_from_slice(&new_main.to_le_bytes());
    payload.extend_from_slice(&(entries as u64).to_le_bytes());
    for (old, new) in &table {
        payload.extend_from_slice(&old.to_le_bytes());
        payload.extend_from_slice(&new.to_le_bytes());
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
            changed_signature: changed.len(),
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

/// The signatures one thread answers for its objects, by object index.
type AnsweredSignatures = Vec<(usize, HashMap<String, Option<String>>)>;

/// Reads one object's symbols and signatures.
fn read_object(path: &Path) -> Result<ObjectInfo, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(ObjectInfo {
        path: path.to_path_buf(),
        krate: String::new(),
        symbols: Symbols::read(&bytes)?,
        signatures: Signatures::read(&bytes)?,
    })
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
            "-Wl,--eh-frame-hdr",
            "-Wl,-z,noexecstack",
            "-Wl,-z,relro,-z,now",
            "-nodefaultlibs",
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
            .any(|name| info.symbols.get(name).is_some_and(|sym| sym.defined));
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
        for name in shared {
            if let Some(sym) = info.symbols.map.get_mut(name) {
                sym.defined = false;
            }
        }
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
            if let Some(found) = info.signatures.static_type(&name)? {
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
        .find_map(|info| info.signatures.static_name(name))
        .unwrap_or_else(|| String::from(name))
}

/// The writable statics `infos` still define, whose state the patched
/// code would not see: any symbol in a writable section, whatever its
/// type, except the personality cell the ABI defines under `DW.ref.`,
/// which the program never writes.
fn fresh_statics<'a>(infos: impl IntoIterator<Item = &'a ObjectInfo>) -> Vec<String> {
    let mut names: Vec<String> = infos
        .into_iter()
        .flat_map(|info| info.symbols.map.iter())
        .filter(|(name, sym)| sym.defined && sym.writable && !name.starts_with("DW.ref."))
        .map(|(name, _)| name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
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
