//! The invocation records the rustc wrapper and the linker shim leave for
//! the driver, and how they are read back.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::{fs, io};

use serde::{Deserialize, Serialize};

/// One recorded process invocation: its arguments, its environment and the
/// directory it ran in.
#[derive(Serialize, Deserialize)]
pub(crate) struct Recorded {
    /// The arguments, starting with the program that ran.
    pub args: Vec<String>,
    /// The environment the process saw.
    pub env: BTreeMap<String, String>,
    /// The directory the process ran in.
    pub cwd: PathBuf,
    /// The build request the invocation belonged to, which tells the
    /// recordings of one package's script under different feature
    /// selections apart. Records written before the field existed
    /// answer none.
    #[serde(default)]
    pub request: Option<String>,
}

impl Recorded {
    /// Serializes `recorded` to `path`.
    ///
    /// # Errors
    ///
    /// Returns the write error, with the path in the message.
    pub(crate) fn save(recorded: &Self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string(recorded).map_err(|e| e.to_string())?;
        fs::write(path, json).map_err(|error| format!("{}: {error}", path.display()))
    }

    /// Reads a record from `path`.
    ///
    /// # Errors
    ///
    /// Returns why the record could not be read or parsed.
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let text =
            fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
        serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
    }

    /// The last argument ending in `suffix`, which for rustc records is the
    /// crate root or an output file of the recorded invocation.
    pub(crate) fn last_arg(&self, suffix: &str) -> Option<PathBuf> {
        self.args
            .iter()
            .rev()
            .find(|arg| arg.ends_with(suffix))
            .map(PathBuf::from)
    }

    /// The crate name the invocation declares, from `--crate-name`.
    pub(crate) fn crate_name(&self) -> Option<&str> {
        self.values("--crate-name").next()
    }

    /// Every crate type the invocation declares, from `--crate-type`.
    pub(crate) fn crate_types(&self) -> Vec<&str> {
        self.values("--crate-type").collect()
    }

    /// The `--extern name=path` libraries the invocation reads, as pairs.
    pub(crate) fn externs(&self) -> Vec<(String, String)> {
        self.values("--extern")
            .filter_map(|value| value.split_once('='))
            .map(|(name, path)| (String::from(name), String::from(path)))
            .collect()
    }

    /// Every `--extern` library the invocation reads, as the alias the
    /// consumer uses and the crate name the artifact carries, which a
    /// renamed dependency keeps.
    pub(crate) fn extern_crates(&self) -> Vec<(String, String)> {
        self.externs()
            .into_iter()
            .map(|(alias, path)| {
                let real = crate_of_artifact(&path).unwrap_or_else(|| alias.clone());
                (alias, real)
            })
            .collect()
    }

    /// The `-C extra-filename` cargo gives the invocation's artifacts,
    /// with its leading dash.
    pub(crate) fn extra_filename(&self) -> Option<String> {
        self.values("-C")
            .find_map(|value| value.strip_prefix("extra-filename="))
            .map(String::from)
            .or_else(|| {
                let mask = crate::roles::operand_mask(&self.args);
                self.args
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| !mask[*index])
                    .find_map(|(_, arg)| arg.strip_prefix("-Cextra-filename="))
                    .map(String::from)
            })
    }

    /// The `--out-dir` the invocation writes its artifacts to.
    pub(crate) fn out_dir(&self) -> Option<&str> {
        self.values("--out-dir").next()
    }

    /// The stem of the artifacts the invocation produces: the crate name
    /// and the extra file name, which names the unit in dep-info files
    /// and in the artifacts its consumers read.
    pub(crate) fn artifact_stem(&self) -> Option<String> {
        let extra = self.extra_filename().unwrap_or_default();
        Some(format!("{}{}", self.crate_name()?, extra))
    }

    /// Every value the invocation gives `flag`, in both the spaced and the
    /// `=`-joined spelling cargo uses.
    fn values<'a>(&'a self, flag: &str) -> impl Iterator<Item = &'a str> {
        values_in(&self.args, flag)
    }
}

