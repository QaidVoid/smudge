//! Tests of the projection from an app's semantics onto AccessKit updates.

use std::collections::BTreeMap;

use smudge_a11y::Projection;
use smudge_core::app::{App, AppConfig, RepaintMode};
use smudge_core::element::{Element, for_each, show};
use smudge_core::geometry::{Color, Size};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::RwSignal;
use smudge_core::reactive::prelude::*;
use smudge_core::semantics::{Checked, Role, Semantics, ValueRange};
use smudge_core::style::px;
use smudge_core::theme::ColorValue;
use smudge_core::widget::{PaintCx, Panel, Widget};

use accesskit::{Action, NodeId, Role as AccessKitRole, Toggled};
use accesskit_consumer::{NodeRef, Tree, TreeChangeHandler};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

/// The node id the projection uses for the window root.
const WINDOW_ROOT: NodeId = NodeId(u64::MAX);

/// A widget whose semantics report the text they were built with as their
/// value, the way a static text does.
struct Text(&'static str);

impl Widget for Text {
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}

    fn semantics(&self) -> Option<Semantics> {
        Some(Semantics {
            value: Some(self.0.to_owned()),
            ..Semantics::new(Role::Label)
        })
    }
}

/// An app whose root is a group holding the widgets one window needs.
fn app_with(build: impl FnOnce() -> Element<Panel>) -> App {
    App::new(
        AppConfig {
            viewport: Size {
                width: 200.0,
                height: 100.0,
            },
            scale: 2.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Damage,
        },
        build,
    )
}

/// Runs a frame and returns the tree update the projection builds.
fn update_after_frame(
    app: &mut App,
    projection: &mut Projection,
    full: bool,
) -> accesskit::TreeUpdate {
    app.frame();
    projection.update(app, 400.0, 200.0, 2.0, full)
}

/// Returns the accesskit node the projection built for the projected node
/// with `role`, failing the test when there is not exactly one.
fn node_with_role(update: &accesskit::TreeUpdate, role: AccessKitRole) -> &accesskit::Node {
    let matches: Vec<_> = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == role)
        .collect();
    assert_eq!(matches.len(), 1, "exactly one node has the role {role:?}");
    &matches[0].1
}

/// Returns the node id the projection assigned to the node with `role`.
fn id_with_role(update: &accesskit::TreeUpdate, role: AccessKitRole) -> NodeId {
    let matches: Vec<_> = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == role)
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(matches.len(), 1, "exactly one node has the role {role:?}");
    matches[0]
}

#[test]
fn a_full_update_carries_the_window_and_every_projected_node() {
    let mut app = app_with(|| {
        Element::new(Panel)
            .role(Role::Group)
            .child(
                Element::new(Panel)
                    .role(Role::Button)
                    .accessible_label("OK")
                    .focusable()
                    .on_press(|| {}),
            )
            .child(Element::new(Text("hello")))
    });
    let mut projection = Projection::new("Smudge");

    let update = update_after_frame(&mut app, &mut projection, true);

    let tree = update
        .tree
        .as_ref()
        .expect("a full update carries the tree");
    assert_eq!(tree.root, WINDOW_ROOT);
    assert_eq!(update.tree_id, accesskit::TreeId::ROOT);

    let window = node_with_role(&update, AccessKitRole::Window);
    assert_eq!(
        window.bounds(),
        Some(accesskit::Rect {
            x0: 0.0,
            y0: 0.0,
            x1: 400.0,
            y1: 200.0,
        })
    );

    let button = node_with_role(&update, AccessKitRole::Button);
    assert_eq!(button.label().map(str::to_owned), Some("OK".to_owned()));
    assert!(button.supports_action(Action::Click) || button.child_supports_action(Action::Click));
    assert!(button.supports_action(Action::Focus));
    assert!(!button.is_disabled());

    let text = node_with_role(&update, AccessKitRole::Label);
    assert_eq!(
        text.label(),
        None,
        "a static text takes its name from its value"
    );
    assert_eq!(text.value().map(str::to_owned), Some("hello".to_owned()));

    // The window, the group, the button and the text, and nothing else.
    assert_eq!(update.nodes.len(), 4);
}

