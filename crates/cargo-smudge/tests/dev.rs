//! Integration tests for the `dev` session, on Linux.
//!
//! Each test copies the fixture app to a temporary directory, runs the
//! session in it with a target directory of its own, edits the fixture's
//! sources like an editor would, and reads what the session and the fixture
//! print. The fixture is headless: it applies every patch and calls its
//! probes through the installed table.

#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

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
        Session::start_with(app, target, &[])
    }

    /// Like [`Session::start`], with extra arguments the session passes
    /// to `cargo smudge dev`.
    fn start_with(app: &Path, target: &Path, extra: &[&str]) -> Session {
        Session::start_env(app, Some(target), extra, &[])
    }

    /// Starts a session with the target directory and environment of its
    /// own: a `None` target lets cargo choose the workspace's directory,
    /// and a `None` value removes the variable.
    fn start_env(
        app: &Path,
        target: Option<&Path>,
        extra: &[&str],
        env: &[(&str, Option<&str>)],
    ) -> Session {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-smudge"));
        command.arg("dev").args(extra).current_dir(app);
        match target {
            Some(target) => {
                command.env("CARGO_TARGET_DIR", target);
            }
            None => {
                command.env_remove("CARGO_TARGET_DIR");
            }
        }
        command.env("CARGO_NET_OFFLINE", "true");
        for (key, value) in env {
            match value {
                Some(value) => {
                    command.env(key, value);
                }
                None => {
                    command.env_remove(key);
                }
            }
        }
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        let mut child = command.spawn().expect("the session could not start");
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

    /// Asks the session to rebuild the app and start it again.
    fn restart(&mut self) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = writeln!(stdin, "r");
            let _ = stdin.flush();
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

/// Copies the fixture workspace to a fresh directory and answers the
/// workspace's directory; the app package lives under `app`.
fn fixture_workspace(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the temporary directory could not be made");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace"),
        &dir,
    );
    dir
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

/// Edits a source of the fixture with every replacement at once, in one
/// write, so the session never sees a half-edited source. Fails when a
/// `from` is not in the file exactly once.
fn edit(app: &Path, edits: &[(&str, &str)]) {
    edit_in(app, "src/main.rs", edits);
}

/// Like [`edit`], for any source of the fixture, relative to its root.
fn edit_in(app: &Path, file: &str, edits: &[(&str, &str)]) {
    let path = app.join(file);
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

/// The count an applied patch line gives before `reason`, such as
/// `omitted for changed captures`.
fn omitted_for(line: &str, reason: &str) -> usize {
    line.split(&format!(" {reason}"))
        .next()
        .and_then(|head| head.rsplit(' ').next())
        .and_then(|count| count.parse().ok())
        .expect("the applied line names the count")
}

/// Skips with a printed reason when no C linker is present, as the session
/// cannot link without one.
fn require_linker() {
    if Command::new("cc").arg("--version").output().is_err() {
        panic!("skipped, because no C linker named `cc` is present");
    }
}

/// The rustc the toolchain runs, from its own sysroot, for wrappers to
/// exec.
fn real_rustc() -> PathBuf {
    let sysroot = Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string());
    sysroot
        .map(|sysroot| Path::new(&sysroot).join("bin").join("rustc"))
        .filter(|rustc| rustc.exists())
        .unwrap_or_else(|| PathBuf::from("rustc"))
}

/// Writes a `rustc` wrapper under `dir` that injects one `--cfg` into
/// every compilation it passes through, keeping `-vV` identical between
/// wrappers, and answers the directory, which a search path or a config
/// names first.
fn write_cfg_rustc(dir: &Path, real: &Path, cfg: &str) -> PathBuf {
    fs::create_dir_all(dir).expect("the compiler directory could not be made");
    let script = format!(
        "#!/bin/sh\nexec \"{}\" \"$@\" --cfg {cfg}\n",
        real.display()
    );
    let path = dir.join("rustc");
    fs::write(&path, script).expect("the compiler wrapper could not be written");
    let permissions = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    fs::set_permissions(&path, permissions).expect("the wrapper could not be made runnable");
    dir.to_path_buf()
}

/// Gates ui's label on the cfg the wrapper injects, so the value the
/// fixture prints names the compiler that ran.
fn gate_label_on_cfg(ws: &Path) {
    edit_in(
        ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "#[cfg(smudge_compiler_a)]\n#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-A\"\n}\n\n#[cfg(smudge_compiler_b)]\n#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-B\"\n}",
        )],
    );
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
fn patches_a_build_with_dwarf_5() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let app = fixture("dwarf5");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-dwarf5-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // DWARF 5 names a function's address through an index into the
    // unit's address table, which the pairing must resolve or it pairs
    // nothing and sends an empty table.
    let flags = "-Cdwarf-version=5";
    let mut session = Session::start_env(&app, Some(&target), &[], &[("RUSTFLAGS", Some(flags))]);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit(&app, &[("\"alpha-1\"", "\"alpha-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let out = session.wait("fixture alpha=alpha-2", 1, STEP_TIMEOUT);
    assert!(
        out.contains("alpha_hot=true"),
        "the body did not run patched under DWARF 5:\n{out}"
    );
    session.quit();
}

#[test]
fn maps_an_added_function() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let (app, mut session) = start("added");
    // The added function must be reached from patched code, not merely
    // linked beside it: alpha's new body calls gamma, so a patch whose
    // new code could not reach the added function crashes the probe
    // instead of passing on the count alone.
    edit(
        &app,
        &[
            (
                "// BETA-FN-START",
                "\n#[inline(never)]\nfn gamma() -> &'static str {\n    \"gamma-1\"\n}\n\n// BETA-FN-START",
            ),
            (
                "fn alpha() -> &'static str {\n    \"alpha-1\"\n}",
                "fn alpha() -> &'static str {\n    gamma()\n}",
            ),
        ],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    assert!(
        line.contains("1 new"),
        "the added function was not counted as new: {line}"
    );
    let out = session.wait("fixture alpha=gamma-1", 1, STEP_TIMEOUT);
    assert!(
        out.contains("alpha_hot=true"),
        "the patched code did not reach the added function:\n{out}"
    );
    session.quit();
}

/// Whether the toolchain's linker can fold identical functions, which
/// the folded-alias test needs.
fn icf_available() -> bool {
    Command::new("ld.lld")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
        && Command::new("nm").arg("--version").output().is_ok()
}

#[test]
fn refuses_a_response_file_the_wrapper_cannot_read() {
    let scratch = std::env::temp_dir().join(format!("smudge-resp-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    fs::write(scratch.join("x.rs"), "pub fn x() -> u32 { 1 }\n").unwrap();
    // The flag the wrapper gates on hides in the file: a wrapper that
    // passes the reference through compiles unverifiable content.
    fs::write(scratch.join("flags"), "-Csplit-debuginfo=unpacked\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_cargo-smudge"))
        .env("CARGO_SMUDGE_DIR", &scratch)
        .arg("rustc")
        .current_dir(&scratch)
        .args([
            "--crate-name=x",
            "--crate-type=lib",
            "x.rs",
            "--out-dir=.",
            "@flags",
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "the response file slipped through the gates"
    );
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        said.contains("response file"),
        "the refusal does not name the response file:\n{said}"
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// Whether the binary folds the two named text symbols onto one
/// address, which is the fold the running process's captured pointers
/// answer to.
fn folded_onto_one_address(binary: &Path, left: &str, right: &str) -> bool {
    let Ok(out) = Command::new("nm").arg(binary).output() else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let address_of = |name: &str| {
        text.lines()
            .find(|line| line.contains(name))
            .and_then(|line| line.split_whitespace().next())
            .map(String::from)
    };
    match (address_of(left), address_of(right)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

#[test]
fn refuses_a_folded_alias_the_replay_drops() {
    if !icf_available() {
        return;
    }
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let app = fixture("folded-alias");
    // The two probes must spell one body, so the link folds them and
    // the running process captures one address for both names.
    edit(
        &app,
        &[(
            "fn beta() -> &'static str {\n    \"beta-1\"\n}",
            "fn beta() -> &'static str {\n    \"alpha-1\"\n}",
        )],
    );
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-folded-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    let flags = "-Clink-arg=-fuse-ld=lld -Clink-arg=-Wl,--icf=all";
    let mut session = Session::start_env(&app, Some(&target), &[], &[("RUSTFLAGS", Some(flags))]);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // A fold the linker skipped makes the scenario moot: the names
    // carry distinct addresses and the drop of one leaves the other's
    // mapping alone.
    let binary = target.join("debug").join("smudge-hotapp");
    if !folded_onto_one_address(&binary, "5alpha", "4beta") {
        session.quit();
        return;
    }
    edit(
        &app,
        &[
            // The replay drops beta and moves alpha, whose shared
            // address the running process still holds for both names.
            (
                "// BETA-FN-START\n#[inline(never)]\nfn beta() -> &'static str {\n    \"alpha-1\"\n}\n// BETA-FN-END\n\n",
                "",
            ),
            (
                "    let beta_address = beta as *const () as usize;\n",
                "    let beta_address = 0usize;\n",
            ),
            (
                "fn alpha() -> &'static str {\n    \"alpha-1\"\n}",
                "fn alpha() -> &'static str {\n    \"alpha-2\"\n}",
            ),
        ],
    );
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    let reason = line_of(&out, "patch 1: not sent");
    assert!(
        reason.contains("share one original address"),
        "the dropped alias did not refuse the mapping:\n{out}"
    );
    assert!(
        reason.contains("alpha") && reason.contains("beta"),
        "the refusal does not name both aliases:\n{out}"
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
            // The running process captured beta's real address before
            // the loop, and never runs this line again, so the value
            // only has to compile for the new build.
            (
                "    let beta_address = beta as *const () as usize;\n",
                "    let beta_address = 0usize;\n",
            ),
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
    // Beta's own address must have left the table, whatever happened to
    // the unrelated entries: the probe carries the address the original
    // binary captured, and the old code behind it still answers. Patch 1
    // legitimately mapped beta, so only the false spelling proves the
    // removal.
    session.wait("fixture beta=beta-1 beta_hot=false", 1, STEP_TIMEOUT);
    assert!(
        session.output().contains("fixture alpha=alpha-2"),
        "the app did not keep working after the removal"
    );
    session.quit();
}

#[test]
fn omits_a_runner_whose_closure_type_changed_its_layout() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let app = fixture("closure-layout");
    // A runner that takes the closure as an erased pointer names the closure
    // type only as a type parameter, which is how a hot shim reads its
    // closure. The closure moves the struct out whole, so it captures the
    // struct and not its fields one by one. A captured struct whose fields
    // swap keeps its size, its alignment and the closure's type id, so only
    // the debug info of the type parameter tells the two layouts apart.
    edit(
        &app,
        &[(
            "fn main() {",
            "/// Two numbers a closure captures.\n#[derive(Clone, Copy)]\n#[repr(C)]\nstruct Captured {\n    first: u32,\n    second: u64,\n}\n\n/// Runs the closure behind an erased pointer.\n#[inline(never)]\nfn erased<F: Fn() -> u64>(env: *const ()) -> u64 {\n    // SAFETY: the fixture only hands its own closure of this type.\n    unsafe { (*env.cast::<F>())() }\n}\n\n/// Runs `f` through its erased runner.\nfn run_erased<F: Fn() -> u64>(f: &F) -> u64 {\n    erased::<F>(std::ptr::from_ref(f).cast())\n}\n\nfn main() {\n    let captured = Captured { first: 1, second: 2 };\n    let read = move || {\n        let whole = captured;\n        u64::from(whole.first) + whole.second\n    };\n    let _ = run_erased(&read);",
        )],
    );
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit(
        &app,
        &[(
            "    first: u32,\n    second: u64,",
            "    second: u64,\n    first: u32,",
        )],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    // Six functions name the closure's type among their parameters, the
    // closure's body, `run_erased` and the generic instances the compiler
    // makes for the closure, and `erased` names it only as its type
    // parameter, which is the seventh: a signature that left type
    // parameters out would map it and omit six. The ones that name the
    // closure's own environment by value or as a type parameter count as
    // changed captures, and the ones that reach it only behind a reference
    // or through a wrapper as changed signatures.
    let signature = omitted_for(&line, "omitted for a changed signature");
    let captures = omitted_for(&line, "omitted for changed captures");
    assert!(
        signature + captures == 7 && captures > 0,
        "the runner of the changed closure was mapped: {line}"
    );
    session.quit();
}

#[test]
fn names_the_app_statics_a_patch_starts_fresh_once() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let app = fixture("app-static");
    // A private static of the app crate is reached by a 32-bit distance,
    // so the patched code gets its own copy; the session says so, once.
    edit(
        &app,
        &[(
            "fn main() {",
            "/// The value the app configured at startup.\nstatic CONFIG: std::sync::OnceLock<u32> = std::sync::OnceLock::new();\n\n/// Reads the configured value.\n#[inline(never)]\nfn configured() -> u32 {\n    CONFIG.get().copied().unwrap_or(0)\n}\n\nfn main() {\n    let _ = CONFIG.set(5);\n    let _ = configured();",
        )],
    );
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit(&app, &[("\"alpha-1\"", "\"alpha-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait(
        "note: the patched code starts fresh copies",
        1,
        STEP_TIMEOUT,
    );
    assert!(
        line_of(&out, "note: the patched code starts fresh copies").contains("CONFIG"),
        "the note does not name the static:\n{out}"
    );
    edit(&app, &[("\"alpha-2\"", "\"alpha-3\"")]);
    let out = session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    assert_eq!(
        out.matches("note: the patched code starts fresh copies")
            .count(),
        1,
        "the note came again for a static it already named:\n{out}"
    );
    session.quit();
}

#[test]
fn omits_closures_an_inserted_closure_renumbered() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let app = fixture("closure-renumbered");
    edit(
        &app,
        &[(
            "fn main() {",
            "/// Runs the closure behind an erased pointer.\n#[inline(never)]\nfn erased<F: Fn() -> u64>(env: *const ()) -> u64 {\n    // SAFETY: the fixture only hands its own closure of this type.\n    unsafe { (*env.cast::<F>())() }\n}\n\n/// Runs `f` through its erased runner.\nfn run_erased<F: Fn() -> u64>(f: &F) -> u64 {\n    erased::<F>(std::ptr::from_ref(f).cast())\n}\n\nfn main() {\n    let seed = 3_u64;\n    // FIRST\n    let read = move || seed + 1;\n    let _ = run_erased(&read);",
        )],
    );
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // An edit of a closure's body keeps every closure's number.
    edit(&app, &[("seed + 1", "seed + 2")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    assert!(
        line.contains(" 0 omitted for renumbered closures"),
        "a body edit left closures out: {line}"
    );
    // A closure of the same shape inserted before it takes its number, so
    // what names the old number must stay out of the table.
    edit(
        &app,
        &[(
            "    // FIRST\n",
            "    let other = move || seed * 7;\n    let _ = run_erased(&other);\n",
        )],
    );
    let out = session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 2: applied");
    assert!(
        !line.contains(" 0 omitted for renumbered closures"),
        "the renumbered closures were mapped: {line}"
    );
    session.quit();
}

#[test]
fn omits_the_runners_of_a_closure_whose_capture_changed_type() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let app = fixture("closure-capture");
    edit(
        &app,
        &[(
            "fn main() {",
            "/// Runs the closure behind an erased pointer.\n#[inline(never)]\nfn erased<F: Fn() -> u64>(env: *const ()) -> u64 {\n    // SAFETY: the fixture only hands its own closure of this type.\n    unsafe { (*env.cast::<F>())() }\n}\n\n/// Runs `f` through its erased runner.\nfn run_erased<F: Fn() -> u64>(f: &F) -> u64 {\n    erased::<F>(std::ptr::from_ref(f).cast())\n}\n\nfn main() {\n    let seed = 3_u32;\n    let read = move || u64::from(seed) + 1;\n    let _ = run_erased(&read);",
        )],
    );
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // A capture retyped at the same size keeps the closure's size, its
    // alignment and its number, so only the member's type tells.
    edit(
        &app,
        &[
            ("let seed = 3_u32;", "let seed = 3.0_f32;"),
            ("u64::from(seed) + 1", "seed as u64 + 1"),
        ],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    assert!(
        !line.contains(" 0 omitted for changed captures"),
        "the runners of the changed closure were mapped: {line}"
    );
    session.wait("note: the captures of", 1, STEP_TIMEOUT);
    let out = session.output();
    assert!(
        line_of(&out, "note: the captures of").contains("`smudge_hotapp::main::{closure#0}`"),
        "the note does not name the closure:\n{out}"
    );
    session.quit();
}

#[test]
fn refuses_a_state_type_whose_layout_changed_until_it_is_restored() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let app = fixture("state-layout");
    // The state probe the hello announces names the state type as its type
    // parameter, as the probe `run_view` compiles for the app's state does.
    edit(
        &app,
        &[
            (
                "fn main() {",
                "/// The state the running app holds.\nstruct State {\n    count: u32,\n    total: u32,\n    inner: Inner,\n}\n\n/// A part of the state.\nstruct Inner {\n    value: u32,\n    other: u32,\n}\n\nfn main() {",
            ),
            ("state_shape::<()>", "state_shape::<State>"),
        ],
    );
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // A type the state reaches, retyped at the same size.
    edit(&app, &[("    value: u32,\n", "    value: f32,\n")]);
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: not sent");
    assert!(
        line.contains(
            "the state type `smudge_hotapp::State` changed its layout through `smudge_hotapp::Inner`"
        ),
        "the refusal does not name the changed type: {line}"
    );
    // The running app still holds the old layout, so the next edit is
    // refused too.
    edit(&app, &[("\"alpha-1\"", "\"alpha-2\"")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        line_of(&out, "patch 2: not sent").contains("the state type"),
        "the second patch was not refused for the state:\n{out}"
    );
    // Restoring the layout lets the waiting edit through.
    edit(&app, &[("    value: f32,\n", "    value: u32,\n")]);
    session.wait("patch 3: applied", 1, STEP_TIMEOUT);
    session.wait("fixture alpha=alpha-2", 1, STEP_TIMEOUT);
    // Two fields of one size swapped keep the size, the alignment and the
    // name of the state, which only the members tell.
    edit(
        &app,
        &[(
            "    count: u32,\n    total: u32,",
            "    total: u32,\n    count: u32,",
        )],
    );
    let out = session.wait("patch 4: not sent", 1, STEP_TIMEOUT);
    assert!(
        line_of(&out, "patch 4: not sent")
            .contains("the state type `smudge_hotapp::State` changed its layout, "),
        "the swap was not refused for the state itself:\n{out}"
    );
    // A renamed state type leaves the probe the app announced out of the
    // rebuilt code, under any name the gate could look for.
    edit(
        &app,
        &[
            ("\nstruct State {", "\nstruct AppState {"),
            ("state_shape::<State>", "state_shape::<AppState>"),
        ],
    );
    let out = session.wait("patch 5: not sent", 1, STEP_TIMEOUT);
    assert!(
        line_of(&out, "patch 5: not sent")
            .contains("the state type `smudge_hotapp::State` was renamed, moved or replaced"),
        "the renamed state type was not refused:\n{out}"
    );
    session.quit();
}

#[test]
fn refuses_a_state_type_of_a_library_crate_whose_layout_changed() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-state-layout");
    let app = ws.join("app");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label()",
            "/// The state the running app holds.\npub struct State {\n    /// The first number.\n    pub first: u32,\n    /// The second number.\n    pub second: u32,\n}\n\n#[inline(never)]\npub fn label()",
        )],
    );
    edit(&app, &[("state_shape::<()>", "state_shape::<ui::State>")]);
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // The edit replays the library and the app, and the app's objects
    // describe the library's type in its new layout.
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "    pub first: u32,\n    /// The second number.\n    pub second: u32,",
            "    pub second: u32,\n    /// The second number.\n    pub first: u32,",
        )],
    );
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        line_of(&out, "patch 1: not sent")
            .contains("the state type `ui::State` changed its layout"),
        "the library's state type was not refused:\n{out}"
    );
    session.quit();
}

#[test]
fn carries_the_library_that_holds_the_state_probe() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-state-probe");
    let app = ws.join("app");
    // The library compiles the probe, as a library that calls `run_view`
    // does, and the app only announces it.
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label()",
            "/// The state the running app holds.\npub struct State {\n    /// The first number.\n    pub first: u32,\n    /// The second number.\n    pub second: u32,\n}\n\n/// The state probe for the state.\n#[inline(never)]\nfn state_shape<S>() -> usize {\n    std::mem::size_of::<S>()\n}\n\n/// The address of the state probe the app announces.\npub fn state_probe() -> usize {\n    state_shape::<State> as fn() -> usize as usize\n}\n\n#[inline(never)]\npub fn label()",
        )],
    );
    edit(
        &app,
        &[(
            "(state_shape::<()> as fn() -> usize as usize as u64)",
            "(ui::state_probe() as u64)",
        )],
    );
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // An edit to the app alone still replays the library that holds the
    // probe, so the patch maps the probe with the view.
    edit(&app, &[("\"view-1\"", "\"view-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    assert!(
        line.contains("replayed ui"),
        "the library that holds the probe was not replayed: {line}"
    );
    session.wait("view=view-2 view_hot=true", 1, STEP_TIMEOUT);
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "    pub first: u32,\n    /// The second number.\n    pub second: u32,",
            "    pub second: u32,\n    /// The second number.\n    pub first: u32,",
        )],
    );
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        line_of(&out, "patch 2: not sent")
            .contains("the state type `ui::State` changed its layout"),
        "the library's state type was not refused:\n{out}"
    );
    // A refused edit in a library the carried one does not reach leaves
    // the carried one in the patches.
    edit_in(&ws, "tls/src/lib.rs", &[("\"tls-1\"", "\"tls-2\"")]);
    let out = session.wait("patch 3: not sent", 1, STEP_TIMEOUT);
    assert!(
        !out.contains("note: ui holds the view or the state probe"),
        "an unrelated refusal dropped the carried library:\n{out}"
    );
    // Once the library can no longer be replayed, it leaves the patches,
    // though an earlier patch replayed it.
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "/// The state the running app holds.",
            "thread_local! {\n    static CACHE: u32 = 0;\n}\n\n/// Reads the cache.\npub fn cached() -> u32 {\n    CACHE.with(|cache| *cache)\n}\n\n/// The state the running app holds.",
        )],
    );
    session.wait(
        "note: ui holds the view or the state probe",
        1,
        STEP_TIMEOUT,
    );
    edit(&app, &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 5: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 5: applied");
    assert!(
        !line.contains("replayed ui"),
        "the library that left the patches rode along: {line}"
    );
    session.quit();
}

#[test]
fn patches_without_a_probe_library_the_replay_cannot_serve() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-state-probe-tls");
    let app = ws.join("app");
    // The library that holds the probe keeps a thread-local, so no patch
    // can replay it: the session must say so once and patch without it,
    // not refuse every patch.
    edit_in(
        &ws,
        "tls/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label()",
            "/// The state the running app holds.\npub struct State {\n    /// A number.\n    pub value: u32,\n}\n\n/// The state probe for the state.\n#[inline(never)]\nfn state_shape<S>() -> usize {\n    std::mem::size_of::<S>()\n}\n\n/// The address of the state probe the app announces.\npub fn state_probe() -> usize {\n    state_shape::<State> as fn() -> usize as usize\n}\n\n#[inline(never)]\npub fn label()",
        )],
    );
    edit(
        &app,
        &[(
            "(state_shape::<()> as fn() -> usize as usize as u64)",
            "(tls::state_probe() as u64)",
        )],
    );
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.wait(
        "note: tls holds the view or the state probe",
        1,
        STEP_TIMEOUT,
    );
    edit(&app, &[("\"view-1\"", "\"view-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    assert!(
        !line.contains("replayed tls"),
        "the library the replay cannot serve rode along: {line}"
    );
    // Without its probe the view stays out of the table, so the app keeps
    // the view it has instead of refusing the patch.
    let out = session.wait("view=view-1 view_hot=false", 1, STEP_TIMEOUT);
    assert!(
        !out.contains("view_hot=true"),
        "the view was mapped without its probe:\n{out}"
    );
    session.quit();
}

#[test]
fn drops_a_probe_library_whose_dependent_the_replay_cannot_serve() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-state-probe-dependent");
    let app = ws.join("app");
    // The probe sits in the deepest library, so carrying it replays the
    // library above it too.
    edit_in(
        &ws,
        "core2/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn seed()",
            "/// The state the running app holds.\npub struct State {\n    /// A number.\n    pub value: u32,\n}\n\n/// The state probe for the state.\n#[inline(never)]\nfn state_shape<S>() -> usize {\n    std::mem::size_of::<S>()\n}\n\n/// The address of the state probe the app announces.\npub fn state_probe() -> usize {\n    state_shape::<State> as fn() -> usize as usize\n}\n\n#[inline(never)]\npub fn seed()",
        )],
    );
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label()",
            "/// The address of the state probe the deepest library holds.\npub fn state_probe() -> usize {\n    engine::state_probe()\n}\n\n#[inline(never)]\npub fn label()",
        )],
    );
    edit(
        &app,
        &[(
            "(state_shape::<()> as fn() -> usize as usize as u64)",
            "(ui::state_probe() as u64)",
        )],
    );
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit(&app, &[("\"view-1\"", "\"view-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    assert!(
        line.contains("core2") && line.contains("ui"),
        "the probe library and the one above it were not replayed: {line}"
    );
    // The library above can no longer be replayed. Only the carried one
    // brought it into the patches, so the carried one leaves them.
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "/// The address of the state probe the deepest library holds.",
            "thread_local! {\n    static CACHE: u32 = 0;\n}\n\n/// Reads the cache.\npub fn cached() -> u32 {\n    CACHE.with(|cache| *cache)\n}\n\n/// The address of the state probe the deepest library holds.",
        )],
    );
    session.wait(
        "note: core2 holds the view or the state probe",
        1,
        STEP_TIMEOUT,
    );
    edit(&app, &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 3: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 3: applied");
    assert!(
        !line.contains("replayed"),
        "a library rode along after the carried one left: {line}"
    );
    session.quit();
}

