//! A headless application crate that only needs to build without a window.

use smudge::prelude::*;

/// The application's root element.
pub fn root() -> Element<Panel> {
    column().child(label("Hello, Smudge"))
}

#[cfg(all(test, feature = "serde"))]
#[test]
fn dock_layout_serializes_through_the_facade() -> Result<(), Box<dyn std::error::Error>> {
    use smudge::widgets::dock::DockLayout;

    let mut layout = DockLayout::default();
    layout.add("editor", &[])?;
    let saved = serde_json::to_string(&layout)?;
    let restored: DockLayout = serde_json::from_str(&saved)?;
    assert_eq!(restored, layout);
    Ok(())
}