/// Where one unit's artifacts sit in cargo's build directory, which cargo
/// lays out in one of two ways depending on its version.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum UnitDir<'a> {
    /// The layout that gives every unit a directory of its own:
    /// `<profile>/build/<package>/<hash>/out`, for libraries, binaries and
    /// build scripts alike.
    Unit {
        /// The profile's build directory, `<profile>/build`.
        build: &'a Path,
        /// The package the unit belongs to.
        package: &'a str,
        /// The unit's hash, which names its directory.
        hash: &'a str,
    },
    /// A build script's directory in the layout that keeps the artifacts
    /// of one kind together: `<profile>/build/<package>-<hash>`.
    FlatScript {
        /// The profile's build directory, `<profile>/build`.
        build: &'a Path,
        /// The package the script belongs to.
        package: &'a str,
    },
    /// Any other directory of that layout, such as `<profile>/deps`,
    /// whose profile's build directory is `<profile>/build`.
    Flat {
        /// The profile directory the artifacts sit in.
        profile: &'a Path,
    },
}

impl<'a> UnitDir<'a> {
    /// Reads where `out_dir`, the `--out-dir` cargo gave an invocation,
    /// sits in the build directory, or `None` for a path with no parent.
    pub(crate) fn of(out_dir: &'a Path) -> Option<UnitDir<'a>> {
        let parent = out_dir.parent()?;
        let name = |path: &'a Path| path.file_name().and_then(|name| name.to_str());
        if name(out_dir) == Some("out")
            && let Some(hash) = name(parent).filter(|hash| is_hash(hash))
            && let Some(package_dir) = parent.parent()
            && let Some(package) = name(package_dir)
            && let Some(build) = package_dir.parent()
            && name(build) == Some("build")
        {
            return Some(UnitDir::Unit {
                build,
                package,
                hash,
            });
        }
        if name(parent) == Some("build")
            && let Some((package, hash)) = name(out_dir).and_then(|dir| dir.rsplit_once('-'))
            && !package.is_empty()
            && is_hash(hash)
        {
            return Some(UnitDir::FlatScript {
                build: parent,
                package,
            });
        }
        Some(UnitDir::Flat { profile: parent })
    }

    /// The profile's build directory, which holds the units of the layout
    /// that gives each its own directory, and the build scripts of the
    /// other.
    pub(crate) fn build_dir(&self) -> PathBuf {
        match self {
            UnitDir::Unit { build, .. } | UnitDir::FlatScript { build, .. } => build.to_path_buf(),
            UnitDir::Flat { profile } => profile.join("build"),
        }
    }

    /// Whether the directory is one cargo can give a build script: any
    /// unit's directory in the layout that gives each unit its own, and a
    /// `<package>-<hash>` build directory in the other.
    pub(crate) fn holds_a_script(&self) -> bool {
        matches!(self, UnitDir::Unit { .. } | UnitDir::FlatScript { .. })
    }
}

/// Whether `text` is a metadata hash cargo names a unit by.
fn is_hash(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_hexdigit())
}

/// The crate name an artifact path carries: the file name between the
/// `lib` prefix and the metadata hash. A dependency renamed in a manifest
/// keeps its real crate name in the artifact, whatever alias its
/// consumer imports it under.
pub(crate) fn crate_of_artifact(value: &str) -> Option<String> {
    let path = Path::new(value);
    let stem = path.file_stem()?.to_str()?;
    let stem = stem.strip_prefix("lib")?;
    crate_of_stem(stem)
}

/// The crate name an artifact stem carries: everything before the
/// metadata hash, or the whole stem when it names no hash.
pub(crate) fn crate_of_stem(stem: &str) -> Option<String> {
    match stem.rsplit_once('-') {
        Some((name, hash)) if !hash.is_empty() && hash.chars().all(|c| c.is_ascii_hexdigit()) => {
            Some(String::from(name))
        }
        // No metadata hash, which a hand-named artifact may lack.
        _ => Some(String::from(stem)),
    }
}

