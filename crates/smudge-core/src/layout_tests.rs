use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::damage::DamageAccumulator;
use crate::geometry::{Color, Corners, PixelRect, Point, Rect, Size};
use crate::paint::PaintOp;
use crate::reactive::{RwSignal, Set};
use crate::style::{Align, Direction, Edges, Length, Overflow, Position, Style};
use crate::tree::{Dirty, WidgetId, WidgetTree};
use crate::widget::{AvailableSpace, MeasureInput, PaintCx, Panel, Widget};

/// A widget with no measurement and no paint.
struct Empty;

impl Widget for Empty {
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}
}

/// A widget that always measures the same size.
struct Fixed(f32, f32);

impl Widget for Fixed {
    fn measure(&mut self, _input: MeasureInput) -> Option<Size> {
        Some(Size {
            width: self.0,
            height: self.1,
        })
    }

    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}
}

/// A widget that records the width it is offered and measures 10 by 10.
struct WidthRecorder(Rc<RefCell<Vec<AvailableSpace>>>);

impl Widget for WidthRecorder {
    fn measure(&mut self, input: MeasureInput) -> Option<Size> {
        self.0.borrow_mut().push(input.available_width);
        Some(Size {
            width: 10.0,
            height: 10.0,
        })
    }

    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}
}

/// A widget that records every measure input and measures 10 by 10.
struct InputRecorder(Rc<RefCell<Vec<MeasureInput>>>);

impl Widget for InputRecorder {
    fn measure(&mut self, input: MeasureInput) -> Option<Size> {
        self.0.borrow_mut().push(input);
        Some(Size {
            width: 10.0,
            height: 10.0,
        })
    }

    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}
}

/// A style with a fixed size and nothing else.
fn fixed(width: f32, height: f32) -> Style {
    let mut style = Style::default();
    style.layout.width = Length::px(width);
    style.layout.height = Length::px(height);
    style
}

/// Writes one `(depth, rect)` line per node, parents before children.
fn dump(tree: &WidgetTree, id: WidgetId, depth: usize, out: &mut String) {
    let rect = tree.rect(id).expect("the node is live");
    out.push_str(&format!(
        "{depth}: ({}, {}) {}x{}\n",
        rect.x0,
        rect.y0,
        rect.width(),
        rect.height()
    ));
    for &child in tree.children(id).expect("the node is live") {
        dump(tree, child, depth + 1, out);
    }
}

#[test]
fn a_paint_only_change_skips_layout() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(200.0, 100.0));
    let child = tree.insert(Some(root), Box::new(Empty), fixed(50.0, 20.0));

    assert!(
        tree.layout(Size {
            width: 200.0,
            height: 100.0,
        }),
        "the first pass runs the layout engine"
    );
    tree.clear_dirty();
    let runs = tree.layout_runs();

    tree.apply_paint(child, |paint| paint.background = Some(Color::BLACK.into()));

    assert!(
        !tree.layout(Size {
            width: 200.0,
            height: 100.0,
        }),
        "a paint change does not run the layout engine"
    );
    assert_eq!(tree.layout_runs(), runs, "the engine did not run again");
}

#[test]
fn widening_the_first_child_moves_and_marks_its_sibling() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(300.0, 100.0));
    let first = tree.insert(Some(root), Box::new(Empty), fixed(50.0, 20.0));
    let second = tree.insert(Some(root), Box::new(Empty), fixed(50.0, 20.0));

    tree.layout(Size {
        width: 300.0,
        height: 100.0,
    });
    tree.clear_dirty();
    assert_eq!(tree.rect(second).expect("the node is live").x0, 50.0);

    tree.apply_layout(first, |layout| layout.width = Length::px(120.0));
    assert!(tree.layout(Size {
        width: 300.0,
        height: 100.0,
    }));

    assert_eq!(
        tree.rect(second).expect("the node is live").x0,
        120.0,
        "the sibling starts after the wider child"
    );
    assert_eq!(
        tree.dirty(second),
        Some(Dirty::Paint),
        "the moved sibling must repaint"
    );
    tree.debug_assert_consistent();
}

