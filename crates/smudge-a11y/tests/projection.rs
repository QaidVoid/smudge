//! Tests of the projection from an app's semantics onto AccessKit updates.

use std::collections::BTreeMap;

use smudge_a11y::Projection;
use smudge_core::app::{App, AppConfig, RepaintMode};
use smudge_core::element::{Element, for_each, show};
use smudge_core::geometry::{Color, Size};
use smudge_core::paint::PaintOp;
use smudge_core::reactive::RwSignal;
use smudge_core::reactive::prelude::*;
use smudge_core::semantics::{
    Announcer, Checked, Politeness, Role, Semantics, TextOffset, TextRun, TextSelection, ValueRange,
};
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

/// Scrolls the view under `at` down by `down` logical pixels and runs the
/// frames that settle it.
fn wheel_at(app: &mut App, at: smudge_core::geometry::Point, down: f32) {
    use smudge_core::input::InputEvent;
    app.handle_event(InputEvent::PointerMoved { position: at });
    app.handle_event(InputEvent::Wheel {
        delta: smudge_core::geometry::Point { x: 0.0, y: down },
    });
    for _ in 0..4 {
        app.frame();
    }
}

/// The place in its set of every node with `role` in `update`, sorted.
fn positions(update: &accesskit::TreeUpdate, role: AccessKitRole) -> Vec<usize> {
    let mut positions: Vec<usize> = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == role)
        .filter_map(|(_, node)| node.position_in_set())
        .collect();
    positions.sort_unstable();
    positions
}

#[test]
fn a_scrolled_virtual_list_reports_each_mounted_row_among_all_its_rows() {
    let mut app = app_with(|| {
        Element::new(Panel).role(Role::Group).column().child(
            smudge_widgets::virtual_list(|| 3000, 20.0, |_| Element::new(Panel).height(px(20.0)))
                .height(px(100.0)),
        )
    });
    for _ in 0..4 {
        app.frame();
    }
    wheel_at(
        &mut app,
        smudge_core::geometry::Point { x: 50.0, y: 50.0 },
        800.0,
    );
    let mut projection = Projection::new("Smudge");
    let update = update_after_frame(&mut app, &mut projection, true);
    let list = node_with_role(&update, AccessKitRole::List);
    assert_eq!(list.size_of_set(), Some(3000), "the list counts every row");
    // The view shows rows 40 to 44 from zero, and four more mount on each
    // side of them.
    let rows = positions(&update, AccessKitRole::ListItem);
    assert_eq!(
        rows,
        (36..49).collect::<Vec<_>>(),
        "only the mounted rows, by their places"
    );
}

#[test]
fn a_row_inserted_above_moves_the_places_of_the_keyed_rows_below() {
    let items: RwSignal<std::rc::Rc<[u32]>> = RwSignal::new((0..100).collect());
    let mut app = app_with(|| {
        let items = items.clone();
        Element::new(Panel).role(Role::Group).column().child(
            smudge_widgets::virtual_list_keyed(
                move || items.get(),
                |item| *item,
                20.0,
                |_| Element::new(Panel).height(px(20.0)),
            )
            .height(px(100.0)),
        )
    });
    for _ in 0..4 {
        app.frame();
    }
    let mut projection = Projection::new("Smudge");
    let update = update_after_frame(&mut app, &mut projection, true);
    assert_eq!(
        node_with_role(&update, AccessKitRole::List).size_of_set(),
        Some(100)
    );
    let before = positions(&update, AccessKitRole::ListItem);
    assert_eq!(before.first(), Some(&0));

    items.set(std::iter::once(1000).chain(0..100).collect());
    for _ in 0..3 {
        app.frame();
    }
    let update = update_after_frame(&mut app, &mut projection, false);
    assert_eq!(
        node_with_role(&update, AccessKitRole::List).size_of_set(),
        Some(101)
    );
    let after = positions(&update, AccessKitRole::ListItem);
    assert!(
        after.windows(2).all(|pair| pair[1] == pair[0] + 1),
        "the rows still hold consecutive places: {after:?}"
    );
    assert_eq!(after.first(), Some(&0), "the new row is the first");
}

