use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use reactive_graph::owner::LocalStorage;

use crate::element::{Element, IntoElement, UpdateQueue, mount, show};
use crate::geometry::Color;
use crate::paint::PaintOp;
use crate::reactive::{self, Owner, WriteSignal, prelude::*};
use crate::style::Length;
use crate::theme::ColorValue;
use crate::tree::{Dirty, WidgetId, WidgetTree};
use crate::widget::{PaintCx, Widget, panel};

/// Mounts `element` as the root of a fresh tree.
fn mount_root(element: impl IntoElement) -> (WidgetTree, UpdateQueue, WidgetId) {
    reactive::init();
    let mut tree = WidgetTree::new();
    let queue = UpdateQueue::default();
    let root = mount(element.into_any(), &mut tree, &queue, None);
    (tree, queue, root)
}

/// Runs the reactive tasks and applies the updates they queue, as a frame does.
fn settle(tree: &mut WidgetTree, queue: &UpdateQueue) {
    for _ in 0..16 {
        reactive::flush();
        if !queue.apply(tree) {
            return;
        }
    }
    panic!("the queue did not settle within sixteen rounds");
}

#[test]
fn a_static_background_lands_in_the_paint_properties() {
    let (tree, _queue, root) =
        mount_root(panel().width(Length::px(120.0)).background(Color::WHITE));

    let style = tree.style(root).expect("the root is live");
    assert_eq!(style.layout.width, Length::px(120.0));
    assert_eq!(style.paint.background, Some(Color::WHITE.into()));
    assert_eq!(
        tree.dirty(root),
        Some(Dirty::Layout),
        "a new node has never been laid out"
    );
    assert!(tree.layout_is_dirty());
}

#[test]
fn a_binding_reads_its_value_when_the_node_mounts() {
    let (color, _set_color) = signal_local(Color::BLACK);
    let (mut tree, queue, root) = mount_root(panel().background(move || color.get().into()));

    queue.apply(&mut tree);

    assert_eq!(
        tree.style(root).expect("the root is live").paint.background,
        Some(Color::BLACK.into()),
        "the first run is queued at mount, before any flush"
    );
}

#[test]
fn a_background_binding_marks_the_node_for_paint_only() {
    let (on, set_on) = signal_local(false);
    let (mut tree, queue, root) = mount_root(panel().background(move || {
        if on.get() {
            Color::BLACK.into()
        } else {
            Color::WHITE.into()
        }
    }));
    settle(&mut tree, &queue);
    tree.clear_dirty();

    set_on.set(true);
    settle(&mut tree, &queue);

    assert_eq!(
        tree.style(root).expect("the root is live").paint.background,
        Some(Color::BLACK.into())
    );
    assert_eq!(tree.dirty(root), Some(Dirty::Paint));
    assert!(!tree.layout_is_dirty(), "a paint change never needs layout");
}

#[test]
fn a_width_binding_marks_the_node_for_layout() {
    let (width, set_width) = signal_local(Length::px(20.0));
    let (mut tree, queue, root) = mount_root(panel().width(move || width.get()));
    settle(&mut tree, &queue);
    tree.clear_dirty();

    set_width.set(Length::px(40.0));
    settle(&mut tree, &queue);

    assert_eq!(
        tree.style(root).expect("the root is live").layout.width,
        Length::px(40.0)
    );
    assert_eq!(tree.dirty(root), Some(Dirty::Layout));
    assert!(tree.layout_is_dirty());
}

#[test]
fn a_binding_change_waits_for_the_queue_to_drain() {
    let (on, set_on) = signal_local(false);
    let (mut tree, queue, root) = mount_root(panel().background(move || {
        if on.get() {
            Color::BLACK.into()
        } else {
            Color::WHITE.into()
        }
    }));
    settle(&mut tree, &queue);

    set_on.set(true);
    reactive::flush();

    assert_eq!(
        tree.style(root).expect("the root is live").paint.background,
        Some(Color::WHITE.into()),
        "the effect queued the change and left the tree alone"
    );

    queue.apply(&mut tree);

    assert_eq!(
        tree.style(root).expect("the root is live").paint.background,
        Some(Color::BLACK.into())
    );
}

#[test]
fn children_are_mounted_in_the_order_they_were_added() {
    let (tree, _queue, root) =
        mount_root(panel().child(panel().background(Color::WHITE)).children([
            panel().background(Color::BLACK),
            panel().background(Color::TRANSPARENT),
        ]));

    let children = tree.children(root).expect("the root is live");
    assert_eq!(children.len(), 3);
    let backgrounds: Vec<Option<ColorValue>> = children
        .iter()
        .map(|child| {
            tree.style(*child)
                .expect("every mounted child is live")
                .paint
                .background
        })
        .collect();
    assert_eq!(
        backgrounds,
        vec![
            Some(Color::WHITE.into()),
            Some(Color::BLACK.into()),
            Some(Color::TRANSPARENT.into())
        ]
    );
    tree.debug_assert_consistent();
}

