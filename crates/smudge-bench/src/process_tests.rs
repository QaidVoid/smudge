//! Tests for the `/proc` parsers behind the machine samples.

use super::{machine_sample, parse_cpu_ticks, parse_load1};

#[test]
fn load1_reads_the_first_number() {
    assert_eq!(parse_load1("0.52 0.49 0.55 1/812 22017"), Some(0.52));
}

#[test]
fn load1_rejects_text_without_a_number() {
    assert_eq!(parse_load1(""), None);
    assert_eq!(parse_load1("none"), None);
}

#[test]
fn cpu_ticks_subtract_idle_and_iowait_from_the_total() {
    // user nice system idle iowait irq softirq steal guest guest_nice
    let (busy, total) = parse_cpu_ticks("cpu  100 0 50 800 0 0 0 0 10 20").expect("parses");
    assert_eq!(total, 950);
    assert_eq!(busy, 150);
}

#[test]
fn cpu_ticks_counts_iowait_as_not_busy() {
    let (busy, total) = parse_cpu_ticks("cpu  100 0 50 800 50 0 0 0").expect("parses");
    assert_eq!(total, 1000);
    assert_eq!(busy, 150);
}

#[test]
fn cpu_ticks_reads_a_second_line_not_at_all() {
    let text = "cpu  100 0 50 800 0 0 0 0\ncpu0 50 0 25 400 0 0 0 0";
    assert_eq!(
        parse_cpu_ticks(text),
        parse_cpu_ticks("cpu  100 0 50 800 0 0 0 0")
    );
}

#[test]
fn cpu_ticks_rejects_a_truncated_line() {
    // Older kernels report user nice system idle only, with no iowait to
    // separate from idle.
    assert_eq!(parse_cpu_ticks("cpu  100 0 50 800"), None);
    assert_eq!(parse_cpu_ticks(""), None);
}

#[test]
fn machine_sample_reads_the_running_kernel() {
    // The values exist only where /proc exists, so the test asks for a
    // reading and checks its shape instead of its magnitude.
    if !std::path::Path::new("/proc/stat").is_file() {
        return;
    }
    let sample = machine_sample();
    assert!(sample.load1.unwrap_or_default() >= 0.0);
    assert!(sample.total_ticks.unwrap_or_default() > 0);
    assert!(sample.busy_ticks.unwrap_or_default() <= sample.total_ticks.unwrap_or_default());
}