#[test]
fn a_keyed_virtual_column_reports_its_rows_content_and_no_list() {
    let items: RwSignal<std::rc::Rc<[u32]>> = RwSignal::new((0..100).collect());
    let mut app = app_with(|| {
        let items = items.clone();
        Element::new(Panel).role(Role::Group).column().child(
            smudge_widgets::virtual_column_keyed(
                move || items.get(),
                |item| *item,
                20.0,
                |_| Element::new(Panel).height(px(20.0)).role(Role::Button),
            )
            .height(px(100.0)),
        )
    });
    for _ in 0..4 {
        app.frame();
    }
    let mut projection = Projection::new("Smudge");
    let update = update_after_frame(&mut app, &mut projection, true);
    assert!(
        update.nodes.iter().all(|(_, node)| {
            node.role() != AccessKitRole::List && node.role() != AccessKitRole::ListItem
        }),
        "no list and no list items"
    );
    assert!(
        update
            .nodes
            .iter()
            .any(|(_, node)| node.role() == AccessKitRole::Button),
        "the rows' own content is reported"
    );
}

#[test]
fn a_set_that_grows_updates_its_size_and_leaves_its_items_alone() {
    let count = RwSignal::new(10usize);
    let mut app = app_with(|| {
        let count = count.clone();
        Element::new(Panel)
            .role(Role::List)
            .accessible_set_size(move || count.get())
            .child(
                Element::new(Panel)
                    .role(Role::ListItem)
                    .accessible_position(3),
            )
    });
    let mut projection = Projection::new("Smudge");
    let update = update_after_frame(&mut app, &mut projection, true);
    assert_eq!(
        node_with_role(&update, AccessKitRole::List).size_of_set(),
        Some(10)
    );
    assert_eq!(
        node_with_role(&update, AccessKitRole::ListItem).position_in_set(),
        Some(3)
    );

    count.set(11);
    let update = update_after_frame(&mut app, &mut projection, false);
    assert_eq!(
        node_with_role(&update, AccessKitRole::List).size_of_set(),
        Some(11)
    );
    assert!(
        update
            .nodes
            .iter()
            .all(|(_, node)| node.role() != AccessKitRole::ListItem),
        "the item's place did not change, so it is not sent again"
    );
}

#[test]
fn an_edit_above_a_scrolled_keyed_list_moves_the_places_of_its_rows() {
    let items: RwSignal<std::rc::Rc<[u32]>> = RwSignal::new((0..100).collect());
    let mut app = app_with(|| {
        let items = items.clone();
        Element::new(Panel).role(Role::Group).column().child(
            smudge_widgets::virtual_list_keyed(
                move || items.get(),
                |item| *item,
                20.0,
                |_| Element::new(Panel).height(px(20.0)),
            )
            .height(px(100.0)),
        )
    });
    for _ in 0..4 {
        app.frame();
    }
    wheel_at(
        &mut app,
        smudge_core::geometry::Point { x: 50.0, y: 50.0 },
        800.0,
    );
    let mut projection = Projection::new("Smudge");
    let update = update_after_frame(&mut app, &mut projection, true);
    let before = positions(&update, AccessKitRole::ListItem);
    assert!(before.first().is_some_and(|first| *first > 0), "{before:?}");

    // The same keys stay mounted, so only the places move.
    items.set(std::iter::once(1000).chain(0..100).collect());
    for _ in 0..3 {
        app.frame();
    }
    let update = update_after_frame(&mut app, &mut projection, false);
    let shifted: Vec<usize> = before.iter().map(|place| place + 1).collect();
    assert_eq!(
        positions(&update, AccessKitRole::ListItem),
        shifted,
        "an insert above moves every row down one"
    );

    items.set((1..100).collect());
    for _ in 0..3 {
        app.frame();
    }
    let update = update_after_frame(&mut app, &mut projection, false);
    let lowered: Vec<usize> = before.iter().map(|place| place - 1).collect();
    assert_eq!(
        positions(&update, AccessKitRole::ListItem),
        lowered,
        "a removal above moves every row up one"
    );
}

