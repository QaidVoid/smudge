//! The `cargo smudge` subcommand: `new` and `init` make a Smudge app from
//! the template, and `dev` builds, runs and hot-patches it.
//!
//! Behind `dev`, one executable plays three roles, chosen by how it is
//! invoked. The driver runs cargo with this executable as the rustc
//! wrapper and the linker, so one build produces the original binary and
//! the records every later patch replays. See the `driver` module for the
//! session, the `patch` module for one patch step and the `scaffold`
//! module for `new` and `init`.

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
mod scaffold;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod stub;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod wire;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    if let Some(code) = roles::run(args.clone()) {
        return code;
    }
    let command = subcommand(&args);
    match scaffold::parse(command) {
        Some(Ok(scaffold)) => return scaffold::run(&scaffold),
        Some(Err(reason)) => {
            eprintln!("cargo smudge: {reason}");
            return usage();
        }
        None => {}
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return linux_main(command);
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    return unsupported_main();
}

/// The command's own arguments, from the subcommand name on. Cargo hands
/// its subcommands their own name first, so `cargo smudge dev` arrives as
/// `smudge dev`, while a direct invocation of the binary arrives as `dev`
/// alone.
fn subcommand(args: &[String]) -> &[String] {
    match args.first().map(String::as_str) {
        Some("smudge") => &args[1..],
        _ => args,
    }
}

/// The command on x86-64 Linux, where the roles and the dev session run.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn linux_main(args: &[String]) -> ExitCode {
    match driver_command(args) {
        Some(options) => driver::run(options),
        None => usage(),
    }
}

/// The command elsewhere, where only `new` and `init` run: `dev` refuses,
/// and names x86-64 Linux.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn unsupported_main() -> ExitCode {
    eprintln!(
        "cargo smudge dev builds, runs and hot-patches Smudge apps on x86-64 \
         Linux, which is the only platform it supports. The new and init \
         commands work everywhere."
    );
    ExitCode::FAILURE
}

/// Parses the `dev` command, answering its options, or none when the
/// arguments are not a usable `dev` invocation.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn driver_command(args: &[String]) -> Option<driver::Options> {
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
fn usage() -> ExitCode {
    eprintln!(
        "usage: cargo smudge new PATH [--name NAME] [--vcs VCS] [--edition YEAR]\n       \
         cargo smudge init [PATH] [--name NAME] [--vcs VCS] [--edition YEAR]\n       \
         cargo smudge dev [--bin NAME | --example NAME] [-p NAME] \
         [--features FEATURES] [--profile NAME]\n\
         While dev runs, type `r` to restart the app and `q` to quit."
    );
    ExitCode::FAILURE
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
#[path = "main_tests.rs"]
mod tests;
