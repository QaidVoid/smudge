//! Signatures read from the DWARF of a binary or of patch objects.
//!
//! A signature is the parameter types and the return type of one function,
//! serialized from the DWARF that `-Cdebuginfo=2` produces. Two builds of
//! the same function serialize to the same string, and a change to a
//! parameter or to the return type changes it, which is what the pairing
//! of a patch needs to know.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;

use gimli::{
    AttributeValue, DebuggingInformationEntry, Dwarf, EndianArcSlice, Error, Reader, Relocate,
    RelocateReader, RunTimeEndian, SectionId, Unit, UnitOffset,
};
use object::{Object, ObjectSection};

/// The reader of one DWARF section, with the relocations of a relocatable
/// object applied.
type SectionReader = RelocateReader<EndianArcSlice<RunTimeEndian>, Reloc>;

/// The parsed DWARF sections of one file.
type Sections = Dwarf<SectionReader>;

/// The relocations of one DWARF section, which the reader applies to
/// addresses and offsets.
#[derive(Debug, Clone)]
struct Reloc(Arc<object::RelocationMap>);

impl Relocate for Reloc {
    fn relocate_address(&self, offset: usize, value: u64) -> Result<u64, Error> {
        Ok(self.0.relocate(offset as u64, value))
    }

    fn relocate_offset(&self, offset: usize, value: usize) -> Result<usize, Error> {
        Ok(self.0.relocate(offset as u64, value as u64) as usize)
    }
}

/// Loads the DWARF sections of the ELF file in `bytes`, with the
/// relocations of a relocatable object applied to addresses and offsets.
///
/// # Errors
///
/// Returns why the file or a section could not be parsed.
fn load_dwarf(bytes: &[u8]) -> Result<Sections, String> {
    let file = object::File::parse(bytes).map_err(|e| e.to_string())?;
    let endian = if file.is_little_endian() {
        RunTimeEndian::Little
    } else {
        RunTimeEndian::Big
    };
    let empty = || {
        RelocateReader::new(
            EndianArcSlice::new(Arc::from(Vec::new().into_boxed_slice()), endian),
            Reloc(Arc::new(object::RelocationMap::default())),
        )
    };
    Dwarf::load(|id: SectionId| -> Result<SectionReader, String> {
        let Some(section) = file.section_by_name(id.name()) else {
            return Ok(empty());
        };
        let data = section.data().map_err(|e| e.to_string())?;
        let mut relocations = object::RelocationMap::default();
        for (offset, relocation) in section.relocations() {
            // A relocation the DWARF format does not use, or one whose
            // target cannot be resolved, contributes nothing.
            relocations.add(&file, offset, relocation).ok();
        }
        Ok(RelocateReader::new(
            EndianArcSlice::new(Arc::from(data), endian),
            Reloc(Arc::new(relocations)),
        ))
    })
    .map_err(|e| e.to_string())
}

/// The signatures of one file: which address has which signature.
///
/// Functions whose types the DWARF does not describe have no signature, so
/// a pairing against them cannot be verified and is left out.
pub(crate) struct Signatures {
    dwarf: Sections,
    /// The parsed units, with the offset their data starts at in
    /// `.debug_info`.
    units: Vec<(usize, Unit<SectionReader>)>,
    /// Which unit holds each function address the DWARF places in code.
    entries: HashMap<u64, usize>,
    /// The subprogram entry offsets of one unit, by function address.
    offsets: HashMap<usize, HashMap<u64, UnitOffset>>,
    /// The subprogram entry offsets of one file, by mangled symbol name.
    /// An object's `low_pc` values cannot be resolved to comparable
    /// addresses, so its functions are found by name instead.
    names: HashMap<String, (usize, UnitOffset)>,
    /// Serialized type descriptions, by unit and unit offset.
    types: HashMap<(usize, usize), String>,
    /// The parent of every die, by unit and unit offset.
    parents: HashMap<usize, HashMap<usize, usize>>,
}

