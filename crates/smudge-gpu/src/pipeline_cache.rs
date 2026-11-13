//! The on-disk cache of vello's compute pipelines.
//!
//! Compiling vello's pipelines costs 570 to 680 ms of CPU and 43 to 58 MB of
//! memory on a GTX 1060, every time a renderer is created. wgpu carries the
//! compiled pipelines between runs in a [`PipelineCache`], whose bytes a
//! driver consumes directly, so a persisted cache needs checks that keep
//! anything else from reaching the driver:
//!
//! - The cache is one file per adapter under an owner-private directory in
//!   the user's cache location, shared by every Smudge app of that user.
//!   The directory and the file are checked for type, owner and permissions
//!   before any byte is read.
//! - A header carries a magic, a format version, the adapter's
//!   [`wgpu::util::pipeline_cache_key`], and the payload's length and hash.
//!   The whole file is verified before the cache is created.
//! - A save writes a unique temporary file and renames it into place, so a
//!   file on disk is always complete or absent while the machine stays up.
//!   A file cut short by a crash or a power loss fails the checks on the
//!   next load and is replaced by the next save.
//! - Every rejection ends in an uncached start: the caller receives no data
//!   and the app runs as on the first start, with no panic and no partial
//!   file.
//!
//! [`GpuRenderer::new`] and [`GpuSurface::new`] open the cache when the
//! device was created with the [`wgpu::Features::PIPELINE_CACHE`] feature of
//! [`device_features`](crate::device_features), and save it once vello's
//! pipelines exist.
//!
//! [`GpuRenderer::new`]: crate::GpuRenderer::new
//! [`GpuSurface::new`]: crate::GpuSurface::new

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use vello::wgpu::{self, Device, PipelineCache};

/// The bytes that start every cache file.
const MAGIC: [u8; 12] = *b"smudge-cache";

/// The version of the file layout. A file written by another layout is
/// rejected and replaced on the next save.
const FORMAT_VERSION: u32 = 1;

/// The length of the header without the adapter key: magic, version, key
/// length, payload length and payload hash.
const HEADER_LEN: usize = MAGIC.len() + 4 + 4 + 8 + 8;

/// How many cache files a process has saved, to keep temporary names unique.
static SAVE_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Opens the pipeline cache of the user this process runs as.
///
/// Returns `None` when no cache can be used: the device was created without
/// [`wgpu::Features::PIPELINE_CACHE`], the adapter has no cache key, or the
/// user has no cache location. Otherwise the cache handle comes back ready
/// for vello's pipelines, holding the file's data when a valid one exists.
/// A first start, a rejected file and a missing file all give a fresh, empty
/// cache, so every start that can save a cache also builds one.
pub(crate) fn open(device: &Device) -> Option<PipelineCache> {
    open_from(device, &cache_dir()?)
}

/// Opens the pipeline cache in `dir`, whatever the user's cache location is.
///
/// See [`open`] for the contract. Tests use this to point the cache at a
/// directory of their own.
pub(crate) fn open_from(device: &Device, dir: &Path) -> Option<PipelineCache> {
    if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
        return None;
    }
    let key = wgpu::util::pipeline_cache_key(&device.adapter_info())?;
    let file = read_validated(dir, &key);
    // SAFETY: wgpu requires `data` to be bytes that `PipelineCache::get_data`
    // returned, and a save to disk cannot prove that. The checks above close
    // the gap instead: the bytes live in an owner-private directory whose
    // type, owner and permissions were checked before the read, the header
    // names this adapter's `pipeline_cache_key`, the format version is fixed,
    // and a hash covers the whole payload, so a truncated, tampered or
    // foreign file never reaches here. With `fallback: true` wgpu falls back
    // to pipelines without a cache when it still rejects the data, so a bad
    // file cannot fail the start either.
    let cache = unsafe {
        device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
            label: Some("smudge pipeline cache"),
            data: file.as_deref(),
            fallback: true,
        })
    };
    Some(cache)
}

