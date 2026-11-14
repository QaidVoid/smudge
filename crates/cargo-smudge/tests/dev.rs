//! Integration tests for the `dev` session, on Linux.
//!
//! Each test copies the fixture app to a temporary directory, runs the
//! session in it with a target directory of its own, edits the fixture's
//! sources like an editor would, and reads what the session and the fixture
//! print. The fixture is headless: it applies every patch and calls its
//! probes through the installed table.

#![cfg(target_os = "linux")]

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The tests share one build lock, so the cargo builds of one test never
/// hold a lock another test's patch step waits behind.
static BUILD: Mutex<()> = Mutex::new(());

/// How long a cold fat build of the fixture may take.
const FAT_TIMEOUT: Duration = Duration::from_secs(300);

/// How long one patch step or one wait may take.
const STEP_TIMEOUT: Duration = Duration::from_secs(120);

/// A running `dev` session with its output collected in the background.
struct Session {
    child: Child,
    out: Arc<Mutex<String>>,
    err: Arc<Mutex<String>>,
    stdin: Option<ChildStdin>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Session {
    /// Starts a session for the app in `app`, building into `target`, which
    /// the tests share so its dependency artifacts are built once.
    fn start(app: &Path, target: &Path) -> Session {
        let mut child = Command::new(env!("CARGO_BIN_EXE_cargo-smudge"))
            .arg("dev")
            .current_dir(app)
            .env("CARGO_TARGET_DIR", target)
            .env("CARGO_NET_OFFLINE", "true")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the session could not start");
        let out = collect(child.stdout.take().expect("no stdout"));
        let err = collect(child.stderr.take().expect("no stderr"));
        let stdin = child.stdin.take();
        Session {
            child,
            out,
            err,
            stdin,
        }
    }

    /// Waits until the output holds `count` occurrences of `needle`, and
    /// answers the whole output. Panics with both streams when the wait
    /// times out.
    fn wait(&self, needle: &str, count: usize, timeout: Duration) -> String {
        let started = Instant::now();
        loop {
            let out = self.output();
            if out.matches(needle).count() >= count {
                return out;
            }
            if started.elapsed() > timeout {
                panic!(
                    "waited {timeout:?} for the {count}. {needle:?} and read:\n\
                     --- stdout ---\n{}\n--- stderr ---\n{}",
                    out,
                    self.errors()
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// The output collected so far.
    fn output(&self) -> String {
        self.out.lock().expect("a lock").clone()
    }

    /// The errors collected so far.
    fn errors(&self) -> String {
        self.err.lock().expect("a lock").clone()
    }

    /// Asks the session to quit and waits for it to end.
    fn quit(&mut self) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = writeln!(stdin, "q");
            let _ = stdin.flush();
        }
        self.stdin = None;
        let started = Instant::now();
        while self.child.try_wait().ok().flatten().is_none() {
            if started.elapsed() > STEP_TIMEOUT {
                panic!("the session ignored the request to quit");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Appends everything `stream` produces to one shared string, on a thread of
/// its own, holding the lock only while a read chunk is pushed.
fn collect(stream: impl std::io::Read + Send + 'static) -> Arc<Mutex<String>> {
    let shared = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&shared);
    std::thread::spawn(move || {
        let mut reader = stream;
        let mut chunk = [0; 4096];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    sink.lock()
                        .expect("a lock")
                        .push_str(&String::from_utf8_lossy(&chunk[..n]));
                }
            }
        }
    });
    shared
}

/// Copies the fixture app to a fresh directory and answers the app's
/// directory.
fn fixture(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the temporary directory could not be made");
    let app = dir.join("app");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/app"),
        &app,
    );
    app
}

/// Copies `from` and everything under it to `to`.
fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("the copy could not be made");
    for entry in fs::read_dir(from).expect("the fixture could not be read") {
        let entry = entry.expect("the fixture could not be read");
        let target = to.join(entry.file_name());
        if entry
            .file_type()
            .expect("the fixture could not be read")
            .is_dir()
        {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("the fixture could not be copied");
        }
    }
}

/// Edits the fixture's `src/main.rs` with every replacement at once, in one
/// write, so the session never sees a half-edited source. Fails when a
/// `from` is not in the file exactly once.
fn edit(app: &Path, edits: &[(&str, &str)]) {
    let path = app.join("src/main.rs");
    let mut source = fs::read_to_string(&path).expect("the source could not be read");
    for (from, to) in edits {
        assert_eq!(
            source.matches(from).count(),
            1,
            "the edit needs exactly one match of {from:?}"
        );
        source = source.replacen(from, to, 1);
    }
    fs::write(&path, source).expect("the source could not be written");
}

/// The `total` of an applied patch line, in milliseconds.
fn total_millis(line: &str) -> u128 {
    let at = line
        .find("total ")
        .expect("the applied line names no total")
        + "total ".len();
    let end = line[at..]
        .find([',', '\n'])
        .expect("the applied line ends oddly");
    let value = line[at..at + end].trim();
    let at = value
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .expect("the total is not a duration");
    let number: f64 = value[..at].parse().expect("the total is not a number");
    let unit = value[at..].trim();
    let millis = match unit.trim() {
        "s" => number * 1000.0,
        "ms" => number,
        "µs" | "us" => number / 1000.0,
        "ns" => number / 1_000_000.0,
        other => panic!("the total names an unknown unit {other:?}"),
    };
    millis as u128
}

/// The count of entries in an applied patch line.
fn entries_of(line: &str) -> usize {
    line.split(" entries")
        .next()
        .and_then(|head| head.rsplit(' ').next())
        .and_then(|count| count.parse().ok())
        .expect("the applied line names its entries")
}

/// Skips with a printed reason when no C linker is present, as the session
/// cannot link without one.
fn require_linker() {
    if Command::new("cc").arg("--version").output().is_err() {
        panic!("skipped, because no C linker named `cc` is present");
    }
}

/// Starts a session for a fresh fixture copy and waits for it to run.
fn start(name: &str) -> (PathBuf, Session) {
    let app = fixture(name);
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    (app, session)
}

/// The one line of the output that carries `marker`.
fn line_of(output: &str, marker: &str) -> String {
    output
        .lines()
        .find(|line| line.contains(marker))
        .expect("the line is there")
        .to_owned()
}

#[test]
fn patches_a_function_body() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let (app, mut session) = start("body");
    edit(&app, &[("\"alpha-1\"", "\"alpha-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    let millis = total_millis(&line);
    assert!(
        millis < 600,
        "the patch took {millis} ms, over the 0.6 s the story promises: {line}"
    );
    let out = session.wait("fixture alpha=alpha-2", 1, STEP_TIMEOUT);
    assert!(
        out.contains("alpha_hot=true"),
        "the body did not run patched:\n{out}"
    );
    session.quit();
}

#[test]
fn maps_an_added_function() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let (app, mut session) = start("added");
    edit(
        &app,
        &[(
            "// BETA-FN-START",
            "\n#[inline(never)]\nfn gamma() -> &'static str {\n    \"gamma-1\"\n}\n\n// BETA-FN-START",
        )],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    assert!(
        line.contains("1 new"),
        "the added function was not counted as new: {line}"
    );
    session.quit();
}

#[test]
fn leaves_a_removed_function_out_of_the_table() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let (app, mut session) = start("removed");
    edit(&app, &[("\"alpha-1\"", "\"alpha-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let entries = entries_of(&line_of(&out, "patch 1: applied"));
    edit(
        &app,
        &[
            (
                "// BETA-FN-START\n#[inline(never)]\nfn beta() -> &'static str {\n    \"beta-1\"\n}\n// BETA-FN-END\n\n",
                "",
            ),
            (
                "        // BETA-REPORT-START\n        let (beta_value, beta_hot) = through(table, beta_address);\n        println!(\"fixture beta={beta_value} beta_hot={beta_hot}\");\n        // BETA-REPORT-END\n",
                "",
            ),
            ("    let beta_address = beta as *const () as usize;\n", ""),
        ],
    );
    let out = session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    let after = entries_of(&line_of(&out, "patch 2: applied"));
    // The removal drops beta from the table, and other functions whose
    // generic instantiations the new build inlined away may drop too, so
    // only a loss is promised.
    assert!(
        after < entries,
        "removing a function left the entry count unchanged"
    );
    assert!(
        session.output().contains("fixture alpha=alpha-2"),
        "the app did not keep working after the removal"
    );
    session.quit();
}

#[test]
fn omits_a_changed_signature_and_applies_the_rest() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let (app, mut session) = start("helper-signature");
    edit(
        &app,
        &[(
            "fn helper(x: u32) -> u32 {\n    x + 1\n}",
            "fn helper(x: u32, y: u32) -> u32 {\n    x + y\n}",
        )],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    assert!(
        line.contains("1 omitted for a changed signature"),
        "the changed helper was not omitted: {line}"
    );
    let out = session.wait("fixture alpha=alpha-1", 1, STEP_TIMEOUT);
    assert!(
        out.contains("alpha_hot=true"),
        "the rest of the patch did not apply:\n{out}"
    );
    // The helper itself must stay cold, running the version the binary
    // was built with, which is what a changed signature promises.
    assert!(
        out.contains("helper=2 helper_hot=false"),
        "the incompatible helper ran anyway:\n{out}"
    );
    session.quit();
}

#[test]
fn refuses_a_view_signature_change_and_keeps_running() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let (app, mut session) = start("view-signature");
    edit(
        &app,
        &[(
            "fn view() -> &'static str {\n    \"view-1\"\n}",
            "fn view(x: u32) -> &'static str {\n    \"view-1\"\n}",
        )],
    );
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("changed its signature"),
        "the refusal names no signature change:\n{out}"
    );
    // The session survives the refusal, and the next good change patches.
    edit(&app, &[("\"alpha-1\"", "\"alpha-2\"")]);
    let out = session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("fixture alpha=alpha-2"),
        "the app did not take the patch after the refused one"
    );
    session.quit();
}

#[test]
fn recovers_from_a_compile_error() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let (app, mut session) = start("compile-error");
    edit(
        &app,
        &[("\"alpha-1\"", "\"alpha-2\" } fn broken() -> u32 {")],
    );
    session.wait("patch 1: compile failed", 1, STEP_TIMEOUT);
    assert!(
        session.errors().contains("error"),
        "the compile error was not printed"
    );
    assert!(
        !session.output().contains("the app exited"),
        "the app exited on a compile error"
    );
    edit(
        &app,
        &[("\"alpha-2\" } fn broken() -> u32 {", "\"alpha-2\"")],
    );
    let out = session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("fixture alpha=alpha-2"),
        "the fixed source did not patch:\n{out}"
    );
    session.quit();
}

#[test]
fn restarts_on_a_manifest_change() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let (app, mut session) = start("manifest");
    let path = app.join("Cargo.toml");
    let manifest = fs::read_to_string(&path).expect("the manifest could not be read");
    fs::write(&path, format!("{manifest}\n# a test touched this\n"))
        .expect("the manifest could not be written");
    session.wait("the build inputs changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 2, FAT_TIMEOUT);
    assert!(
        !session.output().contains("the app exited"),
        "the restart reported the app as exited"
    );
    session.quit();
}