#[test]
fn lists_the_type_probes_a_patch_kept() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let app = fixture("type-probes");
    // Every instance of the announced probe's function probes its type.
    edit(
        &app,
        &[
            (
                "fn main() {",
                "/// A type whose layout the test keeps.\nstruct Steady {\n    value: u32,\n}\n\n/// A type whose layout the test changes.\nstruct Shifting {\n    first: u32,\n    second: u32,\n}\n\nfn main() {\n    let steady = std::hint::black_box(state_shape::<Steady> as fn() -> usize as usize);\n    let shifting = std::hint::black_box(state_shape::<Shifting> as fn() -> usize as usize);",
            ),
            (
                "        let _ = io::stdout().flush();",
                "        println!(\n            \"fixture steady_kept={} shifting_kept={} shifting_mapped={}\",\n            kept(steady),\n            kept(shifting),\n            table.is_some_and(|table| table.map.contains_key(&(shifting as u64)))\n        );\n        let _ = io::stdout().flush();",
            ),
        ],
    );
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit(&app, &[("\"alpha-1\"", "\"alpha-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait(
        "fixture steady_kept=true shifting_kept=true",
        1,
        STEP_TIMEOUT,
    );
    // Two fields of one size swapped keep the size and the alignment, so
    // only the builder can tell the app not to carry the type's values.
    edit(
        &app,
        &[(
            "    first: u32,\n    second: u32,",
            "    second: u32,\n    first: u32,",
        )],
    );
    session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    // The probe of the changed type still maps, so the app finds the
    // original of the probe the patched code reaches.
    session.wait(
        "fixture steady_kept=true shifting_kept=false shifting_mapped=true",
        1,
        STEP_TIMEOUT,
    );
    // Swapping them back matches the binary the app started with, but the
    // values the last patch made hold its layout, so they still do not
    // cross; the patch after that keeps the layout the values now hold.
    edit(
        &app,
        &[(
            "    second: u32,\n    first: u32,",
            "    first: u32,\n    second: u32,",
        )],
    );
    session.wait("patch 3: applied", 1, STEP_TIMEOUT);
    session.wait(
        "fixture steady_kept=true shifting_kept=false",
        2,
        STEP_TIMEOUT,
    );
    edit(&app, &[("\"alpha-2\"", "\"alpha-3\"")]);
    session.wait("patch 4: applied", 1, STEP_TIMEOUT);
    session.wait(
        "fixture steady_kept=true shifting_kept=true",
        2,
        STEP_TIMEOUT,
    );
    session.quit();
}

#[test]
fn never_keeps_a_type_that_holds_a_trait_object() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let app = fixture("type-probes-erased");
    // What a trait object holds has a layout no digest of the holder covers.
    edit(
        &app,
        &[
            (
                "fn main() {",
                "/// A type that holds a value behind a trait object.\nstruct Erased {\n    _inner: Box<dyn std::any::Any>,\n}\n\n/// A type that holds plain data.\nstruct Plain {\n    _value: u32,\n}\n\nfn main() {\n    let erased = std::hint::black_box(state_shape::<Erased> as fn() -> usize as usize);\n    let plain = std::hint::black_box(state_shape::<Plain> as fn() -> usize as usize);",
            ),
            (
                "        let _ = io::stdout().flush();",
                "        println!(\n            \"fixture erased_kept={} plain_kept={}\",\n            kept(erased),\n            kept(plain)\n        );\n        let _ = io::stdout().flush();",
            ),
        ],
    );
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit(&app, &[("\"alpha-1\"", "\"alpha-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture erased_kept=false plain_kept=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn turns_on_the_packages_hot_reload_feature() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let app = fixture("own-feature");
    // A package that declares the feature, as the template does, gets it
    // on under a plain `cargo smudge dev`.
    edit_in(
        &app,
        "Cargo.toml",
        &[("[workspace]", "[features]\nhot-reload = []\n\n[workspace]")],
    );
    edit(
        &app,
        &[(
            "fn main() {",
            "fn main() {\n    println!(\"fixture hot-feature={}\", cfg!(feature = \"hot-reload\"));",
        )],
    );
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.wait("fixture hot-feature=true", 1, STEP_TIMEOUT);
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
fn patches_an_edit_in_a_library_crate() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-lib");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/lib.rs", &[("\"ui-1\"", "\"ui-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    assert!(
        line.contains("replayed ui"),
        "the applied line names no replayed library: {line}"
    );
    let out = session.wait("fixture ui=ui-2 ui_hot=true", 1, STEP_TIMEOUT);
    assert!(
        out.contains("deep=7 deep_hot=true"),
        "the deeper library did not ride along patched:\n{out}"
    );
    session.quit();
}

#[test]
fn patches_an_edit_in_an_indirect_dependency() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-indirect");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "core2/src/lib.rs", &[("    7", "    9")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    assert!(
        line.contains("replayed core2, ui"),
        "the chain did not name the changed crate and its dependent: {line}"
    );
    let out = session.wait("deep=9 deep_hot=true", 1, STEP_TIMEOUT);
    assert!(
        out.contains("ui=ui-1 ui_hot=true"),
        "the library between the changed crate and the app did not patch:\n{out}"
    );
    session.quit();
}

#[test]
fn patches_a_tip_edit_after_a_library_edit() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-tip-after-lib");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/lib.rs", &[("\"ui-1\"", "\"ui-2\"")]);
    session.wait("ui=ui-2 ui_hot=true", 1, STEP_TIMEOUT);
    // The patch after a library patch must carry the library edit too,
    // because every patch replaces the whole table.
    edit(&app, &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    let out = session.wait("ui=ui-2 ui_hot=true", 2, STEP_TIMEOUT);
    assert!(
        out.contains("deep=7 deep_hot=true"),
        "the second patch dropped the first patch's library state:\n{out}"
    );
    session.quit();
}

#[test]
fn patches_after_a_cached_restart() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-cached");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.quit();
    // The second session's fat build comes from cargo's cache, which runs
    // the wrapper for no crate, so the recordings of the first session
    // must carry over.
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/lib.rs", &[("\"ui-1\"", "\"ui-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    assert!(
        line.contains("replayed ui"),
        "the cached session recorded no library to replay: {line}"
    );
    session.wait("ui=ui-2 ui_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn patches_edits_in_two_crates_saved_together() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-two");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // Two saves inside one debounce window must both reach the patch,
    // whichever crate they touch.
    edit_in(&ws, "core2/src/lib.rs", &[("    7", "    9")]);
    edit_in(&ws, "ui/src/lib.rs", &[("\"ui-1\"", "\"ui-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("ui=ui-2 ui_hot=true deep=9 deep_hot=true", 1, FAT_TIMEOUT);
    session.quit();
}

#[test]
fn keeps_a_library_edit_that_shared_a_failing_patch() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-pending");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // A library edit and a broken app edit land in one patch, which fails
    // on the app's compile. The library edit must survive the failure.
    edit_in(&ws, "ui/src/lib.rs", &[("\"ui-1\"", "\"ui-2\"")]);
    edit(
        &app,
        &[(
            "    let ui_address = ui::label",
            "    let broken: u32 = not_defined;\n    let ui_address = ui::label",
        )],
    );
    session.wait("patch 1: compile failed", 1, STEP_TIMEOUT);
    // Fixing only the app source must bring the library edit along.
    edit(
        &app,
        &[(
            "    let broken: u32 = not_defined;\n    let ui_address = ui::label",
            "    let ui_address = ui::label",
        )],
    );
    let out = session.wait("ui=ui-2 ui_hot=true", 1, FAT_TIMEOUT);
    assert!(
        out.contains("deep=7 deep_hot=true"),
        "the recovered patch carried the wrong library state:\n{out}"
    );
    session.quit();
}

#[test]
fn rebuilds_from_clean_after_a_feature_switch() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-features");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A first session on the default features records the default units.
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.quit();
    // A session with the fancy feature must rebuild the workspace's own
    // crates from clean, or the recordings would describe units cargo
    // would serve from another build's cache.
    let mut session = Session::start_with(&app, &target, &["--features", "ui/fancy"]);
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.quit();
    // Back on the default features, the rebuild from clean records the
    // default units again, and an edit in ui patches them.
    let mut session = Session::start(&app, &target);
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/lib.rs", &[("\"ui-1\"", "\"ui-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    assert!(
        line.contains("replayed ui"),
        "the rebuilt session recorded no library to replay: {line}"
    );
    session.wait("ui=ui-2 ui_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn attributes_sources_of_a_package_with_a_library() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-package");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // The binary's own module patches, even though the package also
    // builds a library of the same name in the same directory.
    edit_in(&ws, "app/src/bin_only.rs", &[("\"bin-1\"", "\"bin-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("bin=bin-2 bin_hot=true", 1, STEP_TIMEOUT);
    // The library of the same name cannot replay, and says so.
    edit_in(&ws, "app/src/lib.rs", &[("    1", "    2")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("cannot replay separately"),
        "the library of the app's own package was not refused:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused patch:\n{out}"
    );
    // The app crate itself keeps patching afterwards.
    edit(&app, &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 3: applied", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn replays_a_thread_local_helper_of_a_macro() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-macro");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // The helper serves the macro's expansion only, so its thread-local
    // must not refuse the patch: the macro replays with it and bakes the
    // new tag into the next expansion.
    edit_in(
        &ws,
        "helper/src/lib.rs",
        &[("\"macro-h-1\"", "\"macro-h-2\"")],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    assert!(
        line.contains("replayed helper, mac, ui"),
        "the macro's helper did not ride along replayed: {line}"
    );
    session.wait("branded=macro-h-2 branded_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn keeps_other_edits_behind_a_refused_one() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-mixed");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // A library edit and an edit in the library of the app's own package
    // land together, and the second one refuses the patch. The first
    // edit must survive the refusal.
    edit_in(&ws, "ui/src/lib.rs", &[("\"ui-1\"", "\"ui-2\"")]);
    edit_in(&ws, "app/src/lib.rs", &[("    1", "    2")]);
    let out = session.wait("not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("cannot replay separately"),
        "the library of the app's own package was not refused:\n{out}"
    );
    // The next app edit brings the surviving library edit along.
    edit(&app, &[("\"view-1\"", "\"view-2\"")]);
    let out = session.wait("ui=ui-2 ui_hot=true", 1, FAT_TIMEOUT);
    assert!(
        out.contains("deep=7 deep_hot=true"),
        "the patch after the refusal carried the wrong library state:\n{out}"
    );
    session.quit();
}

#[test]
fn rebuilds_when_the_target_dir_changes() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-targets");
    let app = ws.join("app");
    let target_a = Path::new(env!("CARGO_TARGET_TMPDIR")).join("target-requests-a");
    let target_b = Path::new(env!("CARGO_TARGET_TMPDIR")).join("target-requests-b");
    for target in [&target_a, &target_b] {
        let _ = fs::remove_dir_all(target);
        fs::create_dir_all(target).expect("the target could not be made");
    }
    let mut session = Session::start(&app, &target_a);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.quit();
    // The target directory belongs to the build request: the other
    // target's stored request names a directory this build does not
    // use, so the session must not serve the first target's recordings.
    let stale = target_b.join("smudge-hot");
    fs::create_dir_all(&stale).expect("the records directory could not be made");
    fs::copy(
        target_a.join("smudge-hot").join("build-request.json"),
        stale.join("build-request.json"),
    )
    .expect("the stale request could not be copied");
    let mut session = Session::start(&app, &target_b);
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/lib.rs", &[("\"ui-1\"", "\"ui-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    assert!(
        line.contains("replayed ui"),
        "the rebuilt session recorded no library to replay: {line}"
    );
    session.wait("ui=ui-2 ui_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn refuses_a_source_two_units_compile() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-shared");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // The source belongs to the app crate and to the library of its own
    // package, so a tip-only patch would silently leave the library's
    // copy old: the patch is refused instead.
    edit_in(
        &ws,
        "app/src/shared.rs",
        &[("\"shared-1\"", "\"shared-2\"")],
    );
    let out = session.wait("not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("cannot replay separately"),
        "the source of two units was not refused:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused patch:\n{out}"
    );
    session.quit();
}

#[test]
fn patches_a_module_created_after_the_build() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-late");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // A module the build never saw has no dep-info entry, but it belongs
    // to the app crate, whose directory a library of the same name also
    // claims. The source membership must decide, so the module edits.
    fs::write(
        app.join("src").join("late_mod.rs"),
        "//! A module the build learned about after the fact.\n\n\
         #[allow(dead_code)]\n\
         pub fn tag() -> &'static str {\n    \"late-1\"\n}\n",
    )
    .expect("the late module could not be written");
    edit(&app, &[("mod bin_only;", "mod bin_only;\nmod late_mod;")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    edit_in(&app, "src/late_mod.rs", &[("\"late-1\"", "\"late-2\"")]);
    let out = session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    assert!(
        !out.contains("cannot replay separately"),
        "the late module was treated as the library's:\n{out}"
    );
    session.quit();
}

#[test]
fn refuses_a_thread_local_crate_and_keeps_running() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-tls");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "tls/src/lib.rs", &[("\"tls-1\"", "\"tls-2\"")]);
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("thread-locals"),
        "the refusal names no thread-local:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused patch:\n{out}"
    );
    // The session survives the refusal, and the next good change patches.
    // The thread-local library keeps the version the binary was built
    // with, which the probe line answers.
    edit(&app, &[("\"view-1\"", "\"view-2\"")]);
    let out = session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    let line = out
        .lines()
        .find(|line| line.contains("fixture ui="))
        .expect("the fixture printed its probes")
        .to_owned();
    assert!(
        line.contains("tls=tls-1 tls_hot=false"),
        "the thread-local library did not keep the binary's version: {line}"
    );
    session.quit();
}

#[test]
fn shares_a_librarys_public_static_with_the_running_app() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-shared-static");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // The library keeps a public static the app writes once at startup.
    // Every reference to it goes through a full address, so the patched
    // code reads the running app's copy, and the value the app built.
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "use std::sync::atomic::{AtomicU32, Ordering};\n\n/// The runs the app counted since it started, which a patch's\n/// fresh copy would reset.\npub static RUNS: AtomicU32 = AtomicU32::new(0);\n\n#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}\n\n/// Reads the counter, so the value the app built answers.\n#[inline(never)]\npub fn runs() -> u32 {\n    RUNS.load(Ordering::Relaxed)\n}",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let runs_address = ui::runs as *const () as usize;\n    ui::RUNS.store(41, std::sync::atomic::Ordering::Relaxed);",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (runs_value, runs_hot) = through_deep(table, runs_address);",
            ),
            (
                "bin={bin_value} bin_hot={bin_hot}\"",
                "bin={bin_value} bin_hot={bin_hot} runs={runs_value} runs_hot={runs_hot}\"",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/lib.rs", &[("\"ui-1\"", "\"ui-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = out
        .lines()
        .find(|line| line.contains("fixture ui="))
        .expect("the fixture printed its probes")
        .to_owned();
    assert!(
        line.contains("runs=41 runs_hot=true"),
        "the patched reader did not see the running app's static: {line}"
    );
    session.quit();
}

#[test]
fn refuses_a_public_static_whose_type_changed_layout() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-static-layout");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A public static whose type swaps two fields keeps its size, but the
    // running app's data stands in the old layout: the patched code must
    // not read it, so the edit refuses, naming the static.
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "/// Two numbers the app keeps.\n#[repr(C)]\npub struct Pair {\n    /// The first.\n    pub first: u32,\n    /// The second.\n    pub second: u32,\n}\n\n/// The pair the app keeps since it started.\npub static PAIR: std::sync::Mutex<Pair> = std::sync::Mutex::new(Pair { first: 1, second: 2 });\n\n/// Reads the pair's first number.\n#[inline(never)]\npub fn first() -> u32 {\n    PAIR.lock().map(|pair| pair.first).unwrap_or(0)\n}\n\n#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[(
            "    let bin_address = bin_only::tag as *const () as usize;",
            "    let bin_address = bin_only::tag as *const () as usize;\n    println!(\"first={}\", ui::first());",
        )],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "    /// The first.\n    pub first: u32,\n    /// The second.\n    pub second: u32,",
            "    /// The second.\n    pub second: u32,\n    /// The first.\n    pub first: u32,",
        )],
    );
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("writable statics") && out.contains("PAIR"),
        "the static whose layout changed was shared:\n{out}"
    );
    session.quit();
}

#[test]
fn refuses_a_public_static_whose_element_type_changed_layout() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-static-element");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A vector reaches its elements through an erased pointer, so only
    // its element type tells that the running app's elements stand in the
    // old layout: the edit refuses, naming the static.
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "/// Two numbers the app keeps.\n#[repr(C)]\npub struct Pair {\n    /// The first.\n    pub first: u32,\n    /// The second.\n    pub second: u32,\n}\n\n/// The pair the app keeps since it started.\npub static PAIRS: std::sync::Mutex<Vec<Pair>> = std::sync::Mutex::new(Vec::new());\n\n/// Reads the pair's first number.\n#[inline(never)]\npub fn first() -> u32 {\n    PAIRS.lock().map(|pairs| pairs.first().map_or(0, |pair| pair.first)).unwrap_or(0)\n}\n\n#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[(
            "    let bin_address = bin_only::tag as *const () as usize;",
            "    let bin_address = bin_only::tag as *const () as usize;\n    println!(\"first={}\", ui::first());",
        )],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "    /// The first.\n    pub first: u32,\n    /// The second.\n    pub second: u32,",
            "    /// The second.\n    pub second: u32,\n    /// The first.\n    pub first: u32,",
        )],
    );
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("writable statics") && out.contains("PAIRS"),
        "the static whose layout changed was shared:\n{out}"
    );
    session.quit();
}

#[test]
fn refuses_an_immutable_static_an_edit_makes_writable() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-static-mut");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // An immutable static sits in read-only memory in the running app, so
    // an edit that makes it writable cannot share it: the edit refuses.
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "/// A count the app reads.\npub static COUNT: u64 = 0;\n\n/// Reads the count.\n#[inline(never)]\npub fn count() -> u64 {\n    // SAFETY: the fixture's only thread reads a plain aligned word.\n    unsafe { core::ptr::read_volatile(&raw const COUNT) }\n}\n\n#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[(
            "    let bin_address = bin_only::tag as *const () as usize;",
            "    let bin_address = bin_only::tag as *const () as usize;\n    println!(\"count={}\", ui::count());",
        )],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[
            (
                "pub static COUNT: u64 = 0;",
                "pub static mut COUNT: u64 = 0;",
            ),
            (
                "    unsafe { core::ptr::read_volatile(&raw const COUNT) }",
                "    unsafe {\n        core::ptr::write_volatile(&raw mut COUNT, 41);\n        core::ptr::read_volatile(&raw const COUNT)\n    }",
            ),
        ],
    );
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("writable statics") && out.contains("COUNT"),
        "the static the edit made writable was shared:\n{out}"
    );
    session.quit();
}

#[test]
fn patches_an_edited_immutable_static() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-static-edit");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // An immutable public static carries the edit's new contents, so the
    // patched code reads the new value and not the running app's old one.
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "/// The label the library shows.\npub static LABEL: &str = \"ui-1\";\n\n#[inline(never)]\npub fn label() -> &'static str {\n    LABEL\n}",
        )],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/lib.rs", &[("\"ui-1\"", "\"ui-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-2 ui_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn refuses_a_writable_static_an_edit_adds_to_a_library() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-static-added");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // A static the edit adds has no copy in the running app to share, and
    // every later patch would carry a fresh one, resetting what the patched
    // code kept in it: the edit asks for a restart.
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "/// The calls counted since the patch.\npub static CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);\n\n#[inline(never)]\npub fn label() -> &'static str {\n    CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);\n    \"ui-2\"\n}",
        )],
    );
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("writable statics") && out.contains("CALLS"),
        "the added static was not refused:\n{out}"
    );
    session.quit();
}

#[test]
fn a_generic_method_used_anew_keeps_its_types_signature() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-generic-method");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-generic-method-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // One codegen unit keeps the method's declaration under its type in the
    // same unit as the functions that take the type. A body edit that
    // calls the method with a new type parameter adds that parameter
    // under the type's declaration, which is no change of its layout.
    let manifest = fs::read_to_string(ws.join("Cargo.toml")).expect("the manifest is read");
    fs::write(
        ws.join("Cargo.toml"),
        format!("{manifest}\n[profile.dev]\ncodegen-units = 1\n"),
    )
    .expect("the manifest is written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "/// A counted value.\npub struct Counter {\n    /// The value.\n    pub value: u64,\n}\n\nimpl Counter {\n    /// The value plus the size of `T`.\n    #[inline(never)]\n    pub fn read<T>(&self) -> u64 {\n        self.value + std::mem::size_of::<T>() as u64\n    }\n}\n\n/// Reads the counter.\n#[inline(never)]\npub fn probe(counter: &Counter) -> u64 {\n    counter.value\n}\n\n#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[(
            "    let bin_address = bin_only::tag as *const () as usize;",
            "    let bin_address = bin_only::tag as *const () as usize;\n    println!(\"probe={}\", ui::probe(&ui::Counter { value: 41 }));",
        )],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    counter.value\n", "    counter.read::<u64>()\n")],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    assert!(
        line.contains(" 0 omitted for a changed signature"),
        "the method's type parameter changed its type's signature: {line}"
    );
    session.quit();
}

#[test]
fn refuses_a_librarys_private_writable_static() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-private-static");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // The library keeps a private static the app writes once at startup.
    // The library's code reaches it by a 32-bit distance, which the
    // running app's copy lies too far from, so a replay would hand the
    // patched code a fresh zeroed copy: the edit refuses, naming it, the
    // app keeps running, and the counter keeps the value the app built.
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "use std::sync::atomic::{AtomicU32, Ordering};\n\n/// The runs the app counted since it started, which a patch's\n/// fresh copy would reset.\nstatic RUNS: AtomicU32 = AtomicU32::new(0);\n\n/// Sets the counter the app starts from.\n#[inline(never)]\npub fn set_runs(value: u32) {\n    RUNS.store(value, Ordering::Relaxed);\n}\n\n#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}\n\n/// Reads the counter, so the value the app built answers.\n#[inline(never)]\npub fn runs() -> u32 {\n    RUNS.load(Ordering::Relaxed)\n}",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let runs_address = ui::runs as *const () as usize;\n    ui::set_runs(41);",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (runs_value, runs_hot) = through_deep(table, runs_address);",
            ),
            (
                "bin={bin_value} bin_hot={bin_hot}\"",
                "bin={bin_value} bin_hot={bin_hot} runs={runs_value} runs_hot={runs_hot}\"",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/lib.rs", &[("\"ui-1\"", "\"ui-2\"")]);
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("writable statics") && out.contains("RUNS"),
        "the refusal names no writable static:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused patch:\n{out}"
    );
    // The session survives the refusal, and the next good change patches;
    // the library's counter keeps the value the app built, which the
    // probe line answers.
    edit(&app, &[("\"view-1\"", "\"view-2\"")]);
    let out = session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    let line = out
        .lines()
        .find(|line| line.contains("fixture ui="))
        .expect("the fixture printed its probes")
        .to_owned();
    assert!(
        line.contains("runs=41 runs_hot=false"),
        "the library's static did not keep the app's value: {line}"
    );
    session.quit();
}

#[test]
fn refuses_untyped_assembly_storage() {
    // Storage assembly defines without a `.type` directive has no type the
    // debug information describes, so nothing shows that the running
    // app's data still fits the patched code, and it is not shared.
    assembly_storage(
        false,
        "ws-asm-storage",
        "\".section .data.ui_counter,\\\"aw\\\",@progbits\",\n    \".p2align 2\",\n    \".globl ui_counter\",\n    \"ui_counter:\",\n    \".long 0\",\n",
    );
}

#[test]
fn refuses_common_assembly_storage() {
    // A common symbol names no section until the link places it.
    assembly_storage(false, "ws-asm-common", "\".comm ui_counter,4,4\",\n");
}

/// Defines `ui_counter` in `ui` through the assembly `directives` and
/// checks that an edit of `ui` patches when the storage is `shared` with
/// the running app, and refuses for its writable state otherwise.
fn assembly_storage(shared: bool, name: &str, directives: &str) {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace(name);
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // Storage that assembly defines is as much state as a static the
    // compiler types, whatever symbol type or section it carries.
    let code = format!("std::arch::global_asm!(\n    {directives});\n\n")
        + "unsafe extern \"C\" {\n    static mut ui_counter: u32;\n}\n\n/// Reads the counter the assembly defines.\n#[inline(never)]\npub fn counter() -> u32 {\n    // SAFETY: the fixture's only thread reads a plain aligned word.\n    unsafe { core::ptr::read_volatile(&raw const ui_counter) }\n}\n\n#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}";
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            &code,
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[(
            "    let bin_address = bin_only::tag as *const () as usize;",
            "    let bin_address = bin_only::tag as *const () as usize;\n    println!(\"counter={}\", ui::counter());",
        )],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/lib.rs", &[("\"ui-1\"", "\"ui-2\"")]);
    if shared {
        session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    } else {
        let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
        assert!(
            out.contains("writable statics"),
            "the assembly storage did not refuse:\n{out}"
        );
    }
    session.quit();
}

#[test]
fn restarts_when_a_library_gains_a_build_script() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-script-gained");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // A `build.rs` at the package root becomes a build script without
    // any manifest edit, and what it emits reaches only a new build.
    fs::write(ws.join("ui").join("build.rs"), "fn main() {}\n")
        .expect("ui's build script could not be written");
    session.wait("the build inputs changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 2, FAT_TIMEOUT);
    session.quit();
}

