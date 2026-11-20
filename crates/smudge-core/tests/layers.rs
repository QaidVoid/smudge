//! The layers overlays draw in: their order, how each kind closes, and the
//! order a screen reader meets them in.

use std::cell::Cell;
use std::rc::Rc;

use smudge_core::element::{Element, Layer, Placement};
use smudge_core::geometry::{Color, Point};
use smudge_core::input::{InputEvent, Key};
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::semantics::Role;
use smudge_core::style::{Edges, Length, px};
use smudge_core::widget::{Panel, panel};
use smudge_test::Harness;

/// The color of a dialog's backdrop.
const DIALOG: Color = Color::rgb(20, 20, 120);
/// The color of the popup opened from the app's tree.
const APP_POPUP: Color = Color::rgb(200, 30, 30);
/// The color of the popup opened from inside the dialog.
const DIALOG_POPUP: Color = Color::rgb(30, 200, 30);

/// A counter shared with a handler.
type Count = Rc<Cell<u32>>;

/// Places a node at `left`, `top` of its parent.
fn at(left: f32, top: f32) -> Edges<Length> {
    Edges {
        left: Length::px(left),
        top: Length::px(top),
        right: Length::Auto,
        bottom: Length::Auto,
    }
}

/// A popup of 80 by 50 in `color`, named `name`, that counts its presses.
fn popup(color: Color, name: &'static str, presses: Count) -> Element<Panel> {
    panel()
        .width(px(80.0))
        .height(px(50.0))
        .background(color)
        .role(Role::Button)
        .accessible_label(name)
        .on_press(move || presses.set(presses.get() + 1))
}

/// The signals of the scene's overlays, and the presses each popup heard.
struct Scene {
    dialog: RwSignal<bool>,
    app_popup: RwSignal<bool>,
    dialog_popup: RwSignal<bool>,
    tooltip: RwSignal<bool>,
    toast: RwSignal<bool>,
    app_presses: Count,
    dialog_presses: Count,
}

/// A window of 200 by 120 whose tree opens a popup at 0, 60, a tooltip and
/// a toast, and a dialog that covers the window and opens a popup of its own
/// at 0, 60 too, each while its signal reads `true`.
fn scene() -> (Harness, Scene) {
    let scene = Scene {
        dialog: RwSignal::new(false),
        app_popup: RwSignal::new(false),
        dialog_popup: RwSignal::new(false),
        tooltip: RwSignal::new(false),
        toast: RwSignal::new(false),
        app_presses: Rc::new(Cell::new(0)),
        dialog_presses: Rc::new(Cell::new(0)),
    };
    let (dialog, app_popup, dialog_popup, tooltip, toast) = (
        scene.dialog.clone(),
        scene.app_popup.clone(),
        scene.dialog_popup.clone(),
        scene.tooltip.clone(),
        scene.toast.clone(),
    );
    let (app_presses, dialog_presses) = (
        Rc::clone(&scene.app_presses),
        Rc::clone(&scene.dialog_presses),
    );
    let harness = Harness::new(200.0, 120.0, 1.0, move || {
        let (dialog_popup, dialog_presses) = (dialog_popup.clone(), Rc::clone(&dialog_presses));
        let app_presses = Rc::clone(&app_presses);
        panel()
            .role(Role::Group)
            .accessible_label("Tree")
            .child(
                panel()
                    .absolute()
                    .inset(at(0.0, 40.0))
                    .width(px(10.0))
                    .height(px(20.0))
                    .overlay(app_popup.clone(), Placement::Below, move || {
                        popup(APP_POPUP, "App popup", Rc::clone(&app_presses))
                    })
                    .overlay_in(Layer::Tooltip, tooltip.clone(), Placement::Below, || {
                        panel()
                            .width(px(30.0))
                            .height(px(10.0))
                            .role(Role::Tooltip)
                            .accessible_label("Tooltip")
                    })
                    .overlay_in(Layer::Toast, toast.clone(), Placement::BottomRight, || {
                        panel()
                            .width(px(40.0))
                            .height(px(20.0))
                            .role(Role::Group)
                            .accessible_label("Toast")
                    }),
            )
            .child(panel().width(px(10.0)).height(px(10.0)).overlay_in(
                Layer::Dialog,
                dialog.clone(),
                Placement::Cover,
                move || {
                    let (dialog_popup, dialog_presses) =
                        (dialog_popup.clone(), Rc::clone(&dialog_presses));
                    panel()
                        .background(DIALOG)
                        .role(Role::Group)
                        .accessible_label("Dialog")
                        .child(
                            panel()
                                .absolute()
                                .inset(at(0.0, 40.0))
                                .width(px(10.0))
                                .height(px(20.0))
                                .overlay(dialog_popup.clone(), Placement::Below, move || {
                                    popup(DIALOG_POPUP, "Dialog popup", Rc::clone(&dialog_presses))
                                }),
                        )
                },
            ))
    });
    (harness, scene)
}