/// The artifact stem a path carries, with the `lib` prefix and the
/// extension stripped.
pub(crate) fn stem_of_artifact(value: &str) -> Option<String> {
    let path = Path::new(value);
    let stem = path.file_stem()?.to_str()?;
    Some(String::from(stem.strip_prefix("lib")?))
}

/// The source files one dep-info file lists, absolute against `cwd`,
/// with rustc's Makefile escaping undone and `.` and `..` resolved so
/// the paths compare equal to the ones a directory walk finds.
pub(crate) fn dep_info_sources(path: &Path, cwd: &Path) -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    // The first rule names the dep-info file itself and lists the unit's
    // sources as its dependencies, continuing over lines that end in a
    // backslash. The rule ends at the empty line that separates it from
    // the next one, since a file name may carry a newline of its own,
    // which rustc writes literally between its bytes. The lines split
    // on the newline byte alone, so a carriage return inside a name
    // stays one of its bytes.
    let mut rule = String::new();
    let mut ended = false;
    for line in text.split('\n') {
        if line.is_empty() {
            break;
        }
        // The newline between two lines of one rule belongs to the
        // name the first of them ends with.
        if ended {
            rule.push('\n');
        }
        ended = false;
        // Any other trailing byte, a tab included, belongs to the
        // last name.
        if let Some(head) = line.strip_suffix('\\') {
            rule.push_str(head);
            rule.push(' ');
        } else {
            rule.push_str(line);
            ended = true;
        }
    }
    let Some(section) = rule_section(&rule) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for reading in readings_of(section) {
        names.extend(reading);
    }
    names.sort();
    names.dedup();
    sources_of(names, cwd)
}

/// The source files a compiler log names in its diagnostic spans, plus
/// the files its file-read errors name, resolved the way the dep-info
/// sources are: a compile that fails before it writes dep-info still
/// says which files it read, in the human spans or in the JSON
/// diagnostics cargo's piped output gets, and an input it could not
/// read at all, such as what an `include_str!` names before the file
/// exists, must be learned the same way, so creating the file retries
/// the crate that asked for it.
pub(crate) fn span_sources(log: &str, cwd: &Path, remaps: &[(String, String)]) -> Vec<PathBuf> {
    let mut spans: Vec<String> = Vec::new();
    let mut unread: Vec<String> = Vec::new();
    for line in log.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("-->") {
            if let Some(file) = rest.trim().split(':').next() {
                spans.push(String::from(file));
            }
        } else if trimmed.starts_with('{')
            && let Ok(diagnostic) = serde_json::from_str::<serde_json::Value>(trimmed)
        {
            if let Some(named) = diagnostic.get("spans").and_then(|spans| spans.as_array()) {
                for span in named {
                    if let Some(file) = span.get("file_name").and_then(|file| file.as_str()) {
                        spans.push(String::from(file));
                    }
                }
            }
            if let Some(message) = diagnostic
                .get("message")
                .and_then(|message| message.as_str())
                && let Some(file) = unread_of(message)
            {
                unread.push(file);
            }
        } else if let Some(file) = unread_of(trimmed) {
            unread.push(file);
        }
    }
    // A span names a source the compile read, whatever its extension:
    // a module a `#[path]` attribute names can end in anything, and the
    // compiler's own synthetic paths, which name nothing on disk, stay
    // out because no such file exists to learn. A remapped spelling
    // keeps its physical spelling company: the physical file is what
    // exists, so its existence answers for the pair.
    spans.retain(|file| {
        Path::new(file).extension().is_some_and(|ext| ext == "rs")
            || std::fs::metadata(cwd.join(file)).is_ok()
            || unremap_of_name(file, remaps)
                .iter()
                .any(|candidate| std::fs::metadata(cwd.join(candidate)).is_ok())
    });
    let mut named = spans;
    named.extend(unread);
    // A recorded remapping spells the names the diagnostics carry: the
    // path a span names under a remapping is the remapped spelling,
    // which names no file, so the spelling the compile read joins the
    // candidates beside it and repairing the real file answers.
    if !remaps.is_empty() {
        let mut physical = Vec::with_capacity(named.len());
        for name in &named {
            physical.push(name.clone());
            physical.extend(unremap_of_name(name, remaps));
        }
        named = physical;
    }
    sources_of(named, cwd)
}