#[test]
fn keeps_the_session_through_a_failed_rebuild() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let (app, mut session) = start("failed-rebuild");
    // A manifest edit rebuilds the app, and a compile error in that
    // build waits for the next edit instead of ending the session.
    edit(&app, &[("\"alpha-1\"", "\"alpha-1\" +")]);
    let path = app.join("Cargo.toml");
    let manifest = fs::read_to_string(&path).expect("the manifest could not be read");
    fs::write(&path, format!("{manifest}\n# a test touched this\n"))
        .expect("the manifest could not be written");
    session.wait("waiting for an edit to build again", 1, FAT_TIMEOUT);
    edit(&app, &[("\"alpha-1\" +", "\"alpha-2\"")]);
    session.wait("app running as pid", 2, FAT_TIMEOUT);
    // The rebuilt app patches like the first one did.
    edit(&app, &[("\"alpha-2\"", "\"alpha-3\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture alpha=alpha-3", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn refuses_a_patch_that_uses_a_crates_thread_local() {
    refuses_a_thread_local_of_tls("ws-tls-import", false, Layout::Shared);
}

#[test]
fn refuses_a_thread_local_with_a_separate_build_dir() {
    // Cargo can keep the workspace's intermediate artifacts outside the
    // target directory, where they are no more the standard library's.
    refuses_a_thread_local_of_tls("ws-tls-build-dir", false, Layout::BuildDir);
}

#[test]
fn refuses_a_thread_local_through_a_symlinked_target_dir() {
    // The compiler names every rlib by its canonical path, which a target
    // directory reached through a link does not start with; the workspace's
    // own rlibs must not pass for the standard library's then.
    refuses_a_thread_local_of_tls("ws-tls-linked", false, Layout::Linked);
}

#[test]
fn refuses_a_thread_local_of_a_crate_named_like_the_standard_library() {
    // The crate's name alone says nothing about where it comes from: a
    // workspace crate named `core` keeps its thread-locals refused.
    refuses_a_thread_local_of_tls("ws-tls-core", true, Layout::Shared);
}

/// Where a thread-local test builds.
enum Layout {
    /// The shared target directory.
    Shared,
    /// A target directory reached through a symbolic link.
    Linked,
    /// A target directory of its own with a separate build directory.
    BuildDir,
}

/// Makes the app reach a thread-local of the `tls` crate, named `core`
/// when `as_core`, builds it with `layout`, and checks that an edit of the
/// app refuses for it.
fn refuses_a_thread_local_of_tls(name: &str, as_core: bool, layout: Layout) {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace(name);
    if as_core {
        edit_in(
            &ws,
            "tls/Cargo.toml",
            &[("name = \"tls\"", "name = \"core\"")],
        );
        edit_in(
            &ws,
            "Cargo.toml",
            &[(
                "tls = { path = \"tls\" }",
                "tls = { path = \"tls\", package = \"core\" }",
            )],
        );
    }
    let app = ws.join("app");
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let mut env: Vec<(&str, Option<String>)> = Vec::new();
    let target = match layout {
        Layout::Shared => scratch.join("shared-target"),
        Layout::Linked => {
            let real = scratch.join("ws-tls-linked-real");
            let link = scratch.join("ws-tls-linked-link");
            fs::create_dir_all(&real).expect("the real target could not be made");
            let _ = fs::remove_file(&link);
            std::os::unix::fs::symlink(&real, &link).expect("the link could not be made");
            link.join("t")
        }
        Layout::BuildDir => {
            let build = scratch.join("ws-tls-build-dir-build");
            env.push(("CARGO_BUILD_BUILD_DIR", Some(build.display().to_string())));
            scratch.join("ws-tls-build-dir-target")
        }
    };
    fs::create_dir_all(&target).expect("the target could not be made");
    // A generic function compiles in the crate that calls it, so the app's
    // own objects reach the library's thread-local, which a patch could
    // only copy apart from the one the running code counts in.
    edit_in(
        &ws,
        "tls/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label()",
            "thread_local! {\n    static SEEN: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };\n}\n\n/// Counts a value on this thread and hands it back.\npub fn counted<T>(value: T) -> T {\n    SEEN.with(|seen| seen.set(seen.get() + 1));\n    value\n}\n\n#[inline(never)]\npub fn label()",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[(
            "fn view() -> &'static str {\n    \"view-1\"\n}",
            "fn view() -> &'static str {\n    tls::counted(\"view-1\")\n}",
        )],
    );
    let env: Vec<(&str, Option<&str>)> = env
        .iter()
        .map(|(name, value)| (*name, value.as_deref()))
        .collect();
    let mut session = Session::start_env(&app, Some(&target), &[], &env);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("thread-local") && out.contains("4SEEN"),
        "the refusal names no thread-local:\n{out}"
    );
    session.quit();
}

#[test]
fn sees_a_fix_beside_a_file_dated_ahead() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let (app, mut session) = start("failed-rebuild-ahead");
    // A file whose time stands an hour ahead must not hide the save that
    // fixes the build, which lands earlier than it.
    let ahead = app.join("notes.txt");
    fs::write(&ahead, "ahead\n").expect("the file could not be written");
    fs::File::options()
        .write(true)
        .open(&ahead)
        .and_then(|file| file.set_modified(SystemTime::now() + Duration::from_secs(3600)))
        .expect("the file's time could not be set");
    edit(&app, &[("\"alpha-1\"", "\"alpha-1\" +")]);
    let path = app.join("Cargo.toml");
    let manifest = fs::read_to_string(&path).expect("the manifest could not be read");
    fs::write(&path, format!("{manifest}\n# a test touched this\n"))
        .expect("the manifest could not be written");
    session.wait("waiting for an edit to build again", 1, FAT_TIMEOUT);
    edit(&app, &[("\"alpha-1\" +", "\"alpha-2\"")]);
    session.wait("app running as pid", 2, FAT_TIMEOUT);
    session.quit();
}

#[test]
fn sees_a_fix_in_a_module_named_target() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let app = fixture("failed-rebuild-module");
    // A module of the app lives in `src/target`, which is source, not the
    // build's output, and its fix must wake the wait after a failed build.
    fs::create_dir_all(app.join("src").join("target")).expect("the module dir could not be made");
    fs::write(
        app.join("src").join("target").join("mod.rs"),
        "/// A number from the module.\npub fn number() -> u32 {\n    1\n}\n",
    )
    .expect("the module could not be written");
    edit(
        &app,
        &[(
            "fn main() {",
            "mod target;\n\nfn main() {\n    let _ = target::number();",
        )],
    );
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    let module = app.join("src").join("target").join("mod.rs");
    fs::write(&module, "pub fn number() -> u32 {\n    1 +\n}\n")
        .expect("the module could not be broken");
    let path = app.join("Cargo.toml");
    let manifest = fs::read_to_string(&path).expect("the manifest could not be read");
    fs::write(&path, format!("{manifest}\n# a test touched this\n"))
        .expect("the manifest could not be written");
    session.wait("waiting for an edit to build again", 1, FAT_TIMEOUT);
    fs::write(
        &module,
        "/// A number from the module.\npub fn number() -> u32 {\n    2\n}\n",
    )
    .expect("the module could not be fixed");
    session.wait("app running as pid", 2, FAT_TIMEOUT);
    session.quit();
}

#[test]
fn waits_once_when_the_target_dir_has_another_name() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let app = fixture("failed-rebuild-out");
    // The target directory sits inside the package under a name of its
    // own, and what the failing build writes there is no edit to wait for.
    let target = app.join("out");
    let mut session = Session::start_env(&app, Some(&target), &[], &[]);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit(&app, &[("\"alpha-1\"", "\"alpha-1\" +")]);
    let path = app.join("Cargo.toml");
    let manifest = fs::read_to_string(&path).expect("the manifest could not be read");
    fs::write(&path, format!("{manifest}\n# a test touched this\n"))
        .expect("the manifest could not be written");
    session.wait("waiting for an edit to build again", 1, FAT_TIMEOUT);
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(
        session.output().matches("waiting for an edit").count(),
        1,
        "the build's own output woke the wait:\n{}",
        session.output()
    );
    edit(&app, &[("\"alpha-1\" +", "\"alpha-2\"")]);
    session.wait("app running as pid", 2, FAT_TIMEOUT);
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

/// Gives the copied workspace a helper whose two units carry different
/// dependencies and a fancy feature of ui that pulls helper with its own
/// feature, so a fancy build compiles helper as two units, which one
/// recording cannot replay, and only the union of the units' edges sees
/// the dependency the fancy unit carries.
fn make_helper_ambiguous(ws: &Path) {
    edit_in(
        ws,
        "helper/Cargo.toml",
        &[(
            "edition = \"2024\"",
            "edition = \"2024\"\n\n[dependencies]\ncore2 = { workspace = true, optional = true }\n\n[features]\nfancy = [\"dep:core2\"]",
        )],
    );
    edit_in(
        ws,
        "ui/Cargo.toml",
        &[
            (
                "mac.workspace = true",
                "mac.workspace = true\nhelper = { workspace = true, optional = true }",
            ),
            ("fancy = []", "fancy = [\"dep:helper\", \"helper/fancy\"]"),
        ],
    );
}

/// Adds an `hnum` probe to the copied app, which the macro bakes from the
/// helper's own reading of a source two libraries compile.
fn add_hnum_probe(ws: &Path) {
    edit_in(
        ws,
        "app/src/main.rs",
        &[
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let hnum_address = ui::hnum as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (hnum_value, hnum_hot) = through_deep(table, hnum_address);",
            ),
            (
                "        let _ = io::stdout().flush();",
                "        println!(\n            \"fixture hnum={hnum_value} hnum_hot={hnum_hot}\"\n        );\n        let _ = io::stdout().flush();",
            ),
        ],
    );
}

#[test]
fn patches_a_source_outside_the_crate_directories() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-outside");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // The ui crate root names a file its own directory does not contain,
    // which no watched directory covers.
    fs::write(
        ws.join("ui").join("shared_tag.rs"),
        "//! A source the crate root's directory does not contain.\n\n\
         /// The tag an edit here still patches.\n\
         #[inline(never)]\n\
         pub fn tag() -> &'static str {\n    \"outside-1\"\n}\n",
    )
    .expect("the outside source could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label()",
            "#[path = \"../shared_tag.rs\"]\nmod outside;\n\n#[inline(never)]\npub fn outside_tag() -> &'static str {\n    outside::tag()\n}\n\n#[inline(never)]\npub fn label()",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let outside_address = ui::outside_tag as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (outside_value, outside_hot) = through(table, outside_address);",
            ),
            (
                "        let _ = io::stdout().flush();",
                "        println!(\n            \"fixture outside={outside_value} outside_hot={outside_hot}\"\n        );\n        let _ = io::stdout().flush();",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "ui/shared_tag.rs",
        &[("\"outside-1\"", "\"outside-2\"")],
    );
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("outside=outside-2 outside_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn patches_modules_the_dep_info_names_oddly() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-odd");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A module whose file name carries a space and one whose `#[path]`
    // climbs out of a directory: rustc's dep-info escapes the first and
    // keeps parent components in the second.
    fs::write(
        ws.join("ui").join("src").join("with space.rs"),
        "//! A module whose file name carries a space.\n\n\
         #[inline(never)]\n\
         pub fn tag() -> &'static str {\n    \"space-1\"\n}\n",
    )
    .expect("the spaced module could not be written");
    fs::write(
        ws.join("ui").join("src").join("label.rs"),
        "//! A module a stray path reaches.\n\n\
         #[inline(never)]\n\
         pub fn tag() -> &'static str {\n    \"label-1\"\n}\n",
    )
    .expect("the stray module could not be written");
    fs::create_dir_all(ws.join("ui").join("src").join("nested"))
        .expect("the nested directory could not be made");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label()",
            "#[path = \"with space.rs\"]\nmod space_mod;\n\n#[path = \"nested/../label.rs\"]\nmod label_mod;\n\n#[inline(never)]\npub fn space_tag() -> &'static str {\n    space_mod::tag()\n}\n\n#[inline(never)]\npub fn label_tag() -> &'static str {\n    label_mod::tag()\n}\n\n#[inline(never)]\npub fn label()",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let space_address = ui::space_tag as *const () as usize;\n    let odd_address = ui::label_tag as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (space_value, space_hot) = through(table, space_address);\n        let (odd_value, odd_hot) = through(table, odd_address);",
            ),
            (
                "        let _ = io::stdout().flush();",
                "        println!(\n            \"fixture odd space={space_value} space_hot={space_hot} \\\n             label={odd_value} label_hot={odd_hot}\"\n        );\n        let _ = io::stdout().flush();",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "ui/src/with space.rs",
        &[("\"space-1\"", "\"space-2\"")],
    );
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("space=space-2 space_hot=true", 1, STEP_TIMEOUT);
    edit_in(&ws, "ui/src/label.rs", &[("\"label-1\"", "\"label-2\"")]);
    session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    session.wait("label=label-2 label_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn patches_an_edit_a_replay_taught_the_session() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-late-lib");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // A module the fat build never saw joins the library through a patch,
    // whose replay dep-info is the only place the session can learn it.
    fs::write(
        ws.join("ui").join("src").join("late.rs"),
        "//! A module a replay discovered.\n\n\
         /// The tag an edit here patches through the library.\n\
         #[inline(never)]\n\
         pub fn tag() -> &'static str {\n    \"late-1\"\n}\n",
    )
    .expect("the late module could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "mod late;\n\n#[inline(never)]\npub fn label() -> &'static str {\n    late::tag()\n}",
        )],
    );
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("ui=late-1 ui_hot=true", 1, STEP_TIMEOUT);
    // A restart rebuilds the app with every edit so far, and the new
    // build's session watches the module the edit added.
    session.restart();
    session.wait("rebuilding and restarting the app", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 2, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/late.rs", &[("\"late-1\"", "\"late-2\"")]);
    session.wait("patch 1: applied", 2, STEP_TIMEOUT);
    session.wait("ui=late-2 ui_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn refuses_an_edit_whose_chain_needs_an_ambiguous_crate() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-ambchain");
    make_helper_ambiguous(&ws);
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start_with(&app, &target, &["--features", "ui/fancy"]);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    assert!(
        session
            .output()
            .contains("the build made helper as more than one unit"),
        "the session did not report the two helper units:\n{}",
        session.output()
    );
    // An edit in core2 needs helper in its replay chain, because helper
    // now depends on it and the macro reads helper. Dropping the
    // ambiguous helper from the chain would compile the macro against
    // the stale helper and bake the old expansion in.
    edit_in(&ws, "core2/src/lib.rs", &[("    7", "    9")]);
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("helper must replay") && out.contains("more than one unit"),
        "the chain did not refuse over the ambiguous helper:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused chain:\n{out}"
    );
    // The pending core2 edit keeps refusing later attempts, so no patch
    // can silently drop it.
    edit(&app, &[("\"view-1\"", "\"view-2\"")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("helper must replay"),
        "the second attempt dropped the pending edit:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused chain:\n{out}"
    );
    session.quit();
}

#[test]
fn retires_a_pending_edit_that_can_never_patch() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-ambpending");
    make_helper_ambiguous(&ws);
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start_with(&app, &target, &["--features", "ui/fancy"]);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // A helper edit and an edit in the library of the app's own package
    // land together, and the first one refuses the patch. The second one
    // stays pending.
    edit_in(
        &ws,
        "helper/src/lib.rs",
        &[("\"macro-h-1\"", "\"macro-h-2\"")],
    );
    edit_in(&ws, "app/src/lib.rs", &[("    1", "    2")]);
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("more than one unit"),
        "the ambiguous helper was not refused:\n{out}"
    );
    // The next bin-only save retires the pending library edit instead of
    // letting it ride into a patch that would silently drop it.
    edit_in(&ws, "app/src/bin_only.rs", &[("\"bin-1\"", "\"bin-2\"")]);
    let out = session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("stays unpatched") && out.contains("cannot replay separately"),
        "the pending library edit was not retired:\n{out}"
    );
    let line = line_of(&out, "fixture ui=");
    assert!(
        line.contains("bin=bin-2 bin_hot=true"),
        "the bin edit did not patch: {line}"
    );
    assert!(
        line.contains("branded=macro-h-1"),
        "the refused helper edit patched anyway: {line}"
    );
    session.quit();
}

#[test]
fn replays_both_libraries_of_a_shared_source() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-two-owners");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // Both core2 and helper compile the same source, and the macro bakes
    // the helper's reading of it into ui. One owner alone would leave
    // the other library's copy old while the patch reports success.
    fs::write(
        ws.join("core2").join("src").join("common.rs"),
        "//! A source two libraries compile.\n\n\
         /// The number both libraries read.\n\
         pub const NUMBER: u32 = 7;\n",
    )
    .expect("the shared source could not be written");
    edit_in(
        &ws,
        "core2/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn seed()",
            "mod common;\n\n#[allow(dead_code)]\n#[inline(never)]\npub fn number() -> u32 {\n    common::NUMBER\n}\n\n#[inline(never)]\npub fn seed()",
        )],
    );
    edit_in(
        &ws,
        "helper/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn tag() -> &'static str {\n    TAG.with(|tag| *tag)\n}",
            "#[inline(never)]\npub fn tag() -> &'static str {\n    TAG.with(|tag| *tag)\n}\n\n#[path = \"../../core2/src/common.rs\"]\nmod common;\n\n#[allow(dead_code)]\n#[inline(never)]\npub fn number() -> u32 {\n    common::NUMBER\n}",
        )],
    );
    edit_in(
        &ws,
        "mac/src/lib.rs",
        &[(
            "        .expect(\"the baked tag is a string literal\")\n}",
            "        .expect(\"the baked tag is a string literal\")\n}\n\n/// Bakes the helper's count into the output at expansion time.\n#[proc_macro]\npub fn hnum(_input: TokenStream) -> TokenStream {\n    let baked = helper::number();\n    format!(\"{baked}\")\n        .parse()\n        .expect(\"the baked count is a literal\")\n}",
        )],
    );
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "/// The tag the macro baked in when this crate was compiled",
            "#[inline(never)]\npub fn hnum() -> u32 {\n    mac::hnum!()\n}\n\n/// The tag the macro baked in when this crate was compiled",
        )],
    );
    add_hnum_probe(&ws);
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "core2/src/common.rs",
        &[("pub const NUMBER: u32 = 7;", "pub const NUMBER: u32 = 8;")],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    assert!(
        line.contains("replayed core2, helper"),
        "the patch did not replay both owners of the source: {line}"
    );
    session.wait("hnum=8 hnum_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn rebuilds_when_the_rustflags_change() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-flags");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let flags_a = "-Cdebuginfo=1";
    let flags_b = "-Cdebuginfo=2";
    // The flags belong to the build request, so cargo's cache serves
    // units of the flags the session asks for, never another build's.
    let mut session = Session::start_env(&app, Some(&target), &[], &[("RUSTFLAGS", Some(flags_a))]);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.quit();
    let mut session = Session::start_env(&app, Some(&target), &[], &[("RUSTFLAGS", Some(flags_b))]);
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.quit();
    // The same flags reuse the recordings of the same request.
    let mut session = Session::start_env(&app, Some(&target), &[], &[("RUSTFLAGS", Some(flags_b))]);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    assert!(
        !session.output().contains("the build request changed"),
        "an unchanged request rebuilt from clean:\n{}",
        session.output()
    );
    session.quit();
    // Back to the first flags, the request differs from what the
    // recordings describe, so the workspace rebuilds from clean.
    let mut session = Session::start_env(&app, Some(&target), &[], &[("RUSTFLAGS", Some(flags_a))]);
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.quit();
}

#[test]
fn uses_the_workspaces_target_directory() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-wstarget");
    let app = ws.join("app");
    // With no CARGO_TARGET_DIR, cargo builds the member into the
    // workspace root's directory, which the clean, the build, the records
    // and the extraction must all share.
    let mut session = Session::start_env(&app, None, &[], &[]);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.quit();
    assert!(
        ws.join("target")
            .join("smudge-hot")
            .join("rustc.json")
            .exists(),
        "the records did not land in the workspace's target directory"
    );
    assert!(
        !ws.join("app").join("target").exists(),
        "the session used the member's own target directory"
    );
    // A feature switch must reach the units of the workspace's target,
    // which the wrong directory's clean would leave cached.
    let mut session = Session::start_env(&app, None, &["--features", "ui/fancy"], &[]);
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.quit();
    let mut session = Session::start_env(&app, None, &[], &[]);
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/lib.rs", &[("\"ui-1\"", "\"ui-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    assert!(
        line.contains("replayed ui"),
        "the rebuilt session recorded no library to replay: {line}"
    );
    session.wait("ui=ui-2 ui_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn refuses_a_chain_through_the_package_library() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-libchain");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // The library of the app's own package becomes a macro consumer, so
    // an edit in the macro's helper would have to recompile it, which
    // one patch chain cannot. Dropping the library from the chain would
    // silently leave its copy of the expansion old.
    edit_in(
        &ws,
        "app/Cargo.toml",
        &[(
            "tls.workspace = true",
            "tls.workspace = true\nmac.workspace = true",
        )],
    );
    edit_in(
        &ws,
        "app/src/lib.rs",
        &[(
            "/// Keeps the library's own build honest, which a patch never replays.",
            "/// The tag the macro baked in when this library was compiled, which\n/// a dependency edit can force a recompile of.\n#[inline(never)]\npub fn branded_too() -> &'static str {\n    mac::tag!()\n}\n\n/// Keeps the library's own build honest, which a patch never replays.",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let too_address = smudge_hotapp::branded_too as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (too_value, too_hot) = through(table, too_address);",
            ),
            (
                "        let _ = io::stdout().flush();",
                "        println!(\n            \"fixture too={too_value} too_hot={too_hot}\"\n        );\n        let _ = io::stdout().flush();",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "helper/src/lib.rs",
        &[("\"macro-h-1\"", "\"macro-h-2\"")],
    );
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("cannot replay separately"),
        "the chain through the package library did not refuse:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused chain:\n{out}"
    );
    session.quit();
}

#[test]
fn patches_an_included_source() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-included");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A file the `.rs` walk skips, because an `include_str!` pulls it in,
    // belongs to the library all the same, an extension of its own or
    // none at all.
    fs::write(ws.join("ui").join("src").join("label.txt"), "text-1")
        .expect("the included file could not be written");
    fs::write(ws.join("ui").join("src").join("plain"), "plain-1")
        .expect("the extensionless file could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label()",
            "#[inline(never)]\npub fn included() -> &'static str {\n    include_str!(\"label.txt\")\n}\n\n#[inline(never)]\npub fn plain() -> &'static str {\n    include_str!(\"plain\")\n}\n\n#[inline(never)]\npub fn label()",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let included_address = ui::included as *const () as usize;\n    let plain_address = ui::plain as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (included_value, included_hot) = through(table, included_address);\n        let (plain_value, plain_hot) = through(table, plain_address);",
            ),
            (
                "        let _ = io::stdout().flush();",
                "        println!(\n            \"fixture included={included_value} included_hot={included_hot}\"\n        );\n        println!(\n            \"fixture plain={plain_value} plain_hot={plain_hot}\"\n        );\n        let _ = io::stdout().flush();",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/label.txt", &[("text-1", "text-2")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("included=text-2 included_hot=true", 1, STEP_TIMEOUT);
    edit_in(&ws, "ui/src/plain", &[("plain-1", "plain-2")]);
    session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    session.wait("plain=plain-2 plain_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn patches_a_module_behind_a_symlink() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-symlink");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A `#[path]` through a symlinked directory opens a file whose
    // spelled path and real path differ, and only the file system can
    // say which file the dep-info entry really names.
    fs::create_dir_all(ws.join("ui").join("elsewhere"))
        .expect("the outside directory could not be made");
    fs::write(
        ws.join("ui").join("elsewhere").join("tag.rs"),
        "//! A module behind a symlinked directory.\n\n\
         #[inline(never)]\n\
         pub fn tag() -> &'static str {\n    \"linked-1\"\n}\n",
    )
    .expect("the linked module could not be written");
    std::os::unix::fs::symlink("../elsewhere", ws.join("ui").join("src").join("alias"))
        .expect("the symlink could not be made");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label()",
            "#[path = \"alias/tag.rs\"]\nmod aliased;\n\n#[inline(never)]\npub fn aliased_tag() -> &'static str {\n    aliased::tag()\n}\n\n#[inline(never)]\npub fn label()",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let alias_address = ui::aliased_tag as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (alias_value, alias_hot) = through(table, alias_address);",
            ),
            (
                "        let _ = io::stdout().flush();",
                "        println!(\n            \"fixture alias={alias_value} alias_hot={alias_hot}\"\n        );\n        let _ = io::stdout().flush();",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "ui/elsewhere/tag.rs",
        &[("\"linked-1\"", "\"linked-2\"")],
    );
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("alias=linked-2 alias_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn patches_an_outside_source_a_replay_taught() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-late-out");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // A module outside every watched directory joins the library through
    // a patch, whose replay dep-info is the only place the session can
    // learn to watch it.
    fs::write(
        ws.join("ui").join("late_out.rs"),
        "//! A module outside the watched directories.\n\n\
         /// The tag an edit here patches through the library.\n\
         #[inline(never)]\n\
         pub fn tag() -> &'static str {\n    \"late-out-1\"\n}\n",
    )
    .expect("the outside module could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "#[path = \"../late_out.rs\"]\nmod late_out;\n\n#[inline(never)]\npub fn label() -> &'static str {\n    late_out::tag()\n}",
        )],
    );
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("ui=late-out-1 ui_hot=true", 1, STEP_TIMEOUT);
    edit_in(
        &ws,
        "ui/late_out.rs",
        &[("\"late-out-1\"", "\"late-out-2\"")],
    );
    session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    session.wait("ui=late-out-2 ui_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn refuses_a_chain_a_build_script_reads() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-buildscript");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // The build script bakes what it reads from core2 into ui's own
    // source at compile time, and a patch replays ui against the files
    // that script generated, so a chain that replays core2 would leave
    // the generated number old. Dropping ui from the chain would hide
    // that, so the attempt refuses and the app keeps running.
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The generator of the number ui compiles in.\n\n\
         fn main() {\n    \
         println!(\"cargo:rustc-env=UI_NUMBER={}-{}\", engine::seed(), mac::tag!());\n}\n",
    )
    .expect("the build script could not be written");
    edit_in(
        &ws,
        "ui/Cargo.toml",
        &[(
            "[dependencies]",
            "build = \"build.rs\"\n\n[build-dependencies]\nengine = { package = \"core2\", path = \"../core2\" }\nmac.workspace = true\n\n[dependencies]",
        )],
    );
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label()",
            "#[inline(never)]\npub fn generated() -> u32 {\n    env!(\"UI_NUMBER\").parse().expect(\"the generated number parses\")\n}\n\n#[inline(never)]\npub fn label()",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let generated_address = ui::generated as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (generated_value, generated_hot) = through_deep(table, generated_address);",
            ),
            (
                "        let _ = io::stdout().flush();",
                "        println!(\n            \"fixture generated={generated_value} generated_hot={generated_hot}\"\n        );\n        let _ = io::stdout().flush();",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "core2/src/lib.rs", &[("    7", "    9")]);
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("cannot rerun"),
        "the chain a build script reads did not refuse:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused chain:\n{out}"
    );
    // The script reads helper through the macro it runs, so an edit
    // there refuses through the same guard without naming helper in the
    // script's own arguments.
    edit_in(
        &ws,
        "helper/src/lib.rs",
        &[("\"macro-h-1\"", "\"macro-h-2\"")],
    );
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("cannot rerun"),
        "the chain a build script reads through its dependencies did not refuse:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused chain:\n{out}"
    );
    session.quit();
}

