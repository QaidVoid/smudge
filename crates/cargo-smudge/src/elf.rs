//! Reading the symbols of ELF files, and extracting the object members of
//! the rlibs a fat link keeps.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use object::read::archive::ArchiveFile;
use object::{Object, ObjectSection, ObjectSymbol, SymbolKind};

use crate::record;

/// One symbol of an object file, with what pairing and stubbing need of it.
#[derive(Clone, Debug)]
pub(crate) struct Sym {
    /// The address, as the file defines it: virtual in a linked file,
    /// section-relative in a relocatable object.
    pub address: u64,
    /// The size the symbol table records, which sizes a TLS object.
    pub size: u64,
    /// What the symbol is for.
    pub kind: SymbolKind,
    /// Whether the file defines the symbol, rather than importing it.
    pub defined: bool,
    /// Whether the binding is weak.
    pub weak: bool,
    /// Whether the symbol lives in a section the running program can
    /// write, such as `.data` or `.bss`: state a patch would duplicate.
    pub writable: bool,
}

/// Every named symbol of one file, by name.
///
/// Locals are stored too: the stub looks names up that a patch imported
/// globally, and a statically linked binary keeps the definitions it
/// resolved them to, such as those of `core` and `std`, as local symbols.
#[derive(Clone, Debug, Default)]
pub(crate) struct Symbols {
    /// The symbols by name.
    pub map: BTreeMap<String, Sym>,
}

impl Symbols {
    /// Reads the symbols of the ELF file in `bytes`.
    ///
    /// # Errors
    ///
    /// Returns why the file could not be parsed.
    pub(crate) fn read(bytes: &[u8]) -> Result<Self, String> {
        let file = object::File::parse(bytes).map_err(|e| e.to_string())?;
        let mut map = BTreeMap::new();
        for symbol in file.symbols() {
            let Ok(name) = symbol.name() else { continue };
            if name.is_empty() || matches!(symbol.kind(), SymbolKind::File | SymbolKind::Section) {
                continue;
            }
            let defined = !symbol.is_undefined();
            // A definition wins over an import of the same name, which the
            // table of a linked binary lists as a dynamic relocation.
            if !defined && map.contains_key(name) {
                continue;
            }
            // A section the program writes is state: a patch that carries
            // its own copy of it would reset what the running app has
            // built there since it started. A common symbol is such state
            // too, though it names no section until the link places it.
            let writable = symbol.is_common()
                || symbol
                    .section_index()
                    .and_then(|index| file.section_by_index(index).ok())
                    .is_some_and(|section| holds_state(&section));
            map.insert(
                String::from(name),
                Sym {
                    address: symbol.address(),
                    size: symbol.size(),
                    kind: symbol.kind(),
                    defined,
                    weak: symbol.is_weak(),
                    writable,
                },
            );
        }
        Ok(Self { map })
    }

    /// The symbol `name` maps to, if any.
    pub(crate) fn get(&self, name: &str) -> Option<&Sym> {
        self.map.get(name)
    }

    /// The address of the defined `main`, which a wire-format reference
    /// address is rebased against.
    ///
    /// # Errors
    ///
    /// Returns why when the binary defines no such symbol, which means the
    /// fat link failed to export it.
    pub(crate) fn main_address(&self) -> Result<u64, String> {
        self.get("main")
            .filter(|sym| sym.defined && sym.kind == SymbolKind::Text)
            .map(|sym| sym.address)
            .ok_or_else(|| {
                "the original binary defines no exported `main`; the fat link must \
                 export it, which is how the app finds where it was loaded"
                    .to_string()
            })
    }
}

/// The initialization image of a binary's thread-locals, with what a copy
/// of one object needs besides the bytes.
#[derive(Clone, Debug)]
pub(crate) struct TlsTemplate {
    /// The image, whose offsets the addresses of TLS symbols are.
    pub image: Vec<u8>,
    /// The alignment the image lays its objects out at, never zero so it
    /// always turns into an assembly alignment.
    pub alignment: u32,
    /// The relative relocations inside the image, by image offset and
    /// addend, which its pointer slots live in.
    pub relocations: Vec<(u64, u64)>,
}

