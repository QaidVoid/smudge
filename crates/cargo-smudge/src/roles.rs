//! The roles the same executable plays inside a cargo build.
//!
//! The wrapper runs for every workspace crate rustc compiles, rewrites the
//! app crate's invocation, and records it. The linker shim replaces the
//! real linker in the app crate's link step: it records the arguments, and
//! in the fat build it links the original binary with every dependency
//! object kept and `main` exported.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::record::{self, Recorded};

/// The name of the variable that carries the directory the roles write
/// their records into.
pub(crate) const DIR_ENVIRONMENT: &str = "CARGO_SMUDGE_DIR";

/// The name of the variable that tells the shim which role it plays in the
/// link step: `fat` or `thin`.
pub(crate) const LINK_ENVIRONMENT: &str = "CARGO_SMUDGE_LINK";

/// The name of the variable that carries cargo's target directory, which
/// decides which rlibs a fat link extracts.
pub(crate) const TARGET_ENVIRONMENT: &str = "CARGO_SMUDGE_TARGET";

/// Plays the rustc wrapper for `args`, everything after this executable.
fn wrap_rustc(args: &[String]) -> ExitCode {
    let Some(dir) = std::env::var_os(DIR_ENVIRONMENT) else {
        return abort("the wrapper runs only inside `cargo smudge dev`");
    };
    let dir = PathBuf::from(dir);
    let Some((rustc, rest)) = args.split_first() else {
        return abort("the wrapper received no rustc to run");
    };
    // The tip is the app crate: a bin target, examples included, that
    // compiles a source file. Cargo's own probes, such as the one that
    // asks rustc which linker drivers it knows, also pass `--crate-type
    // bin`, but name no source file, and the probes must pass through.
    let tip = rest
        .windows(2)
        .any(|pair| pair[0] == "--crate-type" && pair[1] == "bin")
        && rest.iter().any(|arg| arg.ends_with(".rs"));
    if !tip {
        return run_through(rustc, rest);
    }
    if rest.iter().any(|arg| {
        arg.split_once("split-debuginfo=")
            .is_some_and(|(_, value)| value != "off")
    }) {
        return abort(
            "the profile moves the debug information out of the objects \
             with split-debuginfo, which the patch builder reads; set \
             split-debuginfo = \"off\" in the profile",
        );
    }
    let mut recorded = Vec::with_capacity(rest.len() + 1);
    recorded.push(String::from(rustc));
    // The signatures the pairing reads need full debug information, so a
    // lower setting from the profile or the flags is raised to it, in
    // either of the spellings cargo and the flags produce.
    let mut index = 0;
    while index < rest.len() {
        let arg = &rest[index];
        if arg == "-C"
            && rest
                .get(index + 1)
                .is_some_and(|next| next.starts_with("debuginfo="))
        {
            index += 2;
            continue;
        }
        if arg.starts_with("-Cdebuginfo=") {
            index += 1;
            continue;
        }
        recorded.push(arg.clone());
        index += 1;
    }
    recorded.push(String::from("-Cdebuginfo=2"));
    let entry = Recorded {
        args: recorded,
        env: record::current_env(),
        cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };
    if let Err(error) = Recorded::save(&entry, &dir.join("rustc.json")) {
        return abort(&format!(
            "the wrapper could not record the invocation: {error}"
        ));
    }
    let (program, rest) = entry
        .args
        .split_first()
        .expect("the record names its program");
    run_through(program, rest)
}

