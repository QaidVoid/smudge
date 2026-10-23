use crate::geometry::{Color, Corners, Rect};
use crate::paint::PaintOp;
use crate::style::{Edges, Style};
use crate::widget::{PaintCx, Panel, Widget};

/// The rectangle the tests paint into.
fn rect() -> Rect {
    Rect::new(0.0, 0.0, 40.0, 20.0)
}

/// Paints a panel with `style` and returns the operations it emitted.
fn paint(style: &Style) -> Vec<PaintOp> {
    let mut ops = Vec::new();
    Panel.paint(
        &PaintCx {
            rect: rect(),
            style,
        },
        &mut ops,
    );
    ops
}

#[test]
fn a_panel_paints_its_background_and_border() {
    let mut style = Style::default();
    style.paint.background = Some(Color::WHITE);
    style.paint.border_color = Some(Color::BLACK);
    style.paint.corner_radius = Corners::all(4.0);
    style.layout.border_width = Edges::all(2.0);

    assert_eq!(
        paint(&style),
        vec![
            PaintOp::FillRect {
                rect: rect(),
                radius: Corners::all(4.0),
                color: Color::WHITE,
            },
            PaintOp::StrokeRect {
                rect: rect(),
                radius: Corners::all(4.0),
                width: 2.0,
                color: Color::BLACK,
            },
        ]
    );
}

#[test]
fn a_panel_without_paint_properties_paints_nothing() {
    assert!(paint(&Style::default()).is_empty());
}

#[test]
fn a_panel_border_with_different_sides_uses_the_widest() {
    let mut style = Style::default();
    style.paint.border_color = Some(Color::BLACK);
    style.layout.border_width = Edges {
        top: 1.0,
        right: 4.0,
        bottom: 2.0,
        left: 3.0,
    };

    assert_eq!(
        paint(&style),
        vec![PaintOp::StrokeRect {
            rect: rect(),
            radius: Corners::ZERO,
            width: 4.0,
            color: Color::BLACK,
        }]
    );
}

#[test]
fn a_panel_border_without_width_paints_nothing() {
    let mut style = Style::default();
    style.paint.border_color = Some(Color::BLACK);

    assert!(
        paint(&style).is_empty(),
        "a border color with no width is invisible"
    );
}