/// The physical spellings one remapped diagnostic name recovers: each
/// pair whose `to` side prefixes the name the way rustc spells its
/// output answers the `from` side joined with the rest. Rustc joins
/// the replacement and the remainder at one path boundary, so a `to`
/// that ends in a separator, or an empty one, needs none added; a
/// pair with an empty `from` side maps nothing at all, which rustc
/// skips the same way.
pub(crate) fn unremap_of_name(name: &str, remaps: &[(String, String)]) -> Vec<String> {
    let mut found = Vec::new();
    for (from, to) in remaps {
        if from.is_empty() {
            continue;
        }
        let Some(rest) = name.strip_prefix(to.as_str()) else {
            continue;
        };
        if rest.is_empty() {
            found.push(from.clone());
        } else if to.ends_with('/') || to.is_empty() {
            if !rest.starts_with('/') {
                found.push(format!("{from}/{rest}"));
            }
        } else if rest.starts_with('/') {
            found.push(format!("{from}{rest}"));
        }
    }
    found
}

/// The `--remap-path-prefix from=to` pairs the invocation records, in
/// both the spaced and the `=`-joined spelling, with the operands of
/// other options left out: the diagnostics a compile prints spell its
/// paths under these pairs, and only their `from` sides name files.
pub(crate) fn unremap_of(args: &[String]) -> Vec<(String, String)> {
    let mask = crate::roles::operand_mask(args);
    let mut pairs = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if !mask[index] {
            if arg == "--remap-path-prefix"
                && let Some(value) = args.get(index + 1)
            {
                // Rustc splits a pair at the last `=`, so the `from`
                // side can carry the character, and it skips a pair
                // whose `from` side is empty, which maps nothing.
                if let Some((from, to)) = value.rsplit_once('=')
                    && !from.is_empty()
                {
                    pairs.push((String::from(from), String::from(to)));
                }
                index += 2;
                continue;
            }
            if let Some(value) = arg.strip_prefix("--remap-path-prefix=")
                && let Some((from, to)) = value.rsplit_once('=')
                && !from.is_empty()
            {
                pairs.push((String::from(from), String::from(to)));
                index += 1;
                continue;
            }
        }
        index += 1;
    }
    pairs
}

/// The file name a rustc file-read error names, which the rendered text
/// quotes between backticks and a plain line spells up to its `:`. A
/// diagnostic that names no file, such as a linker error, answers none.
fn unread_of(message: &str) -> Option<String> {
    let rest = message.split("couldn't read").nth(1)?;
    let rest = rest.trim_start();
    let file = if let Some(quoted) = rest.strip_prefix('`') {
        // Rustc writes a backtick of the name literally, so the closing
        // delimiter is the last one the line carries.
        quoted.rsplit_once('`')?.0
    } else {
        let end = rest
            .find(|ch: char| ch == ':' || ch.is_whitespace())
            .unwrap_or(rest.len());
        &rest[..end]
    };
    if file.is_empty() {
        None
    } else {
        Some(String::from(file))
    }
}