/// Plays the linker shim, with `mode` from [`LINK_ENVIRONMENT`].
fn link(mode: &str, args: &[String]) -> ExitCode {
    let Some(dir) = std::env::var_os(DIR_ENVIRONMENT) else {
        return abort("the linker shim runs only inside `cargo smudge dev`");
    };
    let dir = PathBuf::from(dir);
    if let Err(error) = record::RecordedLink::save(args, &dir.join(format!("link-{mode}.json"))) {
        return abort(&format!("the shim could not record the link: {error}"));
    }
    if mode == "thin" {
        // The replay records the objects and links nothing, which leaves
        // the running app's binary alone.
        return ExitCode::SUCCESS;
    }
    let Some(target) = std::env::var_os(TARGET_ENVIRONMENT) else {
        return abort("the shim received no target directory");
    };
    let extracted = match extract(args, Path::new(&target), &dir) {
        Ok(extracted) => extracted,
        Err(error) => return abort(&error),
    };
    let mut out: Vec<String> = args
        .iter()
        .filter(|arg| !arg.ends_with(".rlib"))
        .cloned()
        .collect();
    let at = out
        .iter()
        .rposition(|arg| arg.ends_with(".o"))
        .map_or(0, |at| at + 1);
    let mut insert: Vec<String> = extracted
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    // The sysroot's rlibs stay whole, since their members the app does not
    // use would only bloat the binary.
    insert.extend(
        args.iter()
            .filter(|arg| arg.ends_with(".rlib") && !Path::new(arg.as_str()).starts_with(&target))
            .cloned(),
    );
    out.splice(at..at, insert);
    out.push(String::from("-Wl,--export-dynamic-symbol,main"));
    let status = match Command::new(real_linker(&dir)).args(&out).status() {
        Ok(status) => status,
        Err(error) => return abort(&format!("the real linker could not run: {error}")),
    };
    ExitCode::from(status.code().unwrap_or(1) as u8)
}

/// Extracts the object members of the workspace rlibs into `dir`.
///
/// The extracted files replace the rlibs on the link line, which a linker
/// includes whole, so no code the app never calls at first is dropped. The
/// directory is cleared first, which also makes it the cache of a session
/// restart.
fn extract(args: &[String], target: &Path, dir: &Path) -> Result<Vec<PathBuf>, String> {
    let extracted = dir.join("extracted");
    let _ = std::fs::remove_dir_all(&extracted);
    std::fs::create_dir_all(&extracted).map_err(|e| format!("{}: {e}", extracted.display()))?;
    crate::elf::extract_rlib_objects(args, target, &extracted)
}

/// The linker cargo configured, or `cc`, which both a fat link and every
/// patch link run.
pub(crate) fn real_linker(dir: &Path) -> String {
    let Ok(entry) = Recorded::load(&dir.join("rustc.json")) else {
        return String::from("cc");
    };
    let self_path = std::env::current_exe()
        .map(|exe| exe.display().to_string())
        .unwrap_or_default();
    let spaced = entry.args.windows(2).filter_map(|pair| {
        (pair[0] == "-C")
            .then(|| pair[1].strip_prefix("linker="))
            .flatten()
    });
    let glued = entry
        .args
        .iter()
        .filter_map(|arg| arg.strip_prefix("-Clinker="));
    spaced
        .chain(glued)
        .find(|value| !value.is_empty() && *value != self_path)
        .map(String::from)
        .unwrap_or_else(|| String::from("cc"))
}

/// Runs `program` with `args`, unchanged, and answers its exit code.
fn run_through(program: &str, args: &[String]) -> ExitCode {
    match Command::new(program).args(args).status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
        Err(error) => abort(&format!("{program}: {error}")),
    }
}

/// Reports `message` on standard error and answers failure.
fn abort(message: &str) -> ExitCode {
    eprintln!("cargo smudge: {message}");
    ExitCode::FAILURE
}

/// Runs the role `args` selects for the process, called with everything
/// after this executable's own path.
///
/// A first argument named `rustc` is the wrapper role, which cargo also
/// uses for its version probe. A link carries an `-o` argument, which the
/// version probe does not, so the shim only takes the invocations that link
/// something and passes the rest through to the program they name.
pub(crate) fn run(args: Vec<String>) -> Option<ExitCode> {
    let program = args
        .first()
        .map(|path| {
            Path::new(path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    if program.starts_with("rustc") {
        return Some(wrap_rustc(&args));
    }
    if let Some(mode) = std::env::var_os(LINK_ENVIRONMENT) {
        if args.iter().any(|arg| arg == "-o") {
            let mode = mode.to_string_lossy().into_owned();
            return Some(link(&mode, &args));
        }
        let (program, rest) = args.split_first()?;
        return Some(run_through(program, rest));
    }
    None
}