/// A text field that reports `role` and holds `value`.
struct Field(Role, &'static str);

impl Widget for Field {
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}

    fn semantics(&self) -> Option<Semantics> {
        Some(Semantics {
            value: Some(self.1.to_owned()),
            ..Semantics::new(self.0)
        })
    }
}

#[test]
fn a_multi_line_text_field_projects_as_a_multi_line_input() {
    let mut app = app_with(|| {
        Element::new(Panel)
            .role(Role::Group)
            .child(Element::new(Field(Role::TextInput, "one line")))
            .child(Element::new(Field(
                Role::MultilineTextInput,
                "first\nsecond",
            )))
    });
    let mut projection = Projection::new("Smudge");

    let update = update_after_frame(&mut app, &mut projection, true);
    let area = node_with_role(&update, AccessKitRole::MultilineTextInput);
    assert_eq!(area.value(), Some("first\nsecond"));
    let field = node_with_role(&update, AccessKitRole::TextInput);
    assert_eq!(field.value(), Some("one line"));

    // The platform adapters read the multi-line state from the role.
    let tree = Tree::new(update, true);
    let group = tree
        .state()
        .root()
        .children()
        .next()
        .expect("the window holds the group");
    let multiline: Vec<_> = group
        .children()
        .map(|node| (node.is_text_input(), node.is_multiline()))
        .collect();
    assert_eq!(multiline, [(true, false), (true, true)]);
}

#[test]
fn bounds_land_where_the_widgets_draw_at_the_app_scale() {
    let mut app = app_with(|| {
        Element::new(Panel).child(
            Element::new(Panel)
                .role(Role::Group)
                .width(px(100.0))
                .height(px(50.0)),
        )
    });
    let mut projection = Projection::new("Smudge");
    app.frame();

    let update = projection.update(&mut app, 400.0, 200.0, 2.0, true);
    let group = node_with_role(&update, AccessKitRole::Group);
    let bounds = group.bounds().expect("a laid-out node has bounds");
    assert_eq!(bounds.x0, 0.0);
    assert_eq!(bounds.y0, 0.0);
    assert_eq!(bounds.x1, 200.0, "100.0 logical at scale 2");
    assert_eq!(bounds.y1, 100.0, "50.0 logical at scale 2");
}

#[test]
fn a_child_of_an_unprojected_node_attaches_to_the_nearest_projected_ancestor() {
    let mut app = app_with(|| {
        Element::new(Panel)
            .role(Role::Group)
            .child(Element::new(Panel).child(Element::new(Text("deep"))))
    });
    let mut projection = Projection::new("Smudge");

    let update = update_after_frame(&mut app, &mut projection, true);

    let text_id = id_with_role(&update, AccessKitRole::Label);
    let group_id = id_with_role(&update, AccessKitRole::Group);
    assert_eq!(
        update
            .nodes
            .iter()
            .find(|(id, _)| *id == WINDOW_ROOT)
            .map(|(_, node)| node.children().to_vec()),
        Some(vec![group_id]),
    );
    assert_eq!(
        node_with_role(&update, AccessKitRole::Group).children(),
        &[text_id],
        "the unprojected panel between them is skipped"
    );
}

#[test]
fn focus_reports_the_focused_node_and_falls_back_to_the_window() {
    let mut app = app_with(|| {
        Element::new(Panel)
            .role(Role::Group)
            .child(
                Element::new(Panel)
                    .role(Role::Button)
                    .accessible_label("OK")
                    .focusable()
                    .on_press(|| {}),
            )
            .child(Element::new(Text("plain")))
    });
    let mut projection = Projection::new("Smudge");
    app.frame();

    let update = projection.update(&mut app, 400.0, 200.0, 2.0, true);
    assert_eq!(update.focus, WINDOW_ROOT, "nothing is focused yet");

    let button_id = app
        .semantics_nodes()
        .iter()
        .find(|node| node.role == Role::Button)
        .map(|node| node.id)
        .expect("the button projects");
    assert!(app.request_focus(button_id));
    app.frame();
    let update = projection.update(&mut app, 400.0, 200.0, 2.0, false);
    assert_eq!(update.focus, NodeId(button_id.as_u64()));
}

