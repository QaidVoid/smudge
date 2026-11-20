//! Tooltips and dropdowns, by pointer and by keyboard.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use smudge_core::element::TOOLTIP_DELAY;
use smudge_core::geometry::Point;
use smudge_core::input::Key;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Role, SemanticsNode};
use smudge_core::style::{Align, px};
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::{TextSystem, label};
use smudge_widgets::{TooltipExt, dropdown};

/// Returns the first projected node with `role`, if the window shows one.
fn with_role(harness: &mut Harness, role: Role) -> Option<SemanticsNode> {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.role == role)
}

/// Returns the projected node named `name`, if the window shows one.
fn named(harness: &mut Harness, name: &str) -> Option<SemanticsNode> {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.label.as_deref() == Some(name))
}

/// A window with a 40 by 20 Save button at its top left whose tooltip reads
/// "Save the map", and a panel under where the tooltip shows that counts its
/// presses.
fn tooltip_scene(under: Rc<Cell<u32>>) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(300.0, 120.0, 1.0, move || {
        let under = Rc::clone(&under);
        panel()
            .column()
            .align_items(Align::Start)
            .child(
                panel()
                    .width(px(40.0))
                    .height(px(20.0))
                    .role(Role::Button)
                    .accessible_label("Save")
                    .on_press(|| {})
                    .tooltip_text("Save the map"),
            )
            .child(
                panel()
                    .width(px(200.0))
                    .height(px(60.0))
                    .on_press(move || under.set(under.get() + 1)),
            )
    });
    harness.frame();
    harness
}

#[test]
fn a_tooltip_shows_after_the_pointer_rests_and_hides_when_it_leaves() {
    let mut harness = tooltip_scene(Rc::new(Cell::new(0)));
    let save = named(&mut harness, "Save").expect("the button shows");
    assert_eq!(save.description.as_deref(), Some("Save the map"));

    harness.move_to(Point { x: 10.0, y: 10.0 });
    harness.advance(TOOLTIP_DELAY / 2);
    assert!(with_role(&mut harness, Role::Tooltip).is_none(), "not yet");
    harness.advance(TOOLTIP_DELAY);
    harness.frame();
    let tip = with_role(&mut harness, Role::Tooltip).expect("the tooltip shows");
    assert!(tip.bounds.y0 >= 20.0, "below the button: {:?}", tip.bounds);

    harness.move_to(Point { x: 250.0, y: 100.0 });
    harness.frame();
    assert!(with_role(&mut harness, Role::Tooltip).is_none());
}

#[test]
fn a_press_hides_the_tooltip_and_the_pointer_passes_through_it() {
    let under = Rc::new(Cell::new(0));
    let mut harness = tooltip_scene(Rc::clone(&under));
    harness.move_to(Point { x: 10.0, y: 10.0 });
    harness.advance(TOOLTIP_DELAY + Duration::from_millis(10));
    harness.frame();
    let tip = with_role(&mut harness, Role::Tooltip).expect("the tooltip shows");

    let inside_tip = Point {
        x: (tip.bounds.x0 + tip.bounds.x1) / 2.0,
        y: (tip.bounds.y0 + tip.bounds.y1) / 2.0,
    };
    harness.click(inside_tip);
    harness.frame();
    assert_eq!(
        under.get(),
        1,
        "the press reached the panel under the tooltip"
    );
    assert!(
        with_role(&mut harness, Role::Tooltip).is_none(),
        "and hid it"
    );

    harness.move_to(Point { x: 10.0, y: 10.0 });
    harness.advance(TOOLTIP_DELAY + Duration::from_millis(10));
    harness.frame();
    assert!(with_role(&mut harness, Role::Tooltip).is_some());
    harness.click(Point { x: 10.0, y: 10.0 });
    harness.frame();
    assert!(
        with_role(&mut harness, Role::Tooltip).is_none(),
        "a press on the node hides it"
    );
}

/// A window with a dropdown of three fruits at its top left, picking
/// `selected`.
fn fruit_scene(selected: RwSignal<usize>) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        let selected = selected.clone();
        panel().align_items(Align::Start).child(
            dropdown(
                || ["Apple", "Banana", "Cherry"].map(String::from).to_vec(),
                selected,
            )
            .accessible_label("Fruit"),
        )
    });
    harness.frame();
    harness
}