impl Signatures {
    /// Reads the signatures of the ELF file in `bytes`.
    ///
    /// # Errors
    ///
    /// Returns why the DWARF could not be parsed. A file without DWARF is
    /// not an error: it simply has no signatures.
    pub(crate) fn read(bytes: &[u8]) -> Result<Self, String> {
        let dwarf = load_dwarf(bytes)?;
        let mut units = Vec::new();
        let mut names = HashMap::new();
        let mut offsets: HashMap<usize, HashMap<u64, UnitOffset>> = HashMap::new();
        let mut headers = dwarf.units();
        while let Some(header) = headers.next().map_err(|e| e.to_string())? {
            let global = header
                .offset()
                .as_debug_info_offset()
                .map(|offset| offset.0)
                .ok_or_else(|| String::from("a unit in .debug_types is not supported"))?;
            let unit = dwarf.unit(header).map_err(|e| e.to_string())?;
            let index = units.len();
            for (address, offset) in subprograms(&unit)? {
                offsets.entry(index).or_default().insert(address, offset);
            }
            for (name, offset) in named_subprograms(&dwarf, &unit)? {
                names.insert(name, (index, offset));
            }
            units.push((global, unit));
        }
        let mut entries = HashMap::new();
        for (index, per_unit) in &offsets {
            for address in per_unit.keys() {
                entries.insert(*address, *index);
            }
        }
        Ok(Self {
            dwarf,
            units,
            entries,
            offsets,
            names,
            types: HashMap::new(),
            parents: HashMap::new(),
        })
    }

    /// The signature of the function at `address`.
    ///
    /// # Errors
    ///
    /// Returns why the signature could not be serialized, which leaves the
    /// pairing to decide without it.
    pub(crate) fn signature(&mut self, address: u64) -> Result<Option<String>, String> {
        let Some(&index) = self.entries.get(&address) else {
            return Ok(None);
        };
        let offset = self.offsets[&index][&address];
        self.signature_at(index, offset)
    }

    /// The signature of the function whose mangled name is `name`.
    ///
    /// # Errors
    ///
    /// Returns why the signature could not be serialized, which leaves the
    /// pairing to decide without it.
    pub(crate) fn signature_by_name(&mut self, name: &str) -> Result<Option<String>, String> {
        let Some(&(index, offset)) = self.names.get(name) else {
            return Ok(None);
        };
        self.signature_at(index, offset)
    }

    /// The signature of the subprogram at `offset` of unit `index`.
    fn signature_at(&mut self, index: usize, offset: UnitOffset) -> Result<Option<String>, String> {
        let mut parser = Parser {
            dwarf: &self.dwarf,
            units: &self.units,
            types: &mut self.types,
            parents: &mut self.parents,
        };
        parser.signature(index, offset)
    }
}

/// Serializes the signatures and types of one file's DWARF.
///
/// The fields are borrowed apart from `Signatures` so a unit can stay
/// borrowed while the caches fill.
struct Parser<'a> {
    /// The DWARF sections, for string and cross-unit lookup.
    dwarf: &'a Sections,
    /// The parsed units, with their `.debug_info` starts, for cross-unit
    /// type references.
    units: &'a [(usize, Unit<SectionReader>)],
    /// The type descriptions built so far, by unit and unit offset.
    types: &'a mut HashMap<(usize, usize), String>,
    /// The parent of every die, by unit start and unit offset.
    parents: &'a mut HashMap<usize, HashMap<usize, usize>>,
}