/// Returns whether the pixel at `x`, `y` is `color`.
fn shows(harness: &Harness, x: u32, y: u32, color: Color) -> bool {
    harness.pixel(x, y) == Some([color.r, color.g, color.b, 255])
}

#[test]
fn a_popup_of_the_app_opened_after_a_dialog_draws_under_it_and_one_of_the_dialog_over_it() {
    let (mut harness, scene) = scene();
    harness.frame();
    scene.dialog.set(true);
    harness.frame();
    scene.app_popup.set(true);
    harness.frame();

    assert!(
        shows(&harness, 20, 80, DIALOG),
        "the dialog covers the app's popup"
    );
    harness.click(Point { x: 20.0, y: 80.0 });
    assert_eq!(scene.app_presses.get(), 0, "and takes its presses");

    scene.dialog_popup.set(true);
    harness.frame();
    assert!(
        shows(&harness, 20, 80, DIALOG_POPUP),
        "the dialog's own popup draws over it"
    );
    harness.click(Point { x: 20.0, y: 80.0 });
    assert_eq!(
        scene.dialog_presses.get(),
        1,
        "and takes the press over their overlap"
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn losing_the_window_focus_closes_popups_and_tooltips_and_leaves_dialogs_and_toasts() {
    let (mut harness, scene) = scene();
    harness.frame();
    for open in [
        &scene.app_popup,
        &scene.tooltip,
        &scene.dialog,
        &scene.toast,
    ] {
        open.set(true);
    }
    harness.frame();

    harness.event(InputEvent::WindowFocus(false));
    harness.frame();

    assert!(!scene.app_popup.get() && !scene.tooltip.get());
    assert!(scene.dialog.get() && scene.toast.get());
}

#[test]
fn escape_and_a_press_outside_write_false_and_a_toast_stays_through_both() {
    let (mut harness, scene) = scene();
    harness.frame();
    scene.toast.set(true);
    scene.app_popup.set(true);
    harness.frame();

    harness.click(Point { x: 150.0, y: 10.0 });
    harness.frame();
    assert!(!scene.app_popup.get(), "a press outside closes the popup");
    assert!(scene.toast.get(), "and leaves the toast");

    scene.dialog.set(true);
    harness.frame();
    harness.press_key(Key::Escape);
    harness.frame();
    assert!(!scene.dialog.get(), "Escape closes the dialog");
    harness.press_key(Key::Escape);
    harness.frame();
    assert!(scene.toast.get(), "and never a toast");

    scene.toast.set(false);
    harness.frame();
    let names: Vec<String> = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .filter_map(|node| node.label)
        .collect();
    assert!(
        !names.iter().any(|name| name == "Toast"),
        "writing false closes it"
    );
}

#[test]
fn a_screen_reader_meets_the_overlays_after_the_tree_in_the_order_they_draw() {
    let (mut harness, scene) = scene();
    harness.frame();
    scene.toast.set(true);
    harness.frame();
    scene.app_popup.set(true);
    scene.tooltip.set(true);
    harness.frame();
    let names = |harness: &mut Harness| -> Vec<String> {
        harness
            .app()
            .semantics_nodes()
            .into_iter()
            .filter_map(|node| node.label)
            .collect()
    };
    assert_eq!(
        names(&mut harness),
        ["Tree", "App popup", "Tooltip", "Toast"]
    );

    scene.dialog.set(true);
    harness.frame();
    scene.dialog_popup.set(true);
    harness.frame();
    assert_eq!(
        names(&mut harness),
        ["Dialog", "Dialog popup", "Toast"],
        "a dialog keeps a reader from everything under it"
    );
}

#[test]
fn opening_and_closing_a_popup_damages_its_own_rectangle_alone() {
    let (mut harness, scene) = scene();
    harness.frame();
    let area = |report: &smudge_test::FrameReport| -> u64 {
        report
            .damage
            .iter()
            .map(|rect| u64::from(rect.width) * u64::from(rect.height))
            .sum()
    };

    scene.app_popup.set(true);
    let opened = harness.frame();
    assert_eq!(area(&opened), 80 * 50, "{:?}", opened.damage);
    harness.assert_matches_full_repaint();

    scene.app_popup.set(false);
    let closed = harness.frame();
    assert_eq!(area(&closed), 80 * 50, "{:?}", closed.damage);
    harness.assert_matches_full_repaint();
}

#[test]
fn a_press_on_a_toast_or_inside_a_sibling_dialog_leaves_a_dialog_open() {
    let (first, second, toast) = (
        RwSignal::new(false),
        RwSignal::new(false),
        RwSignal::new(false),
    );
    let toast_presses: Count = Rc::new(Cell::new(0));
    let (a, b, t, presses) = (
        first.clone(),
        second.clone(),
        toast.clone(),
        Rc::clone(&toast_presses),
    );
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        let presses = Rc::clone(&presses);
        panel()
            .child(
                panel().overlay_in(Layer::Dialog, a.clone(), Placement::Cover, || {
                    panel().background(DIALOG)
                }),
            )
            .child(
                panel().overlay_in(Layer::Dialog, b.clone(), Placement::Cover, || {
                    panel().child(panel().width(px(40.0)).height(px(20.0)).on_press(|| {}))
                }),
            )
            .child(
                panel().overlay_in(Layer::Toast, t.clone(), Placement::BottomRight, move || {
                    let presses = Rc::clone(&presses);
                    panel().child(
                        panel()
                            .width(px(40.0))
                            .height(px(20.0))
                            .on_press(move || presses.set(presses.get() + 1)),
                    )
                }),
            )
    });
    harness.frame();
    for open in [&first, &second, &toast] {
        open.set(true);
        harness.frame();
    }

    harness.click(Point { x: 10.0, y: 10.0 });
    harness.frame();
    assert!(
        first.get() && second.get(),
        "a press inside the top dialog closes no dialog"
    );

    harness.click(Point { x: 180.0, y: 110.0 });
    harness.frame();
    assert_eq!(toast_presses.get(), 1, "the toast heard the press");
    assert!(first.get() && second.get(), "and both dialogs stay open");
}

