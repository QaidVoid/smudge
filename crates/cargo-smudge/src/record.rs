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
