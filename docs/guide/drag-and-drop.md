# Drag and drop

## Drops from the desktop

Use `on_file_hover` to highlight a drop zone and `on_file_drop` to receive the paths. A hover report passes `Some(paths)`, which can be empty on Wayland. The hover handler receives `None` when a report moves the hover away, the platform cancels it, or a drop reaches a handler. The deepest enabled element under the reported position with the corresponding handler receives the event. The drop handler receives paths and a position in logical pixels relative to the element's top left.

```rust
use std::path::PathBuf;

use smudge::core::theme::ColorValue;
use smudge::prelude::*;

let paths = RwSignal::new(Vec::<PathBuf>::new());
let hovering = RwSignal::new(false);
let highlight = hovering.clone();
let dropped = paths.clone();

let zone = panel()
    .size(Size { width: 280.0, height: 96.0 })
    .padding(px(16.0))
    .border(2.0)
    .border_color(move || {
        if highlight.get() {
            ColorValue::from(ColorToken::ACCENT)
        } else {
            ColorValue::from(ColorToken::BORDER)
        }
    })
    .on_file_hover(move |files| hovering.set(files.is_some()))
    .on_file_drop(move |files, _at| dropped.set(files.to_vec()))
    .child(label("Drop files here"));
```

See [Known limits](/reference/limits#platforms) for how the desktop reports positions and for browser support.

## Drags within an app

`drag_source` builds a typed payload when a held primary press moves four logical pixels. `drop_target` accepts payloads of the same type when its predicate returns `true`. The deepest accepting target under the pointer shows its `drop_style`. A release there delivers the payload and a position relative to the target's content box.

```rust
use smudge::prelude::*;

let received = RwSignal::new(String::new());
let shown = received.clone();
let source = panel()
    .padding(px(12.0))
    .background(ColorToken::SURFACE_HOVER)
    .child(label("report.pdf"))
    .drag_source(|| String::from("report.pdf"));
let folder = panel()
    .min_width(px(160.0))
    .padding(px(12.0))
    .border(2.0)
    .border_color(ColorToken::BORDER)
    .drop_style(|style| style.border_color(ColorToken::ACCENT))
    .drop_target(
        |name: &String| name.ends_with(".pdf"),
        move |name, _at| received.set(name),
    )
    .child(label(move || format!("Folder: {}", shown.get())));
let view = row().child(source).child(folder);
```

The default preview is a translucent copy of the source's surface. Use `.drag_preview(|| ...)` to show your own element at the pointer. Pressing Escape or releasing outside an accepting target cancels the drag. `on_drag_end` tells the source whether a target received the payload. When a drag starts inside a scroll view, holding it past an edge scrolls the view in that direction when it has room to scroll.

## Reorderable lists

`reorderable_list(items, key, estimate, row)` takes a `RwSignal<Vec<T>>` and writes the new order back to it. Give each item a unique, stable key and supply the estimated row height in logical pixels. The example below uses IDs as keys so a track keeps its identity when it moves.

```rust
use smudge::prelude::*;
use smudge::widgets::reorderable_list;

fn main() -> Result<(), PlatformError> {
    let tracks = RwSignal::new(vec![(1, "Intro"), (2, "Main theme"), (3, "Outro")]);

    run(WindowOptions::default(), move || {
        column().padding(px(16.0)).child(
            reorderable_list(tracks.clone(), |track| track.0, 40.0, |track| {
                panel()
                    .height(px(40.0))
                    .padding(px(8.0))
                    .child(label(track.1))
            })
            .height(px(160.0)),
        )
    })
}
```

Drag a row over the top or bottom half of another row to move it before or after that row. The accent line marks the insertion point. Focus a row with Tab or a click. Alt+Up and Alt+Down move it one place, and Alt+Home and Alt+End move it to the start and end. The list keeps the moved row focused after a keyboard move, scrolls it into view as needed, and announces each move to a screen reader, for example “Moved to position 3 of 12.”

## Opening a file picker

From a press handler on the window thread, call `smudge::window::handle::with_window_handles` and pass the borrowed handles as the dialog's parent inside the closure. The `pick_file` function in `crates/smudge/examples/files.rs` shows the complete call. The example uses a synchronous picker, so the Smudge window pauses while the picker is open. See [Windows and renderers](/guide/rendering) for when the handles are available.

```sh
cargo run -p smudge --example files
```

Click **Open a file** to open the parented picker, or drop files into the zone. Both paths add to the same list.