#[test]
fn a_dropdown_reports_its_choice_and_opens_its_list_on_the_picked_option() {
    let selected = RwSignal::new(1);
    let mut harness = fruit_scene(selected.clone());
    let field = named(&mut harness, "Fruit").expect("the field shows");
    assert_eq!(field.role, Role::ComboBox);
    assert_eq!(field.value.as_deref(), Some("Banana"));

    let center = Point {
        x: (field.bounds.x0 + field.bounds.x1) / 2.0,
        y: (field.bounds.y0 + field.bounds.y1) / 2.0,
    };
    harness.click(center);
    harness.frame();
    assert!(with_role(&mut harness, Role::ListBox).is_some());
    let banana = named(&mut harness, "Banana").expect("the list shows");
    assert_eq!(banana.role, Role::ListBoxOption);
    assert_eq!(banana.selected, Some(true));
    assert!(banana.focused, "the picked option takes the focus");
    assert_eq!(
        named(&mut harness, "Apple").and_then(|node| node.selected),
        Some(false)
    );

    let cherry = named(&mut harness, "Cherry")
        .expect("the list shows")
        .bounds;
    harness.click(Point {
        x: (cherry.x0 + cherry.x1) / 2.0,
        y: (cherry.y0 + cherry.y1) / 2.0,
    });
    harness.frame();
    assert_eq!(selected.get(), 2);
    assert!(
        with_role(&mut harness, Role::ListBox).is_none(),
        "the list closed"
    );
    assert_eq!(
        named(&mut harness, "Fruit").and_then(|node| node.value),
        Some(String::from("Cherry"))
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn the_keyboard_opens_a_dropdown_walks_it_and_picks_or_cancels() {
    let selected = RwSignal::new(0);
    let mut harness = fruit_scene(selected.clone());
    let field = named(&mut harness, "Fruit")
        .expect("the field shows")
        .bounds;
    harness.click(Point {
        x: field.x0 + 4.0,
        y: field.y0 + 4.0,
    });
    harness.frame();
    harness.press_key(Key::Escape);
    harness.frame();
    assert!(with_role(&mut harness, Role::ListBox).is_none());

    harness.press_key(Key::ArrowDown);
    harness.frame();
    assert!(
        with_role(&mut harness, Role::ListBox).is_some(),
        "Down opens it"
    );
    harness.press_key(Key::ArrowDown);
    assert!(
        named(&mut harness, "Banana").is_some_and(|node| node.focused),
        "Down inside the list walks it"
    );
    harness.press_key(Key::Escape);
    harness.frame();
    assert_eq!(selected.get(), 0, "Escape picks nothing");
    assert!(
        named(&mut harness, "Fruit").is_some_and(|node| node.focused),
        "and gives the focus back to the field"
    );

    harness.press_key(Key::Enter);
    harness.frame();
    harness.press_key(Key::Character(String::from("c")));
    harness.press_key(Key::Enter);
    harness.frame();
    assert_eq!(selected.get(), 2, "c went to Cherry, and Enter picked it");
}

#[test]
fn leaving_before_the_delay_keeps_the_tooltip_hidden() {
    let mut harness = tooltip_scene(Rc::new(Cell::new(0)));

    harness.move_to(Point { x: 10.0, y: 10.0 });
    harness.advance(TOOLTIP_DELAY / 2);
    harness.move_to(Point { x: 250.0, y: 100.0 });
    harness.advance(TOOLTIP_DELAY);
    harness.frame();

    assert!(
        with_role(&mut harness, Role::Tooltip).is_none(),
        "the wait the pointer left ends without a tooltip"
    );
}

#[test]
fn escape_and_a_press_hide_a_tooltip_even_on_a_node_that_takes_no_press() {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(300.0, 120.0, 1.0, || {
        panel()
            .align_items(Align::Start)
            .child(label("?").tooltip_text("What this does"))
    });
    harness.frame();
    let rest = |harness: &mut Harness| {
        harness.move_to(Point { x: 3.0, y: 5.0 });
        harness.advance(TOOLTIP_DELAY + Duration::from_millis(10));
        harness.frame();
    };

    rest(&mut harness);
    assert!(with_role(&mut harness, Role::Tooltip).is_some());
    harness.press_key(Key::Escape);
    harness.frame();
    assert!(
        with_role(&mut harness, Role::Tooltip).is_none(),
        "Escape hides it"
    );

    harness.move_to(Point { x: 250.0, y: 100.0 });
    rest(&mut harness);
    assert!(with_role(&mut harness, Role::Tooltip).is_some());
    harness.click(Point { x: 3.0, y: 5.0 });
    harness.frame();
    assert!(
        with_role(&mut harness, Role::Tooltip).is_none(),
        "a press hides it"
    );
}

#[test]
fn a_dropdown_past_its_list_focuses_the_first_choice_and_an_empty_one_stays_shut() {
    let selected = RwSignal::new(7);
    let mut harness = fruit_scene(selected);
    let field = named(&mut harness, "Fruit")
        .expect("the field shows")
        .bounds;
    harness.click(Point {
        x: field.x0 + 4.0,
        y: field.y0 + 4.0,
    });
    harness.frame();
    assert!(
        named(&mut harness, "Apple").is_some_and(|node| node.focused),
        "the first choice takes the focus"
    );
    assert_eq!(
        named(&mut harness, "Fruit").and_then(|node| node.expanded),
        Some(true)
    );

    TextSystem::bundled_only().install();
    let mut empty = Harness::new(300.0, 200.0, 1.0, || {
        panel()
            .align_items(Align::Start)
            .child(dropdown(Vec::new, RwSignal::new(0)).accessible_label("Nothing"))
    });
    empty.frame();
    let field = named(&mut empty, "Nothing")
        .expect("the field shows")
        .bounds;
    empty.click(Point {
        x: field.x0 + 4.0,
        y: field.y0 + 4.0,
    });
    empty.frame();
    assert!(with_role(&mut empty, Role::ListBox).is_none());
}

#[test]
fn a_dropdown_list_closes_when_its_choices_change_while_open() {
    TextSystem::bundled_only().install();
    let choices = RwSignal::new(vec![String::from("Red"), String::from("Blue")]);
    let read = choices.clone();
    let mut harness = Harness::new(300.0, 200.0, 1.0, move || {
        let read = read.clone();
        panel()
            .align_items(Align::Start)
            .child(dropdown(move || read.get(), RwSignal::new(0)).accessible_label("Color"))
    });
    harness.frame();
    let field = named(&mut harness, "Color")
        .expect("the field shows")
        .bounds;
    harness.click(Point {
        x: field.x0 + 4.0,
        y: field.y0 + 4.0,
    });
    harness.frame();
    assert!(with_role(&mut harness, Role::ListBox).is_some());

    choices.set(vec![String::from("Green")]);
    harness.frame();
    harness.frame();
    assert!(with_role(&mut harness, Role::ListBox).is_none());
}