#[test]
fn a_heading_reports_its_level_and_is_named_by_its_text() {
    let mut app = app_with(|| {
        Element::new(Panel).role(Role::Group).child(
            Element::new(Text("Getting started"))
                .role(Role::Heading)
                .accessible_level(2),
        )
    });
    let mut projection = Projection::new("Smudge");
    let update = update_after_frame(&mut app, &mut projection, true);
    let heading = node_with_role(&update, AccessKitRole::Heading);
    assert_eq!(
        heading.level(),
        Some(1),
        "AccessKit counts the levels from zero"
    );
    assert_eq!(
        heading.label().map(str::to_owned),
        Some("Getting started".to_owned())
    );
    assert_eq!(heading.value(), None, "the text is the name alone");
}

#[test]
fn a_link_is_named_by_its_text_unless_it_has_a_name_of_its_own() {
    let mut app = app_with(|| {
        Element::new(Panel)
            .role(Role::Group)
            .child(
                Element::new(Text("smudge.rs"))
                    .role(Role::Link)
                    .on_press(|| {}),
            )
            .child(
                Element::new(Text("docs"))
                    .role(Role::Link)
                    .accessible_label("Read the docs")
                    .on_press(|| {}),
            )
    });
    let mut projection = Projection::new("Smudge");
    let update = update_after_frame(&mut app, &mut projection, true);
    let mut links: Vec<(Option<String>, Option<String>)> = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == AccessKitRole::Link)
        .map(|(_, node)| {
            (
                node.label().map(str::to_owned),
                node.value().map(str::to_owned),
            )
        })
        .collect();
    links.sort();
    assert_eq!(
        links,
        vec![
            (Some("Read the docs".to_owned()), None),
            (Some("smudge.rs".to_owned()), None),
        ],
        "each link is named once, and no text stands for its address"
    );
}

#[test]
fn a_split_divider_reports_where_it_sits() {
    let ratio = RwSignal::new(0.5f32);
    let mut app = app_with(|| {
        use smudge_widgets::SplitExt;
        Element::new(Panel).role(Role::Group).column().child(
            smudge_widgets::split(
                smudge_core::style::Direction::Row,
                Element::new(Panel),
                Element::new(Panel),
            )
            .ratio(ratio.clone())
            .height(px(50.0)),
        )
    });
    for _ in 0..3 {
        app.frame();
    }
    let mut projection = Projection::new("Smudge");
    let update = update_after_frame(&mut app, &mut projection, true);
    assert_eq!(
        node_with_role(&update, AccessKitRole::Splitter).numeric_value(),
        Some(50.0)
    );
    ratio.set(0.3);
    let update = update_after_frame(&mut app, &mut projection, false);
    assert_eq!(
        node_with_role(&update, AccessKitRole::Splitter).numeric_value(),
        Some(30.0),
        "the divider's value follows it, whole"
    );
}

/// The live status nodes an update carries, by message, with their ids and
/// politeness, in the order the update carries them.
fn announcements(update: &accesskit::TreeUpdate) -> Vec<(NodeId, String, accesskit::Live)> {
    update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == AccessKitRole::Status)
        .map(|(id, node)| {
            (
                *id,
                node.label().unwrap_or_default().to_owned(),
                node.live().unwrap_or(accesskit::Live::Off),
            )
        })
        .collect()
}

/// The children of the window root an update carries, when it carries it.
fn window_children(update: &accesskit::TreeUpdate) -> Option<Vec<NodeId>> {
    update
        .nodes
        .iter()
        .find(|(_, node)| node.role() == AccessKitRole::Window)
        .map(|(_, node)| node.children().to_vec())
}