#[test]
fn refuses_a_chain_a_binary_only_build_script_reads() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-binbuild");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // The app package is binary only, so the build script belongs to the
    // tip's own package, which no library record stands for; the guard
    // still has to see it, or an edit in what the script reads patches
    // over the number the binary baked in.
    fs::remove_file(ws.join("app").join("src").join("lib.rs"))
        .expect("the library could not be removed");
    fs::write(
        ws.join("app").join("build.rs"),
        "//! The generator of the number the binary compiles in.\n\n\
         fn main() {\n    \
         println!(\"cargo:rustc-env=APP_NUMBER={}\", core2::seed());\n}\n",
    )
    .expect("the build script could not be written");
    edit_in(
        &ws,
        "app/Cargo.toml",
        &[(
            "[dependencies]",
            "build = \"build.rs\"\n\n[build-dependencies]\ncore2 = { package = \"core2\", path = \"../core2\" }\n\n[dependencies]",
        )],
    );
    edit_in(
        &ws,
        "app/src/bin_only.rs",
        &[("\n    \"bin-1\"\n}", "\n    env!(\"APP_NUMBER\")\n}")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "core2/src/lib.rs", &[("    7", "    9")]);
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("cannot rerun"),
        "the binary-only package's build script did not refuse:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused chain:\n{out}"
    );
    session.quit();
}

/// Points the symlink `link` at `target` in one step, as a rename over
/// it, so a watcher never sees the link missing.
fn retarget(target: &Path, link: &Path) {
    let staged = link.with_extension("staged");
    std::os::unix::fs::symlink(target, &staged).expect("the new symlink could not be made");
    fs::rename(&staged, link).expect("the symlink could not be retargeted");
}

#[test]
fn patches_a_retargeted_symlink() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-retarget");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // The module's dep-info names the link, and the link later points at
    // another file; the retarget must reach the session even when the
    // new target sat still since before it started, which following the
    // link alone would hide behind the target's old time.
    fs::create_dir_all(ws.join("ui").join("data")).expect("the data directory could not be made");
    fs::write(
        ws.join("ui").join("data").join("old.rs"),
        "//! The module the link first names.\n\n\
         #[inline(never)]\n\
         pub fn tag() -> &'static str {\n    \"old-1\"\n}\n",
    )
    .expect("the old target could not be written");
    fs::write(
        ws.join("ui").join("data").join("new.rs"),
        "//! The module the link names after the retarget.\n\n\
         #[inline(never)]\n\
         pub fn tag() -> &'static str {\n    \"new-1\"\n}\n",
    )
    .expect("the new target could not be written");
    std::os::unix::fs::symlink(
        ws.join("ui").join("data").join("old.rs"),
        ws.join("ui").join("src").join("alias.rs"),
    )
    .expect("the symlink could not be made");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label()",
            "#[path = \"alias.rs\"]\nmod aliased;\n\n#[inline(never)]\npub fn aliased_tag() -> &'static str {\n    aliased::tag()\n}\n\n#[inline(never)]\npub fn label()",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let alias_address = ui::aliased_tag as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (alias_value, alias_hot) = through(table, alias_address);",
            ),
            (
                "        let _ = io::stdout().flush();",
                "        println!(\n            \"fixture alias={alias_value} alias_hot={alias_hot}\"\n        );\n        let _ = io::stdout().flush();",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // The new target was written before the session started, so only
    // the link's own time can tell the retarget happened.
    retarget(
        &ws.join("ui").join("data").join("new.rs"),
        &ws.join("ui").join("src").join("alias.rs"),
    );
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("alias=new-1 alias_hot=true", 1, STEP_TIMEOUT);
    // A target that comes into being while the session runs still
    // retargets the link, and the spelled path keeps naming its owner.
    fs::write(
        ws.join("ui").join("data").join("latest.rs"),
        "//! The module the link names last.\n\n\
         #[inline(never)]\n\
         pub fn tag() -> &'static str {\n    \"latest-1\"\n}\n",
    )
    .expect("the latest target could not be written");
    retarget(
        &ws.join("ui").join("data").join("latest.rs"),
        &ws.join("ui").join("src").join("alias.rs"),
    );
    session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    session.wait("alias=latest-1 alias_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn watches_a_source_a_failed_patch_taught() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-failed-out");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // A module outside every watched directory joins the library through
    // a patch whose compile fails; the next save, in that module alone,
    // must still reach a patch, or the session would wait for an event
    // it never arranged to see.
    fs::write(
        ws.join("ui").join("late_out.rs"),
        "//! A module outside the watched directories.\n\n\
         /// The tag an edit here patches through the library.\n\
         #[inline(never)]\n\
         pub fn tag() -> &'static str {\n    missing()\n}\n",
    )
    .expect("the outside module could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "#[path = \"../late_out.rs\"]\nmod late_out;\n\n#[inline(never)]\npub fn label() -> &'static str {\n    late_out::tag()\n}",
        )],
    );
    session.wait("patch 1: compile failed", 1, STEP_TIMEOUT);
    // Only the outside module carries the fix, so the failed compile's
    // dep-info is the only thing that can make the session watch it.
    edit_in(
        &ws,
        "ui/late_out.rs",
        &[(
            "pub fn tag() -> &'static str {\n    missing()\n}",
            "pub fn tag() -> &'static str {\n    \"late-out-1\"\n}",
        )],
    );
    session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    session.wait("ui=late-out-1 ui_hot=true", 1, STEP_TIMEOUT);
    // The module itself now belongs to the library, so an edit there
    // patches on its own.
    edit_in(
        &ws,
        "ui/late_out.rs",
        &[("\"late-out-1\"", "\"late-out-2\"")],
    );
    session.wait("patch 3: applied", 1, STEP_TIMEOUT);
    session.wait("ui=late-out-2 ui_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn watches_a_source_a_failed_link_taught() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-linkfail");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // A module outside every watched directory joins the library through
    // a patch whose link fails on a symbol the library and the app both
    // define; the next save, in that module alone, must still reach a
    // patch, or the session would wait for an event it never arranged
    // to see.
    fs::write(
        ws.join("ui").join("late_out.rs"),
        "//! A module outside the watched directories.\n\n\
         /// The tag an edit here patches through the library.\n\
         #[inline(never)]\n\
         pub fn tag() -> &'static str {\n    \"late-out-1\"\n}\n\n\
         /// The symbol the patch's link cannot carry twice.\n\
         #[unsafe(no_mangle)]\n\
         pub extern \"C\" fn main() {}\n",
    )
    .expect("the outside module could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "#[path = \"../late_out.rs\"]\nmod late_out;\n\n#[inline(never)]\npub fn label() -> &'static str {\n    late_out::tag()\n}",
        )],
    );
    session.wait("patch 1: not built", 1, STEP_TIMEOUT);
    edit_in(
        &ws,
        "ui/late_out.rs",
        &[(
            "pub extern \"C\" fn main() {}",
            "pub extern \"C\" fn late_main() {}",
        )],
    );
    session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    session.wait("ui=late-out-1 ui_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn rebuilds_when_the_build_target_changes() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-buildtarget");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let triple = format!("{}-unknown-linux-gnu", std::env::consts::ARCH);
    // An explicit target triple selects cargo's cached units of its own,
    // even when it names this host, so the request must carry it and a
    // switch must rebuild from clean rather than serve another build's
    // recordings.
    let mut session = Session::start_env(&app, Some(&target), &[], &[]);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.quit();
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("CARGO_BUILD_TARGET", Some(triple.as_str()))],
    );
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.quit();
    let mut session = Session::start_env(&app, Some(&target), &[], &[]);
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // The recordings describe this build, so a library edit still
    // patches through it.
    edit_in(&ws, "ui/src/lib.rs", &[("\"ui-1\"", "\"ui-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("ui=ui-2 ui_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn rebuilds_when_the_compiler_changes() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-compiler");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A compiler cargo selects by path belongs to the build request,
    // even when it answers `rustc -vV` exactly like the one the search
    // path names, so a switch between the two rebuilds from clean and
    // the same one does not.
    let real = real_rustc();
    let copy = Path::new(env!("CARGO_TARGET_TMPDIR")).join("smudge-rustc-copy");
    let script = format!("#!/bin/sh\nexec \"{}\" \"$@\"\n", real.display());
    fs::write(&copy, script).expect("the compiler copy could not be written");
    let permissions = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    fs::set_permissions(&copy, permissions).expect("the copy could not be made runnable");
    let copy = copy.display().to_string();
    let mut session = Session::start_env(&app, Some(&target), &[], &[]);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.quit();
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("CARGO_BUILD_RUSTC", Some(copy.as_str()))],
    );
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.quit();
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("CARGO_BUILD_RUSTC", Some(copy.as_str()))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    assert!(
        !session.output().contains("the build request changed"),
        "an unchanged compiler request rebuilt from clean:\n{}",
        session.output()
    );
    session.quit();
}

#[test]
fn rebuilds_when_the_search_path_compiler_changes() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-pathc");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // Two compilers under the same name in two search-path directories
    // answer `rustc -vV` exactly alike but bake different cfg values in,
    // so the request must carry where the name resolves to: cargo would
    // otherwise serve the first compiler's cached units without running
    // the wrapper again, and the session would replay their recordings.
    let real = real_rustc();
    let comps = Path::new(env!("CARGO_TARGET_TMPDIR")).join("smudge-path-rustcs");
    let a = write_cfg_rustc(&comps.join("a"), &real, "smudge_compiler_a");
    let b = write_cfg_rustc(&comps.join("b"), &real, "smudge_compiler_b");
    gate_label_on_cfg(&ws);
    let search = std::env::var("PATH").unwrap_or_default();
    let ahead = |first: &Path| format!("{}:{}", first.display(), search);
    // The fixture prints its values only after a patch, so a view edit
    // asks the app what the first compiler baked in.
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("PATH", Some(ahead(&a).as_str()))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-A", 1, STEP_TIMEOUT);
    session.quit();
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("PATH", Some(ahead(&b).as_str()))],
    );
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-B", 1, STEP_TIMEOUT);
    session.quit();
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("PATH", Some(ahead(&b).as_str()))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-3\"", "\"view-4\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-B", 1, STEP_TIMEOUT);
    assert!(
        !session.output().contains("the build request changed"),
        "an unchanged search path rebuilt from clean:\n{}",
        session.output()
    );
    session.quit();
}

#[test]
fn tells_compiler_aliases_apart() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-alias");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // Two search-path names can be links to one dispatcher, which picks
    // the compiler by the name it was called by: canonicalizing both
    // aliases onto the dispatcher erases the difference cargo runs on,
    // so the identity must carry the path the invocation uses.
    let real = real_rustc();
    let comps = Path::new(env!("CARGO_TARGET_TMPDIR")).join("smudge-alias-rustcs");
    let _ = std::fs::remove_dir_all(&comps);
    let dispatcher = comps.join("disp");
    fs::create_dir_all(&dispatcher).expect("the dispatcher directory could not be made");
    let script = format!(
        "#!/bin/sh\ncase \"$0\" in\n  *alias-a*) exec \"{}\" \"$@\" --cfg smudge_compiler_a ;;\n  \
         *) exec \"{}\" \"$@\" --cfg smudge_compiler_b ;;\nesac\n",
        real.display(),
        real.display()
    );
    fs::write(dispatcher.join("rustc"), script).expect("the dispatcher could not be written");
    let permissions = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    fs::set_permissions(dispatcher.join("rustc"), permissions)
        .expect("the dispatcher could not be made runnable");
    for alias in ["alias-a", "alias-b"] {
        let dir = comps.join(alias);
        fs::create_dir_all(&dir).expect("the alias directory could not be made");
        std::os::unix::fs::symlink("../disp/rustc", dir.join("rustc"))
            .expect("the alias link could not be made");
    }
    gate_label_on_cfg(&ws);
    let search = std::env::var("PATH").unwrap_or_default();
    let ahead = |first: &Path| format!("{}:{}", first.display(), search);
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("PATH", Some(ahead(&comps.join("alias-a")).as_str()))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-A", 1, STEP_TIMEOUT);
    session.quit();
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("PATH", Some(ahead(&comps.join("alias-b")).as_str()))],
    );
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-B", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn resolves_relative_search_path_entries_like_cargo() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-relpath");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A relative search-path entry resolves where the compiler invocation
    // runs, which is the workspace root, not the app directory the
    // session sits in: cargo execs the compiler from the workspace, so a
    // link retargeted under the workspace's own `bin` must reach the
    // request even while an absolute entry behind it stays still.
    let real = real_rustc();
    let a = write_cfg_rustc(&ws.join("comps-a"), &real, "smudge_compiler_a");
    let b = write_cfg_rustc(&ws.join("comps-b"), &real, "smudge_compiler_b");
    fs::create_dir_all(ws.join("bin")).expect("the bin directory could not be made");
    let retarget = |dir: &Path| {
        let _ = std::fs::remove_file(ws.join("bin").join("rustc"));
        std::os::unix::fs::symlink(
            format!(
                "../{}/rustc",
                dir.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("comps-a")
            ),
            ws.join("bin").join("rustc"),
        )
        .expect("the bin link could not be made");
    };
    retarget(&a);
    gate_label_on_cfg(&ws);
    let search = std::env::var("PATH").unwrap_or_default();
    let env_path = format!("bin:{}:{}", a.display(), search);
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("PATH", Some(env_path.as_str()))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-A", 1, STEP_TIMEOUT);
    session.quit();
    retarget(&b);
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("PATH", Some(env_path.as_str()))],
    );
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-B", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn rebuilds_when_the_configured_compiler_changes() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-cfgrc");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // The config file selects the compiler when no environment variable
    // does, so a switch inside it must reach the request the same way a
    // switch of the environment does, or cargo serves the first
    // compiler's cached units and the session replays them.
    let real = real_rustc();
    let comps = Path::new(env!("CARGO_TARGET_TMPDIR")).join("smudge-config-rustcs");
    let a = write_cfg_rustc(&comps.join("a"), &real, "smudge_compiler_a");
    let b = write_cfg_rustc(&comps.join("b"), &real, "smudge_compiler_b");
    gate_label_on_cfg(&ws);
    fs::create_dir_all(ws.join(".cargo")).expect("the cargo directory could not be made");
    let config = ws.join(".cargo").join("config.toml");
    let select = |dir: &Path| {
        fs::write(
            &config,
            format!("[build]\nrustc = \"{}\"\n", dir.join("rustc").display()),
        )
        .expect("the config could not be written");
    };
    select(&a);
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // The fixture prints its values only after a patch, so a view edit
    // asks the app what the configured compiler baked in.
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-A", 1, STEP_TIMEOUT);
    session.quit();
    select(&b);
    let mut session = Session::start(&app, &target);
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-B", 1, STEP_TIMEOUT);
    session.quit();
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-3\"", "\"view-4\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-B", 1, STEP_TIMEOUT);
    assert!(
        !session.output().contains("the build request changed"),
        "an unchanged config rebuilt from clean:\n{}",
        session.output()
    );
    session.quit();
    // A literal string with a path relative to the config's own place
    // selects the compiler cargo resolves against the directory the
    // config's `.cargo` sits in, which a third wrapper under the
    // workspace holds; the request must follow it the same way.
    write_cfg_rustc(&ws.join("comps-c"), &real, "smudge_compiler_a");
    fs::write(&config, "[build]\nrustc = './comps-c/rustc'\n")
        .expect("the config could not be written");
    let mut session = Session::start(&app, &target);
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-4\"", "\"view-5\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-A", 1, STEP_TIMEOUT);
    session.quit();
    // A dotted key at the top level may carry whitespace around its
    // separators, which cargo accepts, and the switch under the spaced
    // key must reach the request the same way.
    fs::write(
        &config,
        format!("build . rustc = \"{}\"\n", b.join("rustc").display()),
    )
    .expect("the config could not be written");
    let mut session = Session::start(&app, &target);
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-5\"", "\"view-6\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-B", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn rebuilds_when_the_configured_name_is_escaped() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-escname");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A basic string may spell its name with TOML's escapes, which
    // cargo decodes before the lookup: the request must resolve the
    // decoded name through the same search path, or a switch of the
    // directories finds cargo on the first compiler's cached units
    // while both sessions fingerprint a name nobody can run.
    let real = real_rustc();
    let comps = Path::new(env!("CARGO_TARGET_TMPDIR")).join("smudge-escape-rustcs");
    let a = write_cfg_rustc(&comps.join("a"), &real, "smudge_compiler_a");
    let b = write_cfg_rustc(&comps.join("b"), &real, "smudge_compiler_b");
    fs::rename(a.join("rustc"), a.join("cfg-rustc")).expect("the wrapper could not be renamed");
    fs::rename(b.join("rustc"), b.join("cfg-rustc")).expect("the wrapper could not be renamed");
    gate_label_on_cfg(&ws);
    fs::create_dir_all(ws.join(".cargo")).expect("the cargo directory could not be made");
    // `\u0063` spells the `c` the name starts with.
    fs::write(
        ws.join(".cargo").join("config.toml"),
        "[build]\nrustc = \"\\u0063fg-rustc\"\n",
    )
    .expect("the config could not be written");
    let search = std::env::var("PATH").unwrap_or_default();
    let ahead = |first: &Path| format!("{}:{}", first.display(), search);
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("PATH", Some(ahead(&a).as_str()))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-A", 1, STEP_TIMEOUT);
    session.quit();
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("PATH", Some(ahead(&b).as_str()))],
    );
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-B", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn rebuilds_when_the_env_table_selects_the_compiler() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-envpath");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // Cargo's own `[env]` table reaches the processes it runs, and a
    // bare compiler name resolves through the PATH a forced entry gives
    // them: the request must resolve the name through the configured
    // path, or a switch inside the config leaves cargo on the first
    // compiler's cached units while the session fingerprints a compiler
    // nobody runs.
    let real = real_rustc();
    let comps = Path::new(env!("CARGO_TARGET_TMPDIR")).join("smudge-envpath-rustcs");
    let a = write_cfg_rustc(&comps.join("a"), &real, "smudge_compiler_a");
    let b = write_cfg_rustc(&comps.join("b"), &real, "smudge_compiler_b");
    fs::rename(a.join("rustc"), a.join("cfg-rustc")).expect("the wrapper could not be renamed");
    fs::rename(b.join("rustc"), b.join("cfg-rustc")).expect("the wrapper could not be renamed");
    gate_label_on_cfg(&ws);
    fs::create_dir_all(ws.join(".cargo")).expect("the cargo directory could not be made");
    let config = ws.join(".cargo").join("config.toml");
    let search = std::env::var("PATH").unwrap_or_default();
    let select = |dir: &Path| {
        fs::write(
            &config,
            format!(
                "[build]\nrustc = 'cfg-rustc'\n\n[env.PATH]\nvalue = '{}:{}'\nforce = true\n",
                dir.display(),
                search
            ),
        )
        .expect("the config could not be written");
    };
    select(&a);
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-A", 1, STEP_TIMEOUT);
    session.quit();
    select(&b);
    let mut session = Session::start(&app, &target);
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-B", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn rebuilds_when_a_bare_configured_name_changes() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-barec");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A configured `rustc` without a separator is a search-path lookup
    // for cargo, not a file beside the config, so the request must
    // follow the search path the lookup resolves through, or a switch of
    // the directories leaves cargo on the first compiler's cached units.
    let real = real_rustc();
    let comps = Path::new(env!("CARGO_TARGET_TMPDIR")).join("smudge-bare-rustcs");
    let a = write_cfg_rustc(&comps.join("a"), &real, "smudge_compiler_a");
    let b = write_cfg_rustc(&comps.join("b"), &real, "smudge_compiler_b");
    fs::rename(a.join("rustc"), a.join("cfg-rustc")).expect("the wrapper could not be renamed");
    fs::rename(b.join("rustc"), b.join("cfg-rustc")).expect("the wrapper could not be renamed");
    gate_label_on_cfg(&ws);
    fs::create_dir_all(ws.join(".cargo")).expect("the cargo directory could not be made");
    fs::write(
        ws.join(".cargo").join("config.toml"),
        "[build]\nrustc = 'cfg-rustc'\n",
    )
    .expect("the config could not be written");
    let search = std::env::var("PATH").unwrap_or_default();
    let ahead = |first: &Path| format!("{}:{}", first.display(), search);
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("PATH", Some(ahead(&a).as_str()))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-A", 1, STEP_TIMEOUT);
    session.quit();
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("PATH", Some(ahead(&b).as_str()))],
    );
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-B", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn rebuilds_when_the_home_config_compiler_changes() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-homerc");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // With no CARGO_HOME of its own, cargo reads the config in the
    // `.cargo` directory of the user's home, so a switch inside it must
    // reach the request the same way a switch in the workspace's config
    // does, or cargo serves the first compiler's cached units and the
    // session replays them. The value is a literal string, which the
    // same reading must carry.
    let real = real_rustc();
    let comps = Path::new(env!("CARGO_TARGET_TMPDIR")).join("smudge-home-rustcs");
    let a = write_cfg_rustc(&comps.join("a"), &real, "smudge_compiler_a");
    let b = write_cfg_rustc(&comps.join("b"), &real, "smudge_compiler_b");
    gate_label_on_cfg(&ws);
    let home = Path::new(env!("CARGO_TARGET_TMPDIR")).join("smudge-home");
    fs::create_dir_all(home.join(".cargo")).expect("the home cargo directory could not be made");
    // The empty home leaves cargo without the registry the fixture's
    // dependencies resolve from, so the real one is linked in; the
    // config stays a real file of the scratch home.
    let real_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|dir| dir.join(".cargo")))
        .expect("cargo's real home answers");
    let registry = home.join(".cargo").join("registry");
    let _ = std::fs::remove_dir_all(&registry);
    let _ = std::fs::remove_file(&registry);
    std::os::unix::fs::symlink(real_home.join("registry"), &registry)
        .expect("the registry link could not be made");
    let config = home.join(".cargo").join("config.toml");
    let select = |dir: &Path| {
        fs::write(
            &config,
            format!("[build]\nrustc = '{}'\n", dir.join("rustc").display()),
        )
        .expect("the home config could not be written");
    };
    select(&a);
    let env = [
        ("CARGO_HOME", None),
        ("HOME", Some(home.to_str().expect("the home spells"))),
    ];
    let mut session = Session::start_env(&app, Some(&target), &[], &env);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-A", 1, STEP_TIMEOUT);
    session.quit();
    select(&b);
    let mut session = Session::start_env(&app, Some(&target), &[], &env);
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-B", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn reads_the_legacy_config_file_first() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-legacy");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // When both configuration file names stand in one directory, cargo
    // reads the legacy `config` and warns about it, so the request must
    // fingerprint the compiler the legacy file selects, and follow the
    // TOML file again once the legacy one goes away.
    let real = real_rustc();
    let comps = Path::new(env!("CARGO_TARGET_TMPDIR")).join("smudge-legacy-rustcs");
    let a = write_cfg_rustc(&comps.join("a"), &real, "smudge_compiler_a");
    let b = write_cfg_rustc(&comps.join("b"), &real, "smudge_compiler_b");
    gate_label_on_cfg(&ws);
    fs::create_dir_all(ws.join(".cargo")).expect("the cargo directory could not be made");
    let select = |path: &Path, dir: &Path| {
        fs::write(
            path,
            format!("[build]\nrustc = \"{}\"\n", dir.join("rustc").display()),
        )
        .expect("the config could not be written");
    };
    let toml = ws.join(".cargo").join("config.toml");
    let legacy = ws.join(".cargo").join("config");
    select(&toml, &a);
    select(&legacy, &b);
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-B", 1, STEP_TIMEOUT);
    session.quit();
    fs::remove_file(&legacy).expect("the legacy config could not go");
    let mut session = Session::start(&app, &target);
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-A", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn refuses_a_chain_a_build_dependency_script_reads() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-genbuild");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // The generator is only a build dependency, of ui's build script, so
    // no library of the app's extern graph names its package; its own
    // build script reads core2, and a patch that replays core2 would
    // leave the number it generated old, so the attempt refuses.
    fs::create_dir_all(ws.join("generator").join("src")).expect("the generator could not be made");
    fs::write(
        ws.join("generator").join("Cargo.toml"),
        "[package]\nname = \"generator\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
         [build-dependencies]\ncore2 = { path = \"../core2\" }\n",
    )
    .expect("the generator's manifest could not be written");
    fs::write(
        ws.join("generator").join("build.rs"),
        "//! The generator of the number this crate compiles in.\n\n\
         fn main() {\n    \
         println!(\"cargo:rustc-env=GEN_NUMBER={}\", core2::seed());\n}\n",
    )
    .expect("the generator's build script could not be written");
    fs::write(
        ws.join("generator").join("src").join("lib.rs"),
        "//! The number the build script of this crate baked in.\n\n\
         #[inline(never)]\npub fn number() -> u32 {\n    \
         env!(\"GEN_NUMBER\").parse().expect(\"the generated number parses\")\n}\n",
    )
    .expect("the generator could not be written");
    edit_in(
        &ws,
        "ui/Cargo.toml",
        &[(
            "[features]",
            "[build-dependencies]\ngenerator = { path = \"../generator\" }\n\n[features]",
        )],
    );
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The generator of the number ui compiles in.\n\n\
         fn main() {\n    \
         println!(\"cargo:rustc-env=UI_NUMBER={}\", generator::number());\n}\n",
    )
    .expect("ui's build script could not be written");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "core2/src/lib.rs", &[("    7", "    9")]);
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("the build script of generator reads core2"),
        "the build-only package's script did not refuse:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused chain:\n{out}"
    );
    session.quit();
}

#[test]
fn refuses_an_edit_a_build_script_compiles_from() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-scriptin");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // The script compiles the same module the library does, so an edit
    // to the constant moves the expansion the script generates and the
    // library that reads it together. A patch can replay the library
    // but not rerun the script, so the attempt refuses instead of
    // reporting success over the constant the app never saw.
    fs::write(
        ws.join("ui").join("src").join("config.rs"),
        "//! The label the script and the library share.\n\n\
         pub const LABEL: &str = \"script-1\";\n",
    )
    .expect("the shared module could not be written");
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The emitter of the label the library reads.\n\n\
         #[path = \"src/config.rs\"]\nmod config;\n\n\
         fn main() {\n    \
         println!(\"cargo:rustc-env=UI_LABEL={}\", config::LABEL);\n}\n",
    )
    .expect("ui's build script could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "mod config;\n\n#[inline(never)]\npub fn label() -> &'static str {\n    env!(\"UI_LABEL\")\n}",
        )],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=script-1", 1, STEP_TIMEOUT);
    edit_in(&ws, "ui/src/config.rs", &[("\"script-1\"", "\"script-2\"")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("the build script of ui reads"),
        "the script's own source did not refuse the patch:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused script input:\n{out}"
    );
    // The refusal outlives the attempt: a later save that only touches
    // the app must not consume the stale script and report success over
    // the constant the app never saw.
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 3: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("cannot rerun"),
        "the stale script did not refuse the later save:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on the stale script:\n{out}"
    );
    session.quit();
}

