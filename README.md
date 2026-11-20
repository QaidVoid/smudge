<p align="center">
  <img src="docs/public/logo.svg" alt="" width="128" height="128">
</p>

<h1 align="center">Smudge</h1>

<p align="center">
  A native Rust GUI library that repaints only what changed.<br>
  Retained widgets, fine-grained signals, and a CPU renderer that stays fast without a GPU.
</p>

<p align="center">
  <a href="https://crates.io/crates/smudge"><img src="https://img.shields.io/crates/v/smudge.svg" alt="crates.io"></a>
  <a href="https://docs.rs/smudge"><img src="https://img.shields.io/docsrs/smudge" alt="docs.rs"></a>
  <a href="https://github.com/QaidVoid/smudge/actions/workflows/ci.yml"><img src="https://github.com/QaidVoid/smudge/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
</p>

---

Smudge builds your interface once as a tree of widgets and keeps it. State lives in signals, and a widget property given a closure updates when the signals it reads change. Each frame, the library works out which pixels those updates touched and redraws exactly those, so a click that changes one label repaints that label and the button you pressed, and a frame where nothing changed draws nothing at all.

## Features

- **Damage-tracked repainting.** Only the rectangles that changed are rasterized and presented. Scrolled content keeps its paint and only moves.
- **Fine-grained signals.** Bindings are per property, so a write updates exactly what reads it, and layout runs only when a size could have changed.
- **Two renderers.** A CPU renderer that repaints only the damage, and a GPU renderer behind the `gpu` feature. An app can draw its own wgpu content, such as a 3D view, into the interface.
- **Widgets.** Rows, columns, stacks, buttons, labels, images, scroll views, split panes and virtual lists with rows of any height. Checkboxes, switches, sliders, number fields and tabs. Menus, menu bars, context menus, tooltips and dropdowns, on an overlay layer that escapes every clip.
- **Drawing.** A canvas that strokes and fills paths, with curves, arcs, joins, caps and dashes, in its own coordinates.
- **Text.** Shaped text with variable fonts at any weight, from the bundled fonts, the system's or the app's own. Rich text with links, text that selects and copies, a text input and a text area with undo, input method composition, and the clipboard. Markdown rendered while it streams in.
- **Motion.** Springs that move paint properties, and nodes that fade in and out as they mount and leave, following the system's reduced-motion setting.
- **Input.** Every pointer button, pointer lock with raw motion, arrow-key navigation, focus traps, autofocus and app-wide shortcuts.
- **Accessibility.** The widget tree reaches screen readers on the desktop with roles, names, states, values and announcements, and works from the keyboard.
- **Hot reload and devtools.** `cargo smudge dev` patches a running app on every save. A devtools panel over the app shows what each frame cost, what repainted, why a binding reran, the focus order and the accessibility tree.
- **Desktop and web.** One window API for the desktop, and the CPU renderer in a browser canvas.
- **Testable headless.** An `App` runs without a window, frame by frame, and reports what each frame did.

## Example

```sh
cargo add smudge
```

```rust
use smudge::prelude::*;

fn main() -> Result<(), PlatformError> {
    let count = RwSignal::new(0_u32);
    let press = count.clone();
    let shown = count.clone();

    run_with(
        WindowOptions {
            title: String::from("Counter"),
            size: Size { width: 280.0, height: 140.0 },
            clear_color: Color::rgb(16, 17, 21),
            ..WindowOptions::default()
        },
        move || {
            column()
                .padding(px(24.0))
                .gap(Size { width: 0.0, height: 12.0 })
                .align_items(Align::Center)
                .child(
                    button("Count up")
                        .primary()
                        .on_press(move || press.update(|count| *count += 1)),
                )
                .child(label(move || format!("Clicked {} times", shown.get())))
        },
        |app| app.set_theme(Theme::dark()),
    )
}
```

Set `show_damage: true` to tint what each frame repaints.

To start an app that hot-reloads, on x86-64 Linux:

```sh
cargo install cargo-smudge
cargo smudge new counter
cd counter
cargo smudge dev
```

More examples live in [`crates/smudge/examples`](crates/smudge/examples):

```sh
cargo run --example counter -p smudge   # the example above
cargo run --example form -p smudge      # inputs bound to a live preview
cargo run --example lists -p smudge     # a filtered list of 100,000 rows
cargo run --example editor -p smudge    # menus, a canvas, tabs and controls
cargo run --example pointer -p smudge   # every pointer button and pointer lock
cargo run --example scrolling -p smudge # views that scroll sideways and both ways
cargo run --example tour -p smudge      # split panes, a log and the text widgets
```

Add `--features devtools` and press Ctrl+Shift+D in the window for the devtools panel.

## Status

Smudge is young. Minor versions still change the API, and the changelog marks every breaking change. It is built and tested on Linux under Wayland, and the window layer supports Windows and macOS. The [known limits](docs/reference/limits.md) list what is not there yet, such as horizontal scrolling.

## Documentation

The guide lives in [`docs/`](docs) as Markdown pages, which a static site generator turns into a site. To preview it:

```sh
cd docs
bun install
bun run dev
```

It covers signals, layout, styling, widgets, text and input, rendering, markdown, the web build, accessibility, hot reload, devtools, headless testing, and how a frame works inside. The API reference is on [docs.rs](https://docs.rs/smudge).

## Crates

Most apps depend on `smudge` alone, which re-exports the rest.

| Crate | Purpose |
|---|---|
| `smudge` | The facade and prelude. Start here. |
| `smudge-core` | Widget tree, signals, layout, paint and damage. No window or GPU dependency. |
| `smudge-text` | Fonts, text layout, labels, rich text, the text input and the text area. |
| `smudge-widgets` | Containers, controls, menus, tooltips, dropdowns, tabs, the canvas and virtual lists. |
| `smudge-markdown` | Markdown rendered from a source that grows while it shows. |
| `smudge-cpu` | The CPU renderer. |
| `smudge-gpu` | The GPU renderer, and app-drawn wgpu content. |
| `smudge-platform` | Windows, input, presentation and the web build. |
| `smudge-a11y` | The bridge to the platform accessibility APIs. |
| `smudge-devtools` | The devtools panel drawn over a running app. |
| `cargo-smudge` | `cargo smudge new` and `cargo smudge dev`, which hot-patches a running app. |

[Crates](docs/reference/crates.md) in the guide shows how they fit together.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
