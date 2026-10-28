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

/// The clock ticks per second of `/proc`'s processor counters, USER_HZ, which
/// the Linux user space ABI fixes at 100.
pub(crate) const TICKS_PER_SECOND: f64 = 100.0;

/// The machine's load and processor counters at one moment.
///
/// Everything reads the operating system's own files, so a value this machine
/// does not provide reads as `None`.
#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct MachineSample {
    /// The one-minute load average.
    pub load1: Option<f64>,
    /// The processor ticks spent busy since boot, from `/proc/stat`.
    pub busy_ticks: Option<u64>,
    /// The processor ticks spent on anything since boot, from `/proc/stat`.
    pub total_ticks: Option<u64>,
    /// The processor package's energy counter since boot, in microjoules,
    /// from the kernel's RAPL powercap zone.
    pub package_energy_uj: Option<u64>,
    /// The counter value at which [`MachineSample::package_energy_uj`]
    /// wraps back to zero, in microjoules.
    pub package_energy_range_uj: Option<u64>,
}

/// Reads the machine's load and processor counters.
pub(crate) fn machine_sample() -> MachineSample {
    let ticks = cpu_ticks();
    let energy = package_energy();
    MachineSample {
        load1: load1(),
        busy_ticks: ticks.map(|(busy, _)| busy),
        total_ticks: ticks.map(|(_, total)| total),
        package_energy_uj: energy.0,
        package_energy_range_uj: energy.1,
    }
}

/// Returns the energy the package used between two readings, in microjoules.
///
/// The counter wraps at `range`, so an end below the start is one wrap and
/// the reading spans the range's top. A repeat that crossed the range twice
/// would underreport as one wrap, which repeats of a few seconds do not
/// reach against a range of tens of kilojoules. A missing reading, a zero
/// range or a start beyond the range gives `None`.
pub(crate) fn package_energy_between(
    start: Option<u64>,
    end: Option<u64>,
    range: Option<u64>,
) -> Option<u64> {
    let range = range?;
    let start = start?;
    let end = end?;
    if range == 0 || start > range || end > range {
        return None;
    }
    if end >= start {
        return Some(end - start);
    }
    range.checked_sub(start)?.checked_add(end)
}

/// The powercap zone whose counter [`package_energy`] reads.
///
/// Zone 0 of the kernel's RAPL driver is the whole processor package, the
/// integrated GPU of a package sharing one. On machines the driver does not
/// cover, or where this user may not read the counter, the reading is
/// `None`.
const PACKAGE_ENERGY_ZONE: &str = "/sys/class/powercap/intel-rapl:0";

/// Returns the package energy counter and its wrap range, in microjoules.
///
/// The counter covers the whole package, so other processes' draw is in it
/// too, and wraps at the range the kernel reports beside it.
fn package_energy() -> (Option<u64>, Option<u64>) {
    // A top-level RAPL zone names itself `package-<n>`; its subzones count
    // one domain only, such as the cores, and must not pass for the package.
    if let Ok(name) = std::fs::read_to_string(format!("{PACKAGE_ENERGY_ZONE}/name"))
        && !name.starts_with("package")
    {
        return (None, None);
    }
    (
        read_zone_uj("energy_uj"),
        read_zone_uj("max_energy_range_uj"),
    )
}

/// Reads one microjoule counter out of [`PACKAGE_ENERGY_ZONE`].
fn read_zone_uj(file: &str) -> Option<u64> {
    std::fs::read_to_string(format!("{PACKAGE_ENERGY_ZONE}/{file}"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Returns the one-minute load average from `/proc/loadavg`.
fn load1() -> Option<f64> {
    parse_load1(&std::fs::read_to_string("/proc/loadavg").ok()?)
}

/// Parses the one-minute load average, the first number of the text.
fn parse_load1(text: &str) -> Option<f64> {
    text.split_whitespace().next()?.parse().ok()
}

/// Returns the busy and the total processor ticks since boot, from the first
/// line of `/proc/stat`.
///
/// Busy is everything but idle and iowait, so a machine waiting on disks
/// counts as busy, which is the reading a contaminated run must give. The
/// guest columns are left out: the kernel already counts their time as user
/// time, and summing them would count it twice.
fn cpu_ticks() -> Option<(u64, u64)> {
    parse_cpu_ticks(&std::fs::read_to_string("/proc/stat").ok()?)
}

/// Parses the busy and the total processor ticks out of a `/proc/stat` text.
fn parse_cpu_ticks(text: &str) -> Option<(u64, u64)> {
    let line = text.lines().next()?;
    let columns: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(|column| column.parse().ok())
        .collect::<Option<_>>()?;
    if columns.len() < 5 {
        return None;
    }
    let idle = columns[3] + columns[4];
    let total: u64 = columns.iter().sum();
    Some((total - idle, total))
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;

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
    Some((user_ticks + system_ticks) as f64 / TICKS_PER_SECOND)
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
