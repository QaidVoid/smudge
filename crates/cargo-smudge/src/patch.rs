//! The patch step: replays the app crate's compilation, verifies the
//! signatures of every function it would map, builds the stub, links the
//! patch library and sends the table.

use std::collections::BTreeMap;
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
        /// Functions left out whose signature could not be verified.
        unverified: usize,
        /// Imports the stub resolved to the running binary's code.
        trampolines: usize,
        /// Imports the stub resolved to the running binary's data.
        absolutes: usize,
        /// Imports the stub gave copies of the binary's thread-local
        /// initialization image.
        tls_copies: usize,
        /// Imports the binary itself does not define, which the dynamic
        /// linker resolves when the library loads.
        missing: usize,
    },
    /// The replayed compilation failed, which the app does not see.
    CompileFailed {
        /// The compiler's output.
        log: String,
    },
    /// The patch could not be built, which the app does not see.
    BuildFailed {
        /// Why.
        reason: String,
    },
    /// Nothing was sent, because the change needs a restart.
    RestartNeeded {
        /// Why the change cannot patch.
        reason: String,
    },
    /// The patch was sent and the app refused it.
    AppRefused {
        /// The app's reason.
        reason: String,
    },
    /// The connection broke while one frame went over it, so nothing can
    /// be trusted over it anymore and the session drops it.
    ConnectionLost {
        /// Why the connection broke.
        reason: String,
    },
}

/// Everything one patch step needs from its session.
pub(crate) struct Plan<'a> {
    /// The app crate's recorded invocation, replayed on every change.
    pub rustc: &'a Recorded,
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
    /// stub copies for the thread-locals the patch imports.
    pub tls: &'a TlsTemplate,
    /// The builder's end of the connection.
    pub stream: &'a mut std::os::unix::net::UnixStream,
}

