# Crates

<p class="lead">Smudge is a workspace of small crates with one job each. Most applications depend on `smudge` alone, which re-exports the rest.</p>

| Crate | What it holds | API |
|---|---|---|
| [`smudge`](https://crates.io/crates/smudge) | The facade: re-exports, the prelude, and the examples. | [docs.rs](https://docs.rs/smudge) |
| [`smudge-core`](https://crates.io/crates/smudge-core) | The widget tree, reactive signals, style, layout, paint, damage tracking and the headless `App`. No window, GPU or rasterizer dependency. | [docs.rs](https://docs.rs/smudge-core) |
| [`smudge-text`](https://crates.io/crates/smudge-text) | Fonts, text layout, the label, the text input and the text area. | [docs.rs](https://docs.rs/smudge-text) |
| [`smudge-widgets`](https://crates.io/crates/smudge-widgets) | Containers, the button, the image, the scroll view and the virtual lists. | [docs.rs](https://docs.rs/smudge-widgets) |
| [`smudge-cpu`](https://crates.io/crates/smudge-cpu) | The CPU renderer, which repaints only damaged regions. | [docs.rs](https://docs.rs/smudge-cpu) |
| [`smudge-gpu`](https://crates.io/crates/smudge-gpu) | The GPU renderer. | [docs.rs](https://docs.rs/smudge-gpu) |
| [`smudge-platform`](https://crates.io/crates/smudge-platform) | Windows, input, presentation, renderer choice, and the web build. | [docs.rs](https://docs.rs/smudge-platform) |
| [`smudge-a11y`](https://crates.io/crates/smudge-a11y) | The bridge from the widget tree's semantics to the platform accessibility APIs. | [docs.rs](https://docs.rs/smudge-a11y) |

The repository also holds three crates that are not published: `smudge-bench`, the benchmark tool, `smudge-test`, shared test helpers, and `docs-check`, which compiles the guide's samples.

## How they fit

```text
                            smudge  (facade)
                 ┌────────────────┴─────────────────┐
          smudge-widgets                    smudge-platform
                 │                     ┌──────────┼───────────┐
            smudge-text           smudge-cpu  smudge-gpu  smudge-a11y
                 │                     │     (gpu feature)    │
                 └───────────────► smudge-core ◄──────────────┘
```

The diagram shows the main paths; most crates also depend on `smudge-core` directly. `smudge-core` sits at the bottom and knows nothing about windows or pixels. It produces a display list and a set of damaged rectangles per frame. A renderer turns those into pixels, and the platform crate puts the pixels on screen.

## Through the facade

| Path | Is |
|---|---|
| `smudge::prelude::*` | The types and functions an application names most often. |
| `smudge::core` | `smudge-core` |
| `smudge::text` | `smudge-text` |
| `smudge::widgets` | `smudge-widgets` |
| `smudge::commands` | The window commands, the window state and the cursors, which is `smudge::core::window` under a name apart from `smudge::window` |
| `smudge::window` | `smudge-platform`, with the `window` feature (on by default) |
| `smudge::gpu` | `smudge-gpu`, with the `gpu` feature |

`smudge-cpu` is not re-exported. Depend on it directly to render frames yourself, for example in [headless tests](/guide/testing).

## Versions

The published crates share one version and are released together. Smudge supports Rust 1.89 and newer.
