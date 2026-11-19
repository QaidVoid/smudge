//! The marks the overlay draws over the app, under the panel: the flashes
//! over the pixels the app's recent frames repainted, the tint over the
//! pixels its last frame repainted, and the outlines of its nodes.

use smudge_core::geometry::{Color, Corners, Rect};
use std::rc::Rc;

use smudge_core::inspect::{BoxModel, FocusInfo, PointerInfo};
use smudge_core::paint::PaintOp;
use smudge_core::widget::{PaintCx, Widget};

/// The color of the damage tint. It is the color and opacity the window's
/// fixed damage overlay tints with, so the two match pixel for pixel.
const TINT: Color = Color::rgba(255, 64, 64, 96);

/// The color paint flashing draws with, before each flash's own opacity.
const FLASH: Color = Color::rgb(70, 210, 120);

/// The colors layout bounds outlines with, one per depth in the tree,
/// starting again after the last.
const DEPTHS: [Color; 6] = [
    Color::rgb(255, 120, 80),
    Color::rgb(250, 200, 60),
    Color::rgb(120, 220, 90),
    Color::rgb(60, 200, 230),
    Color::rgb(120, 130, 255),
    Color::rgb(230, 110, 230),
];

/// The colors of the highlighted node's margin, border, padding and
/// content boxes, which stack from the outside in.
const BOXES: [Color; 4] = [
    Color::rgba(246, 178, 107, 110),
    Color::rgba(255, 229, 153, 110),
    Color::rgba(147, 196, 125, 110),
    Color::rgba(111, 168, 220, 140),
];

/// The color of the tab stops and of the focused node.
const FOCUS: Color = Color::rgb(80, 160, 255);

/// The color of the focus trap that holds the keyboard.
const TRAP: Color = Color::rgb(190, 110, 255);

/// The color of the nodes that take a press.
const PRESSABLE: Color = Color::rgb(90, 200, 120);

/// The color of the hovered node and its ancestors.
const HOVERED: Color = Color::rgb(80, 160, 255);

/// The fill of the node a held press began on.
const PRESSED: Color = Color::rgba(255, 80, 80, 90);

/// Pushes a stroke of `width` around `rect` in `color`.
fn stroke(out: &mut Vec<PaintOp>, rect: Rect, width: f32, color: Color) {
    out.push(PaintOp::StrokeRect {
        rect,
        radius: Corners::all(0.0),
        width,
        color,
    });
}

/// The widget at the overlay's root, which paints the marks under the panel.
#[derive(Default)]
pub(crate) struct Marks {
    /// The rectangles the app's recent frames repainted, in logical pixels,
    /// each with its opacity, while paint flashing is on.
    pub(crate) flashes: Vec<(Rect, u8)>,
    /// The rectangles of the app's nodes, in logical pixels, each with its
    /// depth, while layout bounds is on.
    pub(crate) bounds: Vec<(usize, Rect)>,
    /// The boxes of the node the inspector highlights.
    pub(crate) highlight: Option<BoxModel>,
    /// Where the app's focus is, while the focus overlay is on.
    pub(crate) focus: Option<Rc<FocusInfo>>,
    /// What the app's pointer reaches, while the hit testing overlay is on.
    pub(crate) pointer: Option<Rc<PointerInfo>>,
    /// The rectangles the app's last frame repainted, in logical pixels,
    /// while the damage tint is on.
    pub(crate) damage: Vec<Rect>,
}

impl Widget for Marks {
    /// Fills each flashed rectangle at its opacity, then each damaged
    /// rectangle with the tint, then outlines each node's rectangle in the
    /// color of its depth, then fills the highlighted node's boxes from the
    /// outside in.
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
        for &(depth, rect) in &self.bounds {
            out.push(PaintOp::StrokeRect {
                rect,
                radius: Corners::all(0.0),
                width: 1.0,
                color: DEPTHS[depth % DEPTHS.len()],
            });
        }
        if let Some(boxes) = self.highlight {
            let rects = [boxes.margin, boxes.border, boxes.padding, boxes.content];
            for (rect, color) in rects.into_iter().zip(BOXES) {
                out.push(PaintOp::FillRect {
                    rect,
                    radius: Corners::all(0.0),
                    color,
                });
            }
        }
        if let Some(focus) = &self.focus {
            for &(_, rect) in &focus.order {
                stroke(out, rect, 1.0, FOCUS);
            }
            if let Some(trap) = focus.trap {
                stroke(out, trap, 2.0, TRAP);
            }
            if let Some(focused) = focus.focused
                && let Some(&(_, rect)) = focus.order.iter().find(|(id, _)| *id == focused)
            {
                stroke(out, rect, 3.0, FOCUS);
            }
        }
        if let Some(pointer) = &self.pointer {
            for &rect in &pointer.pressable {
                stroke(out, rect, 1.0, PRESSABLE);
            }
            for &rect in &pointer.hovered {
                stroke(out, rect, 2.0, HOVERED);
            }
            if let Some(rect) = pointer.pressed {
                out.push(PaintOp::FillRect {
                    rect,
                    radius: Corners::all(0.0),
                    color: PRESSED,
                });
            }
        }
    }
}