#[test]
fn an_incremental_update_carries_only_the_changed_node() {
    let name = RwSignal::new(String::from("before"));
    let mut app = app_with(|| {
        let name = name.clone();
        Element::new(Panel).role(Role::Group).child(
            Element::new(Panel)
                .role(Role::Button)
                .accessible_label(move || name.get())
                .focusable()
                .on_press(|| {}),
        )
    });
    let mut projection = Projection::new("Smudge");
    app.frame();
    let before = projection.update(&mut app, 400.0, 200.0, 2.0, true);

    name.set(String::from("after"));
    let update = update_after_frame(&mut app, &mut projection, false);

    assert!(
        update.tree.is_none(),
        "an incremental update carries no tree"
    );
    let button = node_with_role(&update, AccessKitRole::Button);
    assert_eq!(button.label().map(str::to_owned), Some("after".to_owned()));
    let button_id = app
        .semantics_nodes()
        .iter()
        .find(|node| node.role == Role::Button)
        .map(|node| NodeId(node.id.as_u64()));
    assert_eq!(
        update.nodes.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![button_id.expect("the button projects")],
        "only the renamed button is in the update",
    );
    let _ = before;
}

#[test]
fn a_press_through_the_projection_runs_the_handler() {
    let pressed = RwSignal::new(0u32);
    let mut app = app_with(|| {
        let pressed = pressed.clone();
        Element::new(Panel).role(Role::Group).child(
            Element::new(Panel)
                .role(Role::Button)
                .accessible_label("OK")
                .on_press(move || pressed.update(|count| *count += 1)),
        )
    });
    let mut projection = Projection::new("Smudge");
    app.frame();
    let _ = projection.update(&mut app, 400.0, 200.0, 2.0, true);

    let button_id = app
        .semantics_nodes()
        .iter()
        .find(|node| node.role == Role::Button)
        .map(|node| node.id)
        .expect("the button projects");
    assert!(app.press(button_id));
    assert_eq!(pressed.get(), 1);
}

#[test]
fn a_disabled_button_reports_disabled_and_presses_nothing() {
    let pressed = RwSignal::new(0u32);
    let mut app = app_with(|| {
        let pressed = pressed.clone();
        Element::new(Panel).role(Role::Group).child(
            Element::new(Panel)
                .role(Role::Button)
                .accessible_label("OK")
                .disabled(true)
                .on_press(move || pressed.update(|count| *count += 1)),
        )
    });
    let mut projection = Projection::new("Smudge");
    app.frame();
    let update = projection.update(&mut app, 400.0, 200.0, 2.0, true);

    let button = node_with_role(&update, AccessKitRole::Button);
    assert!(button.is_disabled());
    assert!(
        button.supports_action(Action::Click),
        "the action stays offered"
    );
    assert!(!button.supports_action(Action::Focus));

    let button_id = app
        .semantics_nodes()
        .iter()
        .find(|node| node.role == Role::Button)
        .map(|node| node.id)
        .expect("the button projects");
    assert!(!app.press(button_id));
    assert_eq!(pressed.get(), 0);
}

#[test]
fn a_hidden_node_leaves_the_projection_and_returns_when_shown() {
    let shown = RwSignal::new(true);
    let mut app = app_with(|| {
        let shown = shown.clone();
        Element::new(Panel).role(Role::Group).child(
            Element::new(Panel)
                .role(Role::Button)
                .accessible_label("OK")
                .hidden(move || !shown.get())
                .focusable()
                .on_press(|| {}),
        )
    });
    let mut projection = Projection::new("Smudge");
    app.frame();
    let update = projection.update(&mut app, 400.0, 200.0, 2.0, true);
    assert!(
        node_with_role(&update, AccessKitRole::Button)
            .bounds()
            .is_some(),
        "the visible button projects"
    );

    shown.set(false);
    let update = update_after_frame(&mut app, &mut projection, false);
    assert!(
        !update
            .nodes
            .iter()
            .any(|(_, node)| node.role() == AccessKitRole::Button),
        "the hidden button is not rebuilt"
    );

    shown.set(true);
    let update = update_after_frame(&mut app, &mut projection, false);
    assert!(
        update
            .nodes
            .iter()
            .any(|(_, node)| node.role() == AccessKitRole::Button),
        "the shown button is rebuilt"
    );
}

