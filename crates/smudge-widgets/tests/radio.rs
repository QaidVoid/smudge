//! Radio groups: choosing by press and by keys, the one Tab stop, the
//! disabled choices the arrows pass over, and what a screen reader hears.

use smudge_core::geometry::{Point, Size};
use smudge_core::input::{Key, Modifiers};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::{Checked, Role, SemanticsNode};
use smudge_core::style::{Align, px};
use smudge_core::theme::Theme;
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::TextSystem;
use smudge_widgets::{column, radio_group};

/// The flavors one window chooses between.
#[derive(Clone, PartialEq, Debug)]
enum Flavor {
    Vanilla,
    Strawberry,
    Mint,
}

/// Returns the first projected node with `role`.
fn with_role(harness: &mut Harness, role: Role) -> SemanticsNode {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.role == role)
        .unwrap_or_else(|| panic!("the window shows a {role:?}"))
}

/// Returns the projected node named `name`.
fn named(harness: &mut Harness, name: &str) -> SemanticsNode {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.label.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("the window shows {name:?}"))
}

/// Returns the name of the node that holds the focus.
fn focused(harness: &mut Harness) -> Option<String> {
    harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.focused)
        .and_then(|node| node.label)
}

/// Returns the middle of `node`.
fn middle(node: &SemanticsNode) -> Point {
    Point {
        x: (node.bounds.x0 + node.bounds.x1) / 2.0,
        y: (node.bounds.y0 + node.bounds.y1) / 2.0,
    }
}

/// Returns the label the projection gives `flavor`.
fn name_of(flavor: &Flavor) -> &'static str {
    match flavor {
        Flavor::Vanilla => "Vanilla",
        Flavor::Strawberry => "Strawberry",
        Flavor::Mint => "Mint",
    }
}

/// A window of 300 by 220 showing the group `build` makes, framed once.
fn window<T: PartialEq + Clone + 'static>(
    selected: RwSignal<T>,
    build: impl Fn(RwSignal<T>) -> smudge_widgets::RadioGroup<T> + 'static,
) -> Harness {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(300.0, 220.0, 1.0, move || {
        let selected = selected.clone();
        panel().align_items(Align::Start).child(build(selected))
    });
    harness.frame();
    harness
}

/// A group of three flavors in a column, Vanilla chosen.
fn flavors(selected: RwSignal<Flavor>) -> smudge_widgets::RadioGroup<Flavor> {
    radio_group(selected)
        .option(Flavor::Vanilla, "Vanilla")
        .option(Flavor::Strawberry, "Strawberry")
        .option(Flavor::Mint, "Mint")
        .accessible_label("Flavor")
}

/// Clears the focus, as a press on the empty part of the window does.
fn clear_focus(harness: &mut Harness) {
    harness.click(Point { x: 290.0, y: 210.0 });
}

#[test]
fn a_press_on_a_choice_or_its_label_chooses_it() {
    let selected = RwSignal::new(Flavor::Vanilla);
    let mut harness = window(selected.clone(), flavors);
    assert_eq!(
        with_role(&mut harness, Role::RadioButton).checked,
        Some(Checked::True),
        "the choice the signal holds starts chosen"
    );

    let strawberry = named(&mut harness, "Strawberry");
    harness.click(middle(&strawberry));
    harness.frame();
    assert_eq!(selected.get(), Flavor::Strawberry, "a press chooses");
    assert_eq!(
        named(&mut harness, "Strawberry").checked,
        Some(Checked::True)
    );
    assert_eq!(named(&mut harness, "Vanilla").checked, Some(Checked::False));

    // The label sits beside the circle, so a press on the row's right end
    // chooses by the label too.
    let strawberry = named(&mut harness, "Strawberry");
    harness.click(Point {
        x: strawberry.bounds.x1 - 4.0,
        y: middle(&strawberry).y,
    });
    harness.frame();
    assert_eq!(selected.get(), Flavor::Strawberry);
}

