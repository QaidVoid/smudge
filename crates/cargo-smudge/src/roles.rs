//! The roles the same executable plays inside a cargo build.
//!
//! The wrapper runs for every workspace crate rustc compiles, rewrites the
//! invocation for full debug information, and records every crate under its
//! own name. The linker shim replaces the real linker in the app crate's
//! link step: it records the arguments, and in the fat build it links the
//! original binary with every dependency object kept and `main` exported.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::record::{self, Recorded};

/// The name of the variable that carries the directory the roles write
/// their records into.
pub(crate) const DIR_ENVIRONMENT: &str = "CARGO_SMUDGE_DIR";

/// The name of the variable that carries the build request key, which a
/// recording keeps so the driver can tell its own build's recordings
/// from the ones another request's build left in the same directory.
pub(crate) const REQUEST_ENVIRONMENT: &str = "CARGO_SMUDGE_REQUEST";

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
    // Build scripts compile for the host, and nothing of them reaches the
    // running app, but their recordings carry the dependencies their
    // expansion reads, which a patch cannot rerun.
    let bin_name = std::env::var("CARGO_BIN_NAME").ok();
    if record::compiles_a_script(rest, bin_name.as_deref()) {
        let stem =
            record::record_stem_of_args(rest).unwrap_or_else(|| String::from("build_script_build"));
        return record_and_run(&dir, &format!("rustc-unit-{stem}.json"), rustc, rest);
    }
    // The tip is the app crate: a bin target, examples included, that
    // compiles a source file. Cargo's own probes, such as the one that
    // asks rustc which linker drivers it knows, also pass `--crate-type
    // bin`, but name no source file, and the probes must pass through.
    let tip = rest
        .windows(2)
        .any(|pair| pair[0] == "--crate-type" && pair[1] == "bin")
        && rest.iter().any(|arg| arg.ends_with(".rs"));
    if tip {
        return record_and_run(&dir, "rustc.json", rustc, rest);
    }
    match record::values_in(rest, "--crate-name").next() {
        // Every other workspace crate that compiles a source file is
        // recorded under its unit's stem, so the two units a build can
        // make of one crate keep their own recordings, and the patch
        // builder sees the edges of both. The rest are probes, which
        // name no source.
        Some(name) if rest.iter().any(|arg| arg.ends_with(".rs")) => {
            let stem = record::artifact_stem_of_args(rest).unwrap_or_else(|| String::from(name));
            record_and_run(&dir, &format!("rustc-unit-{stem}.json"), rustc, rest)
        }
        _ => run_through(rustc, rest),
    }
}

/// The options whose value may ride the next argument, which the scan
/// skips: rustc reads the token after one of these as that option's
/// operand, whatever it spells, so an operand that names a codegen
/// option, say a remapping whose value starts with `-C`, answers for
/// nothing.
const OPERANDS: [&str; 32] = [
    "-A",
    "--allow",
    "--cap-lints",
    "--cfg",
    "--check-cfg",
    "--color",
    "--crate-name",
    "--crate-type",
    "-D",
    "--deny",
    "--diagnostic-width",
    "--edition",
    "--emit",
    "--error-format",
    "--explain",
    "--extern",
    "-F",
    "--forbid",
    "--force-warn",
    "--json",
    "-L",
    "-l",
    "-o",
    "--out-dir",
    "--print",
    "--remap-path-prefix",
    "--remap-path-scope",
    "--sysroot",
    "--target",
    "-W",
    "--warn",
    "-Z",
];

/// Marks each argument rustc reads as another option's operand: the
/// scan walks the invocation the way rustc does, and the token after
/// one of [`OPERANDS`] answers to that option, so the gates and the
/// rewriting must not read it as its own argument.
/// The positions in `rest` another option consumed as its operand, which
/// spell nothing of their own. The codegen gates, the recorded-metadata
/// readers and the replay rewriting share it, so a remapping or
/// configuration value that merely spells an option answers for nothing
/// anywhere. A cluster of the value-less short flags rustc lets cluster
/// can end in an option that takes a value, whose own value rides the
/// rest of the token or the next argument: `-gL next` spends `next` on
/// the search path, and `-gLdir` spends nothing after the token.
pub(crate) fn operand_mask(rest: &[String]) -> Vec<bool> {
    let mut mask = vec![false; rest.len()];
    let mut index = 0;
    while index < rest.len() {
        let takes =
            OPERANDS.contains(&rest[index].as_str()) || clustered_operand_taker(&rest[index]);
        index += 1;
        if takes && index < rest.len() {
            mask[index] = true;
            index += 1;
        }
    }
    mask
}