#[test]
fn a_leaf_gets_its_measured_size() {
    let mut tree = WidgetTree::new();
    let mut root_style = fixed(200.0, 100.0);
    root_style.layout.align_items = Some(Align::Start);
    let root = tree.insert(None, Box::new(Empty), root_style);
    let leaf = tree.insert(Some(root), Box::new(Fixed(30.0, 12.0)), Style::default());

    tree.layout(Size {
        width: 200.0,
        height: 100.0,
    });

    assert_eq!(
        tree.rect(leaf),
        Some(Rect::new(0.0, 0.0, 30.0, 12.0)),
        "the leaf takes the size its widget measured"
    );
}

#[test]
fn nested_rects_match_the_snapshot() {
    let mut tree = WidgetTree::new();
    let mut root_style = Style::default();
    root_style.layout.direction = Direction::Column;
    root_style.layout.padding = Edges::all(Length::px(10.0));
    let root = tree.insert(None, Box::new(Empty), root_style);

    let mut row_style = Style::default();
    row_style.layout.padding = Edges::all(Length::px(5.0));
    let row = tree.insert(Some(root), Box::new(Empty), row_style);
    tree.insert(Some(row), Box::new(Fixed(30.0, 10.0)), Style::default());
    tree.insert(Some(row), Box::new(Fixed(20.0, 10.0)), Style::default());

    tree.layout(Size {
        width: 200.0,
        height: 100.0,
    });

    let mut out = String::new();
    dump(&tree, root, 0, &mut out);
    insta::assert_snapshot!("nested_rects", out);
}

#[test]
fn a_child_of_a_clipping_parent_is_clipped_to_the_parent() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(200.0, 100.0));
    let mut parent_style = fixed(80.0, 40.0);
    parent_style.layout.overflow = Overflow::Clip;
    let parent = tree.insert(Some(root), Box::new(Empty), parent_style);
    let child = tree.insert(Some(parent), Box::new(Empty), fixed(200.0, 200.0));

    tree.layout(Size {
        width: 200.0,
        height: 100.0,
    });

    let parent_rect = tree.rect(parent).expect("the node is live");
    let parent_clip = tree.clip(parent).expect("the node is live");
    assert_eq!(
        parent_clip,
        Rect::new(0.0, 0.0, 200.0, 100.0),
        "the root clip is the viewport"
    );
    assert_eq!(
        tree.clip(child),
        Some(parent_rect.intersect(parent_clip)),
        "the child is clipped to the parent rectangle inside the parent clip"
    );
}

#[test]
fn a_child_is_clipped_by_every_clipping_ancestor() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(200.0, 100.0));

    let mut outer_style = fixed(100.0, 100.0);
    outer_style.layout.overflow = Overflow::Clip;
    let outer = tree.insert(Some(root), Box::new(Empty), outer_style);

    let mut inner_style = fixed(100.0, 100.0);
    inner_style.layout.overflow = Overflow::Clip;
    inner_style.layout.margin.left = Length::px(60.0);
    let inner = tree.insert(Some(outer), Box::new(Empty), inner_style);

    let leaf = tree.insert(Some(inner), Box::new(Empty), fixed(400.0, 400.0));

    tree.layout(Size {
        width: 200.0,
        height: 100.0,
    });

    let inner_rect = tree.rect(inner).expect("the node is live");
    let inner_clip = tree.clip(inner).expect("the node is live");
    assert_eq!(
        tree.clip(leaf),
        Some(inner_rect.intersect(inner_clip)),
        "the leaf is clipped by the outer rectangle and the inner one"
    );
}

#[test]
fn a_clipping_parent_clips_its_children_inside_its_border() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(200.0, 100.0));
    let mut parent_style = fixed(80.0, 40.0);
    parent_style.layout.overflow = Overflow::Clip;
    parent_style.layout.border_width = Edges {
        top: 1.0,
        right: 2.0,
        bottom: 3.0,
        left: 4.0,
    };
    let parent = tree.insert(Some(root), Box::new(Empty), parent_style);
    let child = tree.insert(Some(parent), Box::new(Empty), fixed(200.0, 200.0));

    tree.layout(Size {
        width: 200.0,
        height: 100.0,
    });

    assert_eq!(
        tree.clip(child),
        Some(Rect::new(4.0, 1.0, 78.0, 37.0)),
        "an overflowing child never covers the parent's border"
    );
}