#[test]
fn a_repaint_that_changes_nothing_projected_sends_no_node() {
    let shade = RwSignal::new(Color::WHITE);
    let mut app = app_with(|| {
        let shade = shade.clone();
        Element::new(Panel).role(Role::Group).child(
            Element::new(Text("value"))
                .width(px(40.0))
                .height(px(20.0))
                .background(move || ColorValue::from(shade.get())),
        )
    });
    let mut projection = Projection::new("Smudge");
    app.frame();
    let _ = projection.update(&mut app, 400.0, 200.0, 2.0, true);

    shade.set(Color::BLACK);
    let update = update_after_frame(&mut app, &mut projection, false);
    assert!(
        update.nodes.is_empty(),
        "a node whose projection did not change is not sent again: {:?}",
        update.nodes
    );
}

#[test]
fn an_update_with_no_changes_carries_no_nodes() {
    let mut app = app_with(|| {
        Element::new(Panel).role(Role::Group).child(
            Element::new(Panel)
                .role(Role::Button)
                .accessible_label("OK")
                .on_press(|| {}),
        )
    });
    let mut projection = Projection::new("Smudge");
    app.frame();
    let _ = projection.update(&mut app, 400.0, 200.0, 2.0, true);
    assert!(!app.has_a11y_changes(), "a settled app has no changes");

    app.frame();
    let update = projection.update(&mut app, 400.0, 200.0, 2.0, false);
    assert!(update.tree.is_none());
    assert!(update.nodes.is_empty(), "a quiet frame rebuilds nothing");
}

#[test]
fn an_app_with_no_semantics_projects_only_the_window() {
    let mut app = app_with(|| Element::new(Panel));
    let mut projection = Projection::new("Smudge");

    let update = update_after_frame(&mut app, &mut projection, true);

    assert_eq!(update.nodes.len(), 1);
    assert_eq!(update.nodes[0].0, WINDOW_ROOT);
    assert_eq!(update.nodes[0].1.role(), AccessKitRole::Window);
}

#[test]
fn the_initial_tree_names_each_role_under_its_parent() {
    let mut app = app_with(|| {
        Element::new(Panel)
            .child(
                Element::new(Panel)
                    .role(Role::Button)
                    .accessible_label("Save")
                    .focusable()
                    .on_press(|| {})
                    .child(Element::new(Text("Save"))),
            )
            .child(
                Element::new(Panel)
                    .role(Role::List)
                    .child(Element::new(Panel).child(Element::new(Text("first"))))
                    .child(Element::new(Text("second"))),
            )
    });
    let mut projection = Projection::new("Smudge");

    let update = update_after_frame(&mut app, &mut projection, true);

    let tree = Tree::new(update, true);
    let mut outline = String::new();
    write_outline(tree.state().root(), 0, &mut outline);
    insta::assert_snapshot!(outline);
}

/// Appends one line per node below `node`, indented by depth, with the role,
/// the label and the value.
fn write_outline(node: NodeRef<'_>, depth: usize, out: &mut String) {
    let data = node.data();
    out.push_str(&"  ".repeat(depth));
    out.push_str(&format!("{:?}", data.role()));
    if let Some(label) = data.label() {
        out.push_str(&format!(" label={label:?}"));
    }
    if let Some(value) = data.value() {
        out.push_str(&format!(" value={value:?}"));
    }
    out.push('\n');
    for child in node.children() {
        write_outline(child, depth + 1, out);
    }
}

/// A change handler that ignores the changes, for a tree that is only read.
struct Ignore;

impl TreeChangeHandler for Ignore {
    fn node_added(&mut self, _node: &NodeRef) {}
    fn node_updated(&mut self, _old: &NodeRef, _new: &NodeRef) {}
    fn focus_moved(&mut self, _old: Option<&NodeRef>, _new: Option<&NodeRef>) {}
    fn node_removed(&mut self, _node: &NodeRef) {}
}