#[test]
fn announcements_of_one_frame_are_spoken_in_order_and_stay_until_the_next() {
    let announcer = Announcer::new();
    let shown = RwSignal::new(true);
    let attached = announcer.clone();
    let read = shown.clone();
    let mut app = app_with(move || {
        Element::new(Panel)
            .announcer(&attached)
            .child(Element::new(Panel).role(Role::Group))
            .child(
                Element::new(Panel)
                    .role(Role::Group)
                    .hidden(move || !read.get()),
            )
    });
    let mut projection = Projection::new("Smudge");
    update_after_frame(&mut app, &mut projection, true);

    announcer.announce("Block one.", Politeness::Polite);
    announcer.announce("Disk full.", Politeness::Assertive);
    announcer.announce("Block two.", Politeness::Polite);
    let update = update_after_frame(&mut app, &mut projection, false);
    let mut spoken = announcements(&update);
    spoken.sort_by_key(|(_, message, _)| message.clone());
    let messages: Vec<(&str, accesskit::Live)> = spoken
        .iter()
        .map(|(_, message, live)| (message.as_str(), *live))
        .collect();
    assert_eq!(
        messages,
        vec![
            ("Block one.\nBlock two.", accesskit::Live::Polite),
            ("Disk full.", accesskit::Live::Assertive),
        ],
        "one node per politeness, its messages in the order they came"
    );
    let ids: Vec<NodeId> = spoken.iter().map(|(id, _, _)| *id).collect();
    let children = window_children(&update).expect("the window root holds the announcements");
    assert!(ids.iter().all(|id| children.contains(id)));

    shown.set(false);
    let update = update_after_frame(&mut app, &mut projection, false);
    let children = window_children(&update).expect("the roots changed");
    assert!(
        ids.iter().all(|id| children.contains(id)),
        "the announcements stay while nothing new is announced"
    );

    announcer.announce("Later.", Politeness::Polite);
    let update = update_after_frame(&mut app, &mut projection, false);
    let children = window_children(&update).expect("the new announcement arrives");
    assert!(
        ids.iter().all(|id| !children.contains(id)),
        "the next announcements replace them"
    );
    assert_eq!(announcements(&update).len(), 1);
}

#[test]
fn the_same_message_announced_again_is_spoken_again() {
    let announcer = Announcer::new();
    let attached = announcer.clone();
    let mut app = app_with(move || Element::new(Panel).role(Role::Group).announcer(&attached));
    let mut projection = Projection::new("Smudge");
    update_after_frame(&mut app, &mut projection, true);

    announcer.announce("Saved", Politeness::Polite);
    let first = announcements(&update_after_frame(&mut app, &mut projection, false));
    announcer.announce("Saved", Politeness::Polite);
    let second = announcements(&update_after_frame(&mut app, &mut projection, false));
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].1, "Saved");
    assert_ne!(
        first[0].0, second[0].0,
        "a node that arrives anew, which a reader speaks again"
    );
}

#[test]
fn an_announcer_on_a_hidden_node_speaks_nothing() {
    let announcer = Announcer::new();
    let hidden = RwSignal::new(false);
    let attached = announcer.clone();
    let read = hidden.clone();
    let mut app = app_with(move || {
        Element::new(Panel).role(Role::Group).child(
            Element::new(Panel)
                .hidden(move || read.get())
                .announcer(&attached),
        )
    });
    let mut projection = Projection::new("Smudge");
    update_after_frame(&mut app, &mut projection, true);

    hidden.set(true);
    update_after_frame(&mut app, &mut projection, false);
    announcer.announce("Hidden", Politeness::Polite);
    let update = update_after_frame(&mut app, &mut projection, false);
    assert!(announcements(&update).is_empty());

    hidden.set(false);
    update_after_frame(&mut app, &mut projection, false);
    announcer.announce("Shown", Politeness::Polite);
    let update = update_after_frame(&mut app, &mut projection, false);
    let spoken: Vec<String> = announcements(&update)
        .into_iter()
        .map(|(_, message, _)| message)
        .collect();
    assert_eq!(spoken, vec!["Shown".to_owned()]);
}