impl<'a> Parser<'a> {
    /// The unit at `index`, which borrows the fields and not the parser,
    /// so the caches can fill while a unit stays borrowed.
    fn unit(&self, index: usize) -> Option<&'a Unit<SectionReader>> {
        self.units.get(index).map(|(_, unit)| unit)
    }

    /// The signature of the subprogram whose entry dies at `offset` of
    /// unit `index`.
    fn signature(&mut self, index: usize, offset: UnitOffset) -> Result<Option<String>, String> {
        let Some(unit) = self.unit(index) else {
            return Err(String::from("the subprogram's unit is missing"));
        };
        let die = unit.entry(offset).map_err(string)?;
        let mut return_type = die.attr_value(gimli::DW_AT_type).map_err(string)?;
        // A definition that describes its function through a specification
        // keeps the return type on the die the specification names.
        if return_type.is_none()
            && let Some(AttributeValue::UnitRef(specification)) =
                die.attr_value(gimli::DW_AT_specification).map_err(string)?
        {
            let declaration = unit.entry(specification).map_err(string)?;
            return_type = declaration.attr_value(gimli::DW_AT_type).map_err(string)?;
        }
        let parameters = self.parameters(unit, offset)?;
        let mut parts = Vec::with_capacity(parameters.len() + 1);
        parts.push(self.describe(index, return_type)?);
        for parameter in parameters {
            parts.push(self.describe(index, parameter)?);
        }
        Ok(Some(parts.join("|")))
    }

    /// The type attributes of a die's formal parameters.
    fn parameters(
        &mut self,
        unit: &Unit<SectionReader>,
        offset: UnitOffset,
    ) -> Result<Vec<Option<AttributeValue<SectionReader>>>, String> {
        let mut found = Vec::new();
        let mut children = unit.entries_at_offset(offset).map_err(string)?;
        // The cursor's first die is the one itself, and its parameters are
        // the children after it.
        if children.next_dfs().map_err(string)?.is_none() {
            return Ok(found);
        }
        // The depths the cursor answers are deltas from the previous die, so
        // the children of the one are the entries until the depth is 0
        // again.
        let mut depth = 0usize;
        while let Some((delta, child)) = children.next_dfs().map_err(string)? {
            let next = depth as isize + delta;
            if next <= 0 {
                break;
            }
            depth = next as usize;
            if depth == 1 && child.tag() == gimli::DW_TAG_formal_parameter {
                found.push(child.attr_value(gimli::DW_AT_type).map_err(string)?);
            }
        }
        Ok(found)
    }

    /// Serializes the type an attribute names, or `()` when there is none.
    fn describe(
        &mut self,
        index: usize,
        attribute: Option<AttributeValue<SectionReader>>,
    ) -> Result<String, String> {
        let Some(value) = attribute else {
            return Ok(String::from("()"));
        };
        let Some((unit, offset)) = self.type_ref(index, value) else {
            return Ok(String::from("<unresolved>"));
        };
        self.type_desc(unit, offset)
    }

    /// The unit and offset a type attribute names, following cross-unit
    /// references into their own unit.
    fn type_ref(
        &self,
        index: usize,
        value: AttributeValue<SectionReader>,
    ) -> Option<(usize, UnitOffset)> {
        match value {
            AttributeValue::UnitRef(offset) => Some((index, offset)),
            AttributeValue::DebugInfoRef(global) => self
                .units
                .iter()
                .enumerate()
                .find(|(_, (start, parsed))| {
                    let end = start + parsed.header.length_including_self();
                    global.0 >= *start && global.0 < end
                })
                .map(|(index, (start, _))| (index, UnitOffset(global.0 - start))),
            _ => None,
        }
    }

    /// Serializes the type whose description dies at `offset` of unit
    /// `index`.
    fn type_desc(&mut self, index: usize, offset: UnitOffset) -> Result<String, String> {
        let Some(unit) = self.unit(index) else {
            return Err(String::from("the type's unit is missing"));
        };
        let unit_start = self.units[index].0;
        let key = (unit_start, offset.0);
        if let Some(desc) = self.types.get(&key) {
            return Ok(desc.clone());
        }
        let die = unit.entry(offset).map_err(string)?;
        let inner = |parser: &mut Self| -> Result<String, String> {
            parser.describe(index, die.attr_value(gimli::DW_AT_type).map_err(string)?)
        };
        let desc = match die.tag() {
            gimli::DW_TAG_base_type
            | gimli::DW_TAG_structure_type
            | gimli::DW_TAG_union_type
            | gimli::DW_TAG_enumeration_type => self.qualified_name(index, offset)?,
            gimli::DW_TAG_pointer_type => format!("*{}", inner(self)?),
            gimli::DW_TAG_reference_type => format!("&{}", inner(self)?),
            gimli::DW_TAG_rvalue_reference_type => format!("&&{}", inner(self)?),
            gimli::DW_TAG_const_type => format!("const {}", inner(self)?),
            gimli::DW_TAG_volatile_type => format!("volatile {}", inner(self)?),
            gimli::DW_TAG_restrict_type => format!("restrict {}", inner(self)?),
            gimli::DW_TAG_typedef => {
                let name = self.name_of(unit, &die)?.unwrap_or_default();
                format!("alias {name} = {}", inner(self)?)
            }
            gimli::DW_TAG_array_type => {
                let count = array_count(unit, offset)?;
                format!("[{}; {count}]", inner(self)?)
            }
            gimli::DW_TAG_subroutine_type => {
                let return_type = die.attr_value(gimli::DW_AT_type).map_err(string)?;
                let parameters = self.parameters(unit, offset)?;
                let mut parts = Vec::with_capacity(parameters.len() + 1);
                parts.push(self.describe(index, return_type)?);
                for parameter in parameters {
                    parts.push(self.describe(index, parameter)?);
                }
                format!("fn({})", parts.join(", "))
            }
            _ => self.qualified_name(index, offset)?,
        };
        self.types.insert(key, desc.clone());
        Ok(desc)
    }

    /// The namespace-qualified name of the die at `offset` of unit `index`.
    fn qualified_name(&mut self, index: usize, offset: UnitOffset) -> Result<String, String> {
        let Some(unit) = self.unit(index) else {
            return Err(String::from("the type's unit is missing"));
        };
        let unit_start = self.units[index].0;
        if let Entry::Vacant(slot) = self.parents.entry(unit_start) {
            slot.insert(unit_parents(unit)?);
        }
        let parents = &self.parents[&unit_start];
        let mut parts = Vec::new();
        let mut current = offset;
        loop {
            let die = unit.entry(current).map_err(string)?;
            if let Some(name) = self.name_of(unit, &die)?
                && matches!(
                    die.tag(),
                    gimli::DW_TAG_base_type
                        | gimli::DW_TAG_namespace
                        | gimli::DW_TAG_structure_type
                        | gimli::DW_TAG_union_type
                        | gimli::DW_TAG_enumeration_type
                        | gimli::DW_TAG_class_type
                )
            {
                parts.push(name);
            }
            match parents.get(&current.0) {
                Some(&parent) => current = UnitOffset(parent),
                None => break,
            }
        }
        parts.reverse();
        let joined = parts.join("::");
        Ok(if joined.is_empty() {
            String::from("<unnamed>")
        } else {
            joined
        })
    }

    fn name_of(
        &self,
        unit: &Unit<SectionReader>,
        die: &DebuggingInformationEntry<'_, '_, SectionReader>,
    ) -> Result<Option<String>, String> {
        if let Some(value) = die.attr_value(gimli::DW_AT_name).map_err(string)? {
            let text = self.dwarf.attr_string(unit, value).map_err(string)?;
            let name = text.to_string_lossy().map_err(string)?.into_owned();
            return Ok(Some(name));
        }
        for following in [gimli::DW_AT_specification, gimli::DW_AT_abstract_origin] {
            if let Some(AttributeValue::UnitRef(referred)) =
                die.attr_value(following).map_err(string)?
            {
                let die = unit.entry(referred).map_err(string)?;
                if let Some(name) = self.name_of(unit, &die)? {
                    return Ok(Some(name));
                }
            }
        }
        Ok(None)
    }
}

