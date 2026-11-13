//! Tests for the checks that keep a bad cache file from reaching the driver.

use std::fs;
use std::path::{Path, PathBuf};

use vello::wgpu::{self, Device, Queue};

use super::{cache_dir, open_from, read_validated, save_into, serialize, validate};

const KEY: &str = "wgpu_pipeline_cache_vulkan_4038_8711";
const PAYLOAD: &[u8] = b"the driver consumes these bytes";

/// A directory a test may litter, removed when the test ends.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scratch(name: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!(
        "smudge-pipeline-cache-{}-{name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the scratch directory is created");
    // The cache only reads and writes owner-private directories, so the
    // scratch directory has to be one.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .expect("the scratch mode is set");
    }
    Scratch(dir)
}

fn write_cache_file(dir: &Path, bytes: &[u8]) {
    let file = dir.join(KEY);
    fs::write(&file, bytes).expect("the cache file is written");
    // A save of ours leaves the file private to the owner.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600))
            .expect("the file mode is set");
    }
}

#[test]
fn a_well_formed_file_validates_to_its_payload() {
    let bytes = serialize(PAYLOAD, KEY);
    assert_eq!(validate(&bytes, KEY), Some(PAYLOAD));
}

#[test]
fn a_well_formed_file_is_read_back_from_disk() {
    let dir = scratch("read-back");
    write_cache_file(&dir.0, &serialize(PAYLOAD, KEY));
    let payload = read_validated(&dir.0, KEY);
    assert_eq!(payload.as_deref(), Some(PAYLOAD));
}

#[test]
fn a_file_with_no_payload_yet_is_rejected_without_a_cache() {
    let dir = scratch("missing");
    assert_eq!(read_validated(&dir.0, KEY), None);
}

#[test]
fn a_truncated_file_is_rejected() {
    let bytes = serialize(PAYLOAD, KEY);
    assert_eq!(validate(&bytes[..bytes.len() - 1], KEY), None);
    let half = serialize(&PAYLOAD[..3], KEY);
    assert_eq!(validate(&bytes[..half.len()], KEY), None);
}

#[test]
fn a_file_shorter_than_a_header_is_rejected() {
    assert_eq!(validate(&[], KEY), None);
    assert_eq!(validate(&serialize(PAYLOAD, KEY)[..8], KEY), None);
}

#[test]
fn a_file_with_bytes_past_the_payload_is_rejected() {
    let mut bytes = serialize(PAYLOAD, KEY);
    bytes.push(0);
    assert_eq!(validate(&bytes, KEY), None);
}

#[test]
fn a_header_naming_a_key_longer_than_the_file_is_rejected() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&super::MAGIC);
    bytes.extend_from_slice(&super::FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&u32::MAX.to_le_bytes());
    bytes.extend_from_slice(&(PAYLOAD.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&super::fnv1a(PAYLOAD).to_le_bytes());
    bytes.extend_from_slice(PAYLOAD);
    assert_eq!(validate(&bytes, KEY), None);
}

#[test]
fn a_header_naming_an_untold_payload_length_is_rejected() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&super::MAGIC);
    bytes.extend_from_slice(&super::FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(KEY.len() as u32).to_le_bytes());
    bytes.extend_from_slice(KEY.as_bytes());
    bytes.extend_from_slice(&u64::MAX.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(PAYLOAD);
    assert_eq!(validate(&bytes, KEY), None);
}

#[test]
fn a_file_with_a_flipped_payload_byte_is_rejected() {
    let mut bytes = serialize(PAYLOAD, KEY);
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    assert_eq!(validate(&bytes, KEY), None);
}

#[test]
fn a_file_with_a_flipped_header_byte_is_rejected() {
    let bytes = serialize(PAYLOAD, KEY);
    let version_at = super::MAGIC.len();
    let key_at = version_at + 4;
    let after_key = key_at + KEY.len();
    let payload_len_at = after_key;
    let hash_at = payload_len_at + 8;

    let mut magic = bytes.clone();
    magic[0] ^= 0x01;
    assert_eq!(validate(&magic, KEY), None);

    let mut version = bytes.clone();
    version[version_at] ^= 0x01;
    assert_eq!(validate(&version, KEY), None);

    let mut length = bytes.clone();
    length[payload_len_at] ^= 0x01;
    assert_eq!(validate(&length, KEY), None);

    let mut hash = bytes.clone();
    hash[hash_at] ^= 0x01;
    assert_eq!(validate(&hash, KEY), None);
}

