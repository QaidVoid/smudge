//! Tests for the run entry points that need no window.

use smudge_core::app::RepaintMode;
use smudge_core::geometry::{Color, Size};
use smudge_core::widget::panel;

use super::{WindowOptions, run_with};
use crate::PlatformError;

/// Returns options with an opaque clear color, so a test only has to change
/// the field it checks.
fn options(clear_color: Color) -> WindowOptions {
    WindowOptions {
        title: String::new(),
        size: Size {
            width: 100.0,
            height: 100.0,
        },
        clear_color,
        repaint: RepaintMode::Damage,
        show_damage: false,
        stats: None,
        on_frame: None,
        frame_limit: None,
    }
}

#[test]
fn a_translucent_clear_color_fails_before_any_window_opens() {
    let error = run_with(options(Color::rgba(20, 24, 30, 128)), panel, |_| {})
        .expect_err("a clear color that is not opaque must be rejected");

    assert!(matches!(error, PlatformError::ClearColor));
}