#[test]
fn a_full_update_speaks_what_its_frame_announced() {
    let announcer = Announcer::new();
    let attached = announcer.clone();
    let mut app = app_with(move || Element::new(Panel).role(Role::Group).announcer(&attached));
    let mut projection = Projection::new("Smudge");
    update_after_frame(&mut app, &mut projection, true);
    announcer.announce("Old", Politeness::Polite);
    let old = announcements(&update_after_frame(&mut app, &mut projection, false));

    announcer.announce("Saved", Politeness::Polite);
    let update = update_after_frame(&mut app, &mut projection, true);
    let spoken = announcements(&update);
    assert_eq!(spoken.len(), 1);
    assert_eq!(spoken[0].1, "Saved");
    let children = window_children(&update).expect("a full update carries the window root");
    assert!(children.contains(&spoken[0].0));
    assert!(
        !children.contains(&old[0].0),
        "a full update starts without the old announcements"
    );

    let update = update_after_frame(&mut app, &mut projection, true);
    let children = window_children(&update).expect("a full update carries the window root");
    assert!(
        !children.contains(&spoken[0].0),
        "a full update with nothing announced names no node it did not send"
    );
}

/// A widget that reports the text runs its cell holds, as a label would.
struct Runs(std::rc::Rc<std::cell::RefCell<Vec<TextRun>>>);

impl Widget for Runs {
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}

    fn semantics(&self) -> Option<Semantics> {
        Some(Semantics {
            text_runs: self.0.borrow().clone(),
            ..Semantics::new(Role::Label)
        })
    }
}

/// A run of one character per byte of `text`, each 8 wide, at `y` in the
/// content box, with a word at every character after a space.
fn run(text: &str, y: f32) -> TextRun {
    let count = text.len();
    let word_starts = (0..count)
        .filter(|at| *at == 0 || text.as_bytes()[at - 1] == b' ')
        .map(|at| at as u8)
        .collect();
    TextRun {
        text: text.to_owned(),
        character_lengths: vec![1; count],
        word_starts,
        character_positions: (0..count).map(|at| at as f32 * 8.0).collect(),
        character_widths: vec![8.0; count],
        bounds: smudge_core::geometry::Rect::new(0.0, y, count as f32 * 8.0, y + 10.0),
        start: 0,
        right_to_left: false,
        continues_line: false,
    }
}

/// An app holding a label of `runs`, padded by 5, which a change of `name`
/// projects again.
fn app_with_runs(
    runs: std::rc::Rc<std::cell::RefCell<Vec<TextRun>>>,
    name: RwSignal<String>,
) -> App {
    app_with(move || {
        Element::new(Panel).role(Role::Group).child(
            Element::new(Runs(runs))
                .padding(px(5.0))
                .width(px(100.0))
                .height(px(40.0))
                .accessible_label(move || name.get()),
        )
    })
}

/// The ids of the text runs the label of an update holds, in order.
fn run_ids(update: &accesskit::TreeUpdate) -> Vec<NodeId> {
    let label = node_with_role(update, AccessKitRole::Label);
    let runs: Vec<NodeId> = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == AccessKitRole::TextRun)
        .map(|(id, _)| *id)
        .collect();
    label
        .children()
        .iter()
        .copied()
        .filter(|child| runs.contains(child))
        .collect()
}

#[test]
fn text_runs_reach_a_reader_as_text_it_can_read_and_measure() {
    let runs = std::rc::Rc::new(std::cell::RefCell::new(vec![
        run("ab\n", 0.0),
        run("cd", 10.0),
    ]));
    let mut app = app_with_runs(runs, RwSignal::new(String::from("Text")));
    let mut projection = Projection::new("Smudge");
    let update = update_after_frame(&mut app, &mut projection, true);

    let ids = run_ids(&update);
    assert_eq!(ids.len(), 2, "the label holds its runs, in order");
    let first = update
        .nodes
        .iter()
        .find(|(id, _)| *id == ids[0])
        .map(|(_, node)| node)
        .expect("the update carries the first run");
    let bounds = first.bounds().expect("a run has bounds");
    assert_eq!(
        (bounds.x0, bounds.y0, bounds.x1, bounds.y1),
        (10.0, 10.0, 58.0, 30.0),
        "the run sits in the padded content box, at the app scale"
    );
    assert_eq!(first.character_positions(), Some(&[0.0, 16.0, 32.0][..]));
    assert_eq!(first.character_widths(), Some(&[16.0, 16.0, 16.0][..]));

    let tree = Tree::new(update, true);
    let group = tree
        .state()
        .root()
        .children()
        .next()
        .expect("the window holds the group");
    let label = group.children().next().expect("the group holds the label");
    assert!(label.supports_text_ranges());
    let range = label.document_range();
    assert_eq!(range.text(), "ab\ncd");
    assert!(
        !range.bounding_boxes().is_empty(),
        "a reader can measure the text"
    );
}