#[test]
fn watches_a_build_scripts_content_input() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-scripttxt");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // The script reads a content file no library reads: the session
    // knows the input from the script's dep-info, so it watches it and
    // refuses when it moves, instead of leaving the save unobserved and
    // replaying ui against the expansion the script's last run made.
    fs::write(ws.join("ui").join("config.txt"), "script-1")
        .expect("the content could not be written");
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The emitter of the label the library reads.\n\n\
         fn main() {\n    \
         println!(\"cargo:rustc-env=UI_LABEL={}\", include_str!(\"config.txt\").trim());\n}\n",
    )
    .expect("ui's build script could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    env!(\"UI_LABEL\")")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=script-1", 1, STEP_TIMEOUT);
    edit_in(&ws, "ui/config.txt", &[("script-1", "script-2")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("the build script of ui reads"),
        "the script's content input did not refuse the patch:\n{out}"
    );
    // And the refusal outlives the attempt that saw it.
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 3: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("cannot rerun"),
        "the stale script did not refuse the later save:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on the stale script:\n{out}"
    );
    session.quit();
}

#[test]
fn guards_a_build_script_under_another_name() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-scriptname");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A script under a name of its own compiles as `build_script_codegen`
    // instead of `build_script_build`, and the wrapper must still record
    // it as a script: without the recording the guard is blind, and a
    // patch replays what the script reads while ui keeps compiling
    // against the expansion the script's last run made.
    fs::write(
        ws.join("ui").join("codegen.rs"),
        "//! The generator of the number ui compiles in.\n\n\
         fn main() {\n    \
         println!(\"cargo:rustc-env=UI_NUMBER={}\", engine::seed());\n}\n",
    )
    .expect("the build script could not be written");
    edit_in(
        &ws,
        "ui/Cargo.toml",
        &[(
            "[dependencies]",
            "build = \"codegen.rs\"\n\n[build-dependencies]\nengine = { package = \"core2\", path = \"../core2\" }\n\n[dependencies]",
        )],
    );
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label()",
            "#[inline(never)]\npub fn generated() -> u32 {\n    env!(\"UI_NUMBER\").parse().expect(\"the generated number parses\")\n}\n\n#[inline(never)]\npub fn label()",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let generated_address = ui::generated as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (generated_value, generated_hot) = through_deep(table, generated_address);",
            ),
            (
                "        let _ = io::stdout().flush();",
                "        println!(\n            \"fixture generated={generated_value} generated_hot={generated_hot}\"\n        );\n        let _ = io::stdout().flush();",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "core2/src/lib.rs", &[("    7", "    9")]);
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("the build script of ui reads core2"),
        "the renamed build script did not refuse:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused chain:\n{out}"
    );
    session.quit();
}

#[test]
fn watches_an_input_the_script_reads_at_run_time() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-rerunif");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A file the script reads while it runs never appears in the
    // dep-info of the script's own compile; the run's declared inputs
    // are the session's only source for it, and the restart that reruns
    // the script must follow the edit the session refused.
    fs::write(ws.join("ui").join("config.txt"), "script-1")
        .expect("the content could not be written");
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The emitter of the label the library reads.\n\n\
         fn main() {\n    \
         println!(\"cargo:rerun-if-changed=config.txt\");\n    \
         let label = std::fs::read_to_string(\"config.txt\").expect(\"the input is readable\");\n    \
         println!(\"cargo:rustc-env=UI_LABEL={}\", label.trim());\n}\n",
    )
    .expect("ui's build script could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    env!(\"UI_LABEL\")")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=script-1", 1, STEP_TIMEOUT);
    edit_in(&ws, "ui/config.txt", &[("script-1", "script-2")]);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("the build script of ui reads"),
        "the runtime input did not refuse the patch:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused script input:\n{out}"
    );
    session.quit();
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-3\"", "\"view-4\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=script-2", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn watches_a_directory_the_script_declares() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-rerundir");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A declared directory's content moves one file at a time, which no
    // watched path follows by itself; the newest time under the
    // directory is what tells the session the script's inputs moved,
    // and the restart that reruns the script must bake the new content.
    fs::create_dir_all(ws.join("ui").join("assets"))
        .expect("the assets directory could not be made");
    fs::write(ws.join("ui").join("assets").join("label.txt"), "in-dir-1")
        .expect("the content could not be written");
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The emitter of the label the library reads.\n\n\
         fn main() {\n    \
         println!(\"cargo:rerun-if-changed=assets\");\n    \
         let label = std::fs::read_to_string(\"assets/label.txt\").expect(\"the input is readable\");\n    \
         println!(\"cargo:rustc-env=UI_LABEL={}\", label.trim());\n}\n",
    )
    .expect("ui's build script could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    env!(\"UI_LABEL\")")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=in-dir-1", 1, STEP_TIMEOUT);
    edit_in(&ws, "ui/assets/label.txt", &[("in-dir-1", "in-dir-2")]);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("cannot rerun"),
        "the declared directory did not refuse the patch:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused script input:\n{out}"
    );
    session.quit();
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-3\"", "\"view-4\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=in-dir-2", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn refuses_when_a_declared_directory_hides_behind_a_future_stamp() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-rerundir-future");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A declared directory's guard answers every entry it carries, not
    // the newest stamp among them: a file stamped an hour ahead of the
    // session would hold a watermark still while a neighbor the script
    // reads moved under it.
    fs::create_dir_all(ws.join("ui").join("assets"))
        .expect("the assets directory could not be made");
    fs::write(ws.join("ui").join("assets").join("label.txt"), "in-dir-1")
        .expect("the content could not be written");
    let sentinel = ws.join("ui").join("assets").join("sentinel.txt");
    fs::write(&sentinel, "ahead").expect("the sentinel could not be written");
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&sentinel)
        .expect("the sentinel could not be opened");
    let ahead = std::time::SystemTime::now() + Duration::from_secs(3600);
    file.set_modified(ahead)
        .expect("the stamp could not be moved ahead");
    drop(file);
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The emitter of the label the library reads.\n\n\
         fn main() {\n    \
         println!(\"cargo:rerun-if-changed=assets\");\n    \
         let label = std::fs::read_to_string(\"assets/label.txt\").expect(\"the input is readable\");\n    \
         println!(\"cargo:rustc-env=UI_LABEL={}\", label.trim());\n}\n",
    )
    .expect("ui's build script could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    env!(\"UI_LABEL\")")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=in-dir-1", 1, STEP_TIMEOUT);
    edit_in(&ws, "ui/assets/label.txt", &[("in-dir-1", "in-dir-2")]);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("cannot rerun"),
        "the declared directory under a future stamp did not refuse the patch:\n{out}"
    );
    session.quit();
}

#[test]
fn refuses_when_a_script_input_vanishes() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-scriptgone");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A removed input produces no save the watcher could report: the
    // disappearance itself must mark the session's scripts stale, or a
    // later save would patch over an expansion the next build cannot
    // even reproduce.
    fs::write(ws.join("ui").join("config.txt"), "script-1")
        .expect("the content could not be written");
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The emitter of the label the library reads.\n\n\
         fn main() {\n    \
         println!(\"cargo:rustc-env=UI_LABEL={}\", include_str!(\"config.txt\").trim());\n}\n",
    )
    .expect("ui's build script could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    env!(\"UI_LABEL\")")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=script-1", 1, STEP_TIMEOUT);
    fs::remove_file(ws.join("ui").join("config.txt")).expect("the input could not be removed");
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("cannot rerun"),
        "the vanished input did not refuse the patch:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused script input:\n{out}"
    );
    session.quit();
}

#[test]
fn keeps_the_active_script_of_a_feature_switch() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-scriptfeat");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // The script reads core2 only under the feature that turns its
    // build dependency on; the recording that feature set left behind
    // must not guard the session that builds without it, whose script
    // never touches the dependency, or every edit in core2 would
    // refuse for a reason the current build no longer has.
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The generator of the number ui compiles in.\n\n\
         fn main() {\n    \
         #[cfg(feature = \"scripted\")]\n    \
         println!(\"cargo:rustc-env=UI_NUMBER={}\", engine::seed());\n    \
         #[cfg(not(feature = \"scripted\"))]\n    \
         println!(\"cargo:rustc-env=UI_NUMBER=42\");\n}\n",
    )
    .expect("ui's build script could not be written");
    edit_in(
        &ws,
        "ui/Cargo.toml",
        &[
            ("[dependencies]", "build = \"build.rs\"\n\n[dependencies]"),
            (
                "[features]\nfancy = []",
                "[features]\nfancy = []\nscripted = [\"dep:engine\"]\n\n[build-dependencies]\nengine = { package = \"core2\", path = \"../core2\", optional = true }",
            ),
        ],
    );
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    env!(\"UI_NUMBER\")")],
    );
    let mut session = Session::start_with(&app, &target, &["--features", "ui/scripted"]);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=7", 1, STEP_TIMEOUT);
    // Under the feature, the dependency the script reads refuses.
    edit_in(&ws, "core2/src/lib.rs", &[("    7", "    9")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("the build script of ui reads core2"),
        "the feature's script did not refuse:\n{out}"
    );
    session.quit();
    // Without it, the script's active recording names no dependency, so
    // a further edit in the same dependency patches through the chain
    // the build really has.
    let mut session = Session::start(&app, &target);
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    edit_in(&ws, "core2/src/lib.rs", &[("    9", "    11")]);
    let out = session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("replayed core2"),
        "the dependency the inactive script once read did not replay:\n{out}"
    );
    assert!(
        !out.contains("cannot rerun"),
        "the retired recording refused the active build:\n{out}"
    );
    session.quit();
}

#[test]
fn watches_a_second_variant_of_the_same_script() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-scripttwo");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // Resolver two builds a package's script once per feature set the
    // build names it with, and each run declares its own inputs: the
    // run whose output happens to be newest cannot speak for the other,
    // whose input would otherwise move unseen and let a patch report
    // success over an expansion the next build cannot reproduce. The
    // app carries a script of its own so the feature variant really
    // compiles: cargo builds a build-dependency for a package with no
    // build script never, and the variant's run would stay out of the
    // window the guard reads.
    fs::write(ws.join("ui").join("normal.txt"), "normal-1")
        .expect("the normal content could not be written");
    fs::write(ws.join("ui").join("host.txt"), "host-1")
        .expect("the host content could not be written");
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The emitter of the label the library reads.\n\n\
         fn main() {\n    \
         let scripted = std::env::var_os(\"CARGO_FEATURE_EXTRA\").is_some();\n    \
         let named = if scripted { \"host.txt\" } else { \"normal.txt\" };\n    \
         println!(\"cargo:rerun-if-changed={}\", named);\n    \
         let label = std::fs::read_to_string(named).unwrap_or_else(|_| String::from(\"fallback\"));\n    \
         println!(\"cargo:rustc-env=UI_LABEL={}\", label.trim());\n}\n",
    )
    .expect("ui's build script could not be written");
    fs::write(
        ws.join("app").join("build.rs"),
        "//! The consumer that makes the build-dependency compile.\n\n\
         fn main() {\n    \
         let _ = ui::label();\n}\n",
    )
    .expect("the app's build script could not be written");
    edit_in(
        &ws,
        "ui/Cargo.toml",
        &[(
            "[features]\nfancy = []",
            "[features]\nfancy = []\nextra = []",
        )],
    );
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    env!(\"UI_LABEL\")")],
    );
    edit_in(
        &ws,
        "app/Cargo.toml",
        &[(
            "[dependencies]",
            "[build-dependencies]\nui = { workspace = true, features = [\"extra\"] }\n\n[dependencies]",
        )],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=normal-1", 1, STEP_TIMEOUT);
    // The variant the newest output does not carry declares this file;
    // the patch that follows must refuse either way.
    edit_in(&ws, "ui/normal.txt", &[("normal-1", "normal-2")]);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("the build script of ui reads") && out.contains("normal.txt"),
        "the other variant's input did not refuse the patch:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused script input:\n{out}"
    );
    session.quit();
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-3\"", "\"view-4\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=normal-2", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn watches_a_nested_directory_input() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-rerunsub");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A removal inside a declared directory moves only the time of the
    // directory the file sat in, which no scan of the surviving files
    // follows: the walk must measure the directories themselves, or the
    // patch after the removal replays over an expansion the next build
    // generates from other content.
    fs::create_dir_all(ws.join("ui").join("assets").join("sub"))
        .expect("the assets directory could not be made");
    fs::write(
        ws.join("ui").join("assets").join("sub").join("label.txt"),
        "in-sub-1",
    )
    .expect("the content could not be written");
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The emitter of the label the library reads.\n\n\
         fn main() {\n    \
         println!(\"cargo:rerun-if-changed=assets\");\n    \
         let label = std::fs::read_to_string(\"assets/sub/label.txt\")\n        \
         .map(|content| content.trim().to_string())\n        \
         .unwrap_or_else(|_| String::from(\"fallback\"));\n    \
         println!(\"cargo:rustc-env=UI_LABEL={}\", label);\n}\n",
    )
    .expect("ui's build script could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    env!(\"UI_LABEL\")")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=in-sub-1", 1, STEP_TIMEOUT);
    fs::remove_file(ws.join("ui").join("assets").join("sub").join("label.txt"))
        .expect("the nested input could not be removed");
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("cannot rerun"),
        "the nested removal did not refuse the patch:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused script input:\n{out}"
    );
    session.quit();
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-3\"", "\"view-4\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=fallback", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn watches_a_declared_directory_itself() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-rerundirgone");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A declared directory that is gone answers no content time at all,
    // which reads as still: the disappearance itself must mark the
    // scripts stale, the way a vanished file does, or the patch runs on
    // an expansion the next build makes without the directory.
    fs::create_dir_all(ws.join("ui").join("assets"))
        .expect("the assets directory could not be made");
    fs::write(ws.join("ui").join("assets").join("label.txt"), "in-dir-1")
        .expect("the content could not be written");
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The emitter of the label the library reads.\n\n\
         fn main() {\n    \
         println!(\"cargo:rerun-if-changed=assets\");\n    \
         let label = std::fs::read_to_string(\"assets/label.txt\")\n        \
         .map(|content| content.trim().to_string())\n        \
         .unwrap_or_else(|_| String::from(\"fallback\"));\n    \
         println!(\"cargo:rustc-env=UI_LABEL={}\", label);\n}\n",
    )
    .expect("ui's build script could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    env!(\"UI_LABEL\")")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=in-dir-1", 1, STEP_TIMEOUT);
    fs::rename(
        ws.join("ui").join("assets"),
        ws.join("ui").join("assets-gone"),
    )
    .expect("the directory could not be moved away");
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("cannot rerun"),
        "the moved directory did not refuse the patch:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused script input:\n{out}"
    );
    session.quit();
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-3\"", "\"view-4\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=fallback", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn watches_a_declared_input_it_spells_through_a_parent() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-rerunparent");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A declared input spelled `src/../extra.rs` answers outside `src`,
    // where no walk of the watched directories reaches it, and while it
    // is absent nothing remembers it either: its creation must still
    // refuse the patch, since the script reads it the moment it exists.
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The emitter of the label the library reads.\n\n\
         fn main() {\n    \
         println!(\"cargo:rerun-if-changed=src/../extra.rs\");\n    \
         let label = std::fs::read_to_string(\"extra.rs\")\n        \
         .map(|content| content.trim().to_string())\n        \
         .unwrap_or_else(|_| String::from(\"absent-1\"));\n    \
         println!(\"cargo:rustc-env=UI_LABEL={}\", label);\n}\n",
    )
    .expect("ui's build script could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    env!(\"UI_LABEL\")")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=absent-1", 1, STEP_TIMEOUT);
    fs::write(ws.join("ui").join("extra.rs"), "baked-2")
        .expect("the created input could not be written");
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("the build script of ui reads"),
        "the created input did not refuse the patch:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused script input:\n{out}"
    );
    session.quit();
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-3\"", "\"view-4\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=baked-2", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn watches_a_symlink_an_inner_link_carries() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-linkinner");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // The watched name holds a link of its own, whose target holds
    // another: retargeting the inner link changes what the compile
    // reads while the watched link and both ends stand still, so the
    // walk must splice every link's target into the names it measures,
    // not stop at the link the spelling carries.
    fs::create_dir_all(ws.join("ui").join("a")).expect("the first branch could not be made");
    fs::create_dir_all(ws.join("ui").join("b")).expect("the second branch could not be made");
    fs::write(ws.join("ui").join("a").join("tag.txt"), "chain-a")
        .expect("the first content could not be written");
    fs::write(ws.join("ui").join("b").join("tag.txt"), "chain-b")
        .expect("the second content could not be written");
    std::os::unix::fs::symlink("../redirect", ws.join("ui").join("src").join("data"))
        .expect("the outer link could not be made");
    std::os::unix::fs::symlink("a", ws.join("ui").join("redirect"))
        .expect("the inner link could not be made");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    include_str!(\"data/tag.txt\").trim()")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=chain-a", 1, STEP_TIMEOUT);
    fs::remove_file(ws.join("ui").join("redirect")).expect("the inner link could not be removed");
    std::os::unix::fs::symlink("b", ws.join("ui").join("redirect"))
        .expect("the inner link could not be retargeted");
    session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=chain-b", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn watches_a_library_input_that_vanishes() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-libgone");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A library input that is gone produces no save the watcher could
    // report, and its owners must hear of the loss: the patch that
    // follows cannot compile the library anymore, so it must fail
    // loudly instead of reporting success over content the app already
    // carries.
    fs::write(ws.join("ui").join("content.txt"), "content-1")
        .expect("the content could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "    \"ui-1\"",
            "    include_str!(\"../content.txt\").trim()",
        )],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=content-1", 1, STEP_TIMEOUT);
    fs::remove_file(ws.join("ui").join("content.txt")).expect("the input could not be removed");
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 2: compile failed", 1, STEP_TIMEOUT);
    assert!(
        !out.contains("patch 2: applied"),
        "the vanished input patched over the loss:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a failed compile:\n{out}"
    );
    session.quit();
}

#[test]
fn rebuilds_when_a_manifest_moves_a_feature() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-scriptdefault");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // The feature that turns the script's dependency on can move into
    // the manifest's defaults instead of the command line: the request
    // must follow the manifest, or the second session would hold the
    // retired recording as its own and refuse edits the current build
    // no longer answers for.
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The generator of the number ui compiles in.\n\n\
         fn main() {\n    \
         #[cfg(feature = \"scripted\")]\n    \
         println!(\"cargo:rustc-env=UI_NUMBER={}\", engine::seed());\n    \
         #[cfg(not(feature = \"scripted\"))]\n    \
         println!(\"cargo:rustc-env=UI_NUMBER=42\");\n}\n",
    )
    .expect("ui's build script could not be written");
    edit_in(
        &ws,
        "ui/Cargo.toml",
        &[
            ("[dependencies]", "build = \"build.rs\"\n\n[dependencies]"),
            (
                "[features]\nfancy = []",
                "[features]\nfancy = []\nscripted = [\"dep:engine\"]\ndefault = [\"scripted\"]\n\n[build-dependencies]\nengine = { package = \"core2\", path = \"../core2\", optional = true }",
            ),
        ],
    );
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    env!(\"UI_NUMBER\")")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=7", 1, STEP_TIMEOUT);
    edit_in(&ws, "core2/src/lib.rs", &[("    7", "    9")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("the build script of ui reads core2"),
        "the default feature's script did not refuse:\n{out}"
    );
    session.quit();
    // The manifest drops the default, the request renews, and the
    // retired recording stays out of the session that builds without
    // the dependency.
    edit_in(&ws, "ui/Cargo.toml", &[("\ndefault = [\"scripted\"]", "")]);
    let mut session = Session::start(&app, &target);
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=42", 1, STEP_TIMEOUT);
    edit_in(&ws, "core2/src/lib.rs", &[("    9", "    11")]);
    let out = session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("replayed core2"),
        "the dependency the retired script read did not replay:\n{out}"
    );
    assert!(
        !out.contains("cannot rerun"),
        "the retired recording refused the active build:\n{out}"
    );
    session.quit();
}

#[test]
fn drops_the_guard_of_a_script_the_package_loses() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-scriptgone2");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A package that no longer carries a build script keeps no guard
    // from a recording of another request: the library still belongs to
    // the build, which the fallback alone cannot tell from a package
    // whose script remains, and the edits the dead script once read
    // would refuse forever.
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The generator of the number ui compiles in.\n\n\
         fn main() {\n    \
         println!(\"cargo:rustc-env=UI_NUMBER={}\", engine::seed());\n}\n",
    )
    .expect("ui's build script could not be written");
    edit_in(
        &ws,
        "ui/Cargo.toml",
        &[(
            "[dependencies]",
            "build = \"build.rs\"\n\n[build-dependencies]\nengine = { package = \"core2\", path = \"../core2\" }\n\n[dependencies]",
        )],
    );
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    env!(\"UI_NUMBER\")")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "core2/src/lib.rs", &[("    7", "    9")]);
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("the build script of ui reads core2"),
        "the script did not refuse while it stood:\n{out}"
    );
    session.quit();
    // The script is gone: the manifest loses its build target and the
    // feature selection moves, so the old recording answers no request
    // this build makes.
    edit_in(
        &ws,
        "ui/Cargo.toml",
        &[(
            "build = \"build.rs\"\n\n[build-dependencies]\nengine = { package = \"core2\", path = \"../core2\" }\n\n[dependencies]",
            "[dependencies]",
        )],
    );
    fs::remove_file(ws.join("ui").join("build.rs")).expect("the build script could not be removed");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    env!(\"UI_NUMBER\")", "    \"ui-1\"")],
    );
    let mut session = Session::start_with(&app, &target, &["--features", "ui/fancy"]);
    session.wait("the build request changed", 1, STEP_TIMEOUT);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    edit_in(&ws, "core2/src/lib.rs", &[("    9", "    11")]);
    let out = session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("replayed core2"),
        "the dependency the lost script once read did not replay:\n{out}"
    );
    assert!(
        !out.contains("cannot rerun"),
        "the guard of the lost script refused the active build:\n{out}"
    );
    session.quit();
}

#[test]
fn keeps_the_request_when_a_terminal_variable_moves() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-envnoise");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A cargo variable that only names how cargo talks decides nothing
    // about the units the build makes: the request must not follow it,
    // or every session of a differently decorated terminal rebuilds
    // the workspace from clean over nothing.
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("CARGO_TERM_COLOR", Some("never"))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.quit();
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("CARGO_TERM_COLOR", Some("always"))],
    );
    let out = session.wait("app running as pid", 1, FAT_TIMEOUT);
    assert!(
        !out.contains("the build request changed"),
        "a presentation variable rebuilt the workspace:\n{out}"
    );
    session.quit();
}

