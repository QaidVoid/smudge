//! Mirrors the Rust sample in docs/guide/docking.md.

use smudge::prelude::*;
use smudge::widgets::dock::{Dock, DockError, DockLayout, DockPanel, Zone, dock};

fn initial_layout() -> Result<DockLayout, DockError> {
    let mut layout = DockLayout::default();
    layout.add("editor", &[])?;
    layout.add("files", &[])?;
    layout.move_panel("files", &[], Zone::Left)?;
    layout.set_ratio(&[], 0.25)?;
    Ok(layout)
}

fn workspace(layout: RwSignal<DockLayout>) -> Element<Dock> {
    dock(layout, |key| match key {
        "files" => DockPanel::new("Files", || label("notes.txt")).closable(false),
        "editor" => DockPanel::new("Editor", || {
            let text = RwSignal::new(String::from("Start writing here."));
            text_area(text).accessible_label("Notes").grow(1.0)
        }),
        _ => DockPanel::new("Unknown panel", || label("No content available")),
    })
    .grow(1.0)
}

#[test]
fn layout_changes_move_the_mounted_editor_to_another_group() -> Result<(), DockError> {
    use smudge::core::app::{App, AppConfig, RepaintMode};
    use smudge::core::semantics::Role;

    TextSystem::bundled_only().install();
    let layout = RwSignal::new(initial_layout()?);
    let mounted = layout.clone();
    let mut app = App::new(
        AppConfig {
            viewport: Size {
                width: 640.0,
                height: 480.0,
            },
            scale: 1.0,
            clear_color: Color::WHITE,
            repaint: RepaintMode::Damage,
        },
        move || column().child(workspace(mounted)),
    );
    app.frame();
    let editor_id = |app: &App| {
        app.semantics_nodes()
            .into_iter()
            .find(|node| node.role == Role::TabPanel && node.label.as_deref() == Some("Editor"))
            .expect("the editor panel is visible")
            .id
    };
    let original = editor_id(&app);
    let start = app
        .semantics_node(original)
        .expect("the editor is projected")
        .bounds;

    layout.update(|layout| {
        layout.move_panel(
            "editor",
            &[smudge::widgets::dock::Branch::First],
            Zone::Center,
        )
    })?;
    app.frame();
    assert_eq!(editor_id(&app), original);
    let moved = app
        .semantics_node(original)
        .expect("the editor remains projected")
        .bounds;
    assert!(moved.x0 < start.x0, "the editor moves into the left group");
    Ok(())
}