#[test]
fn a_run_keeps_its_id_while_its_node_lives() {
    let runs = std::rc::Rc::new(std::cell::RefCell::new(vec![
        run("ab\n", 0.0),
        run("cd", 10.0),
    ]));
    let name = RwSignal::new(String::from("Text"));
    let mut app = app_with_runs(runs.clone(), name.clone());
    let mut projection = Projection::new("Smudge");
    let before = run_ids(&update_after_frame(&mut app, &mut projection, true));

    *runs.borrow_mut() = vec![run("xy\n", 0.0), run("zw", 10.0), run("v", 20.0)];
    name.set(String::from("Text again"));
    let after = run_ids(&update_after_frame(&mut app, &mut projection, false));
    assert_eq!(after.len(), 3);
    assert_eq!(
        &after[..2],
        &before[..],
        "the runs that stay keep their ids"
    );

    *runs.borrow_mut() = vec![run("only", 0.0)];
    name.set(String::from("Text once more"));
    let fewer = run_ids(&update_after_frame(&mut app, &mut projection, false));
    assert_eq!(fewer, vec![before[0]]);
}

#[test]
fn the_stretches_of_one_line_read_as_one_line() {
    let mut second = run("cde", 0.0);
    second.start = 3;
    second.continues_line = true;
    second.bounds = smudge_core::geometry::Rect::new(24.0, 0.0, 48.0, 10.0);
    let runs = std::rc::Rc::new(std::cell::RefCell::new(vec![run("ab ", 0.0), second]));
    let mut app = app_with_runs(runs, RwSignal::new(String::from("Text")));
    let mut projection = Projection::new("Smudge");
    let update = update_after_frame(&mut app, &mut projection, true);
    let tree = Tree::new(update, true);
    let label = tree
        .state()
        .root()
        .children()
        .next()
        .and_then(|group| group.children().next())
        .expect("the group holds the label");
    let start = label.document_range().start();
    let mut line = start.to_degenerate_range();
    line.set_end(start.forward_to_line_end());
    assert_eq!(line.text(), "ab cde");

    let end = label.document_range().end();
    let mut back = end.to_degenerate_range();
    back.set_start(end.backward_to_line_start());
    assert_eq!(
        back.text(),
        "ab cde",
        "the line reads back to its start too"
    );
}

#[test]
fn a_range_of_right_to_left_text_is_boxed_where_it_paints() {
    let mut rtl = run("abc", 0.0);
    rtl.right_to_left = true;
    rtl.bounds = smudge_core::geometry::Rect::new(10.0, 0.0, 34.0, 10.0);
    let runs = std::rc::Rc::new(std::cell::RefCell::new(vec![rtl]));
    let mut app = app_with_runs(runs, RwSignal::new(String::from("Text")));
    let mut projection = Projection::new("Smudge");
    let update = update_after_frame(&mut app, &mut projection, true);
    let tree = Tree::new(update, true);
    let label = tree
        .state()
        .root()
        .children()
        .next()
        .and_then(|group| group.children().next())
        .expect("the group holds the label");
    let start = label.document_range().start();
    let mut first_two = start.to_degenerate_range();
    first_two.set_end(start.forward_to_character_end().forward_to_character_end());
    let boxes = first_two.bounding_boxes();
    assert_eq!(boxes.len(), 1);
    let (x0, x1) = (boxes[0].x0, boxes[0].x1);
    assert_eq!(
        (x0, x1),
        (46.0, 78.0),
        "the first two letters fill the right two thirds of the run, at the app scale"
    );
}