#[test]
fn watches_a_lookup_path_through_a_symlink() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-linkdotdot");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A dep-info entry that climbs through a link with `..` spells the
    // link itself, and the link's retarget changes what the compile
    // reads while both ends of every normalized path stand still: the
    // spelled path is the only form that can see it, and the patch that
    // follows must bake the content the link now names.
    fs::create_dir_all(ws.join("ui").join("a").join("sub"))
        .expect("the first branch could not be made");
    fs::create_dir_all(ws.join("ui").join("b").join("sub"))
        .expect("the second branch could not be made");
    fs::write(ws.join("ui").join("a").join("tag.txt"), "look-a")
        .expect("the first content could not be written");
    fs::write(ws.join("ui").join("b").join("tag.txt"), "look-b")
        .expect("the second content could not be written");
    std::os::unix::fs::symlink(
        ws.join("ui").join("a").join("sub"),
        ws.join("ui").join("alias"),
    )
    .expect("the link could not be made");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "    \"ui-1\"",
            "    include_str!(\"../alias/../tag.txt\").trim()",
        )],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=look-a", 1, STEP_TIMEOUT);
    fs::remove_file(ws.join("ui").join("alias")).expect("the link could not be removed");
    std::os::unix::fs::symlink(
        ws.join("ui").join("b").join("sub"),
        ws.join("ui").join("alias"),
    )
    .expect("the link could not be retargeted");
    session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=look-b", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn keeps_installed_edits_through_a_refused_patch() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-refusal");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // A patch the app refuses leaves the app running with the patches it
    // installed before, and the next patch, which replaces the whole
    // table, must still carry the library edit those installed.
    edit_in(&ws, "ui/src/lib.rs", &[("\"ui-1\"", "\"ui-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-2 ui_hot=true", 1, STEP_TIMEOUT);
    // An app-only edit that links but cannot load: the shared library
    // carries a symbol nothing defines, which the loader refuses.
    let broken = "        unsafe extern \"C\" {\n            fn smudge_test_missing();\n        }\n        \
         unsafe { smudge_test_missing() };\n        let table";
    edit_in(&ws, "app/src/main.rs", &[("        let table", broken)]);
    let out = session.wait("patch 2: the app refused", 1, STEP_TIMEOUT);
    assert!(
        out.contains("the app keeps running"),
        "the refusal did not keep the app:\n{out}"
    );
    // The fix is app-only as well, so only the installed libraries the
    // session keeps live can carry the library edit into the new table.
    edit_in(&ws, "app/src/main.rs", &[(broken, "        let table")]);
    let out = session.wait("patch 3: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("replayed ui"),
        "the app-only fix did not replay the library the restart forgot:\n{out}"
    );
    session.wait("fixture ui=ui-2 ui_hot=true", 2, STEP_TIMEOUT);
    assert_eq!(
        session.output().matches("app running as pid").count(),
        1,
        "the refusal restarted the app"
    );
    session.quit();
}

#[test]
fn watches_a_source_a_failed_lex_taught() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-lexfail");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // A module outside every watched directory joins the library through
    // a patch whose compile dies in the lexer, before rustc writes any
    // dep-info; the diagnostic spans must teach the session the file, or
    // the next save, in that module alone, would wait for an event the
    // session never arranged to see.
    fs::write(
        ws.join("ui").join("late_lex.rs"),
        "//! A module the compiler cannot finish reading.\n\n/* never closed\n",
    )
    .expect("the outside module could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "#[path = \"../late_lex.rs\"]\nmod late_lex;\n\n#[inline(never)]\npub fn label() -> &'static str {\n    late_lex::tag()\n}",
        )],
    );
    session.wait("patch 1: compile failed", 1, STEP_TIMEOUT);
    // Only the outside module carries the fix, which only a span of the
    // failed compile could have put on the watch.
    fs::write(
        ws.join("ui").join("late_lex.rs"),
        "//! A module outside the watched directories.\n\n\
         /// The tag an edit here patches through the library.\n\
         #[inline(never)]\n\
         pub fn tag() -> &'static str {\n    \"lex-1\"\n}\n",
    )
    .expect("the outside module could not be fixed");
    session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    session.wait("ui=lex-1 ui_hot=true", 1, STEP_TIMEOUT);
    edit_in(&ws, "ui/late_lex.rs", &[("\"lex-1\"", "\"lex-2\"")]);
    session.wait("patch 3: applied", 1, STEP_TIMEOUT);
    session.wait("ui=lex-2 ui_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn watches_a_source_without_an_rs_extension() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-incfail");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // A module a `#[path]` attribute names can carry any extension: the
    // compile dies in the lexer before dep-info is written, and the
    // diagnostic span names the file, so the span must teach the session
    // the file whatever it ends in, and fixing it alone must reach a
    // patch.
    fs::write(
        ws.join("ui").join("late.inc"),
        "//! A module the compiler cannot finish reading.\n\n/* never closed\n",
    )
    .expect("the outside module could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "#[path = \"../late.inc\"]\nmod late_lex;\n\n#[inline(never)]\npub fn label() -> &'static str {\n    late_lex::tag()\n}",
        )],
    );
    session.wait("patch 1: compile failed", 1, STEP_TIMEOUT);
    fs::write(
        ws.join("ui").join("late.inc"),
        "//! A module outside the watched directories.\n\n\
         /// The tag an edit here patches through the library.\n\
         #[inline(never)]\n\
         pub fn tag() -> &'static str {\n    \"inc-1\"\n}\n",
    )
    .expect("the outside module could not be fixed");
    session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    session.wait("ui=inc-1 ui_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn watches_a_missing_include_content() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-missinc");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // The library starts reading a file that does not exist yet: the
    // compile fails before any dep-info, and the diagnostic spans name
    // only the source that includes the file, so the read error's
    // message is what teaches the session the missing input, and
    // creating it must retry the crate that asked for it.
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "#[inline(never)]\npub fn label() -> &'static str {\n    include_str!(\"../content.txt\")\n}",
        )],
    );
    session.wait("patch 1: compile failed", 1, STEP_TIMEOUT);
    fs::write(ws.join("ui").join("content.txt"), "content-1")
        .expect("the content could not be written");
    session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=content-1 ui_hot=true", 1, STEP_TIMEOUT);
    edit_in(&ws, "ui/content.txt", &[("content-1", "content-2")]);
    session.wait("patch 3: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=content-2 ui_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn patches_through_a_stripped_profile() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-strip");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A profile that strips the debug information takes the signatures
    // the patch builder reads out of the binary the session runs, so
    // the wrapper must drop the strip from every compile it records or
    // the session reports success over a table of no mappings.
    edit_in(
        &ws,
        "Cargo.toml",
        &[(
            "resolver = \"2\"",
            "resolver = \"2\"\n\n[profile.dev]\nstrip = \"debuginfo\"",
        )],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/lib.rs", &[("\"ui-1\"", "\"ui-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let out = session.wait("fixture ui=ui-2", 1, STEP_TIMEOUT);
    assert!(
        out.contains("ui_hot=true"),
        "the patch through the stripped profile mapped nothing:\n{out}"
    );
    session.quit();
}

/// The triple rustc compiles for, which an explicit target build names
/// in its environment.
fn host_triple() -> String {
    let out = Command::new("rustc")
        .arg("-vV")
        .output()
        .expect("rustc could not be asked for its host");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(str::to_owned)
        .expect("rustc did not name its host")
}

#[test]
fn watches_the_script_inputs_of_an_explicit_target() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-scripttarget");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // An explicit target moves the runs of the scripts the app's units
    // consume to a build tree of the target's own, while the scripts
    // themselves compile in the host's tree: the recording's directory
    // alone cannot answer where the outputs stand, and a guard built
    // from it only watches nothing and lets every input move unseen.
    fs::write(ws.join("ui").join("config.txt"), "baked-1")
        .expect("the config could not be written");
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The baker of the configuration the library reads.\n\n\
         fn main() {\n    \
         println!(\"cargo:rerun-if-changed=config.txt\");\n    \
         let baked = std::fs::read_to_string(\"config.txt\").unwrap_or_default();\n    \
         println!(\"cargo:rustc-env=UI_CONFIG={}\", baked.trim());\n}\n",
    )
    .expect("ui's build script could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    env!(\"UI_CONFIG\")")],
    );
    let triple = host_triple();
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("CARGO_BUILD_TARGET", Some(triple.as_str()))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/config.txt", &[("baked-1", "baked-2")]);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("config.txt") && out.contains("cannot rerun"),
        "the input the target tree's run declared did not refuse the patch:\n{out}"
    );
    assert!(
        !out.contains("the app exited"),
        "the app exited on a refused script input:\n{out}"
    );
    session.quit();
    // The next session's build reruns the script, whose expansion bakes
    // the new content, and the patch after it carries it.
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("CARGO_BUILD_TARGET", Some(triple.as_str()))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=baked-2", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn restarts_when_the_configuration_changes() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-cfgwatch");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A configuration file is a build input as much as a manifest is:
    // the flags it gives the compiler reach the next build only, so the
    // session must hear of its change and restart over the build that
    // carries it, not patch on against the old selection.
    fs::create_dir_all(ws.join(".cargo")).expect("the cargo directory could not be made");
    fs::write(
        ws.join(".cargo").join("config.toml"),
        "[target.'cfg(unix)']\nrustflags = []\n",
    )
    .expect("the configuration could not be written");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    fs::write(
        ws.join(".cargo").join("config.toml"),
        "[target.'cfg(unix)']\nrustflags = [\"-Awarnings\"]\n",
    )
    .expect("the configuration could not be rewritten");
    session.wait(
        "the build inputs changed; restarting the build",
        1,
        STEP_TIMEOUT,
    );
    session.wait("app running as pid", 2, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    // The wait counts occurrences, so the second patch must reach two:
    // one line only would also answer the patch the first build
    // already applied.
    session.wait("patch 1: applied", 2, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn watches_an_input_that_turns_into_a_directory() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-dirinput");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A watched input that retargets from a file to a directory stops
    // being a file the walk would visit, and its content cannot compile
    // again: the session must hear of the retarget through the entry's
    // own movement and fail the compile loudly, not report success over
    // the content the running app already carries.
    fs::write(ws.join("ui").join("copy.txt"), "copy-1").expect("the content could not be written");
    fs::create_dir_all(ws.join("ui").join("hollow"))
        .expect("the empty directory could not be made");
    std::os::unix::fs::symlink("../copy.txt", ws.join("ui/src/content.rs"))
        .expect("the link could not be made");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    include_str!(\"content.rs\").trim()")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=copy-1", 1, STEP_TIMEOUT);
    fs::remove_file(ws.join("ui/src/content.rs")).expect("the link could not be removed");
    std::os::unix::fs::symlink("../hollow", ws.join("ui/src/content.rs"))
        .expect("the link could not be retargeted");
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 2: compile failed", 1, STEP_TIMEOUT);
    assert!(
        !out.contains("patch 2: applied"),
        "the patch over a directory input reported success:\n{out}"
    );
    session.quit();
}

#[test]
fn drops_the_guard_of_an_automatic_script_the_package_loses() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-autogone");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A script the manifest never names rides on the bare `build.rs`,
    // and the request it belongs to never changes when the file goes:
    // the guard its recording kept must go with it, or the next session
    // refuses every edit in a package the script no longer reads.
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The generator of the number the library reads.\n\n\
         fn main() {\n    \
         println!(\"cargo:rustc-env=UI_NUMBER={}\", engine::seed());\n}\n",
    )
    .expect("ui's build script could not be written");
    edit_in(
        &ws,
        "ui/Cargo.toml",
        &[(
            "[dependencies]",
            "[build-dependencies]\nengine = { package = \"core2\", path = \"../core2\" }\n\n[dependencies]",
        )],
    );
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    env!(\"UI_NUMBER\")")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=7", 1, STEP_TIMEOUT);
    edit_in(&ws, "core2/src/lib.rs", &[("    7", "    9")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("the build script of ui reads core2"),
        "the automatic script's dependency did not refuse:\n{out}"
    );
    session.quit();
    // The package loses the script without touching its manifest: the
    // request stands, and the guard must not.
    fs::remove_file(ws.join("ui").join("build.rs")).expect("the build script could not be removed");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    env!(\"UI_NUMBER\")", "    \"ui-1\"")],
    );
    let mut session = Session::start(&app, &target);
    let out = session.wait("app running as pid", 1, FAT_TIMEOUT);
    assert!(
        !out.contains("the build request changed"),
        "losing the script changed the request over nothing:\n{out}"
    );
    // The dependency edits again in this session, and with the script
    // gone nothing stands between it and the app: the chain replays it.
    edit_in(&ws, "core2/src/lib.rs", &[("    9", "    11")]);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("replayed core2"),
        "the dependency the lost script once read did not replay:\n{out}"
    );
    assert!(
        !out.contains("cannot rerun"),
        "the ghost guard refused the build that has no script:\n{out}"
    );
    session.wait("deep=11", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn replays_a_library_named_like_a_script() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-scriptname");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A library whose crate name carries the prefix cargo gives build
    // scripts is no script: the unit compiles a library, and the chain
    // must replay it like any other, whatever its name spells.
    edit_in(
        &ws,
        "core2/Cargo.toml",
        &[(
            "[package]",
            "[lib]\nname = \"build_script_core2\"\n\n[package]",
        )],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "core2/src/lib.rs", &[("    7", "    9")]);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("replayed"),
        "the library named like a script did not replay:\n{out}"
    );
    session.wait("deep=9", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn keeps_quiet_when_a_retired_input_vanishes() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-retired");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A watched file an edit retires, say an include a library stops
    // reading, belongs to no patch anymore: its later deletion must
    // answer nothing, for the loss reports a replay nothing reads, and
    // the save after it must patch as usual.
    fs::write(ws.join("ui").join("content.txt"), "content-1")
        .expect("the content could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    include_str!(\"../content.txt\")")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=content-1", 1, STEP_TIMEOUT);
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    include_str!(\"../content.txt\")", "    \"ui-1\"")],
    );
    session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-1", 1, STEP_TIMEOUT);
    fs::remove_file(ws.join("ui").join("content.txt"))
        .expect("the retired content could not be removed");
    std::thread::sleep(Duration::from_secs(3));
    let out = session.output();
    assert!(
        !out.contains("patch 3"),
        "the retired input's deletion woke a patch:\n{out}"
    );
    assert!(
        !out.contains("compile failed") && !out.contains("not sent"),
        "the retired input's deletion disturbed the session:\n{out}"
    );
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    session.wait("patch 3: applied", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn guards_an_indented_directive() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-indent");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // Cargo reads a directive after trimming the line it stands on, so
    // an indent hides nothing: the input the indented line declares
    // must guard the session like any other, or a patch would replay
    // libraries against an expansion the next build cannot reproduce.
    fs::write(ws.join("ui").join("config.txt"), "baked-1")
        .expect("the config could not be written");
    fs::write(
        ws.join("ui").join("build.rs"),
        "//! The baker of the configuration the library reads.\n\n\
         fn main() {\n    \
         println!(\"    cargo:rerun-if-changed=config.txt\");\n    \
         let baked = std::fs::read_to_string(\"config.txt\").unwrap_or_default();\n    \
         println!(\"cargo:rustc-env=UI_CONFIG={}\", baked.trim());\n}\n",
    )
    .expect("ui's build script could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    env!(\"UI_CONFIG\")")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=baked-1", 1, STEP_TIMEOUT);
    edit_in(&ws, "ui/config.txt", &[("baked-1", "baked-2")]);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("config.txt") && out.contains("cannot rerun"),
        "the input the indented directive declared did not refuse:\n{out}"
    );
    session.quit();
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-3\"", "\"view-4\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=baked-2", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn watches_an_input_replaced_by_a_directory() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-filedir");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A watched source that a directory of the same name replaces stops
    // being a file any walk below would visit, and its content cannot
    // compile again: the walk must answer the entry's own movement and
    // fail the compile loudly, not report success over the content the
    // running app already carries.
    fs::write(ws.join("ui").join("src").join("plain.rs"), "plain-1")
        .expect("the content could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    include_str!(\"plain.rs\").trim()")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=plain-1", 1, STEP_TIMEOUT);
    fs::remove_file(ws.join("ui").join("src").join("plain.rs"))
        .expect("the file could not be removed");
    fs::create_dir(ws.join("ui").join("src").join("plain.rs"))
        .expect("the directory could not be made");
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 2: compile failed", 1, STEP_TIMEOUT);
    assert!(
        !out.contains("patch 2: applied"),
        "the patch over a directory of a source's name reported success:\n{out}"
    );
    session.quit();
}

#[test]
fn refuses_a_profile_that_links_with_lto() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-lto");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-lto-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // An LTO profile merges the crates at the link, which the replay of
    // one crate cannot redo, so the session refuses it at the first
    // compile instead of pairing signatures against a binary the
    // bitcode of every dependency reshaped.
    edit_in(
        &ws,
        "Cargo.toml",
        &[(
            "resolver = \"2\"",
            "resolver = \"2\"\n\n[profile.dev]\nlto = \"thin\"",
        )],
    );
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-smudge"))
        .arg("dev")
        .current_dir(&app)
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_NET_OFFLINE", "true")
        .stdin(Stdio::null())
        .output()
        .expect("the session could not start");
    assert!(
        !output.status.success(),
        "the session ran through an LTO profile"
    );
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("LTO"), "the refusal did not name LTO:\n{err}");
}

#[test]
fn rebuilds_from_clean_when_the_request_markers_vanish() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-vanishmark");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.quit();
    // A target whose request marker is gone carries a cache of unknown
    // making: whatever wrote it recorded no units this session can
    // replay, so the workspace's own crates must rebuild from clean
    // instead of the session accepting what no recording describes.
    fs::remove_file(target.join("smudge-hot").join("build-request.json"))
        .expect("the marker could not be removed");
    let mut session = Session::start(&app, &target);
    let out = session.wait("app running as pid", 1, FAT_TIMEOUT);
    assert!(
        out.contains(
            "the build request is unknown, so the workspace's own crates rebuild from clean"
        ),
        "the unknown request kept the cache of no recording:\n{out}"
    );
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn patches_an_unmangled_function() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-nomangle");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // An unmangled export carries its plain name for the symbol table
    // and the debug information alike, so rustc leaves the linkage name
    // out: the object's signatures must answer by the plain name too,
    // or the patch omits the one function the user edited and reports
    // success over the body the running app still carries.
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "fn main() {",
                "#[unsafe(no_mangle)]\npub extern \"C\" fn smudge_plain() -> &'static str {\n    \"plain-1\"\n}\n\nfn main() {",
            ),
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let plain_address = smudge_plain as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (plain_value, plain_hot) = through(table, plain_address);",
            ),
            (
                "             bin={bin_value} bin_hot={bin_hot}\"",
                "             bin={bin_value} bin_hot={bin_hot} plain={plain_value} plain_hot={plain_hot}\"",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"plain-1\"", "\"plain-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let out = session.wait("plain=plain-2", 1, STEP_TIMEOUT);
    assert!(
        out.contains("plain_hot=true"),
        "the unmangled function was left out of the table:\n{out}"
    );
    session.quit();
}

#[test]
fn refuses_a_paired_codegen_lto() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-lto-paired");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-lto-paired-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // The value may ride its own argument after `--codegen`, exactly
    // like after `-C`: the gate must read that spelling too, or a
    // spaced flag starts a session the link of which no replay can
    // redo. The bitcode flag rides along the way cargo sends it for
    // real LTO builds, so the dependencies compile far enough for the
    // workspace's own crates to reach the gate.
    let flags = "--codegen\u{1f}lto=thin\u{1f}-Cembed-bitcode=yes";
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-smudge"))
        .arg("dev")
        .current_dir(&app)
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_ENCODED_RUSTFLAGS", flags)
        .stdin(Stdio::null())
        .output()
        .expect("the session could not start");
    assert!(
        !output.status.success(),
        "the session ran through a spaced --codegen lto flag"
    );
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(
        err.contains("links with LTO"),
        "the refusal did not name the LTO link:\n{err}"
    );
}

#[test]
fn accepts_a_disabled_plugin_lto() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-plugin-off");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-plugin-off-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // A plugin LTO that rustc runs with the crates apart is no LTO the
    // link merges: the session starts and patches like any other.
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("CARGO_ENCODED_RUSTFLAGS", Some("-Clinker-plugin-lto=off"))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn accepts_a_rustc_false_lto_spelling() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-lto-no");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-lto-no-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // Rustc spells a disabled boolean `no` and `n` as well as `off`
    // and `false`: the gate must read every spelling rustc accepts as
    // false, or a valid non-LTO profile cannot start.
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("CARGO_ENCODED_RUSTFLAGS", Some("-Clto=no"))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn accepts_an_overridden_lto_value() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-lto-override");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-lto-override-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // Rustc honors the last occurrence of an option: an enabled value
    // a later disabled one overrides is no LTO the link merges, and
    // the gate must wait for the final value before refusing.
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("CARGO_ENCODED_RUSTFLAGS", Some("-Clto=thin\u{1f}-Clto=off"))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn patches_a_save_that_lets_a_future_stamp_fall() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let app = fixture("future-stamp");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A source may carry a stamp an hour ahead of the session, and the
    // ordinary save that follows pulls it back under: the scan answers
    // each file's observed state, not a session-wide newest time, or
    // the fall would read as already seen.
    let main = app.join("src").join("main.rs");
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&main)
        .expect("the source could not be opened");
    let ahead = std::time::SystemTime::now() + Duration::from_secs(3600);
    file.set_modified(ahead)
        .expect("the stamp could not be moved ahead");
    drop(file);
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit(&app, &[("\"alpha-1\"", "\"alpha-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let out = session.wait("fixture alpha=alpha-2", 1, STEP_TIMEOUT);
    assert!(
        out.contains("alpha_hot=true"),
        "the save under the future stamp did not patch:\n{out}"
    );
    session.quit();
}

#[test]
fn restarts_a_custom_profiles_feature_selection() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-hot-profile");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-hot-profile-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    edit_in(
        &ws,
        "Cargo.toml",
        &[(
            "resolver = \"2\"",
            "resolver = \"2\"\n\n[profile.hot]\ninherits = \"dev\"",
        )],
    );
    edit_in(
        &ws,
        "app/Cargo.toml",
        &[(
            "tls.workspace = true",
            "tls.workspace = true\n\n[features]\nhot-a = []\nhot-b = []",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "fn main() {",
                "#[cfg(feature = \"hot-a\")]\nconst SELECTION: &str = \"select-a\";\n\n#[cfg(feature = \"hot-b\")]\nconst SELECTION: &str = \"select-b\";\n\nfn main() {",
            ),
            (
                "bin={bin_value} bin_hot={bin_hot}\"",
                "bin={bin_value} bin_hot={bin_hot} sel={SELECTION}\"",
            ),
        ],
    );
    // The clean that answers a changed request must answer to the same
    // profile the build names: cargo's package clean takes one profile,
    // and a clean of the default one leaves the custom profile's cache
    // to serve the artifacts of an older selection without running the
    // wrapper, so the session would start and patch another feature
    // selection's build.
    let selections = ["hot-a", "hot-b", "hot-a"];
    let mut bin = 1;
    for (round, selection) in selections.iter().enumerate() {
        let mut session = Session::start_with(
            &app,
            &target,
            &["--profile", "hot", "--features", selection],
        );
        session.wait("app running as pid", 1, FAT_TIMEOUT);
        let from = format!("\"bin-{bin}\"");
        bin += 1;
        let to = format!("\"bin-{bin}\"");
        edit_in(&ws, "app/src/bin_only.rs", &[(&from, &to)]);
        let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
        let line = line_of(&out, "fixture ");
        let want = if *selection == "hot-a" {
            "sel=select-a"
        } else {
            "sel=select-b"
        };
        assert!(
            line.contains(want),
            "round {round} built another selection than {selection} asked:\n{line}"
        );
        session.quit();
    }
}

#[test]
fn guards_the_inputs_of_a_scripts_build_dependencies() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-buildonly");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-buildonly-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // A library only a build script's compile links is a build
    // dependency the app's graph never names, and what it compiles
    // from bakes into what the script generates. Its files must join
    // the script's guard, or an edit to them goes unreported and the
    // next patch serves the expansion the old content made.
    edit_in(
        &ws,
        "Cargo.toml",
        &[(
            "members = [\"app\", \"ui\", \"core2\", \"tls\", \"mac\", \"helper\"]",
            "members = [\"app\", \"ui\", \"core2\", \"tls\", \"mac\", \"helper\", \"generator\"]",
        )],
    );
    fs::create_dir_all(ws.join("generator").join("src")).expect("the generator could not be made");
    fs::write(
        ws.join("generator").join("Cargo.toml"),
        "[package]\nname = \"generator\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("the generator manifest could not be written");
    fs::write(
        ws.join("generator").join("src").join("template.txt"),
        "generated-1\n",
    )
    .expect("the template could not be written");
    fs::write(
        ws.join("generator").join("src").join("lib.rs"),
        "#[inline(never)]\npub fn label() -> &'static str {\n    include_str!(\"template.txt\").trim()\n}\n",
    )
    .expect("the generator could not be written");
    edit_in(
        &ws,
        "ui/Cargo.toml",
        &[(
            "mac.workspace = true",
            "mac.workspace = true\n\n[build-dependencies]\ngenerator = { path = \"../generator\" }",
        )],
    );
    fs::write(
        ws.join("ui").join("build.rs"),
        "fn main() {\n    println!(\"cargo:rustc-env=UI_GENERATED={}\", generator::label());\n}\n",
    )
    .expect("the build script could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"\n}", "    env!(\"UI_GENERATED\")\n}")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    // The template moves with no save of its own package, and the next
    // save must refuse rather than replay the expansion the old content
    // generated. The refusal names the generator the app's graph never
    // reaches; the point is that the edit was seen at all, which no
    // watch of a build dependency's inputs would have reported.
    edit_in(
        &ws,
        "generator/src/template.txt",
        &[("generated-1", "generated-2")],
    );
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    // The refusal is sticky: an app-only save must not consume the
    // event and replay libraries against the stale expansion.
    edit_in(&ws, "app/src/bin_only.rs", &[("\"bin-1\"", "\"bin-2\"")]);
    let out = session.wait("patch 2: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("inputs that moved"),
        "the stale script did not stay stale:\n{out}"
    );
    session.quit();
}

#[test]
fn guards_an_edit_that_lands_while_the_app_connects() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-startup-edit");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-startup-edit-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // A declared directory's baseline must stand before the app starts,
    // like the other watermarks: an edit that lands while the app
    // connects would otherwise fold into a baseline captured beside the
    // handshake and stay unseen forever.
    fs::create_dir_all(ws.join("ui").join("assets")).expect("the assets could not be made");
    fs::write(ws.join("ui").join("assets").join("label.txt"), "script-1\n")
        .expect("the label could not be written");
    fs::write(
        ws.join("ui").join("build.rs"),
        "fn main() {\n    println!(\"cargo:rerun-if-changed=assets\");\n    let label = std::fs::read_to_string(\"assets/label.txt\").unwrap();\n    println!(\"cargo:rustc-env=UI_GENERATED={}\", label.trim());\n}\n",
    )
    .expect("the build script could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"\n}", "    env!(\"UI_GENERATED\")\n}")],
    );
    // The app delays its connection, so the test can land an edit in
    // the window between the app's start and its handshake.
    edit_in(
        &ws,
        "app/src/main.rs",
        &[(
            "    let mut stream = match UnixStream::connect(&socket) {",
            "    std::thread::sleep(std::time::Duration::from_secs(2));\n    let mut stream = match UnixStream::connect(&socket) {",
        )],
    );
    let mut session = Session::start(&app, &target);
    session.wait("starting ", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/assets/label.txt", &[("script-1", "script-2")]);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    assert!(
        out.contains("inputs that moved"),
        "the edit that landed during the connection was swallowed:\n{out}"
    );
    session.quit();
}

#[test]
fn keeps_a_layout_change_out_of_the_table() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-layout");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-layout-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // An aggregate is not its name alone: two builds that spell one
    // type the same way can lay it out differently, and a caller of a
    // function returning it by value must prepare the room the old
    // layout had. The layout joins the fingerprint, or a grown struct
    // would be read with the old one's shape.
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "fn main() {",
                "#[repr(C)]\npub struct Packet {\n    pub value: u64,\n}\n\n#[unsafe(no_mangle)]\npub extern \"C\" fn smudge_packet() -> Packet {\n    Packet { value: 1 }\n}\n\nfn through_packet(table: Option<&JumpTable>, address: usize) -> (u64, bool) {\n    let mapped = table\n        .and_then(|table| table.map.get(&(address as u64)).copied())\n        .filter(|&new| new != 0);\n    match mapped {\n        Some(new) => {\n            let run = unsafe { std::mem::transmute::<usize, fn() -> Packet>(new as usize) };\n            (run().value, true)\n        }\n        None => {\n            let run = unsafe { std::mem::transmute::<usize, fn() -> Packet>(address) };\n            (run().value, false)\n        }\n    }\n}\n\nfn main() {",
            ),
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let packet_address = smudge_packet as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (packet_value, packet_hot) = through_packet(table, packet_address);",
            ),
            (
                "bin={bin_value} bin_hot={bin_hot}\"",
                "bin={bin_value} bin_hot={bin_hot} packet={packet_value} packet_hot={packet_hot}\"",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "pub struct Packet {\n    pub value: u64,\n}",
                "pub struct Packet {\n    pub value: u64,\n    pub extra: u64,\n}",
            ),
            (
                "    Packet { value: 1 }",
                "    Packet { value: 1, extra: 2 }",
            ),
        ],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("omitted for a changed signature"),
        "the layout change did not refuse the mapping:\n{out}"
    );
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("packet=1 packet_hot=false"),
        "the grown struct was mapped anyway:\n{line}"
    );
    session.quit();
}