/// Every node reachable from the root of `tree`, by id, and the focus.
fn reachable(tree: &Tree) -> (BTreeMap<NodeId, accesskit::Node>, NodeId) {
    fn walk(node: NodeRef<'_>, out: &mut BTreeMap<NodeId, accesskit::Node>) {
        out.insert(node.locate().0, node.data().clone());
        for child in node.children() {
            walk(child, out);
        }
    }
    let mut nodes = BTreeMap::new();
    walk(tree.state().root(), &mut nodes);
    (nodes, tree.state().focus_in_tree().locate().0)
}

/// The static texts a list row can show.
const ROW_TEXTS: [&str; 6] = ["zero", "one", "two", "three", "four", "five"];

/// One change to the app the property test drives.
#[derive(Clone, Debug)]
enum Edit {
    /// Shows or hides the text that a slot in the root holds.
    ToggleShown,
    /// Hides or shows the plain panel that holds the second button and an
    /// empty text.
    ToggleHidden,
    /// Disables or enables the first button.
    ToggleDisabled,
    /// Renames the first button.
    Rename(u8),
    /// Replaces the list rows with the ones whose bit is set, in order.
    Rows(u8),
    /// Focuses the projected focusable node at this place, if any.
    Focus(u8),
    /// Resizes the surface to this width.
    Resize(u16),
}

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        Just(Edit::ToggleShown),
        Just(Edit::ToggleHidden),
        Just(Edit::ToggleDisabled),
        any::<u8>().prop_map(Edit::Rename),
        (0u8..64).prop_map(Edit::Rows),
        any::<u8>().prop_map(Edit::Focus),
        (100u16..800).prop_map(Edit::Resize),
    ]
}

/// Writes a failure to `tests/proptest-regressions/`, beside this test, where
/// it is kept under version control.
fn config() -> ProptestConfig {
    ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/proptest-regressions/projection.txt"
        )))),
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn incremental_updates_keep_the_tree_equal_to_a_full_rebuild(
        edits in proptest::collection::vec(edit(), 1..12),
    ) {
        let shown = RwSignal::new(false);
        let hidden = RwSignal::new(false);
        let disabled = RwSignal::new(false);
        let name = RwSignal::new(String::from("first"));
        let rows = RwSignal::new(vec![0u8, 1]);
        let mut app = {
            let (shown, hidden, disabled, name, rows) =
                (shown.clone(), hidden.clone(), disabled.clone(), name.clone(), rows.clone());
            app_with(move || {
                Element::new(Panel)
                    .child(
                        Element::new(Panel)
                            .role(Role::Button)
                            .accessible_label(move || name.get())
                            .disabled(move || disabled.get())
                            .focusable()
                            .on_press(|| {})
                            .child(Element::new(Text("first"))),
                    )
                    .child(show(move || shown.get(), || Element::new(Text("shown"))))
                    .child(
                        Element::new(Panel)
                            .hidden(move || hidden.get())
                            .child(
                                Element::new(Panel)
                                    .role(Role::Button)
                                    .accessible_label("second")
                                    .focusable()
                                    .on_press(|| {}),
                            )
                            // An empty box at the origin, clipped to nothing,
                            // keeps the rectangle and clip a hidden node has,
                            // so showing it moves nothing.
                            .child(
                                Element::new(Panel)
                                    .absolute()
                                    .size(Size::default())
                                    .clip(true)
                                    .child(Element::new(Text("tucked")).absolute()),
                            ),
                    )
                    .child(
                        Element::new(Panel).role(Role::List).child(Element::new(Panel).child(
                            for_each(
                                move || rows.get(),
                                |row| *row,
                                |row| {
                                    Element::new(Panel)
                                        .role(Role::ListItem)
                                        .child(Element::new(Text(ROW_TEXTS[usize::from(row)])))
                                },
                            ),
                        )),
                    )
            })
        };
        let mut projection = Projection::new("Smudge");
        let mut width = 400.0;
        app.frame();
        let mut tree = Tree::new(projection.update(&mut app, width, 200.0, 2.0, true), true);

        for edit in edits {
            match edit {
                Edit::ToggleShown => shown.update(|shown| *shown = !*shown),
                Edit::ToggleHidden => hidden.update(|hidden| *hidden = !*hidden),
                Edit::ToggleDisabled => disabled.update(|disabled| *disabled = !*disabled),
                Edit::Rename(suffix) => name.set(format!("renamed {suffix}")),
                Edit::Rows(bits) => rows.set((0..6).filter(|row| bits & (1 << row) != 0).collect()),
                Edit::Focus(at) => {
                    let focusable: Vec<_> = app
                        .semantics_nodes()
                        .into_iter()
                        .filter(|node| node.focusable)
                        .collect();
                    if !focusable.is_empty() {
                        app.request_focus(focusable[usize::from(at) % focusable.len()].id);
                    }
                }
                Edit::Resize(next) => width = f64::from(next),
            }
            app.frame();
            let update = projection.update(&mut app, width, 200.0, 2.0, false);
            tree.update_and_process_changes(update, &mut Ignore);

            let fresh = Projection::new("Smudge").update(&mut app, width, 200.0, 2.0, true);
            prop_assert_eq!(reachable(&tree), reachable(&Tree::new(fresh, true)));
        }
    }
}