#[test]
fn a_scroll_sends_the_new_bounds_of_what_it_moves() {
    use smudge_widgets::ScrollExt;
    let offset = RwSignal::new(0.0f32);
    let scrolled = offset.clone();
    let mut app = app_with(move || {
        let rows = (0..10).fold(Element::new(Panel).column(), |rows, _| {
            rows.child(
                Element::new(Panel)
                    .role(Role::Button)
                    .height(px(20.0))
                    .shrink(0.0),
            )
        });
        Element::new(Panel).role(Role::Group).child(
            smudge_widgets::scroll(rows)
                .height(px(60.0))
                .offset(scrolled),
        )
    });
    for _ in 0..3 {
        app.frame();
    }
    let mut projection = Projection::new("Smudge");
    let before = update_after_frame(&mut app, &mut projection, true);
    let tops = |update: &accesskit::TreeUpdate| -> BTreeMap<NodeId, f64> {
        update
            .nodes
            .iter()
            .filter(|(_, node)| node.role() == AccessKitRole::Button)
            .filter_map(|(id, node)| node.bounds().map(|bounds| (*id, bounds.y0)))
            .collect()
    };
    let before = tops(&before);

    offset.set(30.0);
    let update = update_after_frame(&mut app, &mut projection, false);
    let after = tops(&update);
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>(),
        "the scroll sends every row it moved"
    );
    for (id, top) in &after {
        assert_eq!(
            before.get(id).map(|before| before - top),
            Some(60.0),
            "each row moves up by the scroll, at the app scale"
        );
    }
}

/// The place at `byte`, at the start of the line when a line wraps there.
fn at(byte: usize) -> TextOffset {
    TextOffset {
        byte,
        ends_line: false,
    }
}

/// A text field that reports the runs and selection it was built with.
struct Selected(Vec<TextRun>, TextSelection);

impl Widget for Selected {
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}

    fn semantics(&self) -> Option<Semantics> {
        Some(Semantics {
            text_runs: self.0.clone(),
            text_selection: Some(self.1),
            ..Semantics::new(Role::TextInput)
        })
    }
}

/// The selection the projection sends for a field of `runs` with the
/// selection from `anchor` to `focus`, as (run, character) pairs, with the
/// text a reader reads from it.
fn selection_of(
    runs: Vec<TextRun>,
    anchor: TextOffset,
    focus: TextOffset,
) -> ((usize, usize), (usize, usize), String) {
    let mut app = app_with(move || {
        Element::new(Panel)
            .role(Role::Group)
            .child(Element::new(Selected(runs, TextSelection { anchor, focus })).height(px(40.0)))
    });
    let mut projection = Projection::new("Smudge");
    let update = update_after_frame(&mut app, &mut projection, true);
    let field = node_with_role(&update, AccessKitRole::TextInput);
    let selection = *field
        .text_selection()
        .expect("the field reports its selection");
    let order: Vec<NodeId> = field.children().to_vec();
    let place = |position: accesskit::TextPosition| {
        (
            order
                .iter()
                .position(|id| *id == position.node)
                .expect("the position names a run of the field"),
            position.character_index,
        )
    };
    let (anchor, focus) = (place(selection.anchor), place(selection.focus));
    let tree = Tree::new(update, true);
    let field = tree
        .state()
        .root()
        .children()
        .next()
        .and_then(|group| group.children().next())
        .expect("the group holds the field");
    let text = field
        .text_selection()
        .map(|range| range.text())
        .unwrap_or_default();
    (anchor, focus, text)
}

/// The runs of the two lines "ab" and "cd", the first ending in its line
/// feed.
fn two_lines() -> Vec<TextRun> {
    let mut second = run("cd", 10.0);
    second.start = 3;
    vec![run("ab\n", 0.0), second]
}

#[test]
fn a_selection_reaches_a_reader_across_lines() {
    let (anchor, focus, text) = selection_of(two_lines(), at(1), at(4));
    assert_eq!((anchor, focus), ((0, 1), (1, 1)));
    assert_eq!(text, "b\nc");
}

#[test]
fn a_caret_at_the_end_of_a_line_sits_on_its_line_feed() {
    let (_, focus, _) = selection_of(two_lines(), at(2), at(2));
    assert_eq!(focus, (0, 2));
}

#[test]
fn a_caret_on_an_empty_last_line_sits_in_its_run() {
    let mut empty = run("", 10.0);
    empty.start = 3;
    let (_, focus, _) = selection_of(vec![run("ab\n", 0.0), empty], at(3), at(3));
    assert_eq!(focus, (1, 0));
}

