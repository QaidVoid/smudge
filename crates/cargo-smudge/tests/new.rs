//! Integration tests for `cargo smudge new` and `cargo smudge init`.
//!
//! Each test makes a package in a fresh directory outside this repository,
//! so cargo never adds it to this workspace, with the switch that writes
//! the smudge dependency without asking the registry. The checks then
//! build the package offline against this workspace's crates, through a
//! patch only the check passes, from this workspace's lockfile, so the
//! files the commands wrote stay as a user gets them.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Mutex;

/// Serializes the checks, which share one target directory.
static CHECK: Mutex<()> = Mutex::new(());

/// The repository's root.
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository's root could not be resolved")
}

/// A fresh, empty directory named after `name`, outside this repository,
/// removed again when the test passes.
struct Scratch(PathBuf);

impl Scratch {
    /// Makes the directory.
    fn new(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("cargo-smudge-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the scratch directory could not be made");
        let dir = dir
            .canonicalize()
            .expect("the scratch directory could not be resolved");
        // Cargo adds a package it makes inside a workspace to that
        // workspace's members, so a scratch directory in this repository
        // would change its manifest.
        assert!(
            !dir.starts_with(repository()),
            "the temporary directory {} lies in the repository",
            dir.display()
        );
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

/// Runs `cargo smudge` with `args` in `dir`.
fn smudge(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-smudge"))
        .current_dir(dir)
        .env("CARGO_SMUDGE_SKIP_ADD", "1")
        .args(args)
        .output()
        .expect("cargo smudge could not start")
}

/// Checks the package in `dir` offline against this workspace's crates,
/// and answers cargo's output.
fn check(dir: &Path) -> Output {
    let _check = CHECK.lock().unwrap_or_else(|lock| lock.into_inner());
    std::fs::copy(repository().join("Cargo.lock"), dir.join("Cargo.lock"))
        .expect("the workspace's lockfile could not be copied");
    let smudge = repository().join("crates/smudge");
    Command::new("cargo")
        .current_dir(dir)
        .env(
            "CARGO_TARGET_DIR",
            Path::new(env!("CARGO_TARGET_TMPDIR")).join("scaffold-target"),
        )
        .args([
            "check",
            "--offline",
            "--config",
            &format!(
                "patch.crates-io.smudge.path={:?}",
                smudge.display().to_string()
            ),
        ])
        .output()
        .expect("cargo could not start")
}

/// Panics with cargo's output unless `output` succeeded.
fn assert_ok(output: &Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Panics unless the package in `dir`, named `demo`, names this command's
/// smudge version and the `hot-reload` feature, carries the template, and
/// checks without warnings of its own.
fn assert_template(dir: &Path) {
    let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).expect("a manifest");
    assert!(
        manifest.contains(&format!("smudge = \"={}\"", env!("CARGO_PKG_VERSION"))),
        "the manifest does not pin this command's smudge version:\n{manifest}"
    );
    assert!(
        manifest.contains("hot-reload = [\"smudge/hot-reload\"]"),
        "the manifest has no hot-reload feature:\n{manifest}"
    );
    let main = std::fs::read_to_string(dir.join("src/main.rs")).expect("a main");
    assert_eq!(
        main,
        include_str!("../template/main.rs"),
        "the main is not the template"
    );
    let output = check(dir);
    assert_ok(&output, "cargo check");
    // Cargo closes the warnings of each package with a line that names it.
    let errors = String::from_utf8_lossy(&output.stderr);
    assert!(
        !errors.contains("warning: `demo`"),
        "the template warns:\n{errors}"
    );
}

#[test]
fn new_makes_an_app_that_checks() {
    let scratch = Scratch::new("new");
    let output = smudge(&scratch.0, &["new", "demo", "--vcs", "none"]);
    assert_ok(&output, "cargo smudge new");
    let said = String::from_utf8_lossy(&output.stdout);
    assert!(
        said.contains("cd demo") && said.contains("cargo smudge dev"),
        "the command named no next steps:\n{said}"
    );
    assert_template(&scratch.0.join("demo"));
}

#[test]
fn init_makes_an_app_in_an_empty_directory() {
    let scratch = Scratch::new("init");
    let output = smudge(&scratch.0, &["init", "--name", "demo", "--vcs", "none"]);
    assert_ok(&output, "cargo smudge init");
    assert_template(&scratch.0);
}

#[test]
fn init_overwrites_no_source() {
    for source in ["src/main.rs", "main.rs"] {
        let scratch = Scratch::new("init-over");
        let dir = &scratch.0;
        let path = dir.join(source);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a source directory");
        std::fs::write(&path, "fn main() {}\n").expect("a source");
        let output = smudge(dir, &["init", "--vcs", "none"]);
        assert!(!output.status.success(), "init went ahead over {source}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(source),
            "the refusal does not name {source}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("the source"),
            "fn main() {}\n",
            "init touched {source}"
        );
        assert!(!dir.join("Cargo.toml").exists(), "init wrote a manifest");
    }
}
