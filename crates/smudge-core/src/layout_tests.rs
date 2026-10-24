use std::cell::RefCell;
use std::rc::Rc;

use crate::geometry::{Color, Rect, Size};
use crate::paint::PaintOp;
use crate::style::{Align, Direction, Edges, Length, Overflow, Style};
use crate::tree::{Dirty, WidgetId, WidgetTree};
use crate::widget::{AvailableSpace, MeasureInput, PaintCx, Widget};

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

    tree.apply_paint(child, |paint| paint.background = Some(Color::BLACK));

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