impl Default for TlsTemplate {
    fn default() -> Self {
        Self {
            image: Vec::new(),
            alignment: 1,
            relocations: Vec::new(),
        }
    }
}

/// Reads the initialization image of the file's thread-locals.
///
/// A pointer in an initializer lives in a relocation whose addend the
/// loader slides by the load base, not in the raw bytes of the image, so
/// the relocations travel with it.
pub(crate) fn tdata_template(bytes: &[u8]) -> TlsTemplate {
    let Ok(file) = object::File::parse(bytes) else {
        return TlsTemplate::default();
    };
    let Some(section) = file.section_by_name(".tdata") else {
        // A binary whose thread-locals are all zero-initialized has no
        // initialization image, but its `.tbss` still sets the alignment
        // the copies of its objects line up at.
        let alignment = file
            .section_by_name(".tbss")
            .map(|section| u32::try_from(section.align()).unwrap_or(1))
            .unwrap_or(1)
            .max(1);
        return TlsTemplate {
            image: Vec::new(),
            alignment,
            relocations: Vec::new(),
        };
    };
    let start = section.address();
    let end = start + section.size();
    let alignment = u32::try_from(section.align()).unwrap_or(1).max(1);
    let image = section.data().map(|data| data.to_vec()).unwrap_or_default();
    let Some(relocations) = file.dynamic_relocations() else {
        return TlsTemplate {
            image,
            alignment,
            relocations: Vec::new(),
        };
    };
    let mut found = Vec::new();
    // The generic kind the crate answers for x86-64 names a PC-relative
    // relocation, and the load-base-relative one the loader slides stays
    // unrecognized there, so the ELF type decides.
    let relative = object::RelocationFlags::Elf {
        r_type: object::elf::R_X86_64_RELATIVE,
    };
    for (offset, relocation) in relocations {
        if offset >= start && offset + 8 <= end && relocation.flags() == relative {
            found.push((offset - start, relocation.addend() as u64));
        }
    }
    TlsTemplate {
        image,
        alignment,
        relocations: found,
    }
}

/// The relocation types that reach a symbol through a full address the
/// loader fills in, which can name the running binary's copy wherever it
/// lies: the global offset table entry, in its plain and relaxable forms,
/// and a 64-bit absolute address.
const FULL_ADDRESS_RELOCATIONS: [u32; 4] = [
    object::elf::R_X86_64_GOTPCREL,
    object::elf::R_X86_64_GOTPCRELX,
    object::elf::R_X86_64_REX_GOTPCRELX,
    object::elf::R_X86_64_64,
];

/// What one object says about sharing its statics with the running binary.
///
/// A static qualifies when it is global and writable, the binary defines a
/// static of that name and size, and every reference to it in the objects
/// a patch links reaches it through a full address: a reference through
/// its section, or one that encodes a 32-bit distance, names a place in
/// the patch library itself, which the binary's copy lies too far from.
#[derive(Debug, Default)]
pub(crate) struct StaticFacts {
    /// The statics the object defines that could be the binary's.
    candidates: Vec<String>,
    /// The binary's writable statics the object reaches through something
    /// other than a full address: their section, or a 32-bit distance.
    near: Vec<String>,
}

