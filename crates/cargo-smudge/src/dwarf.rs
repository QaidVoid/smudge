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

/// A die's place: its unit's `.debug_info` start and its offset.
type DieKey = (usize, usize);

/// The most nested type levels one signature's walk may answer: a
/// deeper chain refuses the signature instead of spending the worker's
/// stack to its last page.
const WALK_DEPTH_LIMIT: usize = 2000;

/// The mark a walk leaves where an edge returns into the component it
/// already spells, followed by the name of the end the edge reaches,
/// which no string a die carries spells on its own: a shape that holds
/// one names the cycle it cut and the type it reached.
const CUT: char = '\u{0}';

/// The error a walk answers when a shared component's named members
/// share a qualified name: two types the spells cannot tell apart, so
/// an edge that retargets between them would keep every answer and a
/// replacement compiled for the other member's layout would map. The
/// signature readers answer `None` for such a function, which leaves
/// it out of the table, instead of failing the whole patch.
pub(crate) const AMBIGUOUS: &str = "the component's members share the name ";

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
    /// The digest and the cut-freedom of every type the walk has
    /// answered, by unit start and unit offset. An answer names the
    /// type's own subgraph and the cycles its strong component runs,
    /// so it reads the same whichever query reaches the type, and one
    /// answer serves every edge.
    answers: HashMap<DieKey, (String, bool)>,
    /// The strong component each type the component pass has reached
    /// belongs to, by unit start and unit offset. A cycle runs inside
    /// one component, so an edge whose two ends share the id is the
    /// edge the walk cuts, whatever spell reached it.
    component: HashMap<DieKey, usize>,
    /// The types of each strong component with more than one type, in
    /// key order: the component's digest spells all of them, so a
    /// change to any member moves every answer the component serves.
    members: HashMap<usize, Vec<DieKey>>,
    /// The components whose named members share a qualified name: two
    /// such members are two types the spells cannot tell apart, so an
    /// edge that retargets between them would keep every answer; the
    /// signatures that spell such a component refuse instead. The
    /// value is the shared name.
    ambiguous: HashMap<usize, Option<String>>,
    /// The fixed-size token of every unnamed member of a shared
    /// component the walk has spelled: the digest of its expansion.
    /// An unnamed wrapper reached by several paths would otherwise
    /// expand once per path, and a graph of shared wrappers branches,
    /// so the expansion would grow by a power of the sharing.
    wrappers: HashMap<DieKey, String>,
    /// The next discovery order the component pass assigns.
    next_order: usize,
    /// The next id the component pass labels a strong component with.
    next_component: usize,
    /// The parent of every die, by unit and unit offset.
    parents: HashMap<usize, HashMap<usize, usize>>,
    /// The namespace-qualified name of every die a description named,
    /// by unit and unit offset.
    qualified: HashMap<DieKey, String>,
    /// The closures each function defines, by the function's path.
    closures: ClosureIndices,
    /// The static variables the DWARF describes, by linkage name.
    variables: HashMap<String, (usize, UnitOffset)>,
}

