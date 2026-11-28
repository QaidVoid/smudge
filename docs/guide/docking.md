# Docking

<p class="lead">Let users arrange a workspace by moving tabs between groups, splitting panes and floating panels inside the window.</p>

Use `dock` when users should be able to change the workspace. For fixed arrangements, compose `split` and `tabs` instead.

## Build a workspace

`DockLayout` is an app-owned value. Its tree describes splits and tab groups, including each group's active panel. It also records floating panels and their requested rectangles in logical pixels. Use unique, stable string keys for the panels. The keys identify content, not the titles users see.

Give `dock` a `RwSignal<DockLayout>` and a factory that can build a `DockPanel` for each key in the layout. Size the dock explicitly or let a parent size it. The dock writes user changes back to the same signal.

```rust
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
```

Create `RwSignal::new(initial_layout()?)` in your app and pass a clone to `workspace`. An empty path (`&[]`) addresses the root. For a nested group, use `Branch::First` and `Branch::Second` to follow splits from the root. Resolve paths against the current tree. Moving or closing a tab can remove an empty group and collapse a split.

To open, close or move panels from app code, edit the value with `layout.update(...)` or replace it with `layout.set(...)`. Use `DockExt::on_close` to allow or veto a user's close request. This hook does not intercept direct layout edits. `DockExt::empty` supplies content when there are no docked panels, even if some are floating.

Moving a panel or switching to another tab does not rebuild its content. The editor in the example keeps its text, selection and undo history while it moves. Closing it disposes that content. Opening the same key again builds a fresh editor.

## Use the pointer

- Drag a tab over another tab in its strip to reorder it. Drop it in another group's center to join that group, or near the middle of an edge below the strip to create a split on that side. The highlighted area shows where it will land.
- Drag a divider to resize the groups on both sides.
- Release a docked tab over unused space inside the window, outside the docking targets, to float it. Drag the floating panel's grip to move it, its edges or corners to resize it, or its tab back to a docked group to dock it again. You can also dock it in an empty workspace. A float does not block interaction with the rest of the window.
- Press Escape during a tab drag or release outside the window to cancel it.

## Use the keyboard

| Action | Keys |
| --- | --- |
| Select another tab in a strip | Left or Right. Home or End selects the first or last tab. |
| Go to another group | F6 or Shift+F6 while focus is in the dock, including in a floating group. Focus returns to the last eligible control in the active panel, then falls back to the first eligible control or the tab. |
| Reorder a docked tab | Alt+Left or Alt+Right on the tab. |
| Move, float, dock or close a panel | Right-click its tab, or focus the tab and press the Menu key or Shift+F10. Then choose an available action. |
| Resize a split | Tab to its divider and use the arrows that match its axis. Home and End move to the extremes. |
| Move or resize a float | Choose **Move floating panel** or **Resize floating panel** from its tab menu. Use the arrow keys. Enter keeps the edit, as does leaving the tab. Escape reverts it. |
| Close a panel | Ctrl+W if the panel permits closing and no other key handler takes the shortcut. |

User moves and closes are announced to screen readers. Layout edits made by app code do not announce a move on their own.

## Save and restore

Enable `smudge`'s `serde` feature to serialize the layout. The app chooses the format, where to save it, and which panel keys should be open on restore. See [Saving a dock layout](/guide/rendering#saving-a-dock-layout) for a versioned JSON recipe that reconciles old saves before mounting.

Try the desktop example:

```sh
cargo run -p smudge --example docking --features serde
```

The workspace has a file list, two editable documents, a mock terminal and an outline. Drag a panel onto the toolbar to float it. Resize a split, close the window normally, then run the same command again to check that the layout returns. The example writes `smudge-docking-layout.json` in the current directory by default. To use another file, pass its path after `--`. The example saves after the native event loop returns successfully. It remembers which tabs you closed, but does not save document text or undo history. Use **Reset layout** to bring closed panels back.