/// A tree whose scrolled node sits 30 pixels right and 10 pixels down from
/// the root, with a flowing child and an absolutely positioned one, laid out
/// once. Returns the tree, the root, the scrolled node and its two children.
fn scrolled_tree() -> (WidgetTree, WidgetId, WidgetId, WidgetId, WidgetId) {
    let mut tree = WidgetTree::new();
    let mut root_style = fixed(200.0, 100.0);
    root_style.layout.direction = Direction::Column;
    let root = tree.insert(None, Box::new(Empty), root_style);
    tree.insert(Some(root), Box::new(Empty), fixed(10.0, 10.0));
    let mut scrolled_style = fixed(80.0, 40.0);
    scrolled_style.layout.margin.left = Length::px(30.0);
    scrolled_style.layout.overflow = Overflow::Clip;
    let scrolled = tree.insert(Some(root), Box::new(Empty), scrolled_style);
    let flowing = tree.insert(Some(scrolled), Box::new(Empty), fixed(80.0, 100.0));
    let mut absolute_style = fixed(4.0, 10.0);
    absolute_style.layout.position = Position::Absolute;
    absolute_style.layout.inset.top = Length::px(0.0);
    absolute_style.layout.inset.right = Length::px(0.0);
    let absolute = tree.insert(Some(scrolled), Box::new(Empty), absolute_style);
    tree.layout(Size {
        width: 200.0,
        height: 100.0,
    });
    tree.clear_dirty();
    (tree, root, scrolled, flowing, absolute)
}

#[test]
fn a_scroll_offset_moves_flowing_children_without_running_layout() {
    let (mut tree, _, scrolled, flowing, absolute) = scrolled_tree();
    let runs = tree.layout_runs();

    tree.set_scroll_offset(scrolled, Point { x: 0.0, y: 25.0 });

    assert_eq!(
        tree.rect(scrolled),
        Some(Rect::new(30.0, 10.0, 110.0, 50.0)),
        "the scrolled node itself stays where the layout put it"
    );
    assert_eq!(
        tree.rect(flowing),
        Some(Rect::new(30.0, -15.0, 110.0, 85.0)),
        "the flowing child moved up by the offset"
    );
    assert_eq!(
        tree.rect(absolute),
        Some(Rect::new(106.0, 10.0, 110.0, 20.0)),
        "the absolutely positioned child stays where its insets put it"
    );
    assert_eq!(
        tree.clip(flowing),
        Some(Rect::new(30.0, 10.0, 110.0, 50.0)),
        "the moved child is still clipped to the scrolled node"
    );
    assert_eq!(tree.layout_runs(), runs, "the engine did not run");
}

#[test]
fn a_scroll_offset_set_under_a_hidden_ancestor_lands_when_it_shows() {
    let (mut tree, root, scrolled, flowing, _) = scrolled_tree();
    let viewport = Size {
        width: 200.0,
        height: 100.0,
    };
    tree.apply_layout(root, |layout| layout.hidden = true);
    tree.layout(viewport);

    tree.set_scroll_offset(scrolled, Point { x: 0.0, y: 25.0 });

    assert_eq!(
        tree.rect(flowing),
        Some(Rect::default()),
        "a hidden subtree keeps the rectangles the layout pass gave it"
    );

    tree.apply_layout(root, |layout| layout.hidden = false);
    tree.layout(viewport);

    assert_eq!(
        tree.rect(flowing),
        Some(Rect::new(30.0, -15.0, 110.0, 85.0)),
        "the offset set while hidden applies once the node shows"
    );
}

#[test]
fn a_hidden_node_and_its_subtree_get_empty_rects() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(200.0, 100.0));
    let mut hidden_style = fixed(50.0, 20.0);
    hidden_style.layout.hidden = true;
    let hidden = tree.insert(Some(root), Box::new(Empty), hidden_style);
    let child = tree.insert(Some(hidden), Box::new(Empty), fixed(50.0, 20.0));

    tree.layout(Size {
        width: 200.0,
        height: 100.0,
    });

    assert!(
        tree.rect(hidden).expect("the node is live").is_empty(),
        "a hidden node has no rectangle"
    );
    assert!(
        tree.rect(child).expect("the node is live").is_empty(),
        "its subtree has no rectangle either"
    );
}

#[test]
fn a_viewport_change_runs_layout_and_sizes_the_root() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), Style::default());

    assert!(tree.layout(Size {
        width: 200.0,
        height: 100.0,
    }));
    tree.clear_dirty();
    let runs = tree.layout_runs();

    assert!(
        !tree.layout(Size {
            width: 200.0,
            height: 100.0,
        }),
        "an unchanged viewport on a clean tree does nothing"
    );

    assert!(
        tree.layout(Size {
            width: 320.0,
            height: 240.0,
        }),
        "a viewport change runs the engine"
    );
    assert_eq!(tree.layout_runs(), runs + 1);
    assert_eq!(
        tree.rect(root),
        Some(Rect::new(0.0, 0.0, 320.0, 240.0)),
        "the root is sized to the viewport"
    );
    tree.debug_assert_consistent();
}