#[test]
fn enter_and_space_choose_the_focused_choice() {
    let selected = RwSignal::new(Flavor::Vanilla);
    let mut harness = window(selected.clone(), flavors);

    harness.press_key(Key::Tab);
    assert_eq!(focused(&mut harness).as_deref(), Some("Vanilla"));
    harness.press_key(Key::Enter);
    assert_eq!(selected.get(), Flavor::Vanilla, "Enter keeps the choice");

    harness.press_key(Key::ArrowDown);
    assert_eq!(selected.get(), Flavor::Strawberry);
    harness.press_key(Key::Space);
    assert_eq!(selected.get(), Flavor::Strawberry, "Space re-chooses");
}

#[test]
fn the_arrows_move_the_choice_and_the_focus_together() {
    let selected = RwSignal::new(Flavor::Vanilla);
    let mut harness = window(selected.clone(), flavors);
    harness.press_key(Key::Tab);

    for (key, expected) in [
        (Key::ArrowDown, Flavor::Strawberry),
        (Key::ArrowRight, Flavor::Mint),
        (Key::ArrowDown, Flavor::Vanilla),
        (Key::ArrowUp, Flavor::Mint),
        (Key::ArrowLeft, Flavor::Strawberry),
        (Key::ArrowUp, Flavor::Vanilla),
    ] {
        harness.press_key(key.clone());
        assert_eq!(selected.get(), expected, "after {key:?} the choice moves");
        assert_eq!(
            focused(&mut harness).as_deref(),
            Some(name_of(&expected)),
            "after {key:?} the focus moves with it"
        );
    }
}

#[test]
fn every_arrow_walks_a_horizontal_group_in_its_order() {
    let selected = RwSignal::new(Flavor::Vanilla);
    let bind = selected.clone();
    let mut harness = window(bind, |selected| {
        radio_group(selected)
            .option(Flavor::Vanilla, "Vanilla")
            .option(Flavor::Strawberry, "Strawberry")
            .horizontal()
    });
    harness.press_key(Key::Tab);

    harness.press_key(Key::ArrowLeft);
    assert_eq!(
        selected.get(),
        Flavor::Strawberry,
        "Left wraps back from the first"
    );
    harness.press_key(Key::ArrowUp);
    assert_eq!(
        selected.get(),
        Flavor::Vanilla,
        "Up walks forward where Left does"
    );
    harness.press_key(Key::ArrowDown);
    assert_eq!(selected.get(), Flavor::Strawberry);
    harness.press_key(Key::ArrowRight);
    assert_eq!(selected.get(), Flavor::Vanilla);
}

#[test]
fn tab_enters_at_the_chosen_choice_and_shift_tab_returns_to_it() {
    let selected = RwSignal::new(Flavor::Mint);
    let mut harness = window(selected.clone(), flavors);

    clear_focus(&mut harness);
    assert_eq!(focused(&mut harness), None);
    harness.press_key(Key::Tab);
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Mint"),
        "Tab enters at the chosen choice"
    );

    harness.press_key(Key::Tab);
    harness.press_key_with(
        Key::Tab,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Mint"),
        "and Shift+Tab comes back to it"
    );
}

#[test]
fn tab_enters_at_the_first_choice_when_none_matches_and_chooses_nothing() {
    let selected = RwSignal::new(Flavor::Strawberry);
    let bind = selected.clone();
    let mut harness = window(bind, |selected| {
        radio_group(selected)
            .option(Flavor::Vanilla, "Vanilla")
            .option(Flavor::Mint, "Mint")
    });
    clear_focus(&mut harness);

    harness.press_key(Key::Tab);
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Vanilla"),
        "no choice matches, so Tab enters at the first"
    );
    assert_eq!(
        selected.get(),
        Flavor::Strawberry,
        "entering the group chooses nothing"
    );
    assert_eq!(named(&mut harness, "Vanilla").checked, Some(Checked::False));
}

