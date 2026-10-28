//! Tests for the `/proc` and powercap parsers behind the machine samples.

use super::{machine_sample, package_energy_between, parse_cpu_ticks, parse_load1};

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
    // A counter that exists always reports the range it wraps at.
    if sample.package_energy_uj.is_some() {
        assert!(sample.package_energy_range_uj.unwrap_or_default() > 0);
    }
}

#[test]
fn package_energy_between_subtracts_without_a_wrap() {
    assert_eq!(
        package_energy_between(Some(100), Some(250), Some(1_000)),
        Some(150)
    );
}

#[test]
fn package_energy_between_spans_the_range_on_one_wrap() {
    // The counter ran from 900 past 1000, wrapped to 0 and ended at 100:
    // 100 microjoules up to the top, 100 from the bottom.
    assert_eq!(
        package_energy_between(Some(900), Some(100), Some(1_000)),
        Some(200)
    );
}

#[test]
fn package_energy_between_takes_an_unchanged_counter_as_no_energy() {
    assert_eq!(
        package_energy_between(Some(500), Some(500), Some(1_000)),
        Some(0)
    );
}

#[test]
fn package_energy_between_needs_both_readings_and_a_range() {
    assert_eq!(package_energy_between(None, Some(100), Some(1_000)), None);
    assert_eq!(package_energy_between(Some(100), None, Some(1_000)), None);
    assert_eq!(package_energy_between(Some(100), Some(200), None), None);
}

#[test]
fn package_energy_between_rejects_a_useless_range() {
    assert_eq!(package_energy_between(Some(100), Some(200), Some(0)), None);
    // A start beyond the range is a counter this arithmetic cannot trust.
    assert_eq!(
        package_energy_between(Some(2_000), Some(2_100), Some(1_000)),
        None
    );
}
