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

Smudge builds your interface once as a tree of widgets and keeps it. State lives in signals, and a widget property given a closure updates when the signals it reads change. Each frame, the library works out which pixels those updates touched and redraws exactly those, so a click that changes one label repaints that label and the button you pressed.

Measured on a Ryzen 7 7700 with the CPU renderer, headless, median frame:

| A frame where | Damage-tracked | Full repaint |
|---|---|---|
| one small box changes among 600, at 1920 by 1080 | 24 µs | 3.7 ms |
| one key is typed into a focused text field | 37 µs | 87 µs |

Scrolling a thousand mixed rows costs 0.31 ms a step, because scrolled content keeps its paint and only moves.

In a virtual machine without a GPU, where a software GPU renderer takes 6 to 24 ms per frame, the same CPU renderer draws a small update in a window in under 0.1 ms.

## Features

- **Damage-tracked repainting.** Only the rectangles that changed are rasterized and presented, and an animation frame with no change skips the present.
- **Fine-grained signals.** Bindings are per property, so a write updates exactly what reads it. Layout runs only when a size could have changed.
- **Two renderers.** A CPU renderer that repaints only the damage, and a GPU renderer behind the `gpu` feature. The automatic choice uses the GPU on real hardware and never on a software rasterizer.
- **Scrolling without repainting.** Scrolled content keeps its paint and only moves.
- **Text and input.** Shaped text with embedded fonts or the system's, labels capped with an ellipsis, a single-line input and a multi-line text area with selection, undo and redo, input method composition, and the system clipboard.
- **Widgets.** Rows, columns, stacks, buttons, labels, images, a scroll view that can follow its end, and virtual lists for very long lists, including keyed rows of varying height that stay in place while rows above them change. All are styled with theme tokens and interaction states.
- **Application plumbing.** Messages from other threads, timers, autofocus, focus traps and app-wide key handlers, and window commands and cursors for a custom title bar.
- **Accessibility.** The widget tree is exposed to screen readers on the desktop, with keyboard focus and activation.
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

More examples live in [`crates/smudge/examples`](crates/smudge/examples):

```sh
cargo run --release --example counter -p smudge
cargo run --release --example form -p smudge
cargo run --release --example lists -p smudge
```

## Status

Version 0.2 adds the multi-line text area, images, the keyed virtual list, a scroll view that follows its end, messages from other threads, focus traps and autofocus, label line caps, and window commands and cursors for a custom title bar. It is built and tested on Linux, and the window layer supports Windows and macOS. The API will change between minor versions while it settles, and 0.2 changes some 0.1 APIs, which the changelog marks as breaking. The [known limits](docs/reference/limits.md) list what is not there yet, such as rich text spans, horizontal scrolling and popups.

## Documentation

The guide lives in [`docs/`](docs) as Markdown pages, which a static site generator turns into a site. To preview it:

```sh
cd docs
bun install
bun run dev
```

It covers signals, layout, styling, widgets, text and input, renderers, the web build, accessibility, headless testing, and how a frame works inside. The API reference is on [docs.rs](https://docs.rs/smudge).

## Crates

| Crate | Purpose |
|---|---|
| `smudge` | The facade and prelude. Start here. |
| `smudge-core` | Widget tree, signals, layout, paint and damage. No window or GPU dependency. |
| `smudge-text` | Fonts, text layout, the label, the text input and the text area. |
| `smudge-widgets` | Containers, button, image, scroll view and virtual lists. |
| `smudge-cpu` | The CPU renderer. |
| `smudge-gpu` | The GPU renderer. |
| `smudge-platform` | Windows, input, presentation and the web build. |
| `smudge-a11y` | The bridge to the platform accessibility APIs. |

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