#[test]
fn a_leaf_is_measured_inside_its_own_size_and_padding() {
    let offered = Rc::new(RefCell::new(Vec::new()));
    let mut tree = WidgetTree::new();
    let mut root_style = fixed(200.0, 100.0);
    root_style.layout.align_items = Some(Align::Start);
    let root = tree.insert(None, Box::new(Empty), root_style);
    let mut leaf_style = Style::default();
    leaf_style.layout.width = Length::px(100.0);
    leaf_style.layout.padding = Edges::all(Length::px(10.0));
    let leaf = tree.insert(
        Some(root),
        Box::new(WidthRecorder(Rc::clone(&offered))),
        leaf_style,
    );

    tree.layout(Size {
        width: 200.0,
        height: 100.0,
    });

    // The flex algorithm also probes the min-content size, which ignores the
    // node's own width by design, so only the definite offers are checked.
    let offered = offered.borrow();
    let definite: Vec<&AvailableSpace> = offered
        .iter()
        .filter(|width| matches!(width, AvailableSpace::Definite(_)))
        .collect();
    assert!(
        !definite.is_empty(),
        "the leaf was measured with a definite width"
    );
    assert!(
        definite
            .iter()
            .all(|width| **width == AvailableSpace::Definite(80.0)),
        "the widget is offered its content box, its own width less its padding, not {offered:?}"
    );
    assert_eq!(
        tree.rect(leaf),
        Some(Rect::new(0.0, 0.0, 100.0, 30.0)),
        "the padding is added back around the measured content"
    );
}

#[test]
fn a_leaf_with_a_fixed_width_gets_known_width_and_one_without_gets_the_offer() {
    let fixed_inputs = Rc::new(RefCell::new(Vec::new()));
    let free_inputs = Rc::new(RefCell::new(Vec::new()));
    let mut tree = WidgetTree::new();
    let mut root_style = Style::default();
    root_style.layout.direction = Direction::Column;
    root_style.layout.align_items = Some(Align::Start);
    let root = tree.insert(None, Box::new(Empty), root_style);
    let mut fixed_style = Style::default();
    fixed_style.layout.width = Length::px(100.0);
    tree.insert(
        Some(root),
        Box::new(InputRecorder(Rc::clone(&fixed_inputs))),
        fixed_style,
    );
    tree.insert(
        Some(root),
        Box::new(InputRecorder(Rc::clone(&free_inputs))),
        Style::default(),
    );

    tree.layout(Size {
        width: 200.0,
        height: 100.0,
    });

    let fixed = fixed_inputs.borrow();
    assert!(!fixed.is_empty(), "the fixed leaf was measured");
    assert!(
        fixed.iter().any(|input| input.known_width == Some(100.0)),
        "a fixed width reaches the widget as known_width, not {fixed:?}"
    );
    assert!(
        fixed
            .iter()
            .all(|input| input.known_width.is_none_or(|width| width == 100.0)),
        "the known width is the node's own width, not {fixed:?}"
    );

    let free = free_inputs.borrow();
    assert!(!free.is_empty(), "the free leaf was measured");
    assert!(
        free.iter().all(|input| input.known_width.is_none()),
        "an unfixed width leaves known_width empty, not {free:?}"
    );
    assert!(
        free.iter()
            .any(|input| input.available_width == AvailableSpace::Definite(200.0)),
        "the widget is offered the root's content box, not {free:?}"
    );
}

/// A widget that records every paint context it sees and paints its whole
/// rectangle.
struct Recorder(Rc<RefCell<Vec<(Rect, Rect)>>>);

impl Widget for Recorder {
    fn paint(&self, cx: &PaintCx<'_>, out: &mut Vec<PaintOp>) {
        self.0.borrow_mut().push((cx.rect, cx.content));
        out.push(PaintOp::FillRect {
            rect: cx.rect,
            radius: Corners::ZERO,
            color: Color::BLACK,
        });
    }
}