/// Resolves source names against `cwd`, the way the dep-info entries
/// and the diagnostic spans spell them.
fn sources_of(dependencies: Vec<String>, cwd: &Path) -> Vec<PathBuf> {
    dependencies
        .into_iter()
        .flat_map(|dependency| {
            let joined = cwd.join(dependency);
            // A path through a symlinked directory must answer where the
            // file really is, which is what the watcher and the owners
            // compare against; a path that does not exist falls back to
            // the lexical form. The spelled form stays an alias of its
            // own, so an event on the symlink still names its owner even
            // after the link was retargeted to another file, and the
            // raw spelling keeps every link the name carries: a
            // normalized `link/../file` would hide the link, whose
            // retarget changes what the compile reads without either
            // end of the path moving.
            let mut forms = vec![joined.clone(), normalize(&joined)];
            if let Ok(real) = std::fs::canonicalize(&joined)
                && real != forms[0]
            {
                forms.push(real);
            }
            forms
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The dependencies of one Makefile rule: everything after its first
/// unescaped colon, split at the spaces rustc separates with, with the
/// escaping rustc writes undone. Rustc escapes a space as `\ ` and
/// writes every other byte of a file name literally, tabs and newlines
/// included, so a backslash before anything else stays in the name and
/// a newline belongs to the name that carries it.
fn rule_section(rule: &str) -> Option<&str> {
    let mut escaped = false;
    let mut at = None;
    for (index, c) in rule.char_indices() {
        if escaped {
            escaped = false;
        } else {
            match c {
                '\\' => escaped = true,
                ':' => {
                    at = Some(index);
                    break;
                }
                _ => {}
            }
        }
    }
    rule.get(at? + 1..)
}

/// The name lists a dependency section reads as, over every reading of
/// its backslash-space pairs: rustc escapes a space in a name with a
/// backslash but writes a backslash in a name plain, so a pair reads
/// as an escape, which joins two names into one, or as plain bytes,
/// which ends the name at the backslash and begins the next after the
/// space. A section with several pairs reads plain up to any one pair
/// and escaped from it on, and every reading joins the learned names,
/// so the watches answer whichever the file system carries. A section
/// of more pairs than the cap reads only all-escaped and all-plain,
/// since each reading costs the whole section.
fn readings_of(text: &str) -> Vec<Vec<String>> {
    const PAIRS_CAP: usize = 20;
    let segments: Vec<&str> = text.split("\\ ").collect();
    let pairs = segments.len() - 1;
    if pairs > PAIRS_CAP {
        return vec![reading(&segments, |_| true), reading(&segments, |_| false)];
    }
    (0..=pairs)
        .map(|plain| reading(&segments, |pair| pair >= plain))
        .collect()
}

/// The names of one reading of the `segments` a section splits into at
/// its backslash-space pairs, where `joins` says which pairs read as an
/// escaped space inside a name. Every other space ends a name.
fn reading(segments: &[&str], joins: impl Fn(usize) -> bool) -> Vec<String> {
    let mut names = Vec::new();
    let mut name = String::new();
    for (index, segment) in segments.iter().enumerate() {
        if index > 0 {
            if joins(index - 1) {
                name.push(' ');
            } else {
                name.push('\\');
                names.push(std::mem::take(&mut name));
            }
        }
        let mut parts = segment.split(' ');
        name.push_str(parts.next().unwrap_or_default());
        for part in parts {
            if !name.is_empty() {
                names.push(std::mem::take(&mut name));
            }
            name.push_str(part);
        }
    }
    if !name.is_empty() {
        names.push(name);
    }
    names
}

/// Resolves `.` and `..` components lexically, which dep-info entries
/// carry when a `#[path]` attribute climbs out of a directory, so a
/// path compares equal to the one the watcher finds.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The stem of the artifacts an invocation produces, from its raw
/// arguments: the crate name and the extra file name, which names the
/// unit in dep-info files and in the artifacts its consumers read.
pub(crate) fn artifact_stem_of_args(args: &[String]) -> Option<String> {
    let name = values_in(args, "--crate-name").next()?;
    let mut extra = String::new();
    let mask = crate::roles::operand_mask(args);
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if !mask[index] {
            if arg == "-C"
                && let Some(value) = args
                    .get(index + 1)
                    .and_then(|next| next.strip_prefix("extra-filename="))
            {
                extra = String::from(value);
                break;
            }
            if let Some(value) = arg.strip_prefix("-Cextra-filename=") {
                extra = String::from(value);
                break;
            }
        }
        index += 1;
    }
    Some(format!("{name}{extra}"))
}

/// The stem a build script's recording is filed under: its artifact stem,
/// with the unit's hash added where its artifacts carry none, as build
/// scripts do in the layout that gives every unit a directory of its own,
/// so the scripts of two packages never share a recording.
pub(crate) fn record_stem_of_args(args: &[String]) -> Option<String> {
    let stem = artifact_stem_of_args(args)?;
    let hash = values_in(args, "--out-dir").find_map(|dir| match UnitDir::of(Path::new(dir)) {
        Some(UnitDir::Unit { hash, .. }) => Some(String::from(hash)),
        _ => None,
    });
    Some(match hash {
        Some(hash) if !stem.ends_with(&format!("-{hash}")) => format!("{stem}-{hash}"),
        _ => stem,
    })
}

/// Whether `args` compile a build script. Cargo names a script's crate
/// after its file, `build_script_build` for `build.rs`, compiles it as a
/// binary into a directory a script can hold, and sets no
/// `CARGO_BIN_NAME` for it, which it sets for every binary target, one
/// named like a script included. `bin_name` is that variable as the
/// compile saw it.
pub(crate) fn compiles_a_script(args: &[String], bin_name: Option<&str>) -> bool {
    bin_name.is_none()
        && values_in(args, "--crate-name").any(|name| name.starts_with("build_script_"))
        && values_in(args, "--crate-type").any(|kind| kind == "bin")
        && values_in(args, "--out-dir")
            .any(|dir| UnitDir::of(Path::new(dir)).is_some_and(|unit| unit.holds_a_script()))
}

/// Every value `args` gives `flag`, in both the spaced and the
/// `=`-joined spelling cargo uses. A token another option consumed as
/// its operand answers for nothing, whatever option it spells.
pub(crate) fn values_in<'a>(args: &'a [String], flag: &str) -> impl Iterator<Item = &'a str> {
    let glued = format!("{flag}=");
    let mask = crate::roles::operand_mask(args);
    let spaced_mask = mask.clone();
    let spaced = args
        .windows(2)
        .enumerate()
        .filter_map(move |(index, pair)| {
            (!spaced_mask[index] && pair[0] == flag).then_some(pair[1].as_str())
        });
    let joined = args.iter().enumerate().filter_map(move |(index, arg)| {
        (!mask[index])
            .then_some(arg)
            .and_then(|arg| arg.strip_prefix(&glued))
    });
    spaced.chain(joined)
}