/// Turns a gimli error into its message, which is all the pairing needs.
fn string(error: Error) -> String {
    error.to_string()
}

/// The addresses and entry offsets of the subprograms of one unit that the
/// DWARF places in code, which are the functions a symbol table pairs.
fn subprograms(unit: &Unit<SectionReader>) -> Result<Vec<(u64, UnitOffset)>, String> {
    let mut found = Vec::new();
    let mut entries = unit.entries();
    while let Some((_, die)) = entries.next_dfs().map_err(string)? {
        if die.tag() != gimli::DW_TAG_subprogram {
            continue;
        }
        if let Some(AttributeValue::Addr(address)) =
            die.attr_value(gimli::DW_AT_low_pc).map_err(string)?
        {
            found.push((address, die.offset()));
        }
    }
    Ok(found)
}

/// The mangled names and entry offsets of the subprograms of one unit that
/// hold code, which are the functions a symbol table pairs by name.
///
/// A definition usually names its function through a specification that
/// points at the declaration carrying the linkage name, so the target is
/// resolved when the definition itself has none.
fn named_subprograms(
    dwarf: &Sections,
    unit: &Unit<SectionReader>,
) -> Result<Vec<(String, UnitOffset)>, String> {
    let mut found = Vec::new();
    let mut entries = unit.entries();
    while let Some((_, die)) = entries.next_dfs().map_err(string)? {
        if die.tag() != gimli::DW_TAG_subprogram {
            continue;
        }
        if die
            .attr_value(gimli::DW_AT_low_pc)
            .map_err(string)?
            .is_none()
        {
            continue;
        }
        let mut name = self_linkage_name(dwarf, unit, die)?;
        if name.is_none()
            && let Some(AttributeValue::UnitRef(specification)) =
                die.attr_value(gimli::DW_AT_specification).map_err(string)?
        {
            let declaration = unit.entry(specification).map_err(string)?;
            name = self_linkage_name(dwarf, unit, &declaration)?;
        }
        if let Some(name) = name {
            found.push((name, die.offset()));
        }
    }
    Ok(found)
}