/// The viewport the scroll tests lay their trees out for.
const SCROLL_VIEWPORT: Size = Size {
    width: 200.0,
    height: 100.0,
};

/// A clipping view at (30, 10), 80 by 40, with a 100 tall flowing child and
/// a 10 by 4 child out of the flow, both painting.
fn painted_scrolled_tree() -> (WidgetTree, WidgetId, WidgetId, WidgetId, WidgetId) {
    let mut tree = WidgetTree::new();
    let mut root_style = fixed(200.0, 100.0);
    root_style.layout.direction = Direction::Column;
    let root = tree.insert(None, Box::new(Empty), root_style);
    tree.insert(Some(root), Box::new(Empty), fixed(10.0, 10.0));
    let mut scrolled_style = fixed(80.0, 40.0);
    scrolled_style.layout.margin.left = Length::px(30.0);
    scrolled_style.layout.overflow = Overflow::Clip;
    let scrolled = tree.insert(Some(root), Box::new(Empty), scrolled_style);
    let flowing = tree.insert(
        Some(scrolled),
        Box::new(Recorder(Rc::new(RefCell::new(Vec::new())))),
        fixed(80.0, 100.0),
    );
    let mut absolute_style = fixed(4.0, 10.0);
    absolute_style.layout.position = Position::Absolute;
    absolute_style.layout.inset.top = Length::px(0.0);
    absolute_style.layout.inset.right = Length::px(0.0);
    absolute_style.paint.background = Some(Color::BLACK.into());
    let absolute = tree.insert(Some(scrolled), Box::new(Panel), absolute_style);
    tree.layout(SCROLL_VIEWPORT);
    (tree, root, scrolled, flowing, absolute)
}

/// Paints one frame and leaves the tree clean, the way a frame ends.
fn paint_one_frame(tree: &mut WidgetTree, damage: &mut DamageAccumulator) {
    tree.paint(damage);
    tree.clear_dirty();
    damage.clear();
}

#[test]
fn a_scroll_repaints_no_moved_node_and_damages_both_inks() {
    let mut damage = DamageAccumulator::new();
    let (mut tree, _, scrolled, flowing, _) = painted_scrolled_tree();
    paint_one_frame(&mut tree, &mut damage);

    tree.set_scroll_offset(scrolled, Point { x: 0.0, y: 25.0 });

    assert_eq!(
        tree.dirty(flowing),
        Some(Dirty::Clean),
        "a scroll keeps the moved child's operations, so it marks nothing for paint"
    );

    let painted = tree.paint(&mut damage);
    let rects = damage.finish(1.0, 200, 100).to_vec();

    assert_eq!(painted, 0, "the scroll frame repaints no node");
    assert_eq!(
        rects,
        vec![PixelRect {
            x: 30,
            y: 10,
            width: 80,
            height: 40,
        }],
        "the damage covers the view, where the ink left and arrived"
    );
}

#[test]
fn the_paint_pass_gives_widgets_paint_coordinates() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut tree = WidgetTree::new();
    let mut root_style = fixed(200.0, 100.0);
    root_style.layout.direction = Direction::Column;
    let root = tree.insert(None, Box::new(Empty), root_style);
    tree.insert(Some(root), Box::new(Empty), fixed(10.0, 10.0));
    let mut scrolled_style = fixed(80.0, 40.0);
    scrolled_style.layout.margin.left = Length::px(30.0);
    scrolled_style.layout.overflow = Overflow::Clip;
    let scrolled = tree.insert(Some(root), Box::new(Empty), scrolled_style);
    let flowing = tree.insert(
        Some(scrolled),
        Box::new(Recorder(Rc::clone(&seen))),
        fixed(80.0, 100.0),
    );

    let mut damage = DamageAccumulator::new();
    tree.layout(SCROLL_VIEWPORT);
    tree.paint(&mut damage);
    tree.clear_dirty();
    damage.clear();

    assert_eq!(
        seen.borrow().last(),
        Some(&(
            Rect::new(30.0, 10.0, 110.0, 110.0),
            Rect::new(30.0, 10.0, 110.0, 110.0),
        )),
        "outside a scrolled view the paint context is the window rectangle"
    );

    tree.set_scroll_offset(scrolled, Point { x: 0.0, y: 25.0 });
    tree.apply_paint(flowing, |paint| {
        paint.background = Some(Color::BLACK.into())
    });
    tree.paint(&mut damage);

    assert_eq!(
        seen.borrow().last(),
        Some(&(
            Rect::new(30.0, 10.0, 110.0, 110.0),
            Rect::new(30.0, 10.0, 110.0, 110.0),
        )),
        "a repaint after a scroll sees the same paint rectangle"
    );
}

