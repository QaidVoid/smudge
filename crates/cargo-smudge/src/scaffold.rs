//! The `new` and `init` commands, which make a Smudge app with cargo's own
//! `new` and `init` and write the template over the source cargo made.
//!
//! They only run cargo and write files, so unlike `dev` they work on every
//! platform.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// The template's `src/main.rs`, which the guide shows as its first sample.
const TEMPLATE: &str = include_str!("../template/main.rs");

/// The feature table the template appends to the manifest cargo wrote, so
/// `cargo smudge dev` turns hot reload on and `cargo run` leaves it off.
const FEATURES: &str = "\n[features]\nhot-reload = [\"smudge/hot-reload\"]\n";

/// The environment variable that makes the commands write the smudge
/// dependency themselves instead of running `cargo add`, as [`make`]
/// describes.
const SKIP_ADD_ENVIRONMENT: &str = "CARGO_SMUDGE_SKIP_ADD";

/// Which of the two commands runs.
#[derive(Debug, PartialEq)]
pub(crate) enum Kind {
    /// `new`, which makes a new package at a path.
    New,
    /// `init`, which makes a package in an existing directory.
    Init,
}

/// One `new` or `init` invocation.
#[derive(Debug, PartialEq)]
pub(crate) struct Scaffold {
    /// Which command runs.
    kind: Kind,
    /// The path the package goes to, which `init` defaults to the current
    /// directory.
    path: Option<String>,
    /// The options passed through to cargo: `--name`, `--vcs` and
    /// `--edition`, each with its value.
    passed: Vec<String>,
}

/// The options the commands pass through to cargo, each taking a value.
const PASSED: [&str; 3] = ["--name", "--vcs", "--edition"];

/// Parses a `new` or `init` invocation, answering none when the arguments
/// name neither command, and the reason when they name one wrongly.
pub(crate) fn parse(args: &[String]) -> Option<Result<Scaffold, String>> {
    let kind = match args.first().map(String::as_str) {
        Some("new") => Kind::New,
        Some("init") => Kind::Init,
        _ => return None,
    };
    let mut path = None;
    let mut passed = Vec::new();
    let mut index = 1;
    while index < args.len() {
        let arg = &args[index];
        if let Some((flag, value)) = arg.split_once('=')
            && PASSED.contains(&flag)
        {
            passed.push(String::from(flag));
            passed.push(String::from(value));
            index += 1;
        } else if PASSED.contains(&arg.as_str()) {
            let Some(value) = args.get(index + 1) else {
                return Some(Err(format!("{arg} needs a value")));
            };
            passed.push(arg.clone());
            passed.push(value.clone());
            index += 2;
        } else if arg.starts_with('-') {
            return Some(Err(format!(
                "{arg} is not an option of this command, which passes only \
                 --name, --vcs and --edition to cargo"
            )));
        } else if path.is_none() {
            path = Some(arg.clone());
            index += 1;
        } else {
            return Some(Err(format!(
                "the command takes one path, and {arg} is a second"
            )));
        }
    }
    if kind == Kind::New && path.is_none() {
        return Some(Err(String::from("new needs the path of the package")));
    }
    Some(Ok(Scaffold { kind, path, passed }))
}

/// Makes the package, and answers success, or reports why it failed.
pub(crate) fn run(scaffold: &Scaffold) -> ExitCode {
    match make(scaffold) {
        Ok(dir) => {
            println!("made the Smudge app in {}", dir.display());
            println!("next steps:");
            if let Some(path) = &scaffold.path {
                println!("  cd {path}");
            }
            println!("  cargo smudge dev");
            ExitCode::SUCCESS
        }
        Err(reason) => {
            eprintln!("cargo smudge: {reason}");
            ExitCode::FAILURE
        }
    }
}