/// The id of the projected node with `role` in `app`.
fn projected(app: &App, role: Role) -> smudge_core::tree::WidgetId {
    app.semantics_nodes()
        .iter()
        .find(|node| node.role == role)
        .map(|node| node.id)
        .expect("the node projects")
}

#[test]
fn a_switch_reports_its_state_and_a_press_toggles_it() {
    let on = RwSignal::new(false);
    let mut app = app_with(|| {
        let (read, flip) = (on.clone(), on.clone());
        Element::new(Panel).role(Role::Group).child(
            Element::new(Panel)
                .role(Role::Switch)
                .accessible_label("Wi-Fi")
                .accessible_checked(move || Checked::from(read.get()))
                .focusable()
                .on_press(move || flip.update(|on| *on = !*on)),
        )
    });
    let mut projection = Projection::new("Smudge");
    let update = update_after_frame(&mut app, &mut projection, true);
    let switch = node_with_role(&update, AccessKitRole::Switch);
    assert_eq!(switch.toggled(), Some(Toggled::False));
    assert!(
        switch.supports_action(Action::Click),
        "a screen reader can press it"
    );

    assert!(app.press(projected(&app, Role::Switch)));
    let update = update_after_frame(&mut app, &mut projection, false);
    let switch = node_with_role(&update, AccessKitRole::Switch);
    assert_eq!(
        switch.toggled(),
        Some(Toggled::True),
        "the press turned it on"
    );
}

#[test]
fn a_checkbox_reports_a_mixed_state() {
    let mut app = app_with(|| {
        Element::new(Panel).role(Role::Group).child(
            Element::new(Panel)
                .role(Role::CheckBox)
                .accessible_label("Select all")
                .accessible_checked(Checked::Mixed)
                .on_press(|| {}),
        )
    });
    let mut projection = Projection::new("Smudge");
    let update = update_after_frame(&mut app, &mut projection, true);
    let checkbox = node_with_role(&update, AccessKitRole::CheckBox);
    assert_eq!(checkbox.toggled(), Some(Toggled::Mixed));
    assert_eq!(
        checkbox.label().map(str::to_owned),
        Some("Select all".to_owned())
    );
}