/// The closure numbers each function defines, by the function's qualified
/// path.
pub(crate) type ClosureIndices =
    std::collections::BTreeMap<String, std::collections::BTreeSet<u32>>;

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
        let mut closures = ClosureIndices::new();
        let mut variables = HashMap::new();
        let mut headers = dwarf.units();
        while let Some(header) = headers.next().map_err(|e| e.to_string())? {
            let global = header
                .offset()
                .as_debug_info_offset()
                .map(|offset| offset.0)
                .ok_or_else(|| String::from("a unit in .debug_types is not supported"))?;
            let unit = dwarf.unit(header).map_err(|e| e.to_string())?;
            let index = units.len();
            let walked = walk_unit(&dwarf, &unit)?;
            for (address, offset) in walked.addressed {
                offsets.entry(index).or_default().insert(address, offset);
            }
            for (name, offset) in walked.named {
                names.insert(name, (index, offset));
            }
            for (parent, number) in walked.closures {
                closures.entry(parent).or_default().insert(number);
            }
            for (name, offset) in walked.variables {
                variables.insert(name, (index, offset));
            }
            units.push((global, unit));
        }
        let mut entries = HashMap::new();
        for (index, per_unit) in &offsets {
            for address in per_unit.keys() {
                entries.insert(*address, *index);
            }
        }
        let signatures = Self {
            dwarf,
            units,
            entries,
            offsets,
            names,
            answers: HashMap::new(),
            component: HashMap::new(),
            members: HashMap::new(),
            ambiguous: HashMap::new(),
            wrappers: HashMap::new(),
            next_order: 0,
            next_component: 0,
            parents: HashMap::new(),
            qualified: HashMap::new(),
            closures,
            variables,
        };
        Ok(signatures)
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
        match self.signature_at(index, offset) {
            Err(error) if error.starts_with(AMBIGUOUS) => Ok(None),
            other => other,
        }
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
        match self.signature_at(index, offset) {
            Err(error) if error.starts_with(AMBIGUOUS) => Ok(None),
            other => other,
        }
    }

    /// The closures each function defines, async blocks and coroutines
    /// among them, by the function's path, read from the environment types
    /// the DWARF names `{closure_env#N}`, `{async_block_env#N}` and the like
    /// under it. The compiler numbers them by their order in the function,
    /// all kinds in one numbering, so one inserted or removed renumbers the
    /// ones after it, which the numbers here show.
    pub(crate) fn closures(&self) -> &ClosureIndices {
        &self.closures
    }

    /// The layout digest of the type of the static whose linkage name is
    /// `name`, in the same spelling a signature gives a parameter's type,
    /// or none when the DWARF does not describe the static.
    ///
    /// # Errors
    ///
    /// Returns why the type could not be serialized.
    pub(crate) fn static_type(&mut self, name: &str) -> Result<Option<String>, String> {
        let Some(&(index, offset)) = self.variables.get(name) else {
            return Ok(None);
        };
        let Some((_, unit)) = self.units.get(index) else {
            return Ok(None);
        };
        let die = unit.entry(offset).map_err(string)?;
        let Some(value) = die.attr_value(gimli::DW_AT_type).map_err(string)? else {
            return Ok(None);
        };
        let mut parser = self.parser();
        match parser.describe(index, Some(value)) {
            Err(error) if error.starts_with(AMBIGUOUS) => Ok(None),
            other => other.map(Some),
        }
    }

    /// The source name of the static whose linkage name is `name`, such as
    /// `CONFIG`, when the DWARF describes it.
    pub(crate) fn static_name(&self, name: &str) -> Option<String> {
        let &(index, offset) = self.variables.get(name)?;
        let (_, unit) = self.units.get(index)?;
        let die = unit.entry(offset).ok()?;
        die_name(&self.dwarf, unit, &die).ok().flatten()
    }

    /// The type the first type parameter of the function at `address`
    /// names, which is the state type for the state probe the app
    /// announces, or none when the DWARF describes no type parameter there.
    ///
    /// # Errors
    ///
    /// Returns why the type could not be serialized.
    pub(crate) fn first_type_parameter(
        &mut self,
        address: u64,
    ) -> Result<Option<Described>, String> {
        let Some(&index) = self.entries.get(&address) else {
            return Ok(None);
        };
        let offset = self.offsets[&index][&address];
        self.parser().first_type_parameter(index, offset)
    }

    /// The type the first type parameter of the function whose mangled
    /// name is `name` names, as [`Signatures::first_type_parameter`]
    /// answers it.
    ///
    /// # Errors
    ///
    /// Returns why the type could not be serialized.
    pub(crate) fn first_type_parameter_by_name(
        &mut self,
        name: &str,
    ) -> Result<Option<Described>, String> {
        let Some(&(index, offset)) = self.names.get(name) else {
            return Ok(None);
        };
        self.parser().first_type_parameter(index, offset)
    }

    /// Whether the function whose mangled name is `name` takes a type
    /// parameter, which tells a generic probe from a function folded onto
    /// its address.
    ///
    /// # Errors
    ///
    /// Returns why the function could not be read.
    pub(crate) fn takes_type_parameter(&mut self, name: &str) -> Result<bool, String> {
        let Some(&(index, offset)) = self.names.get(name) else {
            return Ok(false);
        };
        let mut parser = self.parser();
        let Some(unit) = parser.unit(index) else {
            return Err(String::from("the subprogram's unit is missing"));
        };
        Ok(!parser.parameters(unit, offset)?.1.is_empty())
    }

    /// The mangled names of every instance of the generic function whose
    /// instance has the mangled name `name`, that one included: the
    /// functions of the same path whose names differ only in their type
    /// arguments. Every name counts, so two instances a folding link gave
    /// one address are both found.
    ///
    /// # Errors
    ///
    /// Returns why a function's name could not be read.
    pub(crate) fn instances_of_name(&mut self, name: &str) -> Result<Vec<String>, String> {
        let Some(&(index, offset)) = self.names.get(name) else {
            return Ok(Vec::new());
        };
        let Some(wanted) = self.parser().generic_path(index, offset)? else {
            return Ok(Vec::new());
        };
        let base = wanted.rsplit("::").next().unwrap_or(&wanted).to_owned();
        let candidates: Vec<(String, usize, UnitOffset)> = self
            .names
            .iter()
            .map(|(name, (index, offset))| (name.clone(), *index, *offset))
            .collect();
        let mut parser = self.parser();
        let mut found = Vec::new();
        for (name, index, offset) in candidates {
            // The plain name decides before the path is spelled, which
            // spares spelling the path of every function the file holds.
            let Some(plain) = parser.subprogram_name(index, offset)? else {
                continue;
            };
            if plain.split('<').next() != Some(base.as_str()) {
                continue;
            }
            if parser.generic_path(index, offset)?.as_deref() == Some(wanted.as_str()) {
                found.push(name);
            }
        }
        found.sort_unstable();
        Ok(found)
    }

    /// The crate root of the unit that holds the function at `address`,
    /// as [`Signatures::crate_roots`] spells it.
    ///
    /// # Errors
    ///
    /// Returns why the unit could not be read.
    pub(crate) fn crate_root_of(&self, address: u64) -> Result<Option<String>, String> {
        let Some(&index) = self.entries.get(&address) else {
            return Ok(None);
        };
        crate_root(&self.dwarf, &self.units[index].1)
    }

    /// The crate root of the unit that holds the function whose mangled name
    /// is `name`, as [`Signatures::crate_roots`] spells it.
    ///
    /// # Errors
    ///
    /// Returns why the unit could not be read.
    pub(crate) fn crate_root_of_name(&self, name: &str) -> Result<Option<String>, String> {
        let Some(&(index, _)) = self.names.get(name) else {
            return Ok(None);
        };
        crate_root(&self.dwarf, &self.units[index].1)
    }

    /// The crate roots this file's units were compiled from: each unit's
    /// compilation directory joined with the source of its crate root,
    /// which the compiler names its units after. Two builds of one crate
    /// from the same sources spell the same root. A build that remaps its
    /// compilation directory can give two crates one root, which only makes
    /// the state gate refuse where it need not.
    ///
    /// # Errors
    ///
    /// Returns why a unit could not be read.
    pub(crate) fn crate_roots(&self) -> Result<std::collections::BTreeSet<String>, String> {
        let mut roots = std::collections::BTreeSet::new();
        for (_, unit) in &self.units {
            if let Some(root) = crate_root(&self.dwarf, unit)? {
                roots.insert(root);
            }
        }
        Ok(roots)
    }

    /// The own shape of every named struct, union and enum `described`
    /// reaches, itself included, by qualified name: its size, its
    /// alignment and its members, each member's type given by name, so a
    /// type's shape changes only when its own definition does. Two types
    /// of one name join their distinct shapes. The shapes only name the types a
    /// layout change went through; the digest decides whether it changed.
    ///
    /// # Errors
    ///
    /// Returns why a type could not be read.
    pub(crate) fn shapes(
        &mut self,
        described: &Described,
    ) -> Result<std::collections::BTreeMap<String, String>, String> {
        let (index, offset) = described.at;
        self.parser().shapes(index, offset)
    }

    /// A parser over this file's DWARF that fills its caches.
    fn parser(&mut self) -> Parser<'_> {
        Parser {
            dwarf: &self.dwarf,
            units: &self.units,
            answers: &mut self.answers,
            component: &mut self.component,
            members: &mut self.members,
            ambiguous: &mut self.ambiguous,
            wrappers: &mut self.wrappers,
            next_order: &mut self.next_order,
            next_component: &mut self.next_component,
            parents: &mut self.parents,
            qualified: &mut self.qualified,
        }
    }

    /// The signature of the subprogram at `offset` of unit `index`.
    fn signature_at(&mut self, index: usize, offset: UnitOffset) -> Result<Option<String>, String> {
        let mut parser = self.parser();
        parser.signature(index, offset)
    }
}

/// A type one file's DWARF describes.
pub(crate) struct Described {
    /// The unit and the offset its description dies at.
    at: (usize, UnitOffset),
    /// Its namespace-qualified name.
    pub(crate) name: String,
    /// The digest of its full reachable layout, which changes when the
    /// type or anything it reaches changes its layout.
    pub(crate) digest: String,
}

/// The most nested pointers, arrays and modifiers a member's type label
/// spells before it stops.
const LABEL_DEPTH_LIMIT: usize = 64;