/// Whether the token is a short-option cluster that ends in an option
/// whose value rides the next argument, such as `-gL`: the value-less
/// flags `g`, `w` and `h` cluster, and the letters that take a value
/// (`-L`, `-l`, `-o`, `-Z` and the lint levels) spend what follows the
/// cluster on themselves. A value spelled into the token (`-gLdir`)
/// leaves the next argument alone, which the caller answers by taking
/// no next operand.
fn clustered_operand_taker(arg: &str) -> bool {
    let Some(body) = arg.strip_prefix('-') else {
        return false;
    };
    if body.is_empty() {
        return false;
    }
    let mut letters = body.chars().peekable();
    while letters
        .peek()
        .is_some_and(|letter| matches!(letter, 'g' | 'w' | 'h'))
    {
        letters.next();
    }
    let takes = letters
        .peek()
        .is_some_and(|letter| matches!(letter, 'L' | 'l' | 'o' | 'Z' | 'W' | 'D' | 'A' | 'F'));
    takes && letters.count() == 1
}

/// The `-C`/`--codegen` option a clustered short-option argument
/// carries, such as `-gCsplit_debuginfo=unpacked`: rustc reads the
/// letters after the single dash as separate short options, and the
/// ones that take no value cluster, so a `C` ends the walk and hands
/// the rest of the token to the codegen option. `-O` stays out: its
/// value rides its own spelling, not a cluster's rest.
fn clustered_codegen(arg: &str) -> Option<&str> {
    let body = arg.strip_prefix('-')?;
    for (at, letter) in body.char_indices() {
        match letter {
            'g' | 'w' | 'h' => {}
            'C' => return Some(&body[at + 'C'.len_utf8()..]),
            _ => return None,
        }
    }
    None
}

/// The `-C`/`--codegen` options an invocation carries, in the order
/// rustc reads them, with each name spelled the way rustc reads it:
/// underscores and hyphens answer the same, the value rides the
/// name after an `=`, or the argument after the name, and an
/// argument rustc reads as another option's operand answers for
/// nothing.
fn codegen_options(rest: &[String]) -> Vec<(String, Option<String>)> {
    let mask = operand_mask(rest);
    let mut found = Vec::new();
    let mut index = 0;
    while index < rest.len() {
        if mask[index] {
            index += 1;
            continue;
        }
        let arg = rest[index].as_str();
        let named = if arg == "-C" || arg == "--codegen" {
            // The value rides the next argument in the paired form.
            index += 1;
            rest.get(index).map(String::as_str)
        } else if let Some(value) = clustered_codegen(arg) {
            // A cluster that ends at the `C` takes its value from the
            // next argument, the way rustc reads it.
            if value.is_empty() {
                index += 1;
                rest.get(index).map(String::as_str)
            } else {
                Some(value)
            }
        } else {
            arg.strip_prefix("--codegen=")
        };
        index += 1;
        let Some(flag) = named else {
            continue;
        };
        let (name, value) = flag
            .split_once('=')
            .map_or((flag, None), |(name, value)| (name, Some(value)));
        found.push((name.replace('_', "-"), value.map(String::from)));
    }
    found
}

/// Whether the invocation names an LTO setting that the link answers
/// to, in either of the spellings cargo and the flags produce.
///
/// Rustc honors the last occurrence of each option and reads its name
/// under underscores as under hyphens, so the decision waits for the
/// final value of `lto` and of `linker-plugin-lto` instead of answering
/// the first one it sees. A disabled boolean is spelled `off`, `false`,
/// `no` or `n`; every other value enables the merge, and a bare option
/// enables it too.
fn names_lto(rest: &[String]) -> bool {
    let disabled = |value: &str| matches!(value, "off" | "false" | "no" | "n");
    let mut lto = false;
    let mut plugin = false;
    for (name, value) in codegen_options(rest) {
        match name.as_str() {
            "lto" => lto = value.is_none_or(|value| !disabled(&value)),
            "linker-plugin-lto" => plugin = value.is_none_or(|value| !disabled(&value)),
            _ => {}
        }
    }
    lto || plugin
}