#[test]
fn a_file_from_another_adapter_is_rejected() {
    let bytes = serialize(PAYLOAD, KEY);
    assert_eq!(
        validate(&bytes, "wgpu_pipeline_cache_vulkan_4038_8712"),
        None
    );
    assert_eq!(validate(&bytes, ""), None);
}

#[test]
fn an_empty_or_foreign_file_is_rejected() {
    assert_eq!(validate(&[], KEY), None);
    assert_eq!(validate(b"a short line of text", KEY), None);
}

#[test]
fn a_symlinked_cache_file_is_rejected() {
    let dir = scratch("symlink-file");
    let real = dir.0.join("real");
    write_cache_file(&dir.0, &serialize(PAYLOAD, KEY));
    fs::rename(dir.0.join(KEY), &real).expect("the file is moved");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, dir.0.join(KEY)).expect("the symlink is created");
    #[cfg(windows)]
    {
        // A stock Windows session cannot create symlinks without developer
        // mode, which is no reason to fail the suite.
        if let Err(error) = std::os::windows::fs::symlink_file(&real, dir.0.join(KEY)) {
            eprintln!("skipping: this Windows session cannot create symlinks: {error}");
            return;
        }
    }
    assert_eq!(read_validated(&dir.0, KEY), None);
}

#[test]
fn a_directory_at_the_file_path_is_rejected() {
    let dir = scratch("file-is-dir");
    fs::create_dir(dir.0.join(KEY)).expect("the directory is created");
    assert_eq!(read_validated(&dir.0, KEY), None);
}

#[cfg(unix)]
#[test]
fn a_cache_file_readable_by_others_is_rejected() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("file-mode");
    write_cache_file(&dir.0, &serialize(PAYLOAD, KEY));
    fs::set_permissions(dir.0.join(KEY), fs::Permissions::from_mode(0o644))
        .expect("the mode is set");
    assert_eq!(read_validated(&dir.0, KEY), None);
}

#[cfg(unix)]
#[test]
fn an_unreadable_cache_file_is_rejected() {
    use std::os::unix::fs::PermissionsExt;
    if rustix::process::geteuid().as_raw() == 0 {
        eprintln!("skipping: root reads a file closed to its owner anyway");
        return;
    }
    let dir = scratch("file-closed");
    write_cache_file(&dir.0, &serialize(PAYLOAD, KEY));
    fs::set_permissions(dir.0.join(KEY), fs::Permissions::from_mode(0o000))
        .expect("the mode is set");
    assert_eq!(read_validated(&dir.0, KEY), None);
}

#[cfg(unix)]
#[test]
fn a_cache_directory_open_to_others_is_rejected() {
    use std::os::unix::fs::PermissionsExt;
    for mode in [0o755, 0o777, 0o700 | 0o050, 0o750] {
        let dir = scratch("dir-mode");
        write_cache_file(&dir.0, &serialize(PAYLOAD, KEY));
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(mode)).expect("the mode is set");
        assert_eq!(
            read_validated(&dir.0, KEY),
            None,
            "mode {mode:o} is rejected"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_save_tightens_a_loose_cache_directory_the_user_owns() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("loose-dir");
    fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o755)).expect("the mode is set");
    assert!(
        super::ensure_dir(&dir.0),
        "the user's own directory is repaired"
    );
    let mode = fs::metadata(&dir.0)
        .expect("the directory is read")
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "no group or other bit survives the repair");
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "redox"))))]
#[test]
fn a_fifo_at_the_file_path_is_rejected_before_it_is_read() {
    use rustix::fs::Mode;
    // A FIFO with owner-only permissions passes the owner and mode checks.
    // Only the type check rejects it, before a read could block on it.
    let dir = scratch("fifo");
    let fifo = dir.0.join(KEY);
    rustix::fs::mkfifoat(rustix::fs::CWD, fifo.as_path(), Mode::from_raw_mode(0o600))
        .expect("the FIFO is created");
    assert!(
        !super::entry_is_owner_private(&fifo, false),
        "a FIFO is not a regular file"
    );
}