/// Saves the pipelines `cache` has collected to the user's cache location.
///
/// Nothing is written when the cache holds no data, when the adapter or the
/// cache location is unusable, or when the file cannot be written. Every
/// skip is silent: a lost save costs the next start its speed, nothing else.
pub(crate) fn save(device: &Device, cache: &PipelineCache) {
    if let Some(dir) = cache_dir() {
        save_into(device, cache, &dir);
    }
}

/// Saves the pipelines `cache` has collected into `dir`.
///
/// See [`save`] for the contract. Tests use this to point the cache at a
/// directory of their own.
pub(crate) fn save_into(device: &Device, cache: &PipelineCache, dir: &Path) {
    if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
        return;
    }
    let Some(key) = wgpu::util::pipeline_cache_key(&device.adapter_info()) else {
        return;
    };
    let Some(data) = cache.get_data() else {
        return;
    };
    if data.is_empty() {
        return;
    }
    if !ensure_dir(dir) {
        return;
    }
    let target = dir.join(&key);
    write_atomically(dir, &target, &serialize(&data, &key));
}

/// Reads and validates the cache file `key` names in `dir`.
fn read_validated(dir: &Path, key: &str) -> Option<Vec<u8>> {
    if !entry_is_owner_private(dir, true) {
        return None;
    }
    let file = dir.join(key);
    if !entry_is_owner_private(&file, false) {
        return None;
    }
    let bytes = std::fs::read(&file).ok()?;
    validate(&bytes, key).map(<[u8]>::to_vec)
}

/// Returns the cache directory of the user this process runs as: the
/// `smudge` directory inside `$XDG_CACHE_HOME` when it is set and not empty,
/// inside `%LOCALAPPDATA%` on Windows, inside `~/Library/Caches` on macOS,
/// and inside `~/.cache` elsewhere.
///
/// Returns `None` when no home is known, and the app then runs with no
/// cache.
pub(crate) fn cache_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("XDG_CACHE_HOME")
        && !home.is_empty()
        && Path::new(&home).is_absolute()
    {
        return Some(PathBuf::from(home).join("smudge"));
    }
    Some(base_cache_dir()?.join("smudge"))
}

/// Returns the user's cache location without the crate's own directory.
fn base_cache_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let local = std::env::var_os("LOCALAPPDATA")?;
        if local.is_empty() {
            return None;
        }
        Some(PathBuf::from(local))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = std::env::var_os("HOME")?;
        if home.is_empty() {
            return None;
        }
        if cfg!(target_os = "macos") {
            Some(PathBuf::from(home).join("Library").join("Caches"))
        } else {
            Some(PathBuf::from(home).join(".cache"))
        }
    }
}

/// Creates `dir` as an owner-private directory, accepts it when it already
/// is one, and tightens it when the user owns it but left it open.
fn ensure_dir(dir: &Path) -> bool {
    if let Some(parent) = dir.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::create_dir(dir);
    #[cfg(unix)]
    tighten_mode(dir);
    entry_is_owner_private(dir, true)
}

/// Sets `dir` to mode 700 when the user this process runs as owns the
/// directory.
///
/// A fresh directory inherits the umask's bits, and one the user loosened by
/// hand would otherwise be rejected on every start and never repaired.
#[cfg(unix)]
fn tighten_mode(dir: &Path) {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let Ok(meta) = std::fs::symlink_metadata(dir) else {
        return;
    };
    if meta.is_dir() && meta.uid() == rustix::process::geteuid().as_raw() {
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
}

/// Returns whether `path` holds a directory (`want_dir`) or a regular file
/// that the user this process runs as owns and that no other user can
/// reach. A symlink fails the type check.
fn entry_is_owner_private(path: &Path, want_dir: bool) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    let kind_is_right = if want_dir {
        meta.is_dir()
    } else {
        meta.is_file()
    };
    kind_is_right && owner_private(&meta)
}

/// Returns whether `meta` describes an entry the user owns and no other
/// user holds any permission on.
#[cfg(unix)]
fn owner_private(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    meta.uid() == rustix::process::geteuid().as_raw() && meta.permissions().mode() & 0o077 == 0
}

/// Returns whether `meta` describes an entry of the user's own profile.
///
/// Windows keeps no owner and mode bits in [`std::fs::Metadata`]. The cache
/// lives under `%LOCALAPPDATA%`, whose profile directories are private to
/// the user by default.
#[cfg(not(unix))]
fn owner_private(_meta: &std::fs::Metadata) -> bool {
    true
}

