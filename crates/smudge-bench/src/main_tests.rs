//! Tests for the machine load readings of a repeat summary.

use super::{busy_share, process_ticks};
use crate::process::{MachineSample, Sample};

fn machine(busy: u64, total: u64) -> MachineSample {
    MachineSample {
        load1: None,
        busy_ticks: Some(busy),
        total_ticks: Some(total),
        package_energy_uj: None,
        package_energy_range_uj: None,
    }
}

fn process(cpu_time_s: f64) -> Sample {
    Sample {
        peak_rss_kb: None,
        cpu_time_s: Some(cpu_time_s),
    }
}

#[test]
fn process_time_converts_to_the_machine_counters_unit() {
    assert_eq!(process_ticks(process(1.20), process(2.04)), Some(84));
}

#[test]
fn other_busy_share_leaves_out_the_process_own_ticks() {
    let (start, end) = (machine(1_000, 10_000), machine(2_300, 11_355));
    let whole = busy_share(start, end, 0).expect("a share");
    let others = busy_share(start, end, 84).expect("a share");
    assert!((whole - 1_300.0 / 1_355.0).abs() < 1e-12);
    assert!((others - 1_216.0 / 1_355.0).abs() < 1e-12);
}

#[test]
fn other_busy_share_never_goes_below_zero() {
    // The process counter and the machine counters are read a moment apart,
    // so the process can show a tick the machine has not counted yet.
    let share = busy_share(machine(0, 0), machine(3, 40), 4).expect("a share");
    assert_eq!(share, 0.0);
}

#[test]
fn busy_share_is_unknown_without_elapsed_ticks() {
    assert_eq!(busy_share(machine(5, 50), machine(5, 50), 0), None);
}