#[test]
fn a_caret_after_the_last_character_sits_past_it() {
    let (_, focus, _) = selection_of(vec![run("ab", 0.0)], at(2), at(2));
    assert_eq!(focus, (0, 2));
}

/// A text field whose selection the cell holds.
struct Moving(std::rc::Rc<std::cell::Cell<TextSelection>>);

impl Widget for Moving {
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}

    fn semantics(&self) -> Option<Semantics> {
        Some(Semantics {
            text_runs: two_lines(),
            text_selection: Some(self.0.get()),
            ..Semantics::new(Role::TextInput)
        })
    }
}

#[test]
fn a_move_of_the_caret_alone_is_sent_again() {
    let caret = std::rc::Rc::new(std::cell::Cell::new(TextSelection {
        anchor: at(0),
        focus: at(0),
    }));
    let tint = RwSignal::new(Color::WHITE);
    let (shown, painted) = (caret.clone(), tint.clone());
    let mut app = app_with(move || {
        Element::new(Panel).role(Role::Group).child(
            Element::new(Moving(shown))
                .height(px(40.0))
                .background(move || ColorValue::from(painted.get())),
        )
    });
    let mut projection = Projection::new("Smudge");
    update_after_frame(&mut app, &mut projection, true);

    caret.set(TextSelection {
        anchor: at(4),
        focus: at(4),
    });
    tint.set(Color::BLACK);
    let update = update_after_frame(&mut app, &mut projection, false);
    let field = node_with_role(&update, AccessKitRole::TextInput);
    let focus = field
        .text_selection()
        .expect("the field reports its caret")
        .focus;
    assert_eq!(focus.character_index, 1);
    assert!(
        update
            .nodes
            .iter()
            .all(|(_, node)| node.role() != AccessKitRole::TextRun),
        "the runs did not change, so they are not sent again"
    );
}

#[test]
fn a_caret_at_the_end_of_a_wrapped_line_stays_on_that_line() {
    let mut second = run("cd", 10.0);
    second.start = 2;
    let wrapped = vec![run("ab", 0.0), second];
    let end = TextOffset {
        byte: 2,
        ends_line: true,
    };
    let (_, focus, _) = selection_of(wrapped.clone(), end, end);
    assert_eq!(focus, (0, 2), "past the last character of the first line");
    let (_, focus, _) = selection_of(wrapped, at(2), at(2));
    assert_eq!(focus, (1, 0), "the same byte at the start of the next line");
}

/// A focusable field that counts how often its description is built.
struct Counted(std::rc::Rc<std::cell::Cell<usize>>);

impl Widget for Counted {
    fn paint(&self, _cx: &PaintCx<'_>, _out: &mut Vec<PaintOp>) {}

    fn semantics(&self) -> Option<Semantics> {
        self.0.set(self.0.get() + 1);
        Some(Semantics::new(Role::TextInput))
    }

    fn role(&self) -> Option<Role> {
        Some(Role::TextInput)
    }
}

#[test]
fn an_update_builds_the_focused_fields_description_once() {
    let built = std::rc::Rc::new(std::cell::Cell::new(0));
    let tint = RwSignal::new(Color::WHITE);
    let (counted, painted) = (built.clone(), tint.clone());
    let mut app = app_with(move || {
        Element::new(Panel).role(Role::Group).child(
            Element::new(Counted(counted))
                .height(px(20.0))
                .focusable()
                .background(move || ColorValue::from(painted.get())),
        )
    });
    let mut projection = Projection::new("Smudge");
    let full = update_after_frame(&mut app, &mut projection, true);
    let field = id_with_role(&full, AccessKitRole::TextInput);
    assert!(app.request_focus(smudge_core::tree::WidgetId::from_u64(field.0)));
    update_after_frame(&mut app, &mut projection, false);

    built.set(0);
    tint.set(Color::BLACK);
    let update = update_after_frame(&mut app, &mut projection, false);
    assert_eq!(update.focus, field);
    assert_eq!(
        built.get(),
        1,
        "the focus asks whether the field projects without building it"
    );
}