#[test]
fn a_scroll_without_painted_content_damages_nothing() {
    let (mut tree, _, scrolled, _, _) = scrolled_tree();
    let mut damage = DamageAccumulator::new();
    paint_one_frame(&mut tree, &mut damage);

    tree.set_scroll_offset(scrolled, Point { x: 0.0, y: 25.0 });

    let painted = tree.paint(&mut damage);
    let rects = damage.finish(1.0, 200, 100).to_vec();

    assert_eq!(painted, 0);
    assert!(rects.is_empty(), "nothing painted, so nothing is damaged");
}

#[test]
fn a_scroll_that_moves_a_nested_clip_repaints_nothing_inside_it() {
    let mut damage = DamageAccumulator::new();
    let (mut tree, _, scrolled, flowing, _) = painted_scrolled_tree();
    tree.apply_layout(flowing, |layout| {
        layout.overflow = Overflow::Clip;
        layout.height = Length::px(20.0);
    });
    let inner = tree.insert(
        Some(flowing),
        Box::new(Recorder(Rc::new(RefCell::new(Vec::new())))),
        fixed(60.0, 20.0),
    );
    tree.layout(SCROLL_VIEWPORT);
    paint_one_frame(&mut tree, &mut damage);

    // The card spans 10 to 30 and the view 10 to 50, so a scroll by 5 cuts
    // the card's clip to 10 to 25.
    tree.set_scroll_offset(scrolled, Point { x: 0.0, y: 5.0 });

    assert_eq!(
        tree.dirty(inner),
        Some(Dirty::Clean),
        "the clip moved with the card, which the operations do not hold"
    );
    assert_eq!(
        tree.paint(&mut damage),
        0,
        "the scroll frame repaints no node"
    );
}

/// A widget whose measured height is read from a cell, so a test can grow
/// the content the way a binding does, by marking the node for layout.
struct Growable(Rc<Cell<f32>>);

impl Widget for Growable {
    fn measure(&mut self, _input: MeasureInput) -> Option<Size> {
        Some(Size {
            width: 200.0,
            height: self.0.get(),
        })
    }

    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}
}

/// A clipping view 200 wide and 100 tall with a content child whose height
/// the returned cell controls, and whose follow signal is `following`.
///
/// The view is laid out for [`SCROLL_VIEWPORT`] before it returns, so a test
/// starts from a settled frame and drives the next change itself.
fn following_tree(
    height: f32,
    following: RwSignal<bool>,
) -> (WidgetTree, WidgetId, WidgetId, Rc<Cell<f32>>) {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(200.0, 100.0));
    let mut view_style = fixed(200.0, 100.0);
    view_style.layout.overflow = Overflow::Clip;
    view_style.layout.direction = Direction::Column;
    let view = tree.insert(Some(root), Box::new(Empty), view_style);
    let height = Rc::new(Cell::new(height));
    let content = tree.insert(
        Some(view),
        Box::new(Growable(Rc::clone(&height))),
        Style::default(),
    );
    tree.set_follow(view, following);
    tree.layout(SCROLL_VIEWPORT);
    (tree, view, content, height)
}

/// Grows the content of [`following_tree`] to `to` and lays the tree out
/// once, the way a frame applies the binding that a signal write queued.
fn grow_and_layout(tree: &mut WidgetTree, content: WidgetId, height: &Cell<f32>, to: f32) -> bool {
    height.set(to);
    tree.mark(content, Dirty::Layout);
    tree.layout(SCROLL_VIEWPORT)
}

#[test]
fn a_follow_pin_reaches_the_end_the_pass_laid_out() {
    let (mut tree, _, content, height) = following_tree(183.0, RwSignal::new(true));
    assert_eq!(
        tree.rect(content),
        Some(Rect::new(0.0, -83.0, 200.0, 100.0)),
        "the first frame pins the view at the end of the content"
    );
    tree.clear_dirty();
    let runs = tree.layout_runs();

    assert!(
        grow_and_layout(&mut tree, content, &height, 243.0),
        "growing the content runs a pass"
    );

    assert_eq!(
        tree.rect(content),
        Some(Rect::new(0.0, -143.0, 200.0, 100.0)),
        "the pin uses the heights the pass just laid out, not the painted \
         ones, which are one frame old"
    );
    assert_eq!(
        tree.layout_runs(),
        runs + 1,
        "the pin settles without another pass"
    );
}