#[test]
fn the_room_between_and_beside_toasts_lets_presses_through_to_the_window() {
    let toast = RwSignal::new(false);
    let (under, wide): (Count, Count) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (t, under_presses, wide_presses) = (toast.clone(), Rc::clone(&under), Rc::clone(&wide));
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        let wide_presses = Rc::clone(&wide_presses);
        let under_presses = Rc::clone(&under_presses);
        panel()
            .width(px(200.0))
            .height(px(120.0))
            .on_press(move || under_presses.set(under_presses.get() + 1))
            .overlay_in(Layer::Toast, t.clone(), Placement::BottomRight, move || {
                let wide_presses = Rc::clone(&wide_presses);
                panel()
                    .column()
                    .align_items(smudge_core::style::Align::End)
                    .gap(smudge_core::geometry::Size {
                        width: 0.0,
                        height: 10.0,
                    })
                    .child(
                        panel()
                            .width(px(80.0))
                            .height(px(20.0))
                            .on_press(move || wide_presses.set(wide_presses.get() + 1)),
                    )
                    .child(panel().width(px(20.0)).height(px(20.0)).on_press(|| {}))
            })
    });
    harness.frame();
    toast.set(true);
    harness.frame();

    harness.click(Point { x: 130.0, y: 110.0 });
    harness.click(Point { x: 150.0, y: 95.0 });
    assert_eq!(
        under.get(),
        2,
        "beside the narrow toast and between the two, the window hears it"
    );
    harness.click(Point { x: 150.0, y: 80.0 });
    assert_eq!(
        (under.get(), wide.get()),
        (2, 1),
        "on a toast, the toast does"
    );
}

/// A focusable button named `name` that counts its presses into `presses`.
fn counted(name: &'static str, presses: Count) -> Element<Panel> {
    panel()
        .width(px(40.0))
        .height(px(20.0))
        .role(Role::Button)
        .accessible_label(name)
        .focusable()
        .on_press(move || presses.set(presses.get() + 1))
}

#[test]
fn escape_closes_a_tooltip_inside_a_dialog_before_the_dialog() {
    let (dialog, tooltip) = (RwSignal::new(true), RwSignal::new(false));
    let (d, t) = (dialog.clone(), tooltip.clone());
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        let t = t.clone();
        panel().overlay_in(Layer::Dialog, d.clone(), Placement::Cover, move || {
            panel().trap_focus().child(
                counted("Keep", Rc::new(Cell::new(0)))
                    .autofocus()
                    .overlay_in(Layer::Tooltip, t.clone(), Placement::Below, || {
                        panel().width(px(30.0)).height(px(10.0))
                    }),
            )
        })
    });
    harness.frame();
    harness.frame();
    tooltip.set(true);
    harness.frame();

    harness.press_key(Key::Escape);
    harness.frame();
    assert!(!tooltip.get(), "the tooltip closed");
    assert!(dialog.get(), "and the dialog stayed");
    harness.press_key(Key::Escape);
    harness.frame();
    assert!(!dialog.get(), "the next Escape closes the dialog");
}