/// Runs one patch step for `generation`.
///
/// # Errors
///
/// Returns why the step could not run at all, which is a session error
/// rather than an outcome.
pub(crate) fn run(plan: &mut Plan<'_>, generation: u32) -> Result<Outcome, String> {
    let started = Instant::now();
    let (compile, objects, thin) = match replay(plan.rustc, plan.dir)? {
        Ok(parts) => parts,
        Err(log) => return Ok(Outcome::CompileFailed { log }),
    };
    // Every object of the replay keeps its own signatures, because an
    // address is relative to its own object: the same number names a
    // different function in each of them.
    let mut signatures = Vec::new();
    let mut defined: BTreeMap<String, (Sym, usize)> = BTreeMap::new();
    let mut imported: BTreeMap<String, ()> = BTreeMap::new();
    for (index, object) in objects.iter().enumerate() {
        let bytes = std::fs::read(object).map_err(|e| format!("{}: {e}", object.display()))?;
        let symbols = Symbols::read(&bytes)?;
        signatures.push(Signatures::read(&bytes)?);
        for (name, sym) in symbols.map {
            if sym.defined {
                defined.insert(name, (sym, index));
            } else {
                imported.insert(name, ());
            }
        }
    }
    let imports: Vec<String> = imported
        .into_keys()
        .filter(|name| !defined.contains_key(name))
        .collect();
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
        let old_signature = plan.signatures.signature(old.address)?;
        // The object's addresses cannot be compared against the binary's,
        // so its side of the pairing goes by the symbol's name.
        let new_signature = signatures[*index].signature_by_name(name)?;
        match (&old_signature, &new_signature) {
            (Some(old_sig), Some(new_sig)) if old_sig == new_sig => {
                tabled.push((name.clone(), old.address));
            }
            (Some(_), Some(_)) => {
                eprintln!(
                    "debug: {name} changed: old {:?} new {:?} at object {index}",
                    old_signature, new_signature
                );
                changed.push((name.clone(), old.address))
            }
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
    if let Err(reason) = link(plan, &objects, &stub_object, &thin, &library) {
        return Ok(Outcome::BuildFailed { reason });
    }
    let lib_bytes = std::fs::read(&library).map_err(|e| format!("{}: {e}", library.display()))?;
    let lib_symbols = Symbols::read(&lib_bytes)?;
    let Some(new_main) = lib_symbols
        .get("main")
        .filter(|sym| sym.defined && sym.kind == object::SymbolKind::Text)
        .map(|sym| sym.address)
    else {
        return Ok(Outcome::BuildFailed {
            reason: String::from("the patch library exports no main"),
        });
    };
    let mut table: BTreeMap<u64, u64> = BTreeMap::new();
    for (name, old) in &tabled {
        let Some(new) = lib_symbols
            .get(name)
            .filter(|sym| sym.defined && sym.kind == object::SymbolKind::Text)
            .map(|sym| sym.address)
        else {
            continue;
        };
        if new != 0 {
            table.insert(*old, new);
        }
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
        });
    }
    let outcome = match wire::read_frame(plan.stream) {
        Ok(Some((APPLIED, _))) => Outcome::Applied {
            compile,
            total: started.elapsed(),
            entries,
            added,
            changed_signature: changed.len(),
            unverified,
            trampolines,
            absolutes,
            tls_copies,
            missing,
        },
        Ok(Some((FAILED, reason))) => Outcome::AppRefused {
            reason: String::from_utf8_lossy(&reason).into_owned(),
        },
        Ok(Some((kind, _))) => Outcome::AppRefused {
            reason: format!("the app answered with the unknown frame kind {kind}"),
        },
        // A closed connection is not a refusal: the app may have died,
        // and supervision notices the child's exit and ends the session
        // if it did, so the caller drops the stream rather than reading
        // a late answer into a later patch.
        Ok(None) => Outcome::ConnectionLost {
            reason: String::from("the app closed the connection"),
        },
        // A frame that never arrives, or one that arrives broken, leaves
        // the connection's framing untrustworthy, so the caller drops it
        // rather than reading a late answer into a later patch.
        Err(error) => Outcome::ConnectionLost {
            reason: format!("the app did not answer the patch: {error}"),
        },
    };
    // The replay's objects are stale once the library is linked, and the
    // next replay writes fresh ones.
    for object in &objects {
        let _ = std::fs::remove_file(object);
    }
    Ok(outcome)
}

/// Replays the recorded compilation, answering how long it took, the
/// objects it produced and the link arguments it recorded.
///
/// # Errors
///
/// Returns why the replay could not run at all.
/// What one replay produced: either its duration, objects and arguments,
/// or the compiler log of a failed build.
type Replayed = Result<(Duration, Vec<PathBuf>, Vec<String>), String>;

fn replay(rustc: &Recorded, dir: &Path) -> Result<Replayed, String> {
    let started = Instant::now();
    let output = Command::new(&rustc.args[0])
        .args(&rustc.args[1..])
        .current_dir(&rustc.cwd)
        .env_clear()
        .envs(rustc.env.clone())
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("RUSTC_WRAPPER")
        .env(crate::roles::LINK_ENVIRONMENT, "thin")
        .output()
        .map_err(|e| e.to_string())?;
    let log = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        return Ok(Err(log));
    }
    let thin = crate::record::RecordedLink::load(&dir.join("link-thin.json"))?;
    let objects: Vec<PathBuf> = thin
        .iter()
        .filter(|arg| arg.ends_with(".rcgu.o"))
        .map(PathBuf::from)
        .collect();
    if objects.is_empty() {
        return Err(String::from(
            "the replay recorded no objects, which means the recorded \
             invocation no longer matches the app",
        ));
    }
    Ok(Ok((started.elapsed(), objects, thin)))
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
    let output = Command::new(plan.linker)
        .args(&args)
        .output()
        .map_err(|e| e.to_string())?;
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
        index += 1;
    }
    out
}