#[test]
fn a_following_view_with_short_content_stays_at_offset_zero() {
    let (tree, _, content, _) = following_tree(50.0, RwSignal::new(true));
    assert_eq!(
        tree.rect(content),
        Some(Rect::new(0.0, 0.0, 200.0, 50.0)),
        "content shorter than the view has no end to pin to"
    );
}

#[test]
fn a_follow_signal_that_reads_false_leaves_the_offset_alone() {
    let following = RwSignal::new(false);
    let (mut tree, _, content, height) = following_tree(183.0, following.clone());
    tree.clear_dirty();

    assert!(
        grow_and_layout(&mut tree, content, &height, 243.0),
        "growing the content runs a pass"
    );

    assert_eq!(
        tree.rect(content),
        Some(Rect::new(0.0, 0.0, 200.0, 243.0)),
        "a signal that reads false leaves the offset where it is"
    );

    following.set(true);
    assert!(
        grow_and_layout(&mut tree, content, &height, 303.0),
        "growing the content runs a pass"
    );

    assert_eq!(
        tree.rect(content),
        Some(Rect::new(0.0, -203.0, 200.0, 100.0)),
        "a write of true pins the view on the next pass"
    );
}

#[test]
fn a_hidden_following_view_is_skipped_until_it_shows() {
    let (mut tree, view, content, _) = following_tree(183.0, RwSignal::new(true));
    tree.clear_dirty();

    tree.apply_layout(view, |layout| layout.hidden = true);
    tree.layout(SCROLL_VIEWPORT);

    assert_eq!(
        tree.rect(content),
        Some(Rect::default()),
        "a hidden view has no rectangles to measure"
    );

    tree.apply_layout(view, |layout| layout.hidden = false);
    tree.layout(SCROLL_VIEWPORT);

    assert_eq!(
        tree.rect(content),
        Some(Rect::new(0.0, -83.0, 200.0, 100.0)),
        "the pin lands in the pass that shows the view"
    );
}

#[test]
fn a_follow_pin_ignores_children_out_of_the_flow() {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(200.0, 100.0));
    let mut view_style = fixed(200.0, 100.0);
    view_style.layout.overflow = Overflow::Clip;
    view_style.layout.direction = Direction::Column;
    let view = tree.insert(Some(root), Box::new(Empty), view_style);
    let content = tree.insert(Some(view), Box::new(Fixed(200.0, 50.0)), Style::default());
    let mut badge_style = fixed(20.0, 20.0);
    badge_style.layout.position = Position::Absolute;
    badge_style.layout.inset.top = Length::px(100.0);
    badge_style.layout.inset.left = Length::px(0.0);
    badge_style.layout.inset.right = Length::px(0.0);
    badge_style.layout.inset.bottom = Length::Auto;
    tree.insert(Some(view), Box::new(Empty), badge_style);
    tree.set_follow(view, RwSignal::new(true));

    tree.layout(SCROLL_VIEWPORT);

    assert_eq!(
        tree.rect(content),
        Some(Rect::new(0.0, 0.0, 200.0, 50.0)),
        "a badge that hangs below the fold does not extend the extent the \
         pin measures, because a scroll offset does not move it"
    );
}

#[test]
fn a_following_view_mounted_after_another_was_removed_still_pins() {
    let (mut tree, view, _, _) = following_tree(183.0, RwSignal::new(true));
    let root = tree.root.expect("the tree has a root");
    tree.remove(view);
    tree.layout(SCROLL_VIEWPORT);

    let mut view_style = fixed(200.0, 100.0);
    view_style.layout.overflow = Overflow::Clip;
    view_style.layout.direction = Direction::Column;
    let view = tree.insert(Some(root), Box::new(Empty), view_style);
    let content = tree.insert(
        Some(view),
        Box::new(Growable(Rc::new(Cell::new(250.0)))),
        Style::default(),
    );
    tree.set_follow(view, RwSignal::new(true));
    tree.layout(SCROLL_VIEWPORT);
    assert_eq!(
        tree.rect(content),
        Some(Rect::new(0.0, -150.0, 200.0, 100.0)),
        "the new view pins although the removed one left a stale entry"
    );
    tree.debug_assert_consistent();
}