/// Whether the invocation ends with a split-debuginfo setting that
/// moves the debug information out of the objects, which the patch
/// builder reads. Rustc honors the last occurrence of the option and
/// reads its name under underscores as under hyphens, so the gate
/// waits for the final value under one name, and an argument that
/// merely spells the option inside another value answers for nothing.
fn splits_debuginfo(rest: &[String]) -> bool {
    let mut final_value = None;
    for (name, value) in codegen_options(rest) {
        if name == "split-debuginfo" {
            final_value = Some(value);
        }
    }
    match final_value {
        // The dev profile's default keeps the debug information in the
        // objects, so an invocation that never names the option has
        // nothing to refuse.
        None => false,
        // A spelling rustc would reject cannot stand for an off
        // setting.
        Some(None) => true,
        Some(Some(value)) => value != "off",
    }
}

/// Records one crate's invocation as `file` in `dir`, with the debug
/// information raised to what the signature pairing reads, and runs rustc
/// with the rewritten arguments.
fn record_and_run(dir: &Path, file: &str, rustc: &str, rest: &[String]) -> ExitCode {
    if let Some(arg) = rest.iter().find(|arg| arg.starts_with('@')) {
        return abort(&format!(
            "the invocation reads its arguments from the response file \
             {arg}, which the wrapper cannot read the flags it gates \
             from; pass the flags on the command line or set them in \
             the profile"
        ));
    }
    if splits_debuginfo(rest) {
        return abort(
            "the profile moves the debug information out of the objects \
             with split-debuginfo, which the patch builder reads; set \
             split-debuginfo = \"off\" in the profile",
        );
    }
    // An LTO profile merges the crates at the link, which the replay of
    // a single crate cannot do again: the patch would pair signatures
    // against a binary the bitcode of every dependency reshaped, and
    // the objects it links would carry code the chain never rebuilt.
    if names_lto(rest) {
        return abort(
            "the profile links with LTO, which one replayed crate cannot \
             redo; set lto = false in the profile",
        );
    }
    let mut recorded = Vec::with_capacity(rest.len() + 1);
    recorded.push(String::from(rustc));
    // A lower debug information setting from the profile or the flags is
    // raised to the full one, in either of the spellings cargo and the
    // flags produce, and a strip setting is dropped, since the linker
    // would otherwise take the debug information back out of the binary
    // the signatures are read from, and the profile's stripping answers
    // a release binary, not the one this session patches through.
    let mask = operand_mask(rest);
    let mut index = 0;
    while index < rest.len() {
        let arg = &rest[index];
        if !mask[index] {
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
            if arg == "-C"
                && rest
                    .get(index + 1)
                    .is_some_and(|next| next.starts_with("strip="))
            {
                index += 2;
                continue;
            }
            if arg.starts_with("-Cstrip=") {
                index += 1;
                continue;
            }
        }
        recorded.push(arg.clone());
        index += 1;
    }
    recorded.push(String::from("-Cdebuginfo=2"));
    recorded.push(String::from("-Cstrip=none"));
    let entry = Recorded {
        args: recorded,
        env: record::current_env(),
        cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        request: std::env::var(REQUEST_ENVIRONMENT).ok(),
    };
    if let Err(error) = Recorded::save(&entry, &dir.join(file)) {
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
    linker_of(&entry.args, &self_path)
}

/// The linker the recorded rustc invocation answers to, or `cc` when it
/// names none: rustc honors the last `-C linker` it reads, in any of
/// the spellings cargo and the flags produce, and the wrapper's own
/// path is not a linker but the shim standing in for one.
fn linker_of(args: &[String], self_path: &str) -> String {
    codegen_options(args)
        .into_iter()
        .filter(|(name, _)| name == "linker")
        .filter_map(|(_, value)| value)
        .rfind(|value| !value.is_empty() && value != self_path)
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
    // The compiler's name in the invocation usually decides, which a
    // configured `build.rustc` under another name breaks: cargo hands
    // the configured value through as it is written, so an invocation
    // that names a crate is a rustc one whatever its program is called.
    let names_a_crate = record::values_in(&args, "--crate-name").next().is_some();
    if program.starts_with("rustc") || names_a_crate {
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

#[cfg(test)]
#[path = "roles_tests.rs"]
mod tests;