#[test]
fn an_arrow_that_cannot_move_the_focus_still_moves_the_choice() {
    let selected = RwSignal::new(Flavor::Mint);
    let bind = selected.clone();
    let mut harness = window(bind, |selected| {
        radio_group(selected)
            .option(Flavor::Vanilla, "Vanilla")
            .option_disabled(Flavor::Strawberry, "Strawberry", true)
    });
    clear_focus(&mut harness);
    harness.press_key(Key::Tab);
    assert_eq!(focused(&mut harness).as_deref(), Some("Vanilla"));

    harness.press_key(Key::ArrowDown);
    assert_eq!(
        selected.get(),
        Flavor::Vanilla,
        "Down has nowhere to move the focus, yet it chooses the focused one"
    );

    let selected = RwSignal::new(Flavor::Mint);
    let bind = selected.clone();
    let mut harness = window(bind, |selected| {
        radio_group(selected)
            .option(Flavor::Vanilla, "Vanilla")
            .option(Flavor::Strawberry, "Strawberry")
    });
    clear_focus(&mut harness);
    harness.press_key(Key::Tab);
    harness.press_key(Key::Home);
    assert_eq!(
        selected.get(),
        Flavor::Vanilla,
        "Home at the first choice chooses it"
    );
    assert_eq!(focused(&mut harness).as_deref(), Some("Vanilla"));
}

#[test]
fn a_choice_disabled_while_focused_hands_the_choice_to_the_next_arrow() {
    let vanilla_on = RwSignal::new(true);
    let selected = RwSignal::new(Flavor::Vanilla);
    let bind = selected.clone();
    let switch = vanilla_on.clone();
    let mut harness = window(bind, move |selected| {
        let off = switch.clone();
        radio_group(selected)
            .option_disabled(Flavor::Vanilla, "Vanilla", move || !off.get())
            .option(Flavor::Strawberry, "Strawberry")
    });
    harness.press_key(Key::Tab);
    assert_eq!(focused(&mut harness).as_deref(), Some("Vanilla"));

    vanilla_on.set(false);
    harness.frame();
    assert_eq!(
        focused(&mut harness).as_deref(),
        None,
        "disabling the focused choice takes the focus"
    );

    harness.press_key(Key::ArrowDown);
    assert_eq!(
        selected.get(),
        Flavor::Strawberry,
        "the arrow routes from the group, which still moves the choice"
    );
    assert_eq!(focused(&mut harness).as_deref(), Some("Strawberry"));
}

#[test]
fn a_group_that_is_the_whole_window_is_still_one_tab_stop_at_the_chosen_one() {
    TextSystem::bundled_only().install();
    let selected = RwSignal::new(Flavor::Strawberry);
    let bind = selected.clone();
    let mut harness = Harness::new(300.0, 120.0, 1.0, move || {
        let selected = bind.clone();
        radio_group(selected)
            .option(Flavor::Vanilla, "Vanilla")
            .option(Flavor::Strawberry, "Strawberry")
    });
    harness.frame();

    harness.press_key(Key::Tab);
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Strawberry"),
        "Tab enters at the chosen choice even when the group is the tree's top"
    );
    harness.press_key(Key::Tab);
    assert_eq!(
        focused(&mut harness).as_deref(),
        Some("Strawberry"),
        "the group is one stop, so Tab never walks the choices one by one"
    );

    harness.press_key(Key::ArrowDown);
    assert_eq!(
        selected.get(),
        Flavor::Vanilla,
        "the arrows still walk the choices from the top"
    );
    assert_eq!(focused(&mut harness).as_deref(), Some("Vanilla"));
}