/// Reads what the object in `bytes` says about sharing statics with the
/// running `binary`.
///
/// # Errors
///
/// Returns why the object could not be parsed.
pub(crate) fn static_facts(bytes: &[u8], binary: &Symbols) -> Result<StaticFacts, String> {
    let file = object::File::parse(bytes).map_err(|e| e.to_string())?;
    // The running copy must be state as well: an immutable static an edit
    // made writable sits in read-only memory.
    let running_state = |name: &str| {
        binary.get(name).filter(|running| {
            running.defined
                && running.writable
                && !matches!(running.kind, SymbolKind::Text | SymbolKind::Tls)
        })
    };
    let mut candidates: Vec<(String, object::SectionIndex)> = Vec::new();
    for symbol in file.symbols() {
        let Ok(name) = symbol.name() else { continue };
        if !symbol.is_global()
            || symbol.is_undefined()
            || name.starts_with("DW.ref.")
            || !matches!(symbol.kind(), SymbolKind::Data | SymbolKind::Unknown)
        {
            continue;
        }
        let Some(section) = symbol.section_index() else {
            continue;
        };
        // Only state is shared: an immutable static carries the edit's new
        // contents, which the patched code must read.
        let writable = file
            .section_by_index(section)
            .is_ok_and(|held| holds_state(&held));
        if !writable {
            continue;
        }
        if running_state(name).is_some_and(|running| running.size == symbol.size()) {
            candidates.push((String::from(name), section));
        }
    }
    let mut near = Vec::new();
    for section in file.sections() {
        for (_, relocation) in section.relocations() {
            let object::RelocationTarget::Symbol(index) = relocation.target() else {
                continue;
            };
            let Ok(target) = file.symbol_by_index(index) else {
                continue;
            };
            if target.kind() == SymbolKind::Section {
                // A reference through the section reaches whatever the
                // section holds at an offset, in this object alone.
                for (name, held) in &candidates {
                    if Some(*held) == target.section_index() {
                        near.push(name.clone());
                    }
                }
                continue;
            }
            let Ok(name) = target.name() else { continue };
            let full = matches!(
                relocation.flags(),
                object::RelocationFlags::Elf { r_type } if FULL_ADDRESS_RELOCATIONS.contains(&r_type)
            );
            if !full && running_state(name).is_some() {
                near.push(String::from(name));
            }
        }
    }
    near.sort();
    near.dedup();
    Ok(StaticFacts {
        candidates: candidates.into_iter().map(|(name, _)| name).collect(),
        near,
    })
}

/// The writable statics of the objects whose `facts` are given that a
/// patch can take from the running binary instead of carrying fresh
/// copies of them: the candidates no object reaches but through a full
/// address.
pub(crate) fn shared_statics<'a>(
    facts: impl IntoIterator<Item = &'a StaticFacts> + Clone,
) -> std::collections::BTreeSet<String> {
    let near: std::collections::BTreeSet<&str> = facts
        .clone()
        .into_iter()
        .flat_map(|facts| facts.near.iter().map(String::as_str))
        .collect();
    facts
        .into_iter()
        .flat_map(|facts| facts.candidates.iter())
        .filter(|name| !near.contains(name.as_str()))
        .cloned()
        .collect()
}

/// Writes a copy of the object at `path` to `out` in which the statics
/// `names` are imports, not definitions, so the stub's absolute symbols at
/// the running binary's copies answer every reference to them.
///
/// Only the symbol entries change: their visibility, section index, value
/// and size become those of an undefined symbol, and every relocation keeps
/// naming
/// the same entry. The names must be global symbols of the object, which
/// already sit after its locals.
///
/// # Errors
///
/// Returns why the object could not be read, parsed or written.
pub(crate) fn import_statics(
    path: &Path,
    names: &std::collections::BTreeSet<String>,
    out: &Path,
) -> Result<(), String> {
    let mut bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut entries = Vec::new();
    {
        let file = object::File::parse(&bytes[..]).map_err(|e| e.to_string())?;
        let Some(table) = file.section_by_name(".symtab") else {
            return Err(format!(
                "{}: the object has no symbol table",
                path.display()
            ));
        };
        let (start, _) = table
            .file_range()
            .ok_or_else(|| format!("{}: the symbol table has no file range", path.display()))?;
        for symbol in file.symbols() {
            if symbol.is_global()
                && !symbol.is_undefined()
                && symbol.name().is_ok_and(|name| names.contains(name))
            {
                // An ELF64 symbol entry is 24 bytes: name, info, other,
                // section index, value and size.
                entries.push(start as usize + symbol.index().0 * 24);
            }
        }
    }
    for entry in entries {
        // From the visibility on: an import of default visibility, which
        // every linker resolves to the stub's absolute definition through
        // the global offset table.
        let Some(fields) = bytes.get_mut(entry + 5..entry + 24) else {
            return Err(format!(
                "{}: a symbol entry lies past the file",
                path.display()
            ));
        };
        fields.fill(0);
    }
    record::write_bytes(&bytes, out)
}

