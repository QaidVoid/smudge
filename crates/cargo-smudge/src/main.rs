//! The `cargo smudge` subcommand, which builds, runs and hot-patches a
//! Smudge app.
//!
//! One executable plays three roles, chosen by how it is invoked. The
//! driver behind `dev` runs cargo with this executable as the rustc
//! wrapper and the linker, so one build produces the original binary and
//! the records every later patch replays. See the `driver` module for the
//! session and the `patch` module for one patch step.

#![allow(clippy::print_stdout)]
// A dev tool reports its progress on stdout, like cargo itself.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod driver;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod dwarf;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod elf;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod patch;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod record;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod roles;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod stub;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod wire;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return linux_main(args);
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    return unsupported_main(args);
}

/// The command on x86-64 Linux, where the roles and the dev session run.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn linux_main(args: Vec<String>) -> ExitCode {
    if let Some(code) = roles::run(args.clone()) {
        return code;
    }
    match driver_command(&args) {
        Some(options) => driver::run(options),
        None => usage(),
    }
}

/// The command elsewhere: every role refuses, and names x86-64 Linux.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn unsupported_main(_args: Vec<String>) -> ExitCode {
    eprintln!(
        "cargo smudge builds, runs and hot-patches Smudge apps on x86-64 \
         Linux, which is the only platform this command supports"
    );
    ExitCode::FAILURE
}

/// Parses the `dev` command, answering its options, or none when the
/// arguments are not a usable `dev` invocation.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn driver_command(args: &[String]) -> Option<driver::Options> {
    // Cargo hands its subcommands their own name first, so `cargo smudge
    // dev` arrives here as `smudge dev`, while a direct invocation of the
    // binary arrives as `dev` alone.
    let args = if args.first().map(String::as_str) == Some("smudge") {
        &args[1..]
    } else {
        args
    };
    if args.first().map(String::as_str) != Some("dev") {
        return None;
    }
    let mut options = driver::Options::default();
    let mut index = 1;
    while index < args.len() {
        let (flag, inline) = match args[index].split_once('=') {
            Some((flag, value)) => (flag, Some(String::from(value))),
            None => (args[index].as_str(), None),
        };
        let consumed = if inline.is_some() { 1 } else { 2 };
        let value = inline.or_else(|| args.get(index + 1).cloned())?;
        match flag {
            "--bin" => options.bin = Some(value),
            "--example" => options.example = Some(value),
            "-p" | "--package" => options.package = Some(value),
            "--features" => options.features.push(value),
            "--profile" => options.profile = Some(value),
            _ => return None,
        }
        index += consumed;
    }
    if options.bin.is_some() && options.example.is_some() {
        return None;
    }
    Some(options)
}

/// Reports how to use the command and answers failure.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn usage() -> ExitCode {
    eprintln!(
        "usage: cargo smudge dev [--bin NAME | --example NAME] [-p NAME] \
         [--features FEATURES] [--profile NAME]\n\
         Type `r` to restart the app and `q` to quit."
    );
    ExitCode::FAILURE
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
#[path = "main_tests.rs"]
mod tests;