/// The linkage name a die carries itself, if any.
fn self_linkage_name(
    dwarf: &Sections,
    unit: &Unit<SectionReader>,
    die: &DebuggingInformationEntry<SectionReader, usize>,
) -> Result<Option<String>, String> {
    let Some(value) = die.attr_value(gimli::DW_AT_linkage_name).map_err(string)? else {
        return Ok(None);
    };
    dwarf
        .attr_string(unit, value)
        .map_err(string)
        .and_then(|name| {
            name.to_string_lossy()
                .map(|s| s.into_owned())
                .map_err(string)
        })
        .map(Some)
}

/// The parent of every die of one unit, by offset.
fn unit_parents(unit: &Unit<SectionReader>) -> Result<HashMap<usize, usize>, String> {
    let mut parents = HashMap::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut depth = 0usize;
    let mut entries = unit.entries();
    while let Some((delta, die)) = entries.next_dfs().map_err(string)? {
        // The depths the cursor answers are deltas from the previous die,
        // so the running depth follows them and the stack mirrors it.
        depth = depth.checked_add_signed(delta).unwrap_or(0);
        stack.truncate(depth);
        if let Some(&parent) = stack.last() {
            parents.insert(die.offset().0, parent);
        }
        stack.push(die.offset().0);
    }
    Ok(parents)
}

/// The element count of the array type at `offset`, or 0 when it is not
/// recorded.
fn array_count(unit: &Unit<SectionReader>, offset: UnitOffset) -> Result<u64, String> {
    let mut children = unit.entries_at_offset(offset).map_err(string)?;
    // The cursor's first die is the array itself, and its subrange is the
    // child after it.
    if children.next_dfs().map_err(string)?.is_none() {
        return Ok(0);
    }
    // The depths the cursor answers are deltas from the previous die, so
    // the array's children are the entries until the depth is 0 again.
    let mut depth = 0usize;
    while let Some((delta, child)) = children.next_dfs().map_err(string)? {
        let next = depth as isize + delta;
        if next <= 0 {
            break;
        }
        depth = next as usize;
        if child.tag() != gimli::DW_TAG_subrange_type {
            continue;
        }
        if let Some(AttributeValue::Udata(count)) =
            child.attr_value(gimli::DW_AT_count).map_err(string)?
        {
            return Ok(count);
        }
        if let Some(AttributeValue::Udata(upper)) =
            child.attr_value(gimli::DW_AT_upper_bound).map_err(string)?
        {
            return Ok(upper + 1);
        }
    }
    Ok(0)
}