#[test]
fn keeps_a_swapped_tag_out_of_the_table() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-swapped-tag");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-swapped-tag-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // Which payload a tagged enum's variant carries is decided by its
    // discriminant, so the tag joins the digest beside the members it
    // governs: a swap of the tags leaves the name, the size and the
    // offsets alone, and a mapping that kept the old fingerprint would
    // read one variant's payload as the other's.
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "fn main() {",
                "#[repr(C, u8)]\npub enum Tagged {\n    A(u64) = 0,\n    B(u64) = 1,\n}\n\n#[unsafe(no_mangle)]\npub extern \"C\" fn smudge_tagged(t: Tagged) -> u64 {\n    match t {\n        Tagged::A(v) => v + 10,\n        Tagged::B(v) => v + 20,\n    }\n}\n\nfn through_tagged(table: Option<&JumpTable>, address: usize) -> (u64, bool) {\n    let mapped = table\n        .and_then(|table| table.map.get(&(address as u64)).copied())\n        .filter(|&new| new != 0);\n    match mapped {\n        Some(new) => {\n            let run = unsafe { std::mem::transmute::<usize, fn(Tagged) -> u64>(new as usize) };\n            (run(Tagged::B(3)), true)\n        }\n        None => {\n            let run = unsafe { std::mem::transmute::<usize, fn(Tagged) -> u64>(address) };\n            (run(Tagged::B(3)), false)\n        }\n    }\n}\n\nfn main() {",
            ),
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let tagged_address = smudge_tagged as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (tagged_value, tagged_hot) = through_tagged(table, tagged_address);",
            ),
            (
                "bin={bin_value} bin_hot={bin_hot}\"",
                "bin={bin_value} bin_hot={bin_hot} tagged={tagged_value} tagged_hot={tagged_hot}\"",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "app/src/main.rs",
        &[(
            "pub enum Tagged {\n    A(u64) = 0,\n    B(u64) = 1,\n}",
            "pub enum Tagged {\n    A(u64) = 1,\n    B(u64) = 0,\n}",
        )],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("omitted for a changed signature"),
        "the tag swap did not refuse the mapping:\n{out}"
    );
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("tagged=23 tagged_hot=false"),
        "the swapped tag was mapped anyway:\n{line}"
    );
    session.quit();
}

#[test]
fn keeps_a_swapped_negative_tag_out_of_the_table() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-negative-tag");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-negative-tag-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // A fieldless enum's constants are signed numbers, and two
    // distinct negatives must not collapse into one unreadable
    // spelling: a swap of the tags would then compare equal and map,
    // and the caller's value would mean the other variant.
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "fn main() {",
                "#[repr(i8)]\npub enum Signed {\n    A = -1,\n    B = -2,\n}\n\n#[unsafe(no_mangle)]\npub extern \"C\" fn smudge_signed(s: Signed) -> i8 {\n    match s {\n        Signed::A => 10,\n        Signed::B => 20,\n    }\n}\n\nfn through_signed(table: Option<&JumpTable>, address: usize) -> (i8, bool) {\n    let mapped = table\n        .and_then(|table| table.map.get(&(address as u64)).copied())\n        .filter(|&new| new != 0);\n    match mapped {\n        Some(new) => {\n            let run = unsafe { std::mem::transmute::<usize, fn(Signed) -> i8>(new as usize) };\n            (run(Signed::B), true)\n        }\n        None => {\n            let run = unsafe { std::mem::transmute::<usize, fn(Signed) -> i8>(address) };\n            (run(Signed::B), false)\n        }\n    }\n}\n\nfn main() {",
            ),
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let signed_address = smudge_signed as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (signed_value, signed_hot) = through_signed(table, signed_address);",
            ),
            (
                "bin={bin_value} bin_hot={bin_hot}\"",
                "bin={bin_value} bin_hot={bin_hot} signed={signed_value} signed_hot={signed_hot}\"",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "app/src/main.rs",
        &[(
            "pub enum Signed {\n    A = -1,\n    B = -2,\n}",
            "pub enum Signed {\n    A = -2,\n    B = -1,\n}",
        )],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("omitted for a changed signature"),
        "the negative tag swap did not refuse the mapping:\n{out}"
    );
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("signed=20 signed_hot=false"),
        "the swapped negative tag was mapped anyway:\n{line}"
    );
    session.quit();
}

#[test]
fn keeps_a_borrowed_owned_swap_out_of_the_table() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-ptr-own");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-ptr-own-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // A borrowed and an owned parameter hand the function a pointer
    // that dies differently: the old caller's reference points at its
    // own stack, which the owned version's drop logic frees. The
    // pointer dies carry the names that tell them apart, so the
    // fingerprints must differ.
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "fn main() {",
                "#[unsafe(no_mangle)]\npub extern \"C\" fn smudge_ptr(p: &u64) -> u64 {\n    *p + 1\n}\n\nfn through_ptr(table: Option<&JumpTable>, address: usize, p: &u64) -> (u64, bool) {\n    let mapped = table\n        .and_then(|table| table.map.get(&(address as u64)).copied())\n        .filter(|&new| new != 0);\n    match mapped {\n        Some(new) => {\n            let run = unsafe { std::mem::transmute::<usize, fn(&u64) -> u64>(new as usize) };\n            (run(p), true)\n        }\n        None => {\n            let run = unsafe { std::mem::transmute::<usize, fn(&u64) -> u64>(address) };\n            (run(p), false)\n        }\n    }\n}\n\nfn main() {",
            ),
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let ptr_slot = 5u64;\n    let ptr_address = smudge_ptr as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (ptr_value, ptr_hot) = through_ptr(table, ptr_address, &ptr_slot);",
            ),
            (
                "bin={bin_value} bin_hot={bin_hot}\"",
                "bin={bin_value} bin_hot={bin_hot} ptr={ptr_value} ptr_hot={ptr_hot}\"",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "app/src/main.rs",
        &[(
            "pub extern \"C\" fn smudge_ptr(p: &u64) -> u64 {\n    *p + 1\n}",
            "pub extern \"C\" fn smudge_ptr(p: Box<u64>) -> u64 {\n    *p + 1\n}",
        )],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("omitted for a changed signature"),
        "the borrowed-to-owned swap did not refuse the mapping:\n{out}"
    );
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("ptr=6 ptr_hot=false"),
        "the owned parameter was mapped anyway:\n{line}"
    );
    session.quit();
}

#[test]
fn keeps_an_array_length_swap_out_of_the_table() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-array-len");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-array-len-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // A fixed-length array is carried by its length whatever the
    // register the length decides on: two lengths are two calling
    // ABIs, and the subrange's count reaches the parser in narrow
    // forms the reader must name.
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "fn main() {",
                "#[unsafe(no_mangle)]\npub extern \"C\" fn smudge_arr(xs: [u64; 1]) -> u64 {\n    xs[0] + 1\n}\n\nfn through_arr(table: Option<&JumpTable>, address: usize, xs: [u64; 1]) -> (u64, bool) {\n    let mapped = table\n        .and_then(|table| table.map.get(&(address as u64)).copied())\n        .filter(|&new| new != 0);\n    match mapped {\n        Some(new) => {\n            let run = unsafe { std::mem::transmute::<usize, fn([u64; 1]) -> u64>(new as usize) };\n            (run(xs), true)\n        }\n        None => {\n            let run = unsafe { std::mem::transmute::<usize, fn([u64; 1]) -> u64>(address) };\n            (run(xs), false)\n        }\n    }\n}\n\nfn main() {",
            ),
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let arr_address = smudge_arr as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (arr_value, arr_hot) = through_arr(table, arr_address, [7]);",
            ),
            (
                "bin={bin_value} bin_hot={bin_hot}\"",
                "bin={bin_value} bin_hot={bin_hot} arr={arr_value} arr_hot={arr_hot}\"",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "app/src/main.rs",
        &[(
            "pub extern \"C\" fn smudge_arr(xs: [u64; 1]) -> u64 {\n    xs[0] + 1\n}",
            "pub extern \"C\" fn smudge_arr(xs: [u64; 2]) -> u64 {\n    xs[0] + xs[1] + 1\n}",
        )],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("omitted for a changed signature"),
        "the array length swap did not refuse the mapping:\n{out}"
    );
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("arr=8 arr_hot=false"),
        "the longer array was mapped anyway:\n{line}"
    );
    session.quit();
}

#[test]
fn keeps_a_nested_layout_change_out_of_the_table() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-nested-layout");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-nested-layout-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // An aggregate member's own layout decides what a caller reads
    // behind the offset the member names: a field of the inner type
    // can change its meaning while the outer name, size, alignment
    // and member names all stay, so the member's description descends
    // into what it holds.
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "fn main() {",
                "#[repr(C)]\npub struct Inner2 {\n    pub value: u64,\n}\n\n#[repr(C)]\npub struct Outer2 {\n    pub inner: Inner2,\n}\n\n#[unsafe(no_mangle)]\npub extern \"C\" fn smudge_nested() -> Outer2 {\n    Outer2 {\n        inner: Inner2 { value: 7 },\n    }\n}\n\nfn through_nested(table: Option<&JumpTable>, address: usize) -> (u64, bool) {\n    let mapped = table\n        .and_then(|table| table.map.get(&(address as u64)).copied())\n        .filter(|&new| new != 0);\n    match mapped {\n        Some(new) => {\n            // The probe reads the returned aggregate's eight bytes\n            // raw, which both spellings of the inner field lay out\n            // alike.\n            let run = unsafe { std::mem::transmute::<usize, fn() -> [u64; 1]>(new as usize) };\n            (run()[0], true)\n        }\n        None => {\n            let run = unsafe { std::mem::transmute::<usize, fn() -> [u64; 1]>(address) };\n            (run()[0], false)\n        }\n    }\n}\n\nfn main() {",
            ),
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let nested_address = smudge_nested as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (nested_value, nested_hot) = through_nested(table, nested_address);",
            ),
            (
                "bin={bin_value} bin_hot={bin_hot}\"",
                "bin={bin_value} bin_hot={bin_hot} nested={nested_value} nested_hot={nested_hot}\"",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "pub struct Inner2 {\n    pub value: u64,\n}",
                "pub struct Inner2 {\n    pub value: f64,\n}",
            ),
            ("Inner2 { value: 7 }", "Inner2 { value: 7.0 }"),
        ],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("omitted for a changed signature"),
        "the nested layout change did not refuse the mapping:\n{out}"
    );
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("nested=7 nested_hot=false"),
        "the changed inner field was mapped anyway:\n{line}"
    );
    session.quit();
}

#[test]
fn starts_from_a_package_subdirectory() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-subdir-start");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-subdir-start-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // A session started in a package's subdirectory belongs to that
    // package the way cargo answers for it: the bin is found through
    // the nearest manifest, so an invocation from `app/src` names the
    // app's bin instead of leaving cargo's rustc ambiguous about a
    // package that carries a library too.
    let mut session = Session::start(&ws.join("app").join("src"), &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.quit();
}

#[test]
fn keeps_a_recursive_layout_change_out_of_the_table() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-recursive-layout");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-recursive-layout-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // A type that reaches its own pointer cannot end a description by
    // naming the cycle alone: the shape of everything the pointer
    // reaches decides what a caller must prepare, and a field that
    // moves behind the recursion would leave the replacement reading
    // the old allocation at an offset it does not carry.
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "fn main() {",
                "#[repr(C)]\npub struct Node {\n    pub next: *const Node,\n    pub value: u64,\n}\n\n#[unsafe(no_mangle)]\npub extern \"C\" fn smudge_node(node: *const Node) -> u64 {\n    unsafe { (*node).value }\n}\n\nfn through_node(table: Option<&JumpTable>, address: usize, node: &Node) -> (u64, bool) {\n    let mapped = table\n        .and_then(|table| table.map.get(&(address as u64)).copied())\n        .filter(|&new| new != 0);\n    match mapped {\n        Some(new) => {\n            let run = unsafe {\n                std::mem::transmute::<usize, extern \"C\" fn(*const Node) -> u64>(new as usize)\n            };\n            (run(node), true)\n        }\n        None => {\n            let run = unsafe {\n                std::mem::transmute::<usize, extern \"C\" fn(*const Node) -> u64>(address)\n            };\n            (run(node), false)\n        }\n    }\n}\n\nfn main() {",
            ),
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let node_address = smudge_node as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let node = Node { next: std::ptr::null(), value: 41 };\n        let (node_value, node_hot) = through_node(table, node_address, &node);",
            ),
            (
                "bin={bin_value} bin_hot={bin_hot}\"",
                "bin={bin_value} bin_hot={bin_hot} node={node_value} node_hot={node_hot}\"",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "pub struct Node {\n    pub next: *const Node,\n    pub value: u64,\n}",
                "pub struct Node {\n    pub next: *const Node,\n    pub prefix: u64,\n    pub value: u64,\n}",
            ),
            (
                "let node = Node { next: std::ptr::null(), value: 41 };",
                "let node = Node { next: std::ptr::null(), prefix: 7, value: 41 };",
            ),
        ],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("omitted for a changed signature"),
        "the recursive layout change did not refuse the mapping:\n{out}"
    );
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("node=41 node_hot=false"),
        "the moved field was read through a stale mapping:\n{line}"
    );
    session.quit();
}

#[test]
fn patches_a_recursive_pointee_that_stands_still() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-recursive-still");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-recursive-still-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // A cycle cut the walk answers by its own shape, so two builds of
    // one unchanged recursive type spell the same digest: a body
    // change behind the pointer still patches, and the caller reads
    // the field the offset it always carried.
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "fn main() {",
                "#[repr(C)]\npub struct Node {\n    pub next: *const Node,\n    pub value: u64,\n}\n\n#[unsafe(no_mangle)]\npub extern \"C\" fn smudge_node(node: *const Node) -> u64 {\n    unsafe { (*node).value }\n}\n\nfn through_node(table: Option<&JumpTable>, address: usize, node: &Node) -> (u64, bool) {\n    let mapped = table\n        .and_then(|table| table.map.get(&(address as u64)).copied())\n        .filter(|&new| new != 0);\n    match mapped {\n        Some(new) => {\n            let run = unsafe {\n                std::mem::transmute::<usize, extern \"C\" fn(*const Node) -> u64>(new as usize)\n            };\n            (run(node), true)\n        }\n        None => {\n            let run = unsafe {\n                std::mem::transmute::<usize, extern \"C\" fn(*const Node) -> u64>(address)\n            };\n            (run(node), false)\n        }\n    }\n}\n\nfn main() {",
            ),
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let node_address = smudge_node as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let node = Node { next: std::ptr::null(), value: 41 };\n        let (node_value, node_hot) = through_node(table, node_address, &node);",
            ),
            (
                "bin={bin_value} bin_hot={bin_hot}\"",
                "bin={bin_value} bin_hot={bin_hot} node={node_value} node_hot={node_hot}\"",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "app/src/main.rs",
        &[(
            "    unsafe { (*node).value }\n}",
            "    unsafe { (*node).value + 1 }\n}",
        )],
    );
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&session.wait("fixture ", 1, STEP_TIMEOUT), "fixture ");
    assert!(
        line.contains("node=42 node_hot=true"),
        "the standing-still pointee did not patch:\n{line}"
    );
    session.quit();
}

#[test]
fn keeps_a_mutual_layout_change_out_of_the_table() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-mutual-layout");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-mutual-layout-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // Two types that point at each other cut the walk's cycle through
    // one another, and the digest of the type the walk stood on can
    // name the cut instead of the fields behind it. A fingerprint
    // built on that name survives a field that moves behind the other
    // pointer, and the replacement would read the old allocation at
    // the offset the old layout carried.
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "fn main() {",
                "#[derive(Clone, Copy)]\n#[repr(C)]\npub struct Pair {\n    pub other: *const Peer,\n    pub value: u64,\n}\n\n#[derive(Clone, Copy)]\n#[repr(C)]\npub struct Peer {\n    pub back: *const Pair,\n}\n\n#[unsafe(no_mangle)]\npub extern \"C\" fn smudge_seed(a: Pair) -> u64 {\n    a.value\n}\n\n#[unsafe(no_mangle)]\npub extern \"C\" fn smudge_victim(b: Peer) -> u64 {\n    unsafe { (*b.back).value }\n}\n\nfn through_seed(table: Option<&JumpTable>, address: usize, a: &Pair) -> (u64, bool) {\n    let mapped = table\n        .and_then(|table| table.map.get(&(address as u64)).copied())\n        .filter(|&new| new != 0);\n    match mapped {\n        Some(new) => {\n            let run = unsafe {\n                std::mem::transmute::<usize, extern \"C\" fn(Pair) -> u64>(new as usize)\n            };\n            (run(*a), true)\n        }\n        None => {\n            let run = unsafe {\n                std::mem::transmute::<usize, extern \"C\" fn(Pair) -> u64>(address)\n            };\n            (run(*a), false)\n        }\n    }\n}\n\nfn through_victim(table: Option<&JumpTable>, address: usize, b: &Peer) -> (u64, bool) {\n    let mapped = table\n        .and_then(|table| table.map.get(&(address as u64)).copied())\n        .filter(|&new| new != 0);\n    match mapped {\n        Some(new) => {\n            let run = unsafe {\n                std::mem::transmute::<usize, extern \"C\" fn(Peer) -> u64>(new as usize)\n            };\n            (run(*b), true)\n        }\n        None => {\n            let run = unsafe {\n                std::mem::transmute::<usize, extern \"C\" fn(Peer) -> u64>(address)\n            };\n            (run(*b), false)\n        }\n    }\n}\n\nfn main() {",
            ),
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let seed_address = smudge_seed as *const () as usize;\n    let victim_address = smudge_victim as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let mut pair = Pair { other: std::ptr::null(), value: 41 };\n        let peer = Peer { back: &pair };\n        pair.other = &peer;\n        let (seed_value, seed_hot) = through_seed(table, seed_address, &pair);\n        let (victim_value, victim_hot) = through_victim(table, victim_address, &peer);",
            ),
            (
                "bin={bin_value} bin_hot={bin_hot}\"",
                "bin={bin_value} bin_hot={bin_hot} seed={seed_value} seed_hot={seed_hot} \
                 victim={victim_value} victim_hot={victim_hot}\"",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "pub struct Pair {\n    pub other: *const Peer,\n    pub value: u64,\n}",
                "pub struct Pair {\n    pub other: *const Peer,\n    pub prefix: u64,\n    pub value: u64,\n}",
            ),
            (
                "Pair { other: std::ptr::null(), value: 41 }",
                "Pair { other: std::ptr::null(), prefix: 7, value: 41 }",
            ),
        ],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("omitted for a changed signature"),
        "the mutual layout change did not refuse the mapping:\n{out}"
    );
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("seed=41 seed_hot=false"),
        "the seed ran a replacement over its own moved field:\n{line}"
    );
    assert!(
        line.contains("victim=41 victim_hot=false"),
        "the victim was mapped although its pointee's layout moved:\n{line}"
    );
    session.quit();
}

#[test]
fn refuses_a_script_input_that_stands_behind_an_alias() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-alias-script");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    // The declared input names the file through a link that re-enters
    // an ancestor, whose descent the walk rightly refuses: the saved
    // spelling and the declared one must still meet, or a patch bakes
    // the expansion of the old content.
    std::os::unix::fs::symlink(".", ws.join("ui/src/alias")).unwrap();
    fs::write(ws.join("ui/src/value.rs"), "before").unwrap();
    fs::write(
        ws.join("ui/build.rs"),
        r#"fn main() {
        println!("cargo:rerun-if-changed=src/alias/value.rs");
        println!("cargo:rustc-env=UI_LABEL={}", std::fs::read_to_string("src/alias/value.rs").unwrap());
    }"#,
    )
    .unwrap();
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    env!(\"UI_LABEL\")")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    fs::write(ws.join("ui/src/value.rs"), "after").unwrap();
    let out = session.wait("cannot rerun", 1, STEP_TIMEOUT);
    assert!(
        out.contains("patch 1: not sent"),
        "the aliased script input did not refuse the patch:\n{out}"
    );
    assert!(
        !out.contains("patch 1: applied"),
        "the aliased script input was patched over:\n{out}"
    );
    session.quit();
}

#[test]
fn restarts_when_a_package_manifest_outside_the_watch_moves() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-outside-manifest");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    // The package's crate root sits outside its package directory, so
    // no ancestor of the watched source directories names its
    // manifest: the manifests cargo's own answer carries must join the
    // watch, or a feature that moves under the defaults never restarts
    // the build and the patches keep the old selection.
    fs::create_dir_all(ws.join("shared")).unwrap();
    fs::rename(ws.join("ui/src/lib.rs"), ws.join("shared/ui.rs")).unwrap();
    edit_in(
        &ws,
        "ui/Cargo.toml",
        &[(
            "[dependencies]",
            "[lib]\npath = \"../shared/ui.rs\"\n\n[dependencies]",
        )],
    );
    edit_in(
        &ws,
        "ui/Cargo.toml",
        &[("../shared/ui.rs", ws.join("shared/ui.rs").to_str().unwrap())],
    );
    edit_in(
        &ws,
        "shared/ui.rs",
        &[(
            "    \"ui-1\"",
            "    if cfg!(feature = \"fancy\") { \"ui-fancy\" } else { \"ui-1\" }",
        )],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "ui/Cargo.toml",
        &[("fancy = []", "fancy = []\ndefault = [\"fancy\"]")],
    );
    std::thread::sleep(Duration::from_secs(1));
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    let out = session.wait("the build inputs changed", 1, STEP_TIMEOUT);
    assert!(
        !out.contains("patch 1: applied"),
        "the moved manifest let a patch through:\n{out}"
    );
    session.wait("app running as pid", 2, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("ui=ui-fancy"),
        "the restarted build kept the old feature selection:\n{line}"
    );
    session.quit();
}

#[test]
fn patches_a_source_spelling_that_rides_an_ancestor_alias() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-alias-source");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A module spelled through a link back to an ancestor is a spelling
    // the walk refuses to descend, and whose retarget moves no watched
    // file: only the spelling's own watch can answer the retarget, and
    // the next patch must bake what the retargeted link resolves to.
    fs::create_dir_all(ws.join("ui/src/sub")).unwrap();
    fs::write(
        ws.join("ui/src/sub/value.rs"),
        "pub const TAG: &str = \"old-1\";\n",
    )
    .expect("the aliased module could not be written");
    fs::write(
        ws.join("ui/src/value.rs"),
        "pub const TAG: &str = \"new-1\";\n",
    )
    .expect("the sibling module could not be written");
    std::os::unix::fs::symlink(".", ws.join("ui/src/sub/alias")).unwrap();
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "//! library crate's own replay.",
            "//! library crate's own replay.\n\n#[path = \"sub/alias/value.rs\"]\npub mod value;\n\n#[inline(never)]\npub fn alias_tag() -> &'static str {\n    value::TAG\n}",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let alias_address = ui::alias_tag as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (alias_value, alias_hot) = through(table, alias_address);",
            ),
            (
                "bin={bin_value} bin_hot={bin_hot}\"",
                "bin={bin_value} bin_hot={bin_hot} alias={alias_value} alias_hot={alias_hot}\"",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    fs::remove_file(ws.join("ui/src/sub/alias")).unwrap();
    std::os::unix::fs::symlink("..", ws.join("ui/src/sub/alias")).unwrap();
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("alias=new-1 alias_hot=true"),
        "the retargeted alias did not bake the module it now names:\n{line}"
    );
    session.quit();
}

#[test]
fn refuses_a_dylib_dependency_the_replay_cannot_serve() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-dylib");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A library whose crate type names no rlib replays as a dynamic
    // library, which the chain's `--extern` arguments and the object
    // extraction cannot serve: the attempt must refuse by name instead
    // of failing over an artifact that cannot exist.
    edit_in(
        &ws,
        "core2/Cargo.toml",
        &[(
            "edition = \"2024\"",
            "edition = \"2024\"\n\n[lib]\ncrate-type = [\"dylib\"]",
        )],
    );
    let sysroot = String::from_utf8_lossy(
        &Command::new("rustc")
            .arg("--print")
            .arg("sysroot")
            .output()
            .expect("rustc could not run")
            .stdout,
    )
    .trim()
    .to_string();
    let loader = format!(
        "{}:{}:{}",
        Path::new(&sysroot).join("lib").display(),
        target.join("debug").join("deps").display(),
        std::env::var("LD_LIBRARY_PATH").unwrap_or_default(),
    );
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("LD_LIBRARY_PATH", Some(loader.as_str()))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "core2/src/lib.rs", &[("    7", "    9")]);
    let out = session.wait("patch 1: not sent", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: not sent");
    assert!(
        line.contains("builds as dylib"),
        "the refusal did not name the crate type:\n{line}"
    );
    assert!(
        !out.contains("patch 1: applied"),
        "the dylib dependency was patched over:\n{out}"
    );
    session.quit();
}

#[test]
fn patches_with_an_operand_that_spells_extra_filename() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-remap-extra");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A remapping whose value spells `-Cextra-filename=` is that
    // option's operand, which the replay must carry through untouched:
    // dropping it would leave the remapping to eat the raised debug
    // information, and the pair would apply with nothing mapped.
    let flags = "--remap-path-prefix\u{1f}-Cextra-filename=remapped";
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("CARGO_ENCODED_RUSTFLAGS", Some(flags))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/lib.rs", &[("    \"ui-1\"", "    \"ui-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("ui=ui-2 ui_hot=true"),
        "the remapped operand left the replay without signatures:\n{line}"
    );
    session.quit();
}

#[test]
fn patches_with_an_operand_that_spells_crate_type() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-remap-cratetype");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A remapping whose value spells `--crate-type=proc-macro` is that
    // option's operand: the recorded crate types must stay the ones
    // rustc read, or the library replays under an artifact name no
    // compile ever produced.
    let flags = "--remap-path-prefix\u{1f}--crate-type=proc-macro";
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("CARGO_ENCODED_RUSTFLAGS", Some(flags))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/lib.rs", &[("    \"ui-1\"", "    \"ui-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("ui=ui-2 ui_hot=true"),
        "the remapped operand turned the library into a macro:\n{line}"
    );
    session.quit();
}

