use super::*;

#[test]
fn a_saved_workspace_preserves_floats_and_closed_tabs() -> Result<(), Box<dyn Error>> {
    let default = default_layout()?;
    let mut expected = default.clone();
    expected.close("readme")?;
    expected.float("outline", Rect::new(120.0, 100.0, 380.0, 280.0))?;
    expected.set_ratio(&[], 0.35)?;
    let mut saved = expected.clone();
    saved.add("retired", &[Branch::First])?;
    let json = layout_json(saved)?;
    assert_eq!(restore_layout(&json, default), expected);
    Ok(())
}

#[test]
fn invalid_or_incompatible_json_restores_the_default() -> Result<(), Box<dyn Error>> {
    let default = default_layout()?;
    let wrong_version = serde_json::to_vec(&SavedWorkspace {
        version: LAYOUT_VERSION + 1,
        layout: DockLayout::default(),
    })?;
    for json in [b"not JSON".as_slice(), wrong_version.as_slice()] {
        assert_eq!(restore_layout(json, default.clone()), default);
    }
    Ok(())
}

#[test]
fn reset_restores_the_workspace_after_floating_a_tab() -> Result<(), DockError> {
    use smudge::core::app::{App, AppConfig, RepaintMode};
    use smudge::core::input::{InputEvent, PointerButton};
    use smudge::core::semantics::Role;

    TextSystem::bundled_only().install();
    let default = default_layout()?;
    let layout = RwSignal::new(default.clone());
    let (mounted, reset) = (layout.clone(), default.clone());
    let mut app = App::new(
        AppConfig {
            viewport: Size {
                width: 1000.0,
                height: 680.0,
            },
            scale: 1.0,
            clear_color: Color::BLACK,
            repaint: RepaintMode::Damage,
        },
        move || workspace(mounted, reset),
    );
    app.frame();
    let center = |app: &App, role, label| {
        let bounds = app
            .semantics_nodes()
            .into_iter()
            .find(|node| {
                node.role == role && node.label.as_deref().or(node.value.as_deref()) == Some(label)
            })
            .unwrap_or_else(|| panic!("the workspace shows {role:?} {label}"))
            .bounds;
        Point {
            x: (bounds.x0 + bounds.x1) / 2.0,
            y: (bounds.y0 + bounds.y1) / 2.0,
        }
    };
    let primary = PointerButton::Primary;
    let tab = center(&app, Role::Tab, "notes.txt");
    let target = center(&app, Role::Label, "Drop tabs here to float them");
    app.handle_event(InputEvent::PointerDown {
        position: tab,
        button: primary,
    });
    app.handle_event(InputEvent::PointerMoved { position: target });
    app.handle_event(InputEvent::PointerUp {
        position: target,
        button: primary,
    });
    app.frame();
    assert!(layout.read(|layout| {
        layout
            .floating()
            .iter()
            .any(|group| group.panel() == "notes")
    }));

    let reset = center(&app, Role::Button, "Reset layout");
    app.handle_event(InputEvent::PointerDown {
        position: reset,
        button: primary,
    });
    app.handle_event(InputEvent::PointerUp {
        position: reset,
        button: primary,
    });
    app.frame();
    assert_eq!(layout.get(), default);
    Ok(())
}