/// Writes `bytes` to a unique temporary file in `dir` and renames it over
/// `target`, so `target` is never half a file.
fn write_atomically(dir: &Path, target: &Path, bytes: &[u8]) -> bool {
    let Some(temp) = temp_path(dir, target) else {
        return false;
    };
    if write_temp(&temp, bytes).is_err() {
        let _ = std::fs::remove_file(&temp);
        return false;
    }
    if std::fs::rename(&temp, target).is_err() {
        let _ = std::fs::remove_file(&temp);
        return false;
    }
    true
}

/// Creates `temp` and writes `bytes` into it, from the first byte on
/// readable by the owner only.
fn write_temp(temp: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(temp)?
            .write_all(bytes)
    }
    #[cfg(not(unix))]
    std::fs::write(temp, bytes)
}

/// Returns a unique temporary path beside `target`.
fn temp_path(dir: &Path, target: &Path) -> Option<PathBuf> {
    let name = target.file_name()?.to_str()?;
    let count = SAVE_COUNTER.fetch_add(1, Ordering::Relaxed);
    Some(dir.join(format!(".{name}.{}.{}.tmp", std::process::id(), count)))
}

/// Lays out `payload` under a header that names `key`.
fn serialize(payload: &[u8], key: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER_LEN + key.len() + payload.len());
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(
        &u32::try_from(key.len())
            .expect("the adapter key fits in u32")
            .to_le_bytes(),
    );
    bytes.extend_from_slice(key.as_bytes());
    bytes.extend_from_slice(
        &u64::try_from(payload.len())
            .expect("the payload fits in u64")
            .to_le_bytes(),
    );
    bytes.extend_from_slice(&fnv1a(payload).to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

/// Returns the payload of a cache file when every check holds.
///
/// The magic and the format version must match this file layout, the key in
/// the header must be `key`, and a fixed hash must cover a payload that is
/// exactly as long as the header says. Anything else, including a file too
/// short to hold a header and a file with bytes past the payload, is
/// rejected.
fn validate<'a>(bytes: &'a [u8], key: &str) -> Option<&'a [u8]> {
    if bytes.len() < HEADER_LEN || bytes[..MAGIC.len()] != MAGIC {
        return None;
    }
    let mut at = MAGIC.len();
    let version = read_u32(bytes, &mut at)?;
    if version != FORMAT_VERSION {
        return None;
    }
    let key_len = read_u32(bytes, &mut at)? as usize;
    let key_bytes = bytes.get(at..at.checked_add(key_len)?)?;
    if key_bytes != key.as_bytes() {
        return None;
    }
    at += key_len;
    let payload_len = usize::try_from(read_u64(bytes, &mut at)?).ok()?;
    let hash = read_u64(bytes, &mut at)?;
    if at.checked_add(payload_len) != Some(bytes.len()) {
        return None;
    }
    let payload = &bytes[at..];
    if fnv1a(payload) != hash {
        return None;
    }
    Some(payload)
}

/// Reads a little-endian `u32` at `at` and moves `at` past it.
fn read_u32(bytes: &[u8], at: &mut usize) -> Option<u32> {
    let end = bytes.get(*at..*at + 4)?;
    let value = u32::from_le_bytes(end.try_into().expect("a four byte slice"));
    *at += 4;
    Some(value)
}

/// Reads a little-endian `u64` at `at` and moves `at` past it.
fn read_u64(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let end = bytes.get(*at..*at + 8)?;
    let value = u64::from_le_bytes(end.try_into().expect("an eight byte slice"));
    *at += 8;
    Some(value)
}

/// Hashes `bytes` with FNV-1a, 64 bit.
///
/// The algorithm is fixed in this file because `std::hash::DefaultHasher`
/// may change between Rust releases, which would reject a file the last
/// release wrote. FNV-1a catches accidental corruption and truncation; an
/// attacker who can write into the owner-private directory can recompute
/// it, which the owner and permission checks exist to prevent.
fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[cfg(test)]
#[path = "pipeline_cache_tests.rs"]
mod tests;
