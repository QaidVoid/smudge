//! The marks the overlay draws over the app, under the panel: the tint over
//! the pixels the app's last frame repainted.

use smudge_core::geometry::{Color, Corners, Rect};
use smudge_core::paint::PaintOp;
use smudge_core::widget::{PaintCx, Widget};

/// The color of the damage tint. It is the color and opacity the window's
/// fixed damage overlay tints with, so the two match pixel for pixel.
const TINT: Color = Color::rgba(255, 64, 64, 96);

/// The widget at the overlay's root, which paints the marks under the panel.
#[derive(Default)]
pub(crate) struct Marks {
    /// The rectangles the app's last frame repainted, in logical pixels,
    /// while the damage tint is on.
    pub(crate) damage: Vec<Rect>,
}

impl Widget for Marks {
    /// Fills each damaged rectangle with the tint.
    fn paint(&self, _cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        for &rect in &self.damage {
            out.push(PaintOp::FillRect {
                rect,
                radius: Corners::all(0.0),
                color: TINT,
            });
        }
    }
}
