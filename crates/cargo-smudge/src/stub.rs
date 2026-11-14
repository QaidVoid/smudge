//! The stub object that resolves a patch's imports against the running
//! binary.
//!
//! Every function the patch imports from the binary is a trampoline that
//! jumps to the function's address in the running process, and every other
//! symbol is an absolute symbol at its address in the running process,
//! which the dynamic linker leaves as it is. A thread-local import cannot
//! be resolved that way: a loaded library binds thread-local references to
//! the module that defines them, and the binary keeps its own locals out of
//! its dynamic symbol table. The stub therefore defines every thread-local
//! the patch imports as a copy of the binary's initialization image, and
//! the patched code reads and writes its own instance, whose values start
//! at the initializer. The patch step only lets the standard library's
//! thread-locals through, which hold per-thread caches that inlined
//! standard library code reaches, and refuses a patch that imports any
//! other crate's.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::elf::{Symbols, TlsTemplate};

/// Builds the stub object for the names a patch imports, and returns its
/// path and how many thread-locals it copies.
///
/// `slide` is how far the running process's copy of the binary sits from
/// the addresses it was linked at. `template` is the initialization image
/// of the binary's thread-local storage, whose offsets the addresses of
/// TLS symbols are. The names must already be filtered: names the binary
/// does not define are decided by the caller.
///
/// # Errors
///
/// Returns why the assembly could not be written or assembled.
pub(crate) fn build(
    imports: &[String],
    original: &Symbols,
    slide: u64,
    template: &TlsTemplate,
    dir: &Path,
    generation: u32,
) -> Result<(PathBuf, usize), String> {
    let tdata = &template.image;
    let source = dir.join(format!("stub-{generation}.s"));
    let object = dir.join(format!("stub-{generation}.o"));
    let mut asm = String::from(".section .text.smudge_stub,\"ax\",@progbits\n.p2align 4\n");
    let mut addresses = String::from(".section .rodata\n");
    let mut locals = String::from(".section .tdata,\"aT\",@progbits\n");
    let mut text = 0;
    let mut tls = 0;
    for name in imports {
        let Some(sym) = original.get(name) else {
            continue;
        };
        if !sym.defined {
            // Undefined imports resolve through the app's own libraries at
            // load, and the weak ones among them resolve to nothing.
            continue;
        }
        // The address the running process holds, which a trampoline jumps
        // to and an absolute symbol slides to.
        let address = sym.address + slide;
        if sym.kind == object::SymbolKind::Text {
            asm.push_str(&format!(
                ".globl {name}\n.p2align 4\n{name}:\n    jmp *.Lsmudge_target_{text}(%rip)\n"
            ));
            addresses.push_str(&format!(
                ".Lsmudge_target_{text}:\n    .quad {address:#x}\n"
            ));
            text += 1;
        } else if sym.kind == object::SymbolKind::Tls {
            // The offset a TLS symbol holds is its place in the binary's
            // initialization image, which the copy starts from, and the
            // image's alignment covers the alignment of every object in
            // it.
            let align = template.alignment.trailing_zeros().max(3);
            locals.push_str(&format!(".p2align {align}\n.globl {name}\n{name}:\n"));
            let start = sym.address.min(tdata.len() as u64) as usize;
            let length = sym.size as usize;
            // A pointer slot holds the target's load-time address, which
            // the copy must slide like every other reference to the
            // running process. The relocations are in the image's
            // coordinates, and a slot only counts when it fits whole
            // inside the copy.
            let moved: std::collections::BTreeMap<u64, u64> = template
                .relocations
                .iter()
                .filter_map(|(offset, addend)| {
                    let at = (*offset as usize).checked_sub(start)?;
                    (at + 8 <= length).then_some((at as u64, *addend))
                })
                .collect();
            let mut at = 0usize;
            while at < length {
                // The map only holds slots that fit whole inside the copy.
                if let Some(addend) = moved.get(&(at as u64)) {
                    locals.push_str(&format!("    .quad {:#x}\n", addend + slide));
                    at += 8;
                } else {
                    let byte = tdata.get(start + at).copied().unwrap_or(0);
                    locals.push_str(&format!("    .byte {byte:#x}\n"));
                    at += 1;
                }
            }
            tls += 1;
        } else {
            addresses.push_str(&format!(".globl {name}\n.set {name}, {address:#x}\n"));
        }
    }
    asm.push_str(&addresses);
    asm.push_str(&locals);
    crate::record::write_bytes(asm.as_bytes(), &source)?;
    let status = Command::new(assembler().as_deref().unwrap_or("cc"))
        .arg("-c")
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .output()
        .map_err(|e| format!("{}: {e}", source.display()))?;
    if !status.status.success() {
        return Err(format!(
            "assembling the stub failed: {}",
            String::from_utf8_lossy(&status.stderr).trim()
        ));
    }
    let _ = std::fs::remove_file(&source);
    Ok((object, tls))
}

/// Finds a C compiler the stub can be assembled with, trying the common
/// names once per session.
fn assembler() -> Option<String> {
    static FOUND: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    FOUND
        .get_or_init(|| {
            for name in ["cc", "gcc", "clang"] {
                let probe = Command::new(name).arg("--version").output();
                if matches!(&probe, Ok(output) if output.status.success()) {
                    return Some(String::from(name));
                }
            }
            None
        })
        .clone()
}
