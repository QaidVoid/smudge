use crate::damage::DamageAccumulator;
use crate::geometry::{Color, Corners, PixelRect, Rect, Size};
use crate::paint::PaintOp;
use crate::style::{Align, Length, Overflow, Style};
use crate::tree::WidgetTree;
use crate::widget::{PaintCx, Panel, Widget};

/// A widget that paints a fixed 30 by 10 mark at its origin whatever its size,
/// the way text paints from the top left of its node.
struct Mark;

impl Widget for Mark {
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        out.push(PaintOp::FillRect {
            rect: Rect::new(cx.rect.x0, cx.rect.y0, cx.rect.x0 + 30.0, cx.rect.y0 + 10.0),
            radius: Corners::ZERO,
            color: Color::BLACK,
        });
    }
}

/// The viewport every test lays its tree out for.
const VIEWPORT: Size = Size {
    width: 200.0,
    height: 100.0,
};

/// A style with a fixed size and nothing else.
fn fixed(width: f32, height: f32) -> Style {
    let mut style = Style::default();
    style.layout.width = Length::px(width);
    style.layout.height = Length::px(height);
    style
}

/// A style with a fixed size and a background, which paints one rectangle.
fn boxed(width: f32, height: f32) -> Style {
    let mut style = fixed(width, height);
    style.paint.background = Some(Color::BLACK);
    style
}

/// A style that packs its children against the start of both axes, so their
/// positions do not depend on how the engine distributes leftover space.
fn packing() -> Style {
    let mut style = Style::default();
    style.layout.align_items = Some(Align::Start);
    style
}

/// Paints the tree as a frame does, returning how many nodes painted and the
/// physical damage, and leaves the accumulator empty for the next frame.
fn paint_frame(tree: &mut WidgetTree, damage: &mut DamageAccumulator) -> (u32, Vec<PixelRect>) {
    let painted = tree.paint(damage);
    let rects = damage
        .finish(1.0, VIEWPORT.width as u32, VIEWPORT.height as u32)
        .to_vec();
    damage.clear();
    (painted, rects)
}

/// Returns `true` when one of the damaged rectangles covers `rect` entirely.
fn covers(damage: &[PixelRect], rect: PixelRect) -> bool {
    damage.iter().any(|out| out.contains_rect(rect))
}

#[test]
fn changing_a_box_color_damages_exactly_its_rect() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Panel), packing());
    let mut style = boxed(50.0, 20.0);
    style.layout.margin.left = Length::px(30.0);
    style.layout.margin.top = Length::px(10.0);
    let card = tree.insert(Some(root), Box::new(Panel), style);

    let mut damage = DamageAccumulator::new();
    tree.layout(VIEWPORT);
    let (_, first) = paint_frame(&mut tree, &mut damage);
    let rect = PixelRect {
        x: 30,
        y: 10,
        width: 50,
        height: 20,
    };
    assert_eq!(first, vec![rect], "the box paints its own rectangle");

    tree.apply_paint(card, |paint| paint.background = Some(Color::WHITE));
    let (painted, second) = paint_frame(&mut tree, &mut damage);

    assert_eq!(painted, 1, "only the changed box repaints");
    assert_eq!(
        second,
        vec![rect],
        "the box repaints exactly where it painted before"
    );
}

#[test]
fn moving_a_box_damages_its_old_and_new_rects() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Panel), packing());
    let first = tree.insert(Some(root), Box::new(Panel), boxed(180.0, 20.0));
    tree.insert(Some(root), Box::new(Panel), boxed(20.0, 20.0));

    let mut damage = DamageAccumulator::new();
    tree.layout(VIEWPORT);
    paint_frame(&mut tree, &mut damage);

    tree.apply_layout(first, |layout| layout.width = Length::px(20.0));
    assert!(tree.layout(VIEWPORT), "the narrower box runs layout again");
    let (painted, moved) = paint_frame(&mut tree, &mut damage);

    assert_eq!(painted, 2, "the resized box and the shifted box repaint");
    let old = PixelRect {
        x: 180,
        y: 0,
        width: 20,
        height: 20,
    };
    let new = PixelRect {
        x: 20,
        y: 0,
        width: 20,
        height: 20,
    };
    assert!(
        covers(&moved, old),
        "the area the shifted box left behind is damaged, got {moved:?}"
    );
    assert!(
        covers(&moved, new),
        "the area the shifted box moved to is damaged, got {moved:?}"
    );
}

#[test]
fn a_child_that_overflows_a_clipping_parent_is_damaged_only_inside_the_clip() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Panel), packing());
    let mut style = fixed(80.0, 40.0);
    style.layout.overflow = Overflow::Clip;
    style.layout.margin.left = Length::px(20.0);
    style.layout.margin.top = Length::px(10.0);
    let frame = tree.insert(Some(root), Box::new(Panel), style);
    tree.insert(Some(frame), Box::new(Panel), boxed(200.0, 200.0));

    let mut damage = DamageAccumulator::new();
    tree.layout(VIEWPORT);
    let (_, first) = paint_frame(&mut tree, &mut damage);

    assert_eq!(
        first,
        vec![PixelRect {
            x: 20,
            y: 10,
            width: 80,
            height: 40,
        }],
        "the overflowing child damages only the part inside the parent's clip"
    );
}

#[test]
fn removing_a_node_damages_its_last_painted_bounds() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Panel), packing());
    let card = tree.insert(Some(root), Box::new(Panel), boxed(50.0, 20.0));

    let mut damage = DamageAccumulator::new();
    tree.layout(VIEWPORT);
    paint_frame(&mut tree, &mut damage);

    tree.remove(card);
    let (_, removed) = paint_frame(&mut tree, &mut damage);

    assert_eq!(
        removed,
        vec![PixelRect {
            x: 0,
            y: 0,
            width: 50,
            height: 20,
        }],
        "the pixels the removed node painted are damaged"
    );
}

#[test]
fn a_frame_with_no_dirty_nodes_paints_nothing() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Panel), packing());
    tree.insert(Some(root), Box::new(Panel), boxed(50.0, 20.0));

    let mut damage = DamageAccumulator::new();
    tree.layout(VIEWPORT);
    let (painted, first) = paint_frame(&mut tree, &mut damage);
    assert_eq!(painted, 2, "the first frame paints the root and the box");
    assert!(!first.is_empty(), "the first frame damages what it painted");

    let (painted, second) = paint_frame(&mut tree, &mut damage);

    assert_eq!(painted, 0, "a clean tree paints no node");
    assert!(
        second.is_empty(),
        "a clean tree damages nothing, got {second:?}"
    );
}

#[test]
fn hiding_a_node_damages_only_where_it_painted() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Panel), packing());
    let mut style = fixed(50.0, 20.0);
    style.layout.margin.left = Length::px(100.0);
    style.layout.margin.top = Length::px(50.0);
    let mark = tree.insert(Some(root), Box::new(Mark), style);

    let mut damage = DamageAccumulator::new();
    tree.layout(VIEWPORT);
    paint_frame(&mut tree, &mut damage);

    tree.apply_layout(mark, |layout| layout.hidden = true);
    tree.layout(VIEWPORT);
    let (_, hidden) = paint_frame(&mut tree, &mut damage);

    assert_eq!(
        hidden,
        vec![PixelRect {
            x: 100,
            y: 50,
            width: 30,
            height: 10,
        }],
        "the hidden node clears its old pixels and paints nowhere else"
    );
}