/// Whether `section` holds data the running program writes: data and
/// zero-initialized data, but not the data that is only writable while the
/// loader relocates it, which immutable statics holding pointers sit in
/// and which is read-only from then on.
fn holds_state(section: &object::Section<'_, '_>) -> bool {
    let relocated_only = section
        .name()
        .is_ok_and(|name| name.starts_with(".data.rel.ro"));
    !relocated_only
        && matches!(
            section.kind(),
            object::SectionKind::Data
                | object::SectionKind::UninitializedData
                | object::SectionKind::Common
        )
}

/// Writes the `.rcgu.o` members of every rlib under `target` out to `dir`,
/// and returns their paths.
///
/// The linker is handed the extracted members as plain object files, which a
/// linker includes whole, instead of an archive it would only fill from.
/// Rlibs elsewhere, which means the sysroot, stay on the link line whole.
///
/// # Errors
///
/// Returns the first rlib that could not be read or parsed.
pub(crate) fn extract_rlib_objects(
    args: &[String],
    target: &Path,
    dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    let mut written = Vec::new();
    for arg in args.iter().filter(|arg| arg.ends_with(".rlib")) {
        let path = Path::new(arg);
        if !path.starts_with(target) {
            continue;
        }
        let bytes = std::fs::read(path).map_err(|e| format!("{arg}: {e}"))?;
        let archive = ArchiveFile::parse(&*bytes).map_err(|e| format!("{arg}: {e}"))?;
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("rlib");
        for member in archive.members() {
            let member = member.map_err(|e| format!("{arg}: {e}"))?;
            let name = String::from_utf8_lossy(member.name()).to_string();
            if !name.ends_with(".rcgu.o") || member.size() == 0 {
                continue;
            }
            let member_bytes = member
                .data(&*bytes)
                .map_err(|e| format!("{arg}: {name}: {e}"))?;
            let mut out = dir.join(&name);
            if written.iter().any(|w| w == &out) {
                out = dir.join(format!("{stem}_{name}"));
            }
            record::write_bytes(member_bytes, &out)?;
            written.push(out);
        }
    }
    Ok(written)
}

/// The names the archive symbol indexes of the rlibs under the sysroot's
/// `rustlib` directory define, which are the standard library's: the link
/// takes those rlibs whole from there. Like the compiler, this looks in
/// `lib64/rustlib` when it exists and in `lib/rustlib` otherwise. An rlib counts only when its canonical path
/// lies under that canonical directory, so a name in this set is the
/// standard library's by where it comes from and not by how it is spelled,
/// and a crate of any other origin stays out, even one a project builds
/// under a sysroot such as `/usr`.
///
/// # Errors
///
/// Returns the first sysroot rlib that could not be read or parsed.
pub(crate) fn sysroot_symbols(
    args: &[String],
    sysroot: &Path,
) -> Result<std::collections::BTreeSet<String>, String> {
    let mut names = std::collections::BTreeSet::new();
    // A relative sysroot would resolve against wherever this process runs,
    // where a project's own `lib/rustlib` could sit.
    if !sysroot.is_absolute() {
        return Ok(names);
    }
    let wide = sysroot.join("lib64").join("rustlib");
    let rustlib = if wide.is_dir() {
        wide
    } else {
        sysroot.join("lib").join("rustlib")
    };
    let Ok(libraries) = rustlib.canonicalize() else {
        return Ok(names);
    };
    for arg in args.iter().filter(|arg| arg.ends_with(".rlib")) {
        let Ok(path) = Path::new(arg).canonicalize() else {
            continue;
        };
        if !path.starts_with(&libraries) {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("{arg}: {e}"))?;
        let archive = ArchiveFile::parse(&*bytes).map_err(|e| format!("{arg}: {e}"))?;
        let Some(symbols) = archive.symbols().map_err(|e| format!("{arg}: {e}"))? else {
            continue;
        };
        for symbol in symbols {
            let symbol = symbol.map_err(|e| format!("{arg}: {e}"))?;
            names.insert(String::from_utf8_lossy(symbol.name()).into_owned());
        }
    }
    Ok(names)
}

#[cfg(test)]
#[path = "elf_tests.rs"]
mod tests;