#[test]
fn retries_a_missing_input_whose_name_carries_a_backtick() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-backtick");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // Rustc writes a backtick of the name literally in its read error,
    // so the learned name must run to the last backtick on the line:
    // creating the file under its whole name is what retries the crate.
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    include_str!(\"../data/a`b.txt\")")],
    );
    session.wait("patch 1: compile failed", 1, STEP_TIMEOUT);
    fs::create_dir_all(ws.join("ui/data")).unwrap();
    fs::write(ws.join("ui/data/a`b.txt"), "ui-note-1")
        .expect("the backticked input could not be written");
    session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=ui-note-1", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn patches_a_source_that_rides_a_link_future_stamped() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-alias-future");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-alias-future-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // Both of the link's targets stand an hour ahead, so no newest-of
    // comparison can carry the retarget: the fold of the separate
    // states must answer it, or the next patch bakes the old content.
    fs::create_dir_all(ws.join("ui/data")).unwrap();
    fs::write(ws.join("ui/data/old.txt"), "old-1").unwrap();
    fs::write(ws.join("ui/data/new.txt"), "new-1").unwrap();
    let ahead = std::time::SystemTime::now() + Duration::from_secs(3600);
    for name in ["old.txt", "new.txt"] {
        let file = fs::OpenOptions::new()
            .write(true)
            .open(ws.join("ui/data").join(name))
            .expect("the target could not be opened");
        file.set_modified(ahead)
            .expect("the stamp could not be moved ahead");
    }
    std::os::unix::fs::symlink("../data/old.txt", ws.join("ui/src/alias.txt")).unwrap();
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"", "    include_str!(\"alias.txt\")")],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.wait("fixture ui=old-1", 1, STEP_TIMEOUT);
    fs::remove_file(ws.join("ui/src/alias.txt")).unwrap();
    std::os::unix::fs::symlink("../data/new.txt", ws.join("ui/src/alias.txt")).unwrap();
    edit_in(&ws, "app/src/main.rs", &[("\"view-2\"", "\"view-3\"")]);
    session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    // The retarget replays the library, and the fixture answers with a
    // fresh line once the new ui is installed; the first line still
    // spells the old content.
    let out = session.wait("fixture ui=new-1", 1, STEP_TIMEOUT);
    let line = line_of(&out, "fixture ui=new-1");
    assert!(
        line.contains("ui=new-1 ui_hot=true"),
        "the retarget behind future stamps baked the old content:\n{line}"
    );
    session.quit();
}

#[test]
fn patches_a_branching_recursive_layout_in_time() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-ring");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-ring-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A ring of types each naming the next one twice is a branching
    // cycle: a walk that repeats itself per edge answers for a power
    // of the ring, which a patch cannot wait out. The answers of one
    // context must serve every edge that stands in the same
    // ancestors, and the patch must stay as quick as any other.
    let mut ring = String::new();
    for index in 0..20 {
        let next = format!("Ring{}", (index + 1) % 20);
        ring.push_str(&format!(
            "#[repr(C)]\npub struct Ring{index} {{\n    pub a: *const {next},\n    pub b: *const {next},\n}}\n\n"
        ));
    }
    ring.push_str(
        "#[inline(never)]\npub fn cycle_probe(p: *const Ring0) -> *const Ring0 {\n    p\n}\n",
    );
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "//! library crate's own replay.",
            &format!("//! library crate's own replay.\n\n{ring}"),
        )],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/lib.rs", &[("    \"ui-1\"", "    \"ui-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    let millis = total_millis(&line);
    assert!(
        millis < 2000,
        "the branching ring held the patch for {millis} ms: {line}"
    );
    session.wait("fixture ui=ui-2 ui_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn patches_a_deep_acyclic_type_chain() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-deep-chain");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-deep-chain-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A chain of structs each holding a pointer to the next is acyclic
    // but deep: a signature walk that spends a stack frame per level
    // runs out of the worker's stack partway down, and the session
    // dies instead of the patch failing. The walk must answer the
    // whole chain and the patch must apply.
    let depth = 400;
    let mut chain = String::new();
    chain.push_str("#[repr(C)]\npub struct Link0 {\n    pub tail: *const (),\n}\n\n");
    for index in 1..depth {
        chain.push_str(&format!(
            "#[repr(C)]\npub struct Link{index} {{\n    pub next: *const Link{},\n}}\n\n",
            index - 1
        ));
    }
    // The probe takes the deep end, so its signature reaches every link.
    chain.push_str(&format!(
        "#[inline(never)]\npub fn depth_probe(p: *const Link{last}) -> *const Link{last} {{\n    p\n}}\n",
        last = depth - 1
    ));
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "//! library crate's own replay.",
            &format!("//! library crate's own replay.\n\n{chain}"),
        )],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/lib.rs", &[("    \"ui-1\"", "    \"ui-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    assert_eq!(
        omitted_for(
            &line_of(&out, "patch 1: applied"),
            "omitted for a changed signature"
        ),
        0,
        "an unchanged chain was read as changed"
    );
    session.wait("fixture ui=ui-2 ui_hot=true", 1, STEP_TIMEOUT);
    // A change at the far end of the chain reaches the probe's signature
    // only through every link.
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    pub tail: *const (),", "    pub tail: usize,")],
    );
    let out = session.wait("patch 2: applied", 1, STEP_TIMEOUT);
    assert!(
        omitted_for(
            &line_of(&out, "patch 2: applied"),
            "omitted for a changed signature"
        ) > 0,
        "the change at the end of the chain did not reach the probe"
    );
    session.quit();
}

#[test]
fn patches_a_dense_recursive_layout_in_time() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-dense-ring");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-dense-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // Types of one recursive component that each name every type of
    // the component offer a walk that threads its ancestors a set of
    // answers per ancestor subset, which grows by a power of the
    // component. The component must label itself once, and the patch
    // must stay as quick as any other.
    let width = 18;
    let mut dense = String::new();
    for index in 0..width {
        let mut members = String::new();
        for other in 0..width {
            members.push_str(&format!("    pub f{other}: *const Dense{other},\n"));
        }
        dense.push_str(&format!(
            "#[repr(C)]\npub struct Dense{index} {{\n{members}}}\n\n"
        ));
    }
    dense.push_str(
        "#[inline(never)]\npub fn dense_probe(p: *const Dense0) -> *const Dense0 {\n    p\n}\n",
    );
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "//! library crate's own replay.",
            &format!("//! library crate's own replay.\n\n{dense}"),
        )],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/lib.rs", &[("    \"ui-1\"", "    \"ui-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "patch 1: applied");
    let millis = total_millis(&line);
    assert!(
        millis < 2000,
        "the dense component held the patch for {millis} ms: {line}"
    );
    session.wait("fixture ui=ui-2 ui_hot=true", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn keeps_an_inner_retarget_out_of_the_table() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-inner-retarget");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-inner-retarget-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // Two types of one recursive component, where a probe reads through
    // a field that points at a member of its own component: retargeting
    // that field from one member to another moves no shape and no cut,
    // so the component's answer must name the end each edge reaches, or
    // the replacement maps over data the running binary laid out as
    // another member of the same component.
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "fn main() {",
                "static SCRATCH: [u64; 16] = [0, 0, 0, 0, 0, 0, 41, 0, 0, 0, 0, 0, 0, 0, 0, 0];\n\n#[repr(C)]\npub struct Twin {\n    pub peer: *const Mate,\n    pub pad: [u64; 8],\n    pub value: u64,\n}\n\n#[repr(C)]\npub struct Mate {\n    pub peer: *const Twin,\n    pub changed: *const Mate,\n    pub pad: [u64; 4],\n    pub value: u64,\n}\n\n#[unsafe(no_mangle)]\npub extern \"C\" fn smudge_twin(node: *const Mate) -> u64 {\n    unsafe { (*(*node).changed).value }\n}\n\nfn through_twin(table: Option<&JumpTable>, address: usize, node: &Mate) -> (u64, bool) {\n    let mapped = table\n        .and_then(|table| table.map.get(&(address as u64)).copied())\n        .filter(|&new| new != 0);\n    match mapped {\n        Some(new) => {\n            let run = unsafe {\n                std::mem::transmute::<usize, extern \"C\" fn(*const Mate) -> u64>(new as usize)\n            };\n            (run(node), true)\n        }\n        None => {\n            let run = unsafe {\n                std::mem::transmute::<usize, extern \"C\" fn(*const Mate) -> u64>(address)\n            };\n            (run(node), false)\n        }\n    }\n}\n\nfn main() {",
            ),
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let twin_address = smudge_twin as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let mate = Mate {\n            peer: std::ptr::null(),\n            changed: (std::ptr::addr_of!(SCRATCH) as *const u64 as usize) as *const _,\n            pad: [0; 4],\n            value: 41,\n        };\n        let (twin_value, twin_hot) = through_twin(table, twin_address, &mate);",
            ),
            (
                "bin={bin_value} bin_hot={bin_hot}\"",
                "bin={bin_value} bin_hot={bin_hot} twin={twin_value} twin_hot={twin_hot}\"",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "app/src/main.rs",
        &[(
            "    pub changed: *const Mate,",
            "    pub changed: *const Twin,",
        )],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("omitted for a changed signature"),
        "the inner retarget did not refuse the mapping:\n{out}"
    );
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("twin=41 twin_hot=false"),
        "the retargeted field mapped over the old layout:\n{line}"
    );
    session.quit();
}

#[test]
fn keeps_an_outer_retarget_out_of_the_table() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-outer-retarget");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-outer-retarget-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A field of a type outside a recursive component points into it:
    // retargeting that field between two members leaves the component's
    // shared spell unchanged, so the entry names the member it lands
    // on, or the replacement reads the other member's layout.
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "fn main() {",
                "static SCRATCH: [u64; 16] = [0, 41, 0, 0, 0, 99, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];\n\n#[repr(C)]\npub struct Pair {\n    pub peer: *const Peer,\n    pub value: u64,\n}\n\n#[repr(C)]\npub struct Peer {\n    pub peer: *const Pair,\n    pub pad: [u64; 4],\n    pub value: u64,\n}\n\n#[repr(C)]\npub struct Holder {\n    pub at: *const Pair,\n}\n\n#[unsafe(no_mangle)]\npub extern \"C\" fn smudge_outer(holder: *const Holder) -> u64 {\n    unsafe { (*(*holder).at).value }\n}\n\nfn through_outer(table: Option<&JumpTable>, address: usize, holder: &Holder) -> (u64, bool) {\n    let mapped = table\n        .and_then(|table| table.map.get(&(address as u64)).copied())\n        .filter(|&new| new != 0);\n    match mapped {\n        Some(new) => {\n            let run = unsafe {\n                std::mem::transmute::<usize, extern \"C\" fn(*const Holder) -> u64>(new as usize)\n            };\n            (run(holder), true)\n        }\n        None => {\n            let run = unsafe {\n                std::mem::transmute::<usize, extern \"C\" fn(*const Holder) -> u64>(address)\n            };\n            (run(holder), false)\n        }\n    }\n}\n\nfn main() {",
            ),
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let outer_address = smudge_outer as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let holder = Holder {\n            at: (std::ptr::addr_of!(SCRATCH) as *const u64 as usize) as *const _,\n        };\n        let (outer_value, outer_hot) = through_outer(table, outer_address, &holder);",
            ),
            (
                "bin={bin_value} bin_hot={bin_hot}\"",
                "bin={bin_value} bin_hot={bin_hot} outer={outer_value} outer_hot={outer_hot}\"",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "app/src/main.rs",
        &[("    pub at: *const Pair,", "    pub at: *const Peer,")],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    assert!(
        out.contains("omitted for a changed signature"),
        "the outer retarget did not refuse the mapping:\n{out}"
    );
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("outer=41 outer_hot=false"),
        "the retargeted entry mapped over the old layout:\n{line}"
    );
    session.quit();
}

#[test]
fn keeps_a_shared_member_name_out_of_the_table() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-shared-name");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-shared-name-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // Two anonymous const blocks each define a `Node` through an
    // associated type, and rustc names both `ui::_::Node`: two members
    // of one recursive component that no token can tell apart. The
    // signature refuses the component, so a padding edit cannot map a
    // replacement compiled for the other member's layout over the
    // scratch the probe reads.
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
            "pub struct Left;\npub struct Right;\npub trait HasType {\n    type T;\n}\nconst _: () = {\n    #[repr(C)]\n    pub struct Node {\n        pub next: *const <Right as HasType>::T,\n        pub pad: [u64; 0],\n        pub value: u64,\n    }\n    impl HasType for Left {\n        type T = Node;\n    }\n};\nconst _: () = {\n    #[repr(C)]\n    pub struct Node {\n        pub next: *const <Left as HasType>::T,\n        pub pad: [u64; 4],\n        pub value: u64,\n    }\n    impl HasType for Right {\n        type T = Node;\n    }\n};\n\n/// Reads the value of the member the left name reaches, whose offset\n/// the other member's padding moves.\n#[unsafe(no_mangle)]\npub extern \"C\" fn scc_probe(node: *const <Left as HasType>::T) -> u64 {\n    unsafe { (*node).value }\n}\n\n#[inline(never)]\npub fn label() -> &'static str {\n    \"ui-1\"\n}",
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "fn main() {",
                "static SCC_SCRATCH: [u64; 8] = [0, 41, 0, 0, 0, 99, 0, 0];\n\nfn through_scc(table: Option<&JumpTable>, address: usize) -> (u64, bool) {\n    let mapped = table\n        .and_then(|table| table.map.get(&(address as u64)).copied())\n        .filter(|&new| new != 0);\n    let run = unsafe {\n        std::mem::transmute::<\n            usize,\n            extern \"C\" fn(*const <ui::Left as ui::HasType>::T) -> u64,\n        >(mapped.unwrap_or(address as u64) as usize)\n    };\n    (unsafe { run(SCC_SCRATCH.as_ptr().cast()) }, mapped.is_some())\n}\n\nfn main() {",
            ),
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let scc_address = ui::scc_probe as *const () as usize;\n    println!(\"baseline scc={}\", ui::scc_probe(SCC_SCRATCH.as_ptr().cast()));\n    let _ = io::stdout().flush();",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (scc_value, scc_hot) = through_scc(table, scc_address);\n        println!(\"scc probe={scc_value} hot={scc_hot}\");",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.wait("baseline scc=41", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("        pub pad: [u64; 4],", "        pub pad: [u64; 2],")],
    );
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    // The probe's signature cannot answer, so the table leaves it out:
    // it keeps reading the member it was built for, whose value sits at
    // the first member's offset. A mapping would have read the other
    // member's 99.
    session.wait("scc probe=41 hot=false", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn maps_a_signature_another_function_does_not_reorder() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-digest-order");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-digest-order-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // Two mutually recursive types with a probe over one of them: a
    // function the build adds later shifts the debug information the
    // object carries for the component, and the component's spell must
    // not follow that order, or the probe's answer moves without its
    // types moving and the mapping drops.
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "fn main() {",
                "#[repr(C)]\npub struct Pair {\n    pub other: *const Other,\n    pub value: u64,\n}\n\n#[repr(C)]\npub struct Other {\n    pub back: *const Pair,\n    pub value: u64,\n}\n\n#[unsafe(no_mangle)]\npub extern \"C\" fn smudge_order(x: *const Pair) -> u64 {\n    unsafe { (*x).value }\n}\n\nfn through_order(table: Option<&JumpTable>, address: usize, x: &Pair) -> (u64, bool) {\n    let mapped = table\n        .and_then(|table| table.map.get(&(address as u64)).copied())\n        .filter(|&new| new != 0);\n    match mapped {\n        Some(new) => {\n            let run = unsafe {\n                std::mem::transmute::<usize, extern \"C\" fn(*const Pair) -> u64>(new as usize)\n            };\n            (run(x), true)\n        }\n        None => {\n            let run = unsafe {\n                std::mem::transmute::<usize, extern \"C\" fn(*const Pair) -> u64>(address)\n            };\n            (run(x), false)\n        }\n    }\n}\n\nfn main() {",
            ),
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let order_address = smudge_order as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let pair = Pair { other: std::ptr::null(), value: 41 };\n        let (order_value, order_hot) = through_order(table, order_address, &pair);",
            ),
            (
                "bin={bin_value} bin_hot={bin_hot}\"",
                "bin={bin_value} bin_hot={bin_hot} order={order_value} order_hot={order_hot}\"",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "app/src/main.rs",
        &[(
            "#[repr(C)]\npub struct Pair {",
            "#[unsafe(no_mangle)]\npub extern \"C\" fn smudge_extra(y: *const Other) -> u64 {\n    unsafe { (*y).value }\n}\n\n#[repr(C)]\npub struct Pair {",
        )],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("order=41 order_hot=true"),
        "the added function reordered the probe out of the table:\n{line}"
    );
    session.quit();
}

#[test]
fn retries_an_outside_source_a_remap_hides() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-remap-span");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-remap-span-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // The diagnostics a remapped compile prints spell the remapped
    // name, which names no file: the spelling the compile read must
    // join the learned candidates, or repairing the real source
    // never retries the crate that asked for it.
    let flags = "--remap-path-prefix=ui=remapped-ui";
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("CARGO_ENCODED_RUSTFLAGS", Some(flags))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    fs::write(ws.join("ui/late_out.rs"), "/* unterminated\n").unwrap();
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "//! library crate's own replay.",
            "//! library crate's own replay.\n\n#[path = \"../late_out.rs\"]\npub mod late_out;",
        )],
    );
    session.wait("patch 1: compile failed", 1, STEP_TIMEOUT);
    fs::write(ws.join("ui/late_out.rs"), "pub const TAG: u32 = 5;\n").unwrap();
    // The two writes above can reach the session as two saves, each of
    // which fails, so the repair is the first patch that applies, whatever
    // its number.
    let out = session.wait(": applied", 1, STEP_TIMEOUT);
    assert!(
        line_of(&out, ": applied").contains("replayed ui"),
        "the repaired outside source did not retry the library:\n{out}"
    );
    session.quit();
}

#[test]
fn patches_with_a_clustered_operand_taker() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-cluster-op");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-cluster-op-target");
    fs::create_dir_all(&target).expect("the shared target could not be made");
    // A cluster that ends in the search-path flag spends the next
    // argument on itself, so a crate type it spells answers for
    // nothing: the library must stay a library and the patch must
    // replay it.
    let flags = "-gL\u{1f}--crate-type=proc-macro";
    let mut session = Session::start_env(
        &app,
        Some(&target),
        &[],
        &[("CARGO_ENCODED_RUSTFLAGS", Some(flags))],
    );
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "ui/src/lib.rs", &[("    \"ui-1\"", "    \"ui-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("ui=ui-2 ui_hot=true"),
        "the clustered search path turned the library into a macro:\n{line}"
    );
    session.quit();
}

#[test]
fn patches_through_a_deep_function_type_chain() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-fn-chain");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-fn-chain-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // A chain of function types whose every level names the last one
    // twice shares its subtrees: a fingerprint that spelled the whole
    // chain under every edge would grow by a power of the sharing, so
    // the walk hands every type it answered once to the next as a
    // digest and the patch stays readable.
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "fn main() {",
                "pub type Chain0 = extern \"C\" fn(u64) -> u64;\npub type Chain1 = extern \"C\" fn(Chain0, Chain0) -> Chain0;\npub type Chain2 = extern \"C\" fn(Chain1, Chain1) -> Chain1;\npub type Chain3 = extern \"C\" fn(Chain2, Chain2) -> Chain2;\npub type Chain4 = extern \"C\" fn(Chain3, Chain3) -> Chain3;\npub type Chain5 = extern \"C\" fn(Chain4, Chain4) -> Chain4;\npub type Chain6 = extern \"C\" fn(Chain5, Chain5) -> Chain5;\npub type Chain7 = extern \"C\" fn(Chain6, Chain6) -> Chain6;\npub type Chain8 = extern \"C\" fn(Chain7, Chain7) -> Chain7;\npub type Chain9 = extern \"C\" fn(Chain8, Chain8) -> Chain8;\npub type Chain10 = extern \"C\" fn(Chain9, Chain9) -> Chain9;\npub type Chain11 = extern \"C\" fn(Chain10, Chain10) -> Chain10;\n\n#[unsafe(no_mangle)]\npub extern \"C\" fn smudge_chain(f: Chain11) -> u64 {\n    f as *const () as usize as u64\n}\n\nfn main() {",
            ),
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    std::hint::black_box(smudge_chain as *const () as usize);",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        "app/src/main.rs",
        &[(
            "    f as *const () as usize as u64\n}",
            "    f as *const () as usize as u64 + 1\n}",
        )],
    );
    session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    session.quit();
}

#[test]
fn watches_the_scripts_dependencies_the_selection_names() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-script-variant");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-script-variant-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // A feature made generator, a build dependency of ui's script,
    // read core2; the next selection drops the feature. The recordings
    // of both builds sit in the same target, and the script's guard
    // must follow the artifact its own compile linked, not every
    // recording that shares the crate name: the dropped variant's edge
    // would refuse a patch this session could rightly send.
    fs::create_dir_all(ws.join("generator").join("src")).expect("the generator could not be made");
    fs::write(
        ws.join("generator").join("Cargo.toml"),
        "[package]\nname = \"generator\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\ncore2 = { path = \"../core2\", optional = true }\n\n[features]\nuse-core = [\"dep:core2\"]\n",
    )
    .expect("the generator manifest could not be written");
    fs::write(
        ws.join("generator").join("src").join("lib.rs"),
        "pub fn label() -> &'static str {\n    #[cfg(feature = \"use-core\")]\n    {\n        return \"gen-core\";\n    }\n    #[cfg(not(feature = \"use-core\"))]\n    {\n        \"gen-plain\"\n    }\n}\n",
    )
    .expect("the generator could not be written");
    edit_in(
        &ws,
        "Cargo.toml",
        &[(
            "members = [\"app\", \"ui\", \"core2\", \"tls\", \"mac\", \"helper\"]",
            "members = [\"app\", \"ui\", \"core2\", \"tls\", \"mac\", \"helper\", \"generator\"]",
        )],
    );
    edit_in(
        &ws,
        "ui/Cargo.toml",
        &[
            (
                "mac.workspace = true",
                "mac.workspace = true\n\n[build-dependencies]\ngenerator = { path = \"../generator\" }",
            ),
            (
                "[features]\nfancy = []",
                "[features]\nfancy = []\nuse-core = [\"generator/use-core\"]",
            ),
        ],
    );
    fs::write(
        ws.join("ui").join("build.rs"),
        "fn main() {\n    println!(\"cargo:rustc-env=UI_GENERATED={}\", generator::label());\n}\n",
    )
    .expect("the build script could not be written");
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[("    \"ui-1\"\n}", "    env!(\"UI_GENERATED\")\n}")],
    );
    edit_in(
        &ws,
        "app/Cargo.toml",
        &[(
            "tls.workspace = true",
            "tls.workspace = true\n\n[features]\nuse-core = [\"ui/use-core\"]",
        )],
    );
    // The first selection bakes the core2-reading variant and leaves
    // its recordings behind.
    let mut session = Session::start_with(&app, &target, &["--features", "use-core"]);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    session.quit();
    // The second selection rebuilds the script and its dependency
    // without core2, and an edit to core2 must replay through ui
    // instead of refusing on the dropped variant's edge.
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(&ws, "core2/src/lib.rs", &[("    7\n}", "    9\n}")]);
    edit_in(&ws, "app/src/main.rs", &[("\"view-1\"", "\"view-2\"")]);
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    assert!(
        !out.contains("reads core2"),
        "the dropped variant's edge refused the patch:\n{out}"
    );
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("deep=9 deep_hot=true"),
        "the replay through the current variant did not run:\n{line}"
    );
    session.quit();
}

#[test]
fn watches_a_source_the_walk_reaches_only_deep() {
    require_linker();
    let _build = BUILD.lock().unwrap_or_else(|lock| lock.into_inner());
    let ws = fixture_workspace("ws-deep-source");
    let app = ws.join("app");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ws-deep-source-target");
    fs::create_dir_all(&target).expect("the target could not be made");
    // A source tree may run deeper than any bound a walk keeps against
    // the loops a symlink can form, and a recorded source buried in
    // one must still be watched: the watch answers it by name when the
    // directory walk reaches it, whatever the depth.
    let mut relative = String::new();
    let mut path_list = String::new();
    for index in 1..=34 {
        let name = format!("d{index}");
        relative.push_str(&name);
        relative.push('/');
        path_list.push_str(&name);
        path_list.push('/');
    }
    let buried = ws.join("ui").join("src").join(&relative).join("buried.rs");
    std::fs::create_dir_all(buried.parent().expect("the parent is there"))
        .expect("the deep directories could not be made");
    fs::write(&buried, "pub const TAG: &str = \"buried-1\";\n")
        .expect("the buried source could not be written");
    let module = format!(
        "#[path = \"{relative}buried.rs\"]\npub mod buried;\n\npub fn buried_tag() -> &'static str {{\n    buried::TAG\n}}\n"
    );
    edit_in(
        &ws,
        "ui/src/lib.rs",
        &[(
            "//! library crate's own replay.",
            &format!("//! library crate's own replay.\n\n{module}"),
        )],
    );
    edit_in(
        &ws,
        "app/src/main.rs",
        &[
            (
                "    let bin_address = bin_only::tag as *const () as usize;",
                "    let bin_address = bin_only::tag as *const () as usize;\n    let buried_address = ui::buried_tag as *const () as usize;",
            ),
            (
                "        let (bin_value, bin_hot) = through(table, bin_address);",
                "        let (bin_value, bin_hot) = through(table, bin_address);\n        let (buried_value, buried_hot) = through(table, buried_address);",
            ),
            (
                "bin={bin_value} bin_hot={bin_hot}\"",
                "bin={bin_value} bin_hot={bin_hot} buried={buried_value} buried_hot={buried_hot}\"",
            ),
        ],
    );
    let mut session = Session::start(&app, &target);
    session.wait("app running as pid", 1, FAT_TIMEOUT);
    edit_in(
        &ws,
        &format!("ui/src/{relative}buried.rs"),
        &[("buried-1", "buried-2")],
    );
    let out = session.wait("patch 1: applied", 1, STEP_TIMEOUT);
    let line = line_of(&out, "fixture ");
    assert!(
        line.contains("buried=buried-2 buried_hot=true"),
        "the buried source was not watched:\n{line}"
    );
    session.quit();
}
