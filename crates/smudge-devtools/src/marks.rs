//! The marks the overlay draws over the app, under the panel: the flashes
//! over the pixels the app's recent frames repainted, and the tint over the
//! pixels its last frame repainted.

use smudge_core::geometry::{Color, Corners, Rect};
use smudge_core::paint::PaintOp;
use smudge_core::widget::{PaintCx, Widget};

/// The color of the damage tint. It is the color and opacity the window's
/// fixed damage overlay tints with, so the two match pixel for pixel.
const TINT: Color = Color::rgba(255, 64, 64, 96);

/// The color paint flashing draws with, before each flash's own opacity.
const FLASH: Color = Color::rgb(70, 210, 120);

/// The widget at the overlay's root, which paints the marks under the panel.
#[derive(Default)]
pub(crate) struct Marks {
    /// The rectangles the app's recent frames repainted, in logical pixels,
    /// each with its opacity, while paint flashing is on.
    pub(crate) flashes: Vec<(Rect, u8)>,
    /// The rectangles the app's last frame repainted, in logical pixels,
    /// while the damage tint is on.
    pub(crate) damage: Vec<Rect>,
}

impl Widget for Marks {
    /// Fills each flashed rectangle at its opacity, then each damaged
    /// rectangle with the tint.
    fn paint(&self, _cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        for &(rect, alpha) in &self.flashes {
            out.push(PaintOp::FillRect {
                rect,
                radius: Corners::all(0.0),
                color: Color { a: alpha, ..FLASH },
            });
        }
        for &rect in &self.damage {
            out.push(PaintOp::FillRect {
                rect,
                radius: Corners::all(0.0),
                color: TINT,
            });
        }
    }
}