#[test]
fn a_second_arrow_before_a_frame_walks_the_choices_the_tree_applied() {
    let selected = RwSignal::new(Flavor::Vanilla);
    let bind = selected.clone();
    let mut harness = window(bind, move |selected| {
        let gone = selected.clone();
        radio_group(selected)
            .option_disabled(Flavor::Vanilla, "Vanilla", move || {
                gone.get() == Flavor::Strawberry
            })
            .option(Flavor::Strawberry, "Strawberry")
            .option(Flavor::Mint, "Mint")
    });
    clear_focus(&mut harness);
    harness.press_key(Key::Tab);
    assert_eq!(focused(&mut harness).as_deref(), Some("Vanilla"));

    harness.press_key(Key::ArrowDown);
    assert_eq!(selected.get(), Flavor::Strawberry);
    harness.press_key(Key::ArrowDown);
    assert_eq!(
        selected.get(),
        Flavor::Mint,
        "the second Down still moves the choice to where it lands"
    );
    assert_eq!(focused(&mut harness).as_deref(), Some("Mint"));
}

#[test]
fn the_arrows_pass_over_a_disabled_choice_and_a_press_on_it_does_nothing() {
    let selected = RwSignal::new(Flavor::Vanilla);
    let bind = selected.clone();
    let mut harness = window(bind, |selected| {
        radio_group(selected)
            .option(Flavor::Vanilla, "Vanilla")
            .option_disabled(Flavor::Strawberry, "Strawberry", true)
            .option(Flavor::Mint, "Mint")
    });
    let strawberry = named(&mut harness, "Strawberry");
    assert!(strawberry.disabled, "the projection reports the choice off");

    harness.press_key(Key::Tab);
    harness.press_key(Key::ArrowDown);
    assert_eq!(
        selected.get(),
        Flavor::Mint,
        "Down passes the disabled choice"
    );
    assert_eq!(focused(&mut harness).as_deref(), Some("Mint"));
    harness.press_key(Key::ArrowDown);
    assert_eq!(
        selected.get(),
        Flavor::Vanilla,
        "Down wraps to the first, which the disabled choice never blocks"
    );
    harness.press_key(Key::ArrowUp);
    assert_eq!(
        selected.get(),
        Flavor::Mint,
        "Up passes the disabled choice backwards too"
    );

    harness.click(middle(&strawberry));
    assert_eq!(
        selected.get(),
        Flavor::Mint,
        "a press on a disabled choice does nothing"
    );
    assert_ne!(
        focused(&mut harness).as_deref(),
        Some("Strawberry"),
        "and it cannot take the focus"
    );
}

#[test]
fn the_projection_reports_a_named_group_of_placed_choices() {
    let selected = RwSignal::new(Flavor::Strawberry);
    let mut harness = window(selected.clone(), flavors);

    let group = named(&mut harness, "Flavor");
    assert_eq!(group.role, Role::Group);
    assert_eq!(
        group.set_size,
        Some(3),
        "the group holds how many choices the set has"
    );
    let (vanilla, strawberry, mint) = (
        named(&mut harness, "Vanilla"),
        named(&mut harness, "Strawberry"),
        named(&mut harness, "Mint"),
    );
    assert_eq!(
        group.children,
        vec![vanilla.id, strawberry.id, mint.id],
        "the group holds the choices in order"
    );
    for (node, position, name) in [
        (&vanilla, 0, "Vanilla"),
        (&strawberry, 1, "Strawberry"),
        (&mint, 2, "Mint"),
    ] {
        assert_eq!(node.role, Role::RadioButton);
        assert_eq!(node.position, Some(position), "{name}'s place");
        assert_eq!(node.set_size, None, "the size lives on the group alone");
        assert!(node.focusable, "{name} can take the focus");
    }
    assert_eq!(vanilla.checked, Some(Checked::False));
    assert_eq!(strawberry.checked, Some(Checked::True));
    assert_eq!(mint.checked, Some(Checked::False));
    assert_eq!(
        [vanilla.selected, strawberry.selected, mint.selected],
        [Some(false), Some(true), Some(false)],
        "the chosen choice is the one Tab enters at"
    );
}

