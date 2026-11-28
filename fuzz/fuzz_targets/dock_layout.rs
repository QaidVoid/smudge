//! Checks normalization of docking layouts decoded from JSON.
//!
//! Accepted layouts must have unique panel keys, valid groups and ratios,
//! and finite floats with the model's 160 by 96 logical pixel minimum.
//! Reconciliation must leave exactly the requested keys and be idempotent.
//! Saving and loading the reconciled layout must preserve it exactly.

#![no_main]

use std::collections::HashSet;

use libfuzzer_sys::fuzz_target;
use smudge_widgets::dock::{Branch, DockLayout, DockNode};

fn check_tree<'a>(node: &'a DockNode, seen: &mut HashSet<&'a str>) {
    match node {
        DockNode::Group { panels, active } => {
            assert!(panels.contains(active));
            assert!(panels.iter().all(|panel| seen.insert(panel)));
        }
        DockNode::Split {
            ratio,
            first,
            second,
            ..
        } => {
            assert!((0.0..=1.0).contains(ratio));
            check_tree(first, seen);
            check_tree(second, seen);
        }
    }
}

fn check_layout(layout: &DockLayout) -> HashSet<&str> {
    let mut seen = HashSet::new();
    if let Some(root) = layout.root() {
        check_tree(root, &mut seen);
    }
    for floating in layout.floating() {
        assert!(seen.insert(floating.panel()));
        let rect = floating.rect();
        assert!(rect.width().is_finite() && rect.width() >= 160.0);
        assert!(rect.height().is_finite() && rect.height() >= 96.0);
    }
    seen
}

fuzz_target!(|bytes: &[u8]| {
    let Ok(mut layout) = serde_json::from_slice::<DockLayout>(bytes) else {
        return;
    };
    check_layout(&layout);
    let known = ["editor", "outline", "files", "search", "outline"];
    let fallback = [Branch::Second];
    layout.reconcile(&known, &fallback);
    assert_eq!(check_layout(&layout), known.into_iter().collect());
    let saved = serde_json::to_vec(&layout).expect("a normalized layout can be saved");
    let mut restored: DockLayout =
        serde_json::from_slice(&saved).expect("a saved layout can be loaded");
    assert_eq!(restored, layout);
    restored.reconcile(&known, &fallback);
    assert_eq!(restored, layout);
});