/// Makes the package and answers its directory.
///
/// The smudge dependency is added with `cargo add`, which asks the registry
/// for the version, unless [`SKIP_ADD_ENVIRONMENT`] is set, when it is
/// written as `cargo add` would write it. Only this crate's tests set it,
/// since they build offline, and it is the commands' one test-only path.
///
/// # Errors
///
/// Returns why a step failed: a file `init` would overwrite, or a cargo
/// command that failed. Once cargo has made the package, a later failure
/// also says what was left and how to start again.
fn make(scaffold: &Scaffold) -> Result<PathBuf, String> {
    let dir = PathBuf::from(scaffold.path.as_deref().unwrap_or("."));
    if scaffold.kind == Kind::Init {
        // `cargo init` would build on a source it finds, the root
        // `main.rs` included, and the template would then replace it, so
        // the command refuses as `cargo init` does for a manifest.
        for file in ["Cargo.toml", "src/main.rs", "main.rs"] {
            let found = dir.join(file);
            if found.exists() {
                return Err(format!(
                    "{} already exists, and init does not overwrite it",
                    found.display()
                ));
            }
        }
    }
    let command = match scaffold.kind {
        Kind::New => "new",
        Kind::Init => "init",
    };
    cargo(
        Path::new("."),
        [command, "--bin"]
            .into_iter()
            .map(String::from)
            .chain(scaffold.passed.iter().cloned())
            .chain(std::iter::once(dir.display().to_string())),
    )?;
    finish(&dir).map_err(|reason| match scaffold.kind {
        Kind::New => format!(
            "{reason}. Cargo made the package at {}, which is not finished: \
             remove that directory and run new again",
            dir.display()
        ),
        // The directory was there before, so only what cargo wrote in it
        // goes, which is what the refusal above looks for.
        Kind::Init => {
            let place = match &scaffold.path {
                Some(path) => path.as_str(),
                None => "the current directory",
            };
            format!(
                "{reason}. Cargo made the package in {place}, which is not \
                 finished: remove the Cargo.toml and src/main.rs it wrote \
                 there and run init again"
            )
        }
    })?;
    Ok(dir)
}

/// Adds the smudge dependency and the feature to the package cargo made in
/// `dir`, and writes the template over its source.
///
/// # Errors
///
/// Returns why a step failed.
fn finish(dir: &Path) -> Result<(), String> {
    let manifest = dir.join("Cargo.toml");
    let version = env!("CARGO_PKG_VERSION");
    if std::env::var_os(SKIP_ADD_ENVIRONMENT).is_some() {
        add_without_registry(&manifest, version)?;
    } else {
        cargo(dir, add_arguments(version))?;
    }
    append(&manifest, FEATURES)?;
    let main = dir.join("src").join("main.rs");
    std::fs::write(&main, TEMPLATE).map_err(|error| format!("{}: {error}", main.display()))
}

/// The arguments of the `cargo add` that adds smudge at `version`, run in
/// the package's directory. The app's smudge speaks the wire format of this
/// command, so it is pinned to this command's own version.
pub(crate) fn add_arguments(version: &str) -> Vec<String> {
    vec![String::from("add"), format!("smudge@={version}")]
}

/// Runs cargo in `dir` with `args`.
///
/// # Errors
///
/// Returns why cargo could not start, or that it failed, whose own
/// output already said why.
fn cargo(dir: &Path, args: impl IntoIterator<Item = String>) -> Result<(), String> {
    let args: Vec<String> = args.into_iter().collect();
    let status = Command::new("cargo")
        .current_dir(dir)
        .args(&args)
        .status()
        .map_err(|error| format!("cargo could not start: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo {} failed", args.join(" ")))
    }
}

/// Writes the smudge dependency as `cargo add` would, into the
/// `[dependencies]` table cargo wrote, for the tests that set
/// [`SKIP_ADD_ENVIRONMENT`].
///
/// # Errors
///
/// Returns why the manifest could not be read or written, or that it holds
/// no `[dependencies]` table.
fn add_without_registry(manifest: &Path, version: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(manifest)
        .map_err(|error| format!("{}: {error}", manifest.display()))?;
    let Some(at) = text.find("[dependencies]\n") else {
        return Err(format!(
            "{} has no [dependencies] table",
            manifest.display()
        ));
    };
    let end = at + "[dependencies]\n".len();
    let text = format!("{}smudge = \"={version}\"\n{}", &text[..end], &text[end..]);
    std::fs::write(manifest, text).map_err(|error| format!("{}: {error}", manifest.display()))
}

/// Appends `text` to the file at `path`, leaving what is there as it is.
///
/// # Errors
///
/// Returns why the file could not be opened or written.
fn append(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
#[path = "scaffold_tests.rs"]
mod tests;