#[test]
fn a_choice_change_repaints_only_the_two_choices_it_touches() {
    let selected = RwSignal::new(Flavor::Vanilla);
    let mut harness = window(selected.clone(), flavors);
    harness.settle();
    let (vanilla, strawberry) = (
        named(&mut harness, "Vanilla").bounds,
        named(&mut harness, "Strawberry").bounds,
    );

    let hover = named(&mut harness, "Strawberry");
    harness.move_to(middle(&hover));
    harness.frame();
    harness.click(middle(&hover));
    let report = harness.frame();
    assert!(
        !report.layout_ran,
        "a choice change repaints without laying out"
    );
    let pixels = harness.app().stats().damage_pixels as f32;
    let budget = vanilla.area() + strawberry.area();
    assert!(
        pixels <= budget,
        "{pixels} damage pixels stayed within the two choices' {budget}"
    );
    // The damage is in physical pixels and the window runs at scale 1.0, so
    // the two are the same size here.
    for rect in &report.damage {
        let (left, top) = (rect.x as f32, rect.y as f32);
        let (right, bottom) = (left + rect.width as f32, top + rect.height as f32);
        let on_vanilla = left >= vanilla.x0 - 1.0
            && right <= vanilla.x1 + 1.0
            && top >= vanilla.y0 - 1.0
            && bottom <= vanilla.y1 + 1.0;
        let on_strawberry = left >= strawberry.x0 - 1.0
            && right <= strawberry.x1 + 1.0
            && top >= strawberry.y0 - 1.0
            && bottom <= strawberry.y1 + 1.0;
        assert!(
            on_vanilla || on_strawberry,
            "damage stayed on the choice that gained the dot and the one that lost it: {rect:?}"
        );
    }
    harness.assert_matches_full_repaint();
}

/// A window with a column group of three flavors, one disabled, a second
/// group with no choice matching its signal, and a horizontal group whose
/// second choice the pointer rests on.
fn states_window(theme: Theme) -> Harness {
    TextSystem::bundled_only().install();
    let (flavor, served, side) = (
        RwSignal::new(Flavor::Vanilla),
        RwSignal::new(Flavor::Mint),
        RwSignal::new(true),
    );
    let (flavor, served, side) = (flavor.clone(), served.clone(), side.clone());
    let mut harness = Harness::new(220.0, 240.0, 1.0, move || {
        let (flavor, served, side) = (flavor.clone(), served.clone(), side.clone());
        column()
            .padding(px(10.0))
            .gap(Size {
                width: 0.0,
                height: 10.0,
            })
            .align_items(Align::Start)
            .child(
                radio_group(flavor)
                    .option(Flavor::Vanilla, "Vanilla")
                    .option_disabled(Flavor::Strawberry, "Strawberry", true)
                    .option(Flavor::Mint, "Mint")
                    .accessible_label("Flavor"),
            )
            .child(
                radio_group(served)
                    .option(Flavor::Vanilla, "Cone")
                    .option(Flavor::Strawberry, "Cup")
                    .accessible_label("Served in"),
            )
            .child(
                radio_group(side)
                    .option(true, "For here")
                    .option(false, "To go")
                    .horizontal()
                    .accessible_label("Where"),
            )
    });
    harness.app().set_theme(theme);
    harness.settle();
    harness.press_key(Key::Tab);
    let to_go = named(&mut harness, "To go");
    harness.move_to(middle(&to_go));
    harness.frame();
    harness
}

#[test]
fn the_states_of_a_radio_group_in_the_light_theme() {
    insta::assert_binary_snapshot!(".png", states_window(Theme::light()).png());
}

#[test]
fn the_states_of_a_radio_group_in_the_dark_theme() {
    insta::assert_binary_snapshot!(".png", states_window(Theme::dark()).png());
}