#[test]
fn a_progress_bar_reports_its_value_in_its_range_as_it_moves() {
    let done = RwSignal::new(0.25f64);
    let total = RwSignal::new(1.0f64);
    let mut app = app_with(|| {
        let (done, total) = (done.clone(), total.clone());
        Element::new(Panel).role(Role::Group).child(
            Element::new(Panel)
                .role(Role::ProgressBar)
                .accessible_label("Upload")
                .accessible_range(move || ValueRange {
                    value: done.get(),
                    min: 0.0,
                    max: total.get(),
                }),
        )
    });
    let mut projection = Projection::new("Smudge");
    let update = update_after_frame(&mut app, &mut projection, true);
    let bar = node_with_role(&update, AccessKitRole::ProgressIndicator);
    assert_eq!(bar.numeric_value(), Some(0.25));
    assert_eq!(bar.min_numeric_value(), Some(0.0));
    assert_eq!(bar.max_numeric_value(), Some(1.0));

    done.set(0.75);
    let update = update_after_frame(&mut app, &mut projection, false);
    let bar = node_with_role(&update, AccessKitRole::ProgressIndicator);
    assert_eq!(
        bar.numeric_value(),
        Some(0.75),
        "the new progress reaches the tree"
    );

    total.set(3.0);
    let update = update_after_frame(&mut app, &mut projection, false);
    let bar = node_with_role(&update, AccessKitRole::ProgressIndicator);
    assert_eq!(
        bar.max_numeric_value(),
        Some(3.0),
        "the new total reaches the tree"
    );

    done.set(5.0);
    let update = update_after_frame(&mut app, &mut projection, false);
    let bar = node_with_role(&update, AccessKitRole::ProgressIndicator);
    assert_eq!(
        bar.numeric_value(),
        Some(3.0),
        "a value past the range reads as its end"
    );

    done.set(f64::NAN);
    let update = update_after_frame(&mut app, &mut projection, false);
    let bar = node_with_role(&update, AccessKitRole::ProgressIndicator);
    assert_eq!(
        bar.numeric_value(),
        None,
        "a value that is not finite reads as busy"
    );

    done.set(1.0);
    total.set(f64::NAN);
    let update = update_after_frame(&mut app, &mut projection, false);
    let bar = node_with_role(&update, AccessKitRole::ProgressIndicator);
    assert_eq!(
        bar.numeric_value(),
        None,
        "a range that is not finite reads as busy"
    );

    total.set(-1.0);
    let update = update_after_frame(&mut app, &mut projection, false);
    let bar = node_with_role(&update, AccessKitRole::ProgressIndicator);
    assert_eq!(
        bar.numeric_value(),
        None,
        "a range that runs backwards reads as busy"
    );

    total.set(f64::INFINITY);
    let update = update_after_frame(&mut app, &mut projection, false);
    let bar = node_with_role(&update, AccessKitRole::ProgressIndicator);
    assert_eq!(bar.numeric_value(), None, "an endless range reads as busy");
}

#[test]
fn a_checkbox_without_a_press_handler_reads_as_read_only() {
    let mut app = app_with(|| {
        Element::new(Panel)
            .role(Role::Group)
            .child(
                Element::new(Panel)
                    .role(Role::CheckBox)
                    .accessible_checked(true),
            )
            .child(
                Element::new(Panel)
                    .role(Role::Switch)
                    .accessible_checked(true)
                    .on_press(|| {}),
            )
    });
    let mut projection = Projection::new("Smudge");
    let update = update_after_frame(&mut app, &mut projection, true);
    assert!(node_with_role(&update, AccessKitRole::CheckBox).is_read_only());
    assert!(!node_with_role(&update, AccessKitRole::Switch).is_read_only());
}

/// A widget that reports itself as a checked checkbox with a value in a
/// range, the way a widget of an app's own would.
struct OwnCheckBox;

impl Widget for OwnCheckBox {
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}

    fn semantics(&self) -> Option<Semantics> {
        Some(Semantics {
            checked: Some(Checked::True),
            range: Some(ValueRange {
                value: 3.0,
                min: 0.0,
                max: 5.0,
            }),
            ..Semantics::new(Role::CheckBox)
        })
    }
}

#[test]
fn a_widget_reports_its_own_state_and_the_element_overrides_it() {
    let mut app = app_with(|| {
        Element::new(Panel)
            .role(Role::Group)
            .child(Element::new(OwnCheckBox))
            .child(
                Element::new(OwnCheckBox)
                    .role(Role::Switch)
                    .accessible_checked(false)
                    .accessible_range(ValueRange {
                        value: 1.0,
                        min: 0.0,
                        max: 2.0,
                    }),
            )
    });
    let mut projection = Projection::new("Smudge");
    let update = update_after_frame(&mut app, &mut projection, true);
    let own = node_with_role(&update, AccessKitRole::CheckBox);
    assert_eq!(own.toggled(), Some(Toggled::True));
    assert_eq!(own.numeric_value(), Some(3.0));
    assert_eq!(own.max_numeric_value(), Some(5.0));
    let overridden = node_with_role(&update, AccessKitRole::Switch);
    assert_eq!(overridden.toggled(), Some(Toggled::False));
    assert_eq!(overridden.numeric_value(), Some(1.0));
    assert_eq!(overridden.max_numeric_value(), Some(2.0));
}