#[test]
fn a_widget_binding_updates_the_widget_and_marks_it() {
    /// A widget with one property of its own.
    struct Dot(u8);

    impl Widget for Dot {
        fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}
    }

    let (value, set_value) = signal_local(1u8);
    let (mut tree, queue, root) = mount_root(Element::new(Dot(0)).bind(
        move || value.get(),
        |dot, value| {
            dot.0 = value;
            Dirty::Paint
        },
    ));
    settle(&mut tree, &queue);

    assert_eq!(
        tree.widget_mut::<Dot>(root).map(|dot| dot.0),
        Some(1),
        "the first run applies the bound value"
    );

    tree.clear_dirty();
    set_value.set(2);
    settle(&mut tree, &queue);

    assert_eq!(tree.widget_mut::<Dot>(root).map(|dot| dot.0), Some(2));
    assert_eq!(tree.dirty(root), Some(Dirty::Paint));
    assert!(!tree.layout_is_dirty());
}

#[test]
fn show_mounts_its_child_while_the_signal_is_true() {
    let (on, set_on) = signal_local(false);
    let (color, set_color) = signal_local(Color::WHITE);
    let runs = Rc::new(Cell::new(0u32));
    let counter = Rc::clone(&runs);

    let (mut tree, queue, slot) = mount_root(show(
        move || on.get(),
        move || {
            let counter = Rc::clone(&counter);
            panel().background(move || {
                counter.set(counter.get() + 1);
                color.get().into()
            })
        },
    ));
    settle(&mut tree, &queue);

    assert_eq!(tree.node_count(), 1, "nothing is mounted yet");
    assert_eq!(runs.get(), 0);

    set_on.set(true);
    settle(&mut tree, &queue);

    assert_eq!(tree.node_count(), 2, "the child is mounted");
    assert_eq!(
        tree.children(slot).expect("the slot is live").len(),
        1,
        "the child is mounted under the slot"
    );
    assert_eq!(runs.get(), 1, "the child's binding ran once");
    tree.debug_assert_consistent();

    set_color.set(Color::BLACK);
    settle(&mut tree, &queue);
    assert_eq!(runs.get(), 2, "the child's binding follows its signal");

    set_on.set(false);
    settle(&mut tree, &queue);

    assert_eq!(tree.node_count(), 1, "the child is removed");
    assert_eq!(
        tree.children(slot).expect("the slot is live").len(),
        0,
        "the slot no longer lists the child"
    );
    tree.debug_assert_consistent();

    set_color.set(Color::WHITE);
    settle(&mut tree, &queue);
    assert_eq!(runs.get(), 2, "the removed child's binding stopped");
}

#[test]
fn show_keeps_the_signals_its_child_created_while_the_child_stays_mounted() {
    let (visible, _set_visible) = signal_local(true);
    let (unrelated, set_unrelated) = signal_local(0u32);
    let builds = Rc::new(Cell::new(0u32));
    let first_writer: Rc<RefCell<Option<WriteSignal<Color, LocalStorage>>>> =
        Rc::new(RefCell::new(None));
    let counter = Rc::clone(&builds);
    let stash = Rc::clone(&first_writer);

    let (mut tree, queue, slot) = mount_root(show(
        move || {
            unrelated.get();
            visible.get()
        },
        move || {
            counter.set(counter.get() + 1);
            let (color, set_color) = signal_local(Color::BLACK);
            stash.borrow_mut().get_or_insert(set_color);
            panel().background(move || color.get().into())
        },
    ));
    settle(&mut tree, &queue);
    let child = tree.children(slot).expect("the slot is live")[0];

    set_unrelated.set(1);
    settle(&mut tree, &queue);
    assert_eq!(
        tree.children(slot).expect("the slot is live"),
        &[child],
        "a check that leaves the child mounted keeps the same child"
    );
    assert_eq!(
        builds.get(),
        1,
        "the child is built once while it stays mounted"
    );

    let set_color = first_writer.borrow().expect("the child was built");
    set_color.set(Color::WHITE);
    settle(&mut tree, &queue);
    assert_eq!(
        tree.style(child)
            .expect("the child is live")
            .paint
            .background,
        Some(Color::WHITE.into()),
        "the signal the child created still drives its binding"
    );
}

#[test]
fn show_disposes_the_scope_its_child_was_built_in() {
    let (visible, set_visible) = signal_local(true);
    let disposed = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&disposed);

    let (mut tree, queue, slot) = mount_root(show(
        move || visible.get(),
        move || {
            let flag = Arc::clone(&flag);
            Owner::on_cleanup(move || flag.store(true, Ordering::Relaxed));
            panel()
        },
    ));
    settle(&mut tree, &queue);

    assert_eq!(
        tree.children(slot).expect("the slot is live").len(),
        1,
        "the child is mounted"
    );
    assert!(
        !disposed.load(Ordering::Relaxed),
        "the scope the child was built in stays alive while it is mounted"
    );

    set_visible.set(false);
    settle(&mut tree, &queue);

    assert!(
        disposed.load(Ordering::Relaxed),
        "removing the child disposes the scope it was built in"
    );
}