/// A linker invocation, recorded so the driver can replay its arguments.
///
/// The wrapper records the arguments the linker driver received. rustc keeps
/// all paths it passes absolute, which the pairing of fat and thin records
/// relies on.
#[derive(Serialize, Deserialize)]
pub(crate) struct RecordedLink(pub Vec<String>);

impl RecordedLink {
    /// Serializes `args` to `path`.
    ///
    /// # Errors
    ///
    /// Returns the write error, with the path in the message.
    pub(crate) fn save(args: &[String], path: &Path) -> Result<(), String> {
        let json =
            serde_json::to_string(&RecordedLink(args.to_vec())).map_err(|e| e.to_string())?;
        fs::write(path, json).map_err(|error| format!("{}: {error}", path.display()))
    }

    /// Reads a link record from `path`.
    ///
    /// # Errors
    ///
    /// Returns why the record could not be read or parsed.
    pub(crate) fn load(path: &Path) -> Result<Vec<String>, String> {
        let json: RecordedLink = serde_json::from_str(
            &fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?,
        )
        .map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(json.0)
    }
}

/// Reads the environment of the current process as a record's environment.
pub(crate) fn current_env() -> BTreeMap<String, String> {
    std::env::vars().collect()
}

/// Writes `bytes` to `path`.
///
/// # Errors
///
/// Returns the write error, with the path in the message.
pub(crate) fn write_bytes(bytes: &[u8], path: &Path) -> Result<(), String> {
    fs::write(path, bytes).map_err(|error| format!("{}: {error}", path.display()))
}

/// Removes `path`, ignoring the error when it does not exist.
pub(crate) fn remove_file(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
#[path = "record_tests.rs"]
mod tests;
