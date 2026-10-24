//! Process and machine information for the run's environment header.
//!
//! Everything here reads the operating system's own files and programs. A
//! value this platform does not provide reads as `None`, which the output
//! writes as JSON `null`.

/// The process's resource use at one moment.
#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct Sample {
    /// The largest resident set size the process reached so far, in kB.
    pub peak_rss_kb: Option<u64>,
    /// The processor time the process has used so far, in seconds.
    pub cpu_time_s: Option<f64>,
}

/// Reads the process's resource use.
pub(crate) fn sample() -> Sample {
    Sample {
        peak_rss_kb: peak_rss_kb(),
        cpu_time_s: cpu_time_s(),
    }
}

/// Returns the model name of the first processor `/proc/cpuinfo` describes.
pub(crate) fn cpu_model() -> Option<String> {
    let status = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    find_field(&status, "model name")
}

/// Returns the running kernel's release.
pub(crate) fn kernel_release() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|release| release.trim().to_string())
}

/// Returns the peak resident set size from `/proc/self/status`, in kB.
fn peak_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    find_field(&status, "VmHWM")?
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()
}

/// Returns the processor time the process has used, in seconds.
///
/// The ticks of `/proc/self/stat` are counted in USER_HZ, which the Linux
/// user space ABI fixes at 100.
fn cpu_time_s() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let user_ticks: u64 = fields.get(11)?.parse().ok()?;
    let system_ticks: u64 = fields.get(12)?.parse().ok()?;
    Some((user_ticks + system_ticks) as f64 / 100.0)
}

/// Returns the value of the first field named `name` in a `/proc` file.
///
/// Fields are lines of the form `name : value`.
fn find_field(contents: &str, name: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let (field, value) = line.split_once(':')?;
        field
            .trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    })
}