/// The type attributes of a list of dies, as a subprogram's parameters
/// name them.
type TypeAttributes = Vec<Option<AttributeValue<SectionReader>>>;

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
    /// The digest and the cut-freedom of the types the walks answered,
    /// by unit start and unit offset.
    answers: &'a mut HashMap<DieKey, (String, bool)>,
    /// The strong component of each type the component pass labeled,
    /// by unit start and unit offset.
    component: &'a mut HashMap<DieKey, usize>,
    /// The types of each strong component with more than one type, in
    /// key order.
    members: &'a mut HashMap<usize, Vec<DieKey>>,
    /// The name two named members of each component share, if any.
    ambiguous: &'a mut HashMap<usize, Option<String>>,
    /// The fixed-size token of every unnamed member of a shared
    /// component the walk has spelled.
    wrappers: &'a mut HashMap<DieKey, String>,
    /// The next discovery order the component pass assigns.
    next_order: &'a mut usize,
    /// The next id the component pass labels a strong component with.
    next_component: &'a mut usize,
    /// The parent of every die, by unit start and unit offset.
    parents: &'a mut HashMap<usize, HashMap<usize, usize>>,
    /// The namespace-qualified name of every die a description named,
    /// by unit and unit offset.
    qualified: &'a mut HashMap<DieKey, String>,
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
        let mut specification = None;
        if return_type.is_none()
            && let Some(AttributeValue::UnitRef(specified)) =
                die.attr_value(gimli::DW_AT_specification).map_err(string)?
        {
            specification = Some(specified);
            let declaration = unit.entry(specified).map_err(string)?;
            return_type = declaration.attr_value(gimli::DW_AT_type).map_err(string)?;
        }
        let convention = self.convention_of(unit, &die, specification)?;
        let (parameters, mut templates) = self.parameters(unit, offset)?;
        if templates.is_empty()
            && let Some(specified) = specification
        {
            templates = self.parameters(unit, specified)?.1;
        }
        let mut parts = Vec::with_capacity(parameters.len() + templates.len() + 2);
        parts.push(format!(
            "cc={}",
            convention.map_or(String::new(), |value| value.to_string())
        ));
        parts.push(self.describe(index, return_type)?);
        for parameter in parameters {
            parts.push(self.describe(index, parameter)?);
        }
        // A type parameter can shape what the function reads without
        // appearing among its parameters, as a closure type does in a
        // function handed the closure as an erased pointer, so its layout
        // joins the signature too.
        for template in templates {
            parts.push(format!("T:{}", self.describe(index, template)?));
        }
        Ok(Some(parts.join("|")))
    }

    /// The calling convention a subprogram die answers to, which the
    /// specification carries when the definition leaves it out. Two
    /// functions of different conventions are different functions to a
    /// caller, whatever their signatures say, so the value joins the
    /// fingerprint and its absence does too, since the conventions the
    /// compiler leaves unrecorded all answer to the C family's shape.
    fn convention_of(
        &self,
        unit: &Unit<SectionReader>,
        die: &DebuggingInformationEntry<'_, '_, SectionReader>,
        specification: Option<UnitOffset>,
    ) -> Result<Option<u64>, String> {
        let mut found = die
            .attr_value(gimli::DW_AT_calling_convention)
            .map_err(string)?
            .as_ref()
            .and_then(unsigned_of);
        if found.is_none()
            && let Some(offset) = specification
        {
            let declaration = unit.entry(offset).map_err(string)?;
            found = declaration
                .attr_value(gimli::DW_AT_calling_convention)
                .map_err(string)?
                .as_ref()
                .and_then(unsigned_of);
        }
        Ok(found)
    }

    /// The name the subprogram at `offset` of unit `index` carries, on its
    /// own die or on the declaration its specification names.
    fn subprogram_name(&self, index: usize, offset: UnitOffset) -> Result<Option<String>, String> {
        let Some(unit) = self.unit(index) else {
            return Err(String::from("the subprogram's unit is missing"));
        };
        let die = unit.entry(offset).map_err(string)?;
        self.name_of(unit, &die)
    }

    /// The path of the generic function the subprogram at `offset` of unit
    /// `index` is an instance of: the namespaces and types it sits under,
    /// and its name without type arguments.
    fn generic_path(&mut self, index: usize, offset: UnitOffset) -> Result<Option<String>, String> {
        let Some(name) = self.subprogram_name(index, offset)? else {
            return Ok(None);
        };
        let Some(unit) = self.unit(index) else {
            return Err(String::from("the subprogram's unit is missing"));
        };
        // A definition that describes itself through a specification sits
        // where its declaration does.
        let die = unit.entry(offset).map_err(string)?;
        let place = match die.attr_value(gimli::DW_AT_specification).map_err(string)? {
            Some(AttributeValue::UnitRef(specified)) => specified,
            _ => offset,
        };
        let base = name.split('<').next().unwrap_or(&name).to_owned();
        let scope = self.qualified_name(index, place)?;
        Ok(Some(if scope == "<unnamed>" {
            base
        } else {
            format!("{scope}::{base}")
        }))
    }

    /// The type the first type parameter of the subprogram at `offset`
    /// of unit `index` names, which the specification carries when the
    /// definition leaves it out.
    fn first_type_parameter(
        &mut self,
        index: usize,
        offset: UnitOffset,
    ) -> Result<Option<Described>, String> {
        let Some(unit) = self.unit(index) else {
            return Err(String::from("the subprogram's unit is missing"));
        };
        let mut templates = self.parameters(unit, offset)?.1;
        if templates.is_empty()
            && let Some(AttributeValue::UnitRef(specified)) = unit
                .entry(offset)
                .map_err(string)?
                .attr_value(gimli::DW_AT_specification)
                .map_err(string)?
        {
            templates = self.parameters(unit, specified)?.1;
        }
        let Some(Some(value)) = templates.into_iter().next() else {
            return Ok(None);
        };
        let Some(at) = self.type_ref(index, value.clone()) else {
            return Ok(None);
        };
        let mut name = self.qualified_name(at.0, at.1)?;
        if name == "<unnamed>" {
            name = self.label_of(index, Some(value), 0)?;
        }
        let digest = self.layout_digest(at.0, at.1)?;
        Ok(Some(Described { at, name, digest }))
    }

    /// The own shape of every named aggregate the type at `offset` of
    /// unit `index` reaches, by qualified name, as
    /// [`Signatures::shapes`] answers it. The walk keeps its own queue,
    /// so a deep graph costs no stack.
    fn shapes(
        &mut self,
        index: usize,
        offset: UnitOffset,
    ) -> Result<std::collections::BTreeMap<String, String>, String> {
        let mut shapes: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
            std::collections::BTreeMap::new();
        let mut seen = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::from([(index, offset)]);
        while let Some((index, offset)) = queue.pop_front() {
            let Some(key) = self.key_of(index, offset) else {
                return Err(String::from("the type's unit is missing"));
            };
            if !seen.insert(key) {
                continue;
            }
            let Some(unit) = self.unit(index) else {
                return Err(String::from("the type's unit is missing"));
            };
            let tag = unit.entry(offset).map_err(string)?.tag();
            if matches!(
                tag,
                gimli::DW_TAG_structure_type
                    | gimli::DW_TAG_union_type
                    | gimli::DW_TAG_enumeration_type
            ) {
                let name = self.qualified_name(index, offset)?;
                if name != "<unnamed>" {
                    let shape = self.own_shape(index, unit, offset)?;
                    shapes.entry(name).or_default().insert(shape);
                }
            }
            queue.extend(self.edges_of(index, offset)?);
        }
        // The shapes of one name join in sorted order, so the order the
        // walk met them in leaves the answer alone.
        Ok(shapes
            .into_iter()
            .map(|(name, shapes)| (name, shapes.into_iter().collect::<Vec<_>>().join(" | ")))
            .collect())
    }

    /// The size, the alignment and the members of one aggregate, each
    /// member's type given by its label.
    fn own_shape(
        &mut self,
        index: usize,
        unit: &Unit<SectionReader>,
        offset: UnitOffset,
    ) -> Result<String, String> {
        let die = unit.entry(offset).map_err(string)?;
        let mut parts = Vec::new();
        for (label, attribute) in [
            ("size", gimli::DW_AT_byte_size),
            ("align", gimli::DW_AT_alignment),
        ] {
            if let Some(value) = die
                .attr_value(attribute)
                .map_err(string)?
                .as_ref()
                .and_then(unsigned_of)
            {
                parts.push(format!("{label}={value}"));
            }
        }
        let members = self.own_members(index, unit, offset)?;
        if !members.is_empty() {
            parts.push(format!("m:{}", members.join(",")));
        }
        Ok(parts.join(";"))
    }

    /// The members one aggregate lays out, the way the layout walk
    /// descends into them, each member's type given by its label.
    fn own_members(
        &mut self,
        index: usize,
        unit: &Unit<SectionReader>,
        offset: UnitOffset,
    ) -> Result<Vec<String>, String> {
        let mut found = Vec::new();
        let mut children = unit.entries_at_offset(offset).map_err(string)?;
        if children.next_dfs().map_err(string)?.is_none() {
            return Ok(found);
        }
        let mut cursor = 0usize;
        while let Some((delta, child)) = children.next_dfs().map_err(string)? {
            let next = cursor as isize + delta;
            if next <= 0 {
                break;
            }
            cursor = next as usize;
            // Only the direct children: a variant part's variants and a
            // variant's members are read through their own walk.
            if cursor != 1 {
                continue;
            }
            match child.tag() {
                gimli::DW_TAG_member => {
                    let name = self.name_of(unit, child)?.unwrap_or_default();
                    let location = child
                        .attr_value(gimli::DW_AT_data_member_location)
                        .map_err(string)?
                        .as_ref()
                        .and_then(unsigned_of)
                        .map_or(String::from("?"), |value| value.to_string());
                    let label = self.label_of(
                        index,
                        child.attr_value(gimli::DW_AT_type).map_err(string)?,
                        0,
                    )?;
                    found.push(format!("{name}@{location}:{label}"));
                }
                gimli::DW_TAG_template_type_parameter => {
                    let label = self.label_of(
                        index,
                        child.attr_value(gimli::DW_AT_type).map_err(string)?,
                        0,
                    )?;
                    found.push(format!("T:{label}"));
                }
                gimli::DW_TAG_variant => {
                    let tag = match child.attr_value(gimli::DW_AT_discr_value).map_err(string)? {
                        Some(value) => number_of(&value).unwrap_or_else(|| String::from("?")),
                        None => String::from("default"),
                    };
                    let members = self.own_members(index, unit, child.offset())?;
                    found.push(format!("variant={tag}:{}", members.join(",")));
                }
                gimli::DW_TAG_variant_part => {
                    found.extend(self.own_members(index, unit, child.offset())?);
                }
                gimli::DW_TAG_enumerator => {
                    let name = self.name_of(unit, child)?.unwrap_or_default();
                    let value = child
                        .attr_value(gimli::DW_AT_const_value)
                        .map_err(string)?
                        .as_ref()
                        .and_then(number_of);
                    found.push(format!(
                        "{name}={}",
                        value.unwrap_or_else(|| String::from("?"))
                    ));
                }
                _ => {}
            }
        }
        Ok(found)
    }

    /// The name a member's type goes by in a shape: its qualified name,
    /// the name its die records, or for an unnamed pointer, array or
    /// modifier the label of what it carries.
    fn label_of(
        &mut self,
        index: usize,
        attribute: Option<AttributeValue<SectionReader>>,
        depth: usize,
    ) -> Result<String, String> {
        let Some(value) = attribute else {
            return Ok(String::from("()"));
        };
        let Some((index, offset)) = self.type_ref(index, value) else {
            return Ok(String::from("<unresolved>"));
        };
        let name = self.qualified_name(index, offset)?;
        if name != "<unnamed>" {
            return Ok(name);
        }
        let Some(unit) = self.unit(index) else {
            return Err(String::from("the type's unit is missing"));
        };
        let die = unit.entry(offset).map_err(string)?;
        if let Some(name) = self.name_of(unit, &die)? {
            return Ok(name);
        }
        if depth >= LABEL_DEPTH_LIMIT {
            return Ok(String::from("<deep>"));
        }
        let carried = die.attr_value(gimli::DW_AT_type).map_err(string)?;
        let prefix = match die.tag() {
            gimli::DW_TAG_array_type => {
                let element = self.label_of(index, carried, depth + 1)?;
                let count =
                    array_count(unit, offset)?.map_or(String::from("?"), |count| count.to_string());
                return Ok(format!("[{element}; {count}]"));
            }
            gimli::DW_TAG_pointer_type => "*",
            gimli::DW_TAG_reference_type => "&",
            gimli::DW_TAG_rvalue_reference_type => "&&",
            gimli::DW_TAG_const_type => "const ",
            gimli::DW_TAG_volatile_type => "volatile ",
            gimli::DW_TAG_restrict_type => "restrict ",
            gimli::DW_TAG_typedef => "alias ",
            _ => return Ok(String::from("<unnamed>")),
        };
        Ok(format!(
            "{prefix}{}",
            self.label_of(index, carried, depth + 1)?
        ))
    }

    /// The type attributes of a die's formal parameters and of its template
    /// type parameters.
    fn parameters(
        &mut self,
        unit: &Unit<SectionReader>,
        offset: UnitOffset,
    ) -> Result<(TypeAttributes, TypeAttributes), String> {
        let mut found = Vec::new();
        let mut templates = Vec::new();
        let mut children = unit.entries_at_offset(offset).map_err(string)?;
        // The cursor's first die is the one itself, and its parameters are
        // the children after it.
        if children.next_dfs().map_err(string)?.is_none() {
            return Ok((found, templates));
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
            } else if depth == 1 && child.tag() == gimli::DW_TAG_template_type_parameter {
                templates.push(child.attr_value(gimli::DW_AT_type).map_err(string)?);
            }
        }
        Ok((found, templates))
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
    /// `index`: its qualified name with the digest of its full
    /// reachable layout.
    ///
    /// The description is a function of the type's own subgraph alone:
    /// it never depends on which function was examined first, since
    /// the two builds being paired can carry different sets of
    /// functions. The digest comes from a walk whose every cycle cut
    /// happens inside the type's own walk, so a recursive type answers
    /// with its whole shape and not its name alone: a field that moves
    /// behind a recursive pointer changes the answer.
    fn type_desc(&mut self, index: usize, offset: UnitOffset) -> Result<String, String> {
        let name = self.qualified_name(index, offset)?;
        let digest = self.layout_digest(index, offset)?;
        Ok(format!("{name}#{digest}"))
    }

    /// The digest of the type's full reachable layout, which one walk
    /// answers per type and every query reads.
    fn layout_digest(&mut self, index: usize, offset: UnitOffset) -> Result<String, String> {
        let Some(key) = self.key_of(index, offset) else {
            return Err(String::from("the type's unit is missing"));
        };
        if let Some((digest, _)) = self.answers.get(&key) {
            return Ok(digest.clone());
        }
        let (digest, _) = self.walk_digest(index, offset, None, 0)?;
        Ok(digest)
    }

    /// The digest of one type's own walk, and whether the answer cut a
    /// cycle anywhere beneath it. A cycle runs inside one strong
    /// component, so a type of a shared component spells its whole
    /// component: every member's own shape with each edge inside the
    /// component cut, and every edge out of it walked. A change to any
    /// member of the component then moves the answer, whatever type
    /// the query named, and one answer serves every member.
    fn walk_digest(
        &mut self,
        index: usize,
        offset: UnitOffset,
        parent: Option<usize>,
        depth: usize,
    ) -> Result<(String, bool), String> {
        let Some(key) = self.key_of(index, offset) else {
            return Err(String::from("the type's unit is missing"));
        };
        let component = self.component_of(index, offset)?;
        let mut wrapped = false;
        if parent == Some(component) {
            let named = self.qualified_name(index, offset)?;
            if named != "<unnamed>" {
                // Two members of the component can carry one qualified
                // name, and then no token can say which of them the
                // edge reaches: the signature refuses, and the pairing
                // leaves the function out of the table.
                if let Some(name) = self.ambiguity_of(component)? {
                    return Err(format!("{AMBIGUOUS}{name}"));
                }
                return Ok((format!("{CUT}{named}"), false));
            }
            wrapped = true;
        }
        if let Some((digest, independent)) = self.answers.get(&key) {
            return Ok((digest.clone(), *independent));
        }
        if depth >= WALK_DEPTH_LIMIT {
            return Err(format!(
                "the type walk passed {WALK_DEPTH_LIMIT} levels, which \
                 the signature cannot answer"
            ));
        }
        if wrapped {
            // An unnamed die on an edge inside the component is a
            // wrapper between the two ends: it answers with its own
            // fixed-size token, the digest of its expansion, whose
            // inner edges landed on the named ends the expansion cut.
            // The token is cached per die, so a graph of shared
            // wrappers costs one expansion each and not one per path,
            // which would grow by a power of the sharing.
            if let Some(token) = self.wrappers.get(&key) {
                return Ok((token.clone(), false));
            }
            let (shape, _) = self.walk_shape(index, offset, component, depth)?;
            let token = format!("W{}", digest_of(&shape));
            self.wrappers.insert(key, token.clone());
            return Ok((token, false));
        }
        let mut shapes: Vec<(DieKey, String)> = Vec::new();
        let mut independent = true;
        for member in self.component_members(component, key) {
            let Some((member_index, member_offset)) = self.locate_of(member) else {
                return Err(String::from("the type's unit is missing"));
            };
            let (shape, child_independent) =
                self.walk_shape(member_index, member_offset, component, depth)?;
            shapes.push((member, shape));
            independent &= child_independent;
        }
        // The spell of the component names every member, so the order
        // the walk visited them in cannot decide the answer: two
        // builds that emit the same types in a different order still
        // spell one component alike.
        shapes.sort_unstable_by(|a, b| a.1.cmp(&b.1));
        let parts: Vec<String> = shapes.iter().map(|(_, shape)| shape.clone()).collect();
        let spell = parts.join(" ");
        // An edge from outside the component enters at one member of
        // it, and the member decides what the edge carries: the
        // members share the spell of the whole component, so the entry
        // names itself, or an outside field that retargets between two
        // members of an unchanged component would keep the answer. A
        // named member names itself; an unnamed one (a pointer die
        // rustc shares between the fields that point at one type) is
        // its own token, the digest of its expansion, whose end names
        // the member it lands on.
        let shared = self.members.contains_key(&component);
        if let Some(name) = self.ambiguity_of(component)? {
            return Err(format!("{AMBIGUOUS}{name}"));
        }
        // The spell is hashed once: each member combines its entry with
        // the fixed-size digest, so a component of n members costs
        // linear work and not n hashes of a spell of n parts.
        let spelled = digest_of(&spell);
        let mut answered: Option<String> = None;
        for (member, shape) in &shapes {
            let member_digest = if shared {
                let Some((member_index, member_offset)) = self.locate_of(*member) else {
                    return Err(String::from("the type's unit is missing"));
                };
                let named = self.qualified_name(member_index, member_offset)?;
                let entry = if named == "<unnamed>" {
                    format!("W{}", digest_of(shape))
                } else {
                    named
                };
                digest_of(&format!("{entry} {spelled}"))
            } else {
                spelled.clone()
            };
            self.answers
                .insert(*member, (member_digest.clone(), independent));
            if *member == key {
                answered = Some(member_digest);
            }
        }
        let digest = answered.unwrap_or(spelled);
        Ok((digest, independent))
    }

    /// The name two named members of the component share, if any: the
    /// spells name edges and entries by their ends, and two ends of
    /// one name cannot be told apart, so a retarget between them would
    /// keep every answer the component serves. The answer is computed
    /// once per component and cached.
    fn ambiguity_of(&mut self, component: usize) -> Result<Option<String>, String> {
        if let Some(found) = self.ambiguous.get(&component) {
            return Ok(found.clone());
        }
        let Some(members) = self.members.get(&component).cloned() else {
            return Ok(None);
        };
        let mut names = Vec::with_capacity(members.len());
        for member in &members {
            let Some((member_index, member_offset)) = self.locate_of(*member) else {
                return Err(String::from("the type's unit is missing"));
            };
            let name = self.qualified_name(member_index, member_offset)?;
            if name != "<unnamed>" {
                names.push(name);
            }
        }
        names.sort();
        let found = names
            .windows(2)
            .find(|pair| pair[0] == pair[1])
            .map(|pair| pair[0].clone());
        self.ambiguous.insert(component, found.clone());
        Ok(found)
    }

    /// The types one component spells, in key order: a component with
    /// several types spells all of them, so the query's own type need
    /// not be first.
    fn component_members(&self, component: usize, key: DieKey) -> Vec<DieKey> {
        match self.members.get(&component) {
            Some(members) => members.clone(),
            None => vec![key],
        }
    }

    /// The unit and offset a stored key names.
    fn locate_of(&self, key: DieKey) -> Option<(usize, UnitOffset)> {
        self.units
            .iter()
            .position(|(start, _)| *start == key.0)
            .map(|index| (index, UnitOffset(key.1)))
    }

    /// The unit start and offset `index` and `offset` name, the key
    /// every per-type answer is stored under.
    fn key_of(&self, index: usize, offset: UnitOffset) -> Option<DieKey> {
        self.units.get(index).map(|(start, _)| (*start, offset.0))
    }

    /// The strong component the type at `offset` of unit `index`
    /// belongs to, labeled through one iterative Tarjan pass over the
    /// type graph its edges reach, cached per type. The pass carries
    /// its own stacks, so a graph however deep or dense labels without
    /// recursion and in time linear in the types it reaches.
    fn component_of(&mut self, index: usize, offset: UnitOffset) -> Result<usize, String> {
        let Some(root) = self.key_of(index, offset) else {
            return Err(String::from("the type's unit is missing"));
        };
        if let Some(id) = self.component.get(&root) {
            return Ok(*id);
        }
        // One frame per discovered type: the type, its discovery order,
        // its low link, the edges it carries and the edge the pass
        // One frame per discovered type: the type, the edges it
        // carries and the edge the pass visits next; the discovery
        // orders and low links live beside them. A type still on the
        // branch owes its component a label; a type whose label stands
        // belongs to a finished component this pass can never join.
        struct Frame {
            node: DieKey,
            edges: Vec<(usize, UnitOffset)>,
            cursor: usize,
        }
        let mut frames: Vec<Frame> = Vec::new();
        let mut order: HashMap<DieKey, usize> = HashMap::new();
        let mut low: HashMap<DieKey, usize> = HashMap::new();
        let mut branch: Vec<DieKey> = Vec::new();
        let root_edges = self.edges_of(index, offset)?;
        order.insert(root, *self.next_order);
        low.insert(root, *self.next_order);
        *self.next_order += 1;
        branch.push(root);
        frames.push(Frame {
            node: root,
            edges: root_edges,
            cursor: 0,
        });
        while let Some(top) = frames.last_mut() {
            if top.cursor < top.edges.len() {
                let (child_index, child_offset) = top.edges[top.cursor];
                top.cursor += 1;
                let Some(child) = self.key_of(child_index, child_offset) else {
                    return Err(String::from("the type's unit is missing"));
                };
                if self.component.contains_key(&child) {
                    continue;
                }
                if let Some(&child_order) = order.get(&child) {
                    // A type the pass discovered but has not left is on
                    // the branch, and the edge back to it holds the two
                    // ends' components together.
                    let node_low = low[&top.node];
                    if child_order < node_low {
                        low.insert(top.node, child_order);
                    }
                    continue;
                }
                let edges = self.edges_of(child_index, child_offset)?;
                order.insert(child, *self.next_order);
                low.insert(child, *self.next_order);
                *self.next_order += 1;
                branch.push(child);
                frames.push(Frame {
                    node: child,
                    edges,
                    cursor: 0,
                });
            } else {
                let frame = frames.pop().expect("the pass stood on a frame");
                if let Some(parent) = frames.last_mut() {
                    let node_low = low[&frame.node];
                    if node_low < low[&parent.node] {
                        low.insert(parent.node, node_low);
                    }
                }
                if low[&frame.node] == order[&frame.node] {
                    let id = *self.next_component;
                    *self.next_component += 1;
                    let mut group = Vec::new();
                    while let Some(member) = branch.pop() {
                        self.component.insert(member, id);
                        group.push(member);
                        if member == frame.node {
                            break;
                        }
                    }
                    if group.len() > 1 {
                        group.sort_unstable();
                        self.members.insert(id, group);
                    }
                }
            }
        }
        Ok(self.component[&root])
    }

    /// The type edges one type DIE carries, which the component pass
    /// follows: the same attributes the digest walk reads, collected
    /// without building any strings.
    fn edges_of(
        &mut self,
        index: usize,
        offset: UnitOffset,
    ) -> Result<Vec<(usize, UnitOffset)>, String> {
        let mut found = Vec::new();
        let Some(unit) = self.unit(index) else {
            return Err(String::from("the type's unit is missing"));
        };
        let die = unit.entry(offset).map_err(string)?;
        match die.tag() {
            gimli::DW_TAG_structure_type
            | gimli::DW_TAG_union_type
            | gimli::DW_TAG_enumeration_type => {
                self.member_edges(index, unit, offset, &mut found)?;
            }
            gimli::DW_TAG_pointer_type
            | gimli::DW_TAG_reference_type
            | gimli::DW_TAG_rvalue_reference_type
            | gimli::DW_TAG_const_type
            | gimli::DW_TAG_volatile_type
            | gimli::DW_TAG_restrict_type
            | gimli::DW_TAG_typedef
            | gimli::DW_TAG_array_type => {
                if let Some(value) = die.attr_value(gimli::DW_AT_type).map_err(string)?
                    && let Some(edge) = self.type_ref(index, value)
                {
                    found.push(edge);
                }
            }
            gimli::DW_TAG_subroutine_type => {
                if let Some(value) = die.attr_value(gimli::DW_AT_type).map_err(string)?
                    && let Some(edge) = self.type_ref(index, value)
                {
                    found.push(edge);
                }
                for parameter in self.parameters(unit, die.offset())?.0 {
                    if let Some(value) = parameter
                        && let Some(edge) = self.type_ref(index, value)
                    {
                        found.push(edge);
                    }
                }
            }
            _ => {}
        }
        Ok(found)
    }

    /// The type edges the members, variant parts, variants and template
    /// type parameters under an aggregate carry, the same descent the
    /// digest walk takes.
    fn member_edges(
        &self,
        index: usize,
        unit: &Unit<SectionReader>,
        offset: UnitOffset,
        found: &mut Vec<(usize, UnitOffset)>,
    ) -> Result<(), String> {
        let mut children = unit.entries_at_offset(offset).map_err(string)?;
        if children.next_dfs().map_err(string)?.is_none() {
            return Ok(());
        }
        let mut depth = 0usize;
        while let Some((delta, child)) = children.next_dfs().map_err(string)? {
            let next = depth as isize + delta;
            if next <= 0 {
                break;
            }
            depth = next as usize;
            match child.tag() {
                gimli::DW_TAG_member => {
                    if let Some(value) = child.attr_value(gimli::DW_AT_type).map_err(string)?
                        && let Some(edge) = self.type_ref(index, value)
                    {
                        found.push(edge);
                    }
                }
                // Only the aggregate's own type parameters, its direct
                // children: a generic method declared under it carries
                // parameters of its own, which say nothing of the layout.
                gimli::DW_TAG_template_type_parameter if depth == 1 => {
                    if let Some(value) = child.attr_value(gimli::DW_AT_type).map_err(string)?
                        && let Some(edge) = self.type_ref(index, value)
                    {
                        found.push(edge);
                    }
                }
                gimli::DW_TAG_variant | gimli::DW_TAG_variant_part => {
                    self.member_edges(index, unit, child.offset(), found)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
    /// The shape one type walk spells, which names the type and hands
    /// every type it reaches to the walk as a digest, and whether the
    /// walk cut no cycle anywhere beneath it: the whole reachable
    /// graph lands in the answer once, however the types share their
    /// subtrees.
    fn walk_shape(
        &mut self,
        index: usize,
        offset: UnitOffset,
        component: usize,
        depth: usize,
    ) -> Result<(String, bool), String> {
        let Some(unit) = self.unit(index) else {
            return Err(String::from("the type's unit is missing"));
        };
        let die = unit.entry(offset).map_err(string)?;
        let plain = |name: String| Ok((name, true));
        match die.tag() {
            gimli::DW_TAG_base_type => plain(self.qualified_name(index, offset)?),
            // A named aggregate is not its name alone: two builds that
            // spell a type the same way can lay it out differently,
            // and the size, the alignment and the members decide what
            // a caller must prepare, so they join the walk.
            gimli::DW_TAG_structure_type
            | gimli::DW_TAG_union_type
            | gimli::DW_TAG_enumeration_type => {
                let name = self.qualified_name(index, offset)?;
                let (layout, independent) =
                    self.walk_layout(index, unit, offset, component, depth)?;
                Ok((format!("{name} {layout}"), independent))
            }
            // The name a pointer die records (`&u64`, `*const u64`,
            // `Box<u64>`) tells borrowed from owned from raw, which
            // the pointee alone answers nothing about, so it joins the
            // walk whatever toolchain spells it with.
            gimli::DW_TAG_pointer_type => {
                let (carried, independent) = self.walk_attribute(
                    index,
                    die.attr_value(gimli::DW_AT_type).map_err(string)?,
                    component,
                    depth,
                )?;
                Ok((
                    format!("{} *{carried}", self.die_name(unit, &die)?),
                    independent,
                ))
            }
            gimli::DW_TAG_reference_type => {
                let (carried, independent) = self.walk_attribute(
                    index,
                    die.attr_value(gimli::DW_AT_type).map_err(string)?,
                    component,
                    depth,
                )?;
                Ok((
                    format!("{} &{carried}", self.die_name(unit, &die)?),
                    independent,
                ))
            }
            gimli::DW_TAG_rvalue_reference_type => {
                let (carried, independent) = self.walk_attribute(
                    index,
                    die.attr_value(gimli::DW_AT_type).map_err(string)?,
                    component,
                    depth,
                )?;
                Ok((
                    format!("{} &&{carried}", self.die_name(unit, &die)?),
                    independent,
                ))
            }
            gimli::DW_TAG_const_type => {
                let (carried, independent) = self.walk_attribute(
                    index,
                    die.attr_value(gimli::DW_AT_type).map_err(string)?,
                    component,
                    depth,
                )?;
                Ok((format!("const {carried}"), independent))
            }
            gimli::DW_TAG_volatile_type => {
                let (carried, independent) = self.walk_attribute(
                    index,
                    die.attr_value(gimli::DW_AT_type).map_err(string)?,
                    component,
                    depth,
                )?;
                Ok((format!("volatile {carried}"), independent))
            }
            gimli::DW_TAG_restrict_type => {
                let (carried, independent) = self.walk_attribute(
                    index,
                    die.attr_value(gimli::DW_AT_type).map_err(string)?,
                    component,
                    depth,
                )?;
                Ok((format!("restrict {carried}"), independent))
            }
            gimli::DW_TAG_typedef => {
                let name = self.name_of(unit, &die)?.unwrap_or_default();
                let (carried, independent) = self.walk_attribute(
                    index,
                    die.attr_value(gimli::DW_AT_type).map_err(string)?,
                    component,
                    depth,
                )?;
                Ok((format!("alias {name} = {carried}"), independent))
            }
            gimli::DW_TAG_array_type => {
                let (element, independent) = self.walk_attribute(
                    index,
                    die.attr_value(gimli::DW_AT_type).map_err(string)?,
                    component,
                    depth,
                )?;
                let shape = match array_count(unit, die.offset())? {
                    Some(count) => format!("[{element}; {count}]"),
                    // A length the die carries in a form the number
                    // reader cannot name stays unverifiable instead of
                    // posing as a zero-length array.
                    None => format!("[{element}; ?]"),
                };
                Ok((shape, independent))
            }
            gimli::DW_TAG_subroutine_type => {
                let convention = die
                    .attr_value(gimli::DW_AT_calling_convention)
                    .map_err(string)?
                    .as_ref()
                    .and_then(unsigned_of);
                let parameters = self.parameters(unit, die.offset())?.0;
                let mut parts = Vec::with_capacity(parameters.len() + 2);
                parts.push(format!(
                    "cc={}",
                    convention.map_or(String::new(), |value| value.to_string())
                ));
                let mut independent = true;
                // The return type rides the same attribute the other
                // edges do, so the walk answers it the same way.
                let (carried, child_independent) = self.walk_attribute(
                    index,
                    die.attr_value(gimli::DW_AT_type).map_err(string)?,
                    component,
                    depth,
                )?;
                independent &= child_independent;
                parts.push(carried);
                for parameter in parameters {
                    let (carried, child_independent) =
                        self.walk_attribute(index, parameter, component, depth)?;
                    independent &= child_independent;
                    parts.push(carried);
                }
                Ok((format!("fn({})", parts.join(", ")), independent))
            }
            _ => plain(self.qualified_name(index, offset)?),
        }
    }
    /// The name a die records, which two spells of one pointee can
    /// carry differently: a die without one answers with nothing.
    fn die_name(
        &self,
        unit: &Unit<SectionReader>,
        die: &DebuggingInformationEntry<'_, '_, SectionReader>,
    ) -> Result<String, String> {
        Ok(self.name_of(unit, die)?.unwrap_or_default())
    }

    /// The digest the type an attribute names takes in the walk, or
    /// the spell an absent or unresolvable attribute answers with,
    /// and whether the walk beneath cut no cycle.
    fn walk_attribute(
        &mut self,
        index: usize,
        attribute: Option<AttributeValue<SectionReader>>,
        component: usize,
        depth: usize,
    ) -> Result<(String, bool), String> {
        let Some(value) = attribute else {
            return Ok((String::from("()"), true));
        };
        let Some((type_index, offset)) = self.type_ref(index, value) else {
            return Ok((String::from("<unresolved>"), true));
        };
        self.walk_digest(type_index, offset, Some(component), depth + 1)
    }

    /// The size, alignment and members an aggregate dies with, which a
    /// name alone cannot answer for: two spells of one type can differ
    /// in what a caller must prepare for it. The flag answers whether
    /// the walk beneath cut no cycle.
    fn walk_layout(
        &mut self,
        index: usize,
        unit: &Unit<SectionReader>,
        offset: UnitOffset,
        component: usize,
        depth: usize,
    ) -> Result<(String, bool), String> {
        let die = unit.entry(offset).map_err(string)?;
        let mut parts = Vec::new();
        if let Some(size) = die
            .attr_value(gimli::DW_AT_byte_size)
            .map_err(string)?
            .as_ref()
            .and_then(unsigned_of)
        {
            parts.push(format!("size={size}"));
        }
        if let Some(align) = die
            .attr_value(gimli::DW_AT_alignment)
            .map_err(string)?
            .as_ref()
            .and_then(unsigned_of)
        {
            parts.push(format!("align={align}"));
        }
        let (members, independent) = self.walk_members(index, unit, offset, component, depth)?;
        if !members.is_empty() {
            parts.push(format!("m:{}", members.join(",")));
        }
        Ok((parts.join(";"), independent))
    }

    /// The members an aggregate carries, descending into the variant
    /// parts and variants an enum lays its fields under, and the
    /// enumerators a fieldless one names, with whether the walk
    /// beneath cut no cycle.
    fn walk_members(
        &mut self,
        index: usize,
        unit: &Unit<SectionReader>,
        offset: UnitOffset,
        component: usize,
        depth: usize,
    ) -> Result<(Vec<String>, bool), String> {
        let mut found = Vec::new();
        let mut independent = true;
        let mut children = unit.entries_at_offset(offset).map_err(string)?;
        // The cursor's first die is the aggregate itself, and its
        // members are the children after it.
        if children.next_dfs().map_err(string)?.is_none() {
            return Ok((found, true));
        }
        let mut cursor = 0usize;
        while let Some((delta, child)) = children.next_dfs().map_err(string)? {
            let next = cursor as isize + delta;
            if next <= 0 {
                break;
            }
            cursor = next as usize;
            match child.tag() {
                gimli::DW_TAG_member => {
                    let (member, child_independent) =
                        self.walk_member(index, unit, child, component, depth)?;
                    independent &= child_independent;
                    found.push(member);
                }
                // A type parameter can shape the data a member only points
                // at through an erased pointer, as a vector's element type
                // does its buffer, so its layout joins the aggregate's. Only
                // the aggregate's own count, its direct children: a generic
                // method declared under it carries parameters of its own.
                gimli::DW_TAG_template_type_parameter if cursor == 1 => {
                    let (parameter, child_independent) =
                        self.walk_member(index, unit, child, component, depth)?;
                    independent &= child_independent;
                    found.push(format!("T:{parameter}"));
                }
                // The tag a variant answers to decides which payload a
                // caller reads, so it joins the digest beside the
                // members it governs; a variant without one is the
                // default, and a tag the die carries in a form the
                // number reader cannot name marks the digest instead
                // of pretending to a value.
                gimli::DW_TAG_variant => {
                    let tag = match child.attr_value(gimli::DW_AT_discr_value).map_err(string)? {
                        Some(value) => number_of(&value).unwrap_or_else(|| String::from("?")),
                        None => String::from("default"),
                    };
                    let (members, child_independent) =
                        self.walk_members(index, unit, child.offset(), component, depth)?;
                    independent &= child_independent;
                    found.push(format!("variant={tag}:{}", members.join(",")));
                }
                gimli::DW_TAG_variant_part => {
                    let (members, child_independent) =
                        self.walk_members(index, unit, child.offset(), component, depth)?;
                    independent &= child_independent;
                    found.extend(members);
                }
                gimli::DW_TAG_enumerator => {
                    let name = self.name_of(unit, child)?.unwrap_or_default();
                    let value = child
                        .attr_value(gimli::DW_AT_const_value)
                        .map_err(string)?
                        .as_ref()
                        .and_then(number_of);
                    found.push(format!(
                        "{name}={}",
                        value.unwrap_or_else(|| String::from("?"))
                    ));
                }
                _ => {}
            }
        }
        Ok((found, independent))
    }

    /// One member as its name, offset and the digest of the type it
    /// carries, with whether the walk beneath cut no cycle: the
    /// offset a caller reads a field at and the whole layout the
    /// digest covers decide what the field answers to, however deep
    /// the types nest and wherever their cycles run.
    fn walk_member(
        &mut self,
        index: usize,
        unit: &Unit<SectionReader>,
        die: &DebuggingInformationEntry<'_, '_, SectionReader>,
        component: usize,
        depth: usize,
    ) -> Result<(String, bool), String> {
        let name = self.name_of(unit, die)?.unwrap_or_default();
        let location = die
            .attr_value(gimli::DW_AT_data_member_location)
            .map_err(string)?
            .as_ref()
            .and_then(unsigned_of);
        let (carried, independent) = self.walk_attribute(
            index,
            die.attr_value(gimli::DW_AT_type).map_err(string)?,
            component,
            depth,
        )?;
        Ok((
            format!(
                "{name}@{}:{carried}",
                location.map_or(String::from("?"), |value| value.to_string())
            ),
            independent,
        ))
    }

    /// The namespace-qualified name of the die at `offset` of unit `index`.
    fn qualified_name(&mut self, index: usize, offset: UnitOffset) -> Result<String, String> {
        let Some(unit) = self.unit(index) else {
            return Err(String::from("the type's unit is missing"));
        };
        let unit_start = self.units[index].0;
        if let Some(joined) = self.qualified.get(&(unit_start, offset.0)) {
            return Ok(joined.clone());
        }
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
        let joined = if joined.is_empty() {
            String::from("<unnamed>")
        } else {
            joined
        };
        self.qualified
            .insert((unit_start, offset.0), joined.clone());
        Ok(joined)
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

/// The 64-bit FNV-1a digest of a description's bytes, spelled in hex,
/// which a member's line carries instead of the whole description of
/// its type: equal types spell the same digest and a changed type
/// cannot keep it.
fn digest_of(desc: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in desc.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The function and the number of the closure, async block or coroutine
/// whose environment type has the qualified name `qualified`, such as
/// `app::main::{closure_env#2}` or `app::main::{async_block_env#1}`.
pub(crate) fn closure_of(qualified: &str) -> Option<(String, u32)> {
    // Type arguments can name other closures, so the environment's own
    // name is the last one before them.
    let head = qualified.split('<').next().unwrap_or(qualified);
    let at = head.rfind("::{")?;
    let rest = &head[at + "::{".len()..];
    let (kind, number) = rest.split_once("_env#")?;
    if kind.is_empty()
        || !kind
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        return None;
    }
    let digits: String = number.chars().take_while(char::is_ascii_digit).collect();
    let number = digits.parse().ok()?;
    Some((head[..at].to_string(), number))
}

/// Turns a gimli error into its message, which is all the pairing needs.
fn string(error: Error) -> String {
    error.to_string()
}

/// The unsigned number an attribute value carries, which the size,
/// offset and convention attributes take in their numeric forms.
fn unsigned_of(value: &AttributeValue<SectionReader>) -> Option<u64> {
    match value {
        AttributeValue::Udata(value) => Some(*value),
        AttributeValue::Data1(value) => Some(u64::from(*value)),
        AttributeValue::Data2(value) => Some(u64::from(*value)),
        AttributeValue::Data4(value) => Some(u64::from(*value)),
        AttributeValue::Data8(value) => Some(*value),
        AttributeValue::Sdata(value) => u64::try_from(*value).ok(),
        // The convention attribute reaches the parser in its own
        // spelling, which a toolchain that records conventions uses.
        AttributeValue::CallingConvention(value) => Some(u64::from(value.0)),
        _ => None,
    }
}

/// The number an attribute value carries, signed values included,
/// which the discriminants and enumerator constants take: two builds
/// that spell one enum with different tags or constants must not
/// compare equal because a negative value failed an unsigned reading.
fn number_of(value: &AttributeValue<SectionReader>) -> Option<String> {
    match value {
        AttributeValue::Udata(value) => Some(value.to_string()),
        AttributeValue::Data1(value) => Some(value.to_string()),
        AttributeValue::Data2(value) => Some(value.to_string()),
        AttributeValue::Data4(value) => Some(value.to_string()),
        AttributeValue::Data8(value) => Some(value.to_string()),
        AttributeValue::Sdata(value) => Some(value.to_string()),
        _ => None,
    }
}

/// The addresses and entry offsets of the subprograms of one unit that the
/// DWARF places in code, which are the functions a symbol table pairs.
/// What one walk over a unit's dies collects: its subprograms with code,
/// by address and by the name a symbol table pairs them with, the
/// closures each function defines, and its statics by linkage name. One
/// traversal serves all of them, since each reads the same dies apart from
/// the attributes it picks.
#[derive(Default)]
struct Walked {
    /// The subprograms with code, by address.
    addressed: Vec<(u64, UnitOffset)>,
    /// The same subprograms, by the name a symbol table pairs them with.
    named: Vec<(String, UnitOffset)>,
    /// The closures the unit names, by function path and number.
    closures: Vec<(String, u32)>,
    /// The statics the unit describes, by linkage name.
    variables: Vec<(String, UnitOffset)>,
}

/// Walks one unit's dies once and answers what [`Walked`] holds. The path
/// of a closure comes from the names of the dies that enclose it, which
/// the walk keeps on a stack as it goes, so no second pass is needed.
fn walk_unit(dwarf: &Sections, unit: &Unit<SectionReader>) -> Result<Walked, String> {
    let mut walked = Walked::default();
    let mut enclosing: Vec<(isize, Option<String>)> = Vec::new();
    let mut depth: isize = 0;
    let mut entries = unit.entries();
    while let Some((delta, die)) = entries.next_dfs().map_err(string)? {
        depth += delta;
        while enclosing.last().is_some_and(|(at, _)| *at >= depth) {
            enclosing.pop();
        }
        let tag = die.tag();
        if matches!(
            tag,
            gimli::DW_TAG_namespace
                | gimli::DW_TAG_structure_type
                | gimli::DW_TAG_union_type
                | gimli::DW_TAG_enumeration_type
                | gimli::DW_TAG_class_type
        ) {
            let name = die_name(dwarf, unit, die)?;
            // Closures, async blocks and coroutines share one numbering
            // per function, and each kind names its environment type
            // `{<kind>_env#N}`.
            if tag == gimli::DW_TAG_structure_type
                && let Some(name) = name.as_deref()
                && name.starts_with('{')
                && name.contains("_env#")
            {
                let mut path: Vec<&str> = enclosing
                    .iter()
                    .filter_map(|(_, name)| name.as_deref())
                    .collect();
                path.push(name);
                if let Some(closure) = closure_of(&path.join("::")) {
                    walked.closures.push(closure);
                }
            }
            enclosing.push((depth, name));
            continue;
        }
        if tag == gimli::DW_TAG_variable {
            if let Some(name) = self_linkage_name(dwarf, unit, die)? {
                walked.variables.push((name, die.offset()));
            }
            continue;
        }
        if tag != gimli::DW_TAG_subprogram {
            continue;
        }
        // DWARF 5 names the address through an index into the unit's
        // address table, which the reader resolves like a plain address.
        let Some(low_pc) = die.attr_value(gimli::DW_AT_low_pc).map_err(string)? else {
            continue;
        };
        let Some(address) = dwarf.attr_address(unit, low_pc).map_err(string)? else {
            continue;
        };
        walked.addressed.push((address, die.offset()));
        let mut name = self_linkage_name(dwarf, unit, die)?;
        if name.is_none()
            && let Some(AttributeValue::UnitRef(specification)) =
                die.attr_value(gimli::DW_AT_specification).map_err(string)?
        {
            let declaration = unit.entry(specification).map_err(string)?;
            name = self_linkage_name(dwarf, unit, &declaration)?;
        }
        // Rust leaves the linkage name out when it spells the plain
        // name, which an unmangled export does: the definition's own
        // name pairs it then, and a name neither the definition nor
        // the declaration's linkage spells can still sit on the
        // declaration itself, where a method's declaration lives under
        // its type.
        if name.is_none() {
            name = die_name(dwarf, unit, die)?;
        }
        if name.is_none()
            && let Some(AttributeValue::UnitRef(specification)) =
                die.attr_value(gimli::DW_AT_specification).map_err(string)?
        {
            let declaration = unit.entry(specification).map_err(string)?;
            name = die_name(dwarf, unit, &declaration)?;
        }
        if let Some(name) = name {
            walked.named.push((name, die.offset()));
        }
    }
    Ok(walked)
}

/// The crate root a unit was compiled from: its compilation directory
/// joined with the part of its name before `/@/`, which is the source of
/// the crate root in the names the compiler gives its units.
fn crate_root(dwarf: &Sections, unit: &Unit<SectionReader>) -> Result<Option<String>, String> {
    let mut entries = unit.entries();
    let Some((_, die)) = entries.next_dfs().map_err(string)? else {
        return Ok(None);
    };
    let Some(name) = die_name(dwarf, unit, die)? else {
        return Ok(None);
    };
    let source = name.split("/@/").next().unwrap_or(&name);
    let directory = match die.attr_value(gimli::DW_AT_comp_dir).map_err(string)? {
        Some(value) => dwarf
            .attr_string(unit, value)
            .map_err(string)?
            .to_string_lossy()
            .map_err(string)?
            .into_owned(),
        None => String::new(),
    };
    Ok(Some(
        std::path::Path::new(&directory)
            .join(source)
            .to_string_lossy()
            .into_owned(),
    ))
}

/// The plain name a die carries itself, if any.
fn die_name(
    dwarf: &Sections,
    unit: &Unit<SectionReader>,
    die: &DebuggingInformationEntry<SectionReader, usize>,
) -> Result<Option<String>, String> {
    let Some(value) = die.attr_value(gimli::DW_AT_name).map_err(string)? else {
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
/// The element count an array type dies with, which the subrange
/// carries as a count or as a bound: the bound is the last index, so a
/// lower bound other than zero shifts it, and checked arithmetic keeps
/// an extreme bound from wrapping into a lie. A length in a form the
/// number reader cannot name answers none.
fn array_count(unit: &Unit<SectionReader>, offset: UnitOffset) -> Result<Option<u64>, String> {
    let mut children = unit.entries_at_offset(offset).map_err(string)?;
    // The cursor's first die is the array itself, and its subrange is the
    // child after it.
    if children.next_dfs().map_err(string)?.is_none() {
        return Ok(None);
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
        if let Some(count) = child
            .attr_value(gimli::DW_AT_count)
            .map_err(string)?
            .as_ref()
            .and_then(unsigned_of)
        {
            return Ok(Some(count));
        }
        let lower = child
            .attr_value(gimli::DW_AT_lower_bound)
            .map_err(string)?
            .as_ref()
            .and_then(unsigned_of)
            .unwrap_or(0);
        if let Some(upper) = child
            .attr_value(gimli::DW_AT_upper_bound)
            .map_err(string)?
            .as_ref()
            .and_then(unsigned_of)
        {
            return Ok(upper
                .checked_sub(lower)
                .and_then(|span| span.checked_add(1)));
        }
    }
    Ok(None)
}