// The owner branch of the check has no test: a file owned by another user
// cannot be arranged without root, and the test runner is not root. The
// wrong-kind branch has no FIFO test on macOS either, where rustix's
// mkfifoat does not exist; there only the symlink test pins the type check.

#[test]
fn a_file_at_the_directory_path_is_rejected() {
    let dir = scratch("dir-is-file");
    let file = dir.0.join("smudge");
    fs::write(&file, b"not a directory").expect("the file is written");
    assert_eq!(read_validated(&file, KEY), None);
}

#[test]
fn the_cache_directory_is_named_after_the_crate_in_the_user_cache() {
    let Some(dir) = cache_dir() else {
        eprintln!("skipping: this user has no cache location");
        return;
    };
    assert_eq!(
        dir.file_name().and_then(|name| name.to_str()),
        Some("smudge")
    );
}

fn headless_device() -> Option<(Device, Queue)> {
    let adapter = futures_executor::block_on(
        crate::instance().request_adapter(&wgpu::RequestAdapterOptions::default()),
    )
    .ok()?;
    let (device, queue) =
        futures_executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: crate::device_limits(&adapter),
            required_features: crate::device_features(&adapter),
            ..Default::default()
        }))
        .ok()?;
    Some((device, queue))
}

#[test]
fn a_device_without_the_cache_feature_yields_no_cache() {
    let Some(adapter) = futures_executor::block_on(
        crate::instance().request_adapter(&wgpu::RequestAdapterOptions::default()),
    )
    .ok() else {
        eprintln!("skipping: no GPU adapter available for the pipeline cache");
        return;
    };
    let device = futures_executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: crate::device_limits(&adapter),
        ..Default::default()
    }))
    .ok();
    let Some((device, _queue)) = device else {
        eprintln!("skipping: the adapter could not provide a device");
        return;
    };
    let dir = scratch("no-feature");
    assert_eq!(open_from(&device, &dir.0), None);
}

#[test]
fn a_saved_cache_round_trips_through_a_device() {
    let Some((device, _queue)) = headless_device() else {
        eprintln!("skipping: no GPU adapter available for the pipeline cache");
        return;
    };
    let Some(key) = wgpu::util::pipeline_cache_key(&device.adapter_info()) else {
        eprintln!("skipping: this adapter keeps no application pipeline cache");
        return;
    };
    let dir = scratch("round-trip");
    let cache = open_from(&device, &dir.0)
        .expect("a device with the pipeline cache feature always yields a cache");

    let renderer = vello::Renderer::new(&device, crate::renderer::vello_options(Some(&cache)))
        .expect("the device cannot create the vello pipelines");
    drop(renderer);

    save_into(&device, &cache, &dir.0);
    let file = dir.0.join(&key);
    assert!(file.exists(), "a save with data writes the file");
    let leftovers = fs::read_dir(&dir.0)
        .expect("the directory is read")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .count();
    assert_eq!(leftovers, 0, "a finished save leaves no temporary file");
    let written = fs::read(&file).expect("the file is read");
    assert!(
        validate(&written, &key).is_some(),
        "the file this process wrote passes its own checks"
    );

    save_into(&device, &cache, &dir.0);
    let rewritten = fs::read(&file).expect("the file is read again");
    assert_eq!(rewritten, written, "a second save writes the same bytes");
    assert!(open_from(&device, &dir.0).is_some(), "the cache reopens");
}

#[test]
fn a_corrupt_cache_file_yields_a_fresh_cache_instead_of_failing() {
    let Some((device, _queue)) = headless_device() else {
        eprintln!("skipping: no GPU adapter available for the pipeline cache");
        return;
    };
    let Some(key) = wgpu::util::pipeline_cache_key(&device.adapter_info()) else {
        eprintln!("skipping: this adapter keeps no application pipeline cache");
        return;
    };
    let dir = scratch("corrupt");
    let mut bytes = serialize(PAYLOAD, &key);
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    let file = dir.0.join(&key);
    fs::write(&file, bytes).expect("the corrupt file is written");
    // The save of ours would have left the file owner-private, so the
    // rejection below comes from the flipped byte, not from loose modes.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600))
            .expect("the file mode is set");
    }
    open_from(&device, &dir.0)
        .expect("a rejected file still gives a cache, so the start only loses its speed");
}