/// A column of three growable children 20 tall, under a root, with the
/// middle child pinned 50 below the column's top. Laid out once.
fn pinned_column() -> (WidgetTree, WidgetId, [WidgetId; 3], [Rc<Cell<f32>>; 3]) {
    let mut tree = WidgetTree::new();
    let root = tree.insert(None, Box::new(Empty), fixed(200.0, 100.0));
    let mut column_style = Style::default();
    column_style.layout.direction = Direction::Column;
    let column = tree.insert(Some(root), Box::new(Empty), column_style);
    let heights = [(); 3].map(|_| Rc::new(Cell::new(20.0)));
    let children = [0, 1, 2].map(|at| {
        tree.insert(
            Some(column),
            Box::new(Growable(Rc::clone(&heights[at]))),
            Style::default(),
        )
    });
    tree.set_pin(column, Some((1, 50.0)));
    tree.layout(SCROLL_VIEWPORT);
    (tree, column, children, heights)
}

#[test]
fn a_pinned_child_keeps_its_place_while_a_child_before_it_grows() {
    let (mut tree, _, children, heights) = pinned_column();
    assert_eq!(
        tree.rect(children[0]),
        Some(Rect::new(0.0, 30.0, 200.0, 50.0))
    );
    assert_eq!(
        tree.rect(children[1]),
        Some(Rect::new(0.0, 50.0, 200.0, 70.0))
    );

    heights[0].set(45.0);
    tree.mark(children[0], Dirty::Layout);
    tree.layout(SCROLL_VIEWPORT);
    assert_eq!(
        tree.rect(children[1]),
        Some(Rect::new(0.0, 50.0, 200.0, 70.0)),
        "the pinned child stays where the pin puts it"
    );
    assert_eq!(
        tree.rect(children[0]),
        Some(Rect::new(0.0, 5.0, 200.0, 50.0)),
        "the grown child stacks upward from it, edge to edge"
    );
    assert_eq!(
        tree.rect(children[2]),
        Some(Rect::new(0.0, 70.0, 200.0, 90.0))
    );
    tree.debug_assert_consistent();
}

#[test]
fn a_pinned_child_that_grows_pushes_the_children_after_it() {
    let (mut tree, _, children, heights) = pinned_column();
    heights[1].set(32.0);
    tree.mark(children[1], Dirty::Layout);
    tree.layout(SCROLL_VIEWPORT);
    assert_eq!(
        tree.rect(children[1]),
        Some(Rect::new(0.0, 50.0, 200.0, 82.0))
    );
    assert_eq!(
        tree.rect(children[2]),
        Some(Rect::new(0.0, 82.0, 200.0, 102.0)),
        "the next child starts where the grown one ends"
    );
    tree.debug_assert_consistent();
}

#[test]
fn a_new_pin_moves_the_children_without_a_layout_pass() {
    let (mut tree, column, children, _) = pinned_column();
    let runs = tree.layout_runs();
    tree.set_pin(column, Some((2, 10.0)));
    assert_eq!(
        tree.rect(children[2]),
        Some(Rect::new(0.0, 10.0, 200.0, 30.0))
    );
    assert_eq!(
        tree.rect(children[0]),
        Some(Rect::new(0.0, -30.0, 200.0, -10.0))
    );
    assert!(
        !tree.layout(SCROLL_VIEWPORT),
        "nothing asks for a layout pass"
    );
    assert_eq!(tree.layout_runs(), runs);

    tree.set_pin(column, Some((7, 0.0)));
    assert_eq!(
        tree.rect(children[2]),
        Some(Rect::new(0.0, 10.0, 200.0, 30.0)),
        "an index past the last child leaves the children where they are"
    );
    tree.debug_assert_consistent();
}

#[test]
fn a_hidden_pinned_child_leaves_the_children_where_they_are() {
    let (mut tree, _, children, _) = pinned_column();
    tree.apply_layout(children[1], |layout| layout.hidden = true);
    tree.layout(SCROLL_VIEWPORT);
    assert_eq!(
        tree.rect(children[0]),
        Some(Rect::new(0.0, 30.0, 200.0, 50.0)),
        "a hidden pinned child has no place to keep, so the offset stays"
    );
    assert_eq!(
        tree.rect(children[2]),
        Some(Rect::new(0.0, 50.0, 200.0, 70.0))
    );
    tree.debug_assert_consistent();
}