#[test]
fn escape_leaves_a_toast_that_holds_the_focus() {
    let toast = RwSignal::new(true);
    let t = toast.clone();
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        panel().overlay_in(Layer::Toast, t.clone(), Placement::BottomRight, || {
            panel().child(counted("Undo", Rc::new(Cell::new(0))).autofocus())
        })
    });
    harness.frame();
    harness.frame();
    harness.press_key(Key::Escape);
    harness.frame();
    assert!(toast.get());
}

#[test]
fn the_context_menu_key_ignores_a_focused_node_a_dialog_hides() {
    let dialog = RwSignal::new(false);
    let menus: Count = Rc::new(Cell::new(0));
    let (d, opened) = (dialog.clone(), Rc::clone(&menus));
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        let opened = Rc::clone(&opened);
        panel()
            .child(
                counted("Row", Rc::new(Cell::new(0)))
                    .autofocus()
                    .on_context_menu(move |_| opened.set(opened.get() + 1)),
            )
            .child(
                panel().overlay_in(Layer::Dialog, d.clone(), Placement::Cover, || {
                    panel().role(Role::Group).accessible_label("Please wait")
                }),
            )
    });
    harness.frame();
    harness.frame();
    harness.press_key(Key::ContextMenu);
    assert_eq!(menus.get(), 1, "the row's menu opens with no dialog");

    dialog.set(true);
    harness.frame();
    harness.press_key(Key::ContextMenu);
    assert_eq!(menus.get(), 1, "and not while a dialog hides the row");
}

#[test]
fn an_overlay_switched_to_cover_the_window_and_back_takes_each_size_and_keeps_its_own() {
    let placement = RwSignal::new(Placement::Below);
    let p = placement.clone();
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        let p = p.clone();
        panel().width(px(10.0)).height(px(10.0)).overlay(
            RwSignal::new(true),
            move || p.get(),
            || {
                panel()
                    .width(px(150.0))
                    .role(Role::Group)
                    .accessible_label("Sheet")
                    .child(panel().width(px(40.0)).height(px(30.0)))
            },
        )
    });
    let size = |harness: &mut Harness| {
        let bounds = harness
            .app()
            .semantics_nodes()
            .into_iter()
            .find(|node| node.label.as_deref() == Some("Sheet"))
            .expect("the overlay shows")
            .bounds;
        (bounds.width(), bounds.height())
    };
    harness.frame();
    assert_eq!(size(&mut harness), (150.0, 30.0));

    placement.set(Placement::Cover);
    harness.frame();
    assert_eq!(
        size(&mut harness),
        (200.0, 120.0),
        "covering, it takes the window"
    );

    placement.set(Placement::BottomRight);
    harness.frame();
    assert_eq!(
        size(&mut harness),
        (150.0, 30.0),
        "and the size it asks for again after"
    );
}

#[test]
fn a_menu_that_fades_out_under_a_new_dialog_leaves_the_focus_off_its_button() {
    let (menu, dialog) = (RwSignal::new(true), RwSignal::new(false));
    let (m, d) = (menu.clone(), dialog.clone());
    let mut harness = Harness::new(200.0, 120.0, 1.0, move || {
        panel()
            .child(counted("Menu", Rc::new(Cell::new(0))).overlay(
                m.clone(),
                Placement::Below,
                || {
                    panel()
                        .trap_focus()
                        .exit(smudge_core::animation::Spring::SMOOTH)
                        .child(counted("Item", Rc::new(Cell::new(0))).autofocus())
                },
            ))
            .child(
                panel().overlay_in(Layer::Dialog, d.clone(), Placement::Cover, || {
                    panel().role(Role::Group).accessible_label("Saving")
                }),
            )
    });
    harness.settle();
    let name = |harness: &mut Harness, id| {
        harness
            .app()
            .semantics_nodes()
            .into_iter()
            .find(|node| Some(node.id) == id)
            .and_then(|node| node.label)
    };
    let focused = harness.app().focused();
    assert_eq!(name(&mut harness, focused).as_deref(), Some("Item"));
    let button = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .find(|node| node.label.as_deref() == Some("Menu"))
        .expect("the button shows")
        .id;

    dialog.set(true);
    menu.set(false);
    for _ in 0..60 {
        harness.advance(std::time::Duration::from_millis(50));
        harness.frame();
    }
    assert!(
        name(&mut harness, Some(button)).is_none(),
        "the dialog hides the button"
    );

    assert_ne!(
        harness.app().focused(),
        Some(button),
        "the focus never lands behind the dialog"
    );
}
