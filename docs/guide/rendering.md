# Windows and renderers

<p class="lead">`run` opens a window, builds the interface once, and draws frames only when something changed. Two renderers draw the same display list: a CPU renderer that repaints only the damage, and a GPU renderer for real graphics adapters.</p>

## Opening a window

```rust
use smudge::prelude::*;

fn main() -> Result<(), PlatformError> {
    run_with(
        WindowOptions {
            title: String::from("Notes"),
            size: Size { width: 640.0, height: 480.0 },
            clear_color: Color::rgb(16, 17, 21),
            repaint: Default::default(),
            show_damage: false,
            renderer: RendererChoice::default(),
            stats: None,
            on_frame: None,
            on_event: None,
            frame_limit: None,
        },
        || label("Hello"),
        |app| app.set_theme(Theme::dark()),
    )
}
```

`run` takes over the calling thread until the window closes, so call it from `main`. The build closure runs once, when the window first appears.

A frame runs after a signal write, a resize, a scale change or input that changed something. When nothing changes, the window sleeps until the next event or timer.

`run_with` does the same and takes a third closure that receives the `App` once before the first frame. Use it to bind the theme, install a per-frame tick for animation, or set the clipboard:

```rust
run_with(options, build, |app| {
    app.bind_theme(|| Theme::dark());
})?;
```

## `WindowOptions`

| Field | Meaning |
|---|---|
| `title` | The title bar text. |
| `size` | The content area in logical pixels. |
| `clear_color` | The color behind everything. Must be opaque; a run with a translucent one fails before a window opens. |
| `repaint` | `RepaintMode::Damage`, the default, repaints only what changed. `RepaintMode::Full` repaints the whole surface every frame, for comparison. |
| `show_damage` | Tints the pixels each frame repainted. A debugging aid on the CPU renderer. |
| `renderer` | Which renderer draws. See below. |
| `stats` | A `PresentStatsHandle` to read what the latest present did. |
| `on_frame` | A hook called after every presented frame with its statistics. |
| `on_event` | A hook that can inject input before every frame but the first. |
| `frame_limit` | Closes the window after this many presented frames. |

The last three exist for measurement and automated runs. An application leaves them at `None`.

## The two renderers

| | CPU renderer | GPU renderer |
|---|---|---|
| Available | Always, on the desktop and the web | With the `gpu` feature, on the desktop |
| Draws | Only the damaged rectangles, onto the previous frame | The whole frame, every frame |
| Strong at | Small updates: typing, hovering, a ticking counter, scrolling | Frames where most of the window changes, large windows at high scale factors |
| Presents | Only the damaged region, when the platform supports it | The whole surface |

Both draw the same display list, and a parity test compares their pixels on the shapes, borders and text the library uses, at scale 1 and 2, so they stay visually the same.

### Choosing one

`RendererChoice` picks the renderer:

- `RendererChoice::Auto` uses the GPU renderer when the window's adapter is a real GPU. It stays on the CPU renderer when the only adapter is a software rasterizer, when there is no adapter, when the window cannot present GPU frames, or while `show_damage` is on. It is the default in builds with the `gpu` feature.
- `RendererChoice::Cpu` always uses the CPU renderer. It is the default without the `gpu` feature.
- `RendererChoice::Gpu` always uses the GPU renderer, and fails the run instead of falling back.

The `SMUDGE_RENDERER` environment variable, set to `cpu`, `gpu` or `auto`, overrides the choice when the window opens, so a user can switch renderer without a rebuild:

```sh
SMUDGE_RENDERER=cpu ./my-app
```

::: tip Why software GPUs are avoided
A software rasterizer pretending to be a GPU redraws the whole window on the CPU every frame. On a virtual machine without GPU passthrough, the GPU renderer on such an adapter was between 2.5 and over 200 times slower than the CPU renderer on every measured scene. `Auto` therefore never picks one.
:::

## Seeing the damage

`show_damage: true` blends a translucent red over the pixels each frame repainted. The overlay lives only in the window's buffer, never in the renderer's pixels, so the next frame starts clean. Turn it on when a change repaints more than you expected.

## Reading present statistics

```rust
let stats = PresentStatsHandle::new();
let reader = stats.clone();
// Pass `stats: Some(stats)` in WindowOptions, then from anywhere on the main thread:
let latest = reader.get();
println!("copied {} bytes, damage only: {}", latest.bytes_copied, latest.with_damage);
```

`PresentStats` reports the buffer age, whether the present carried only the damage, how many bytes it copied, the surface size, and how long the raster and the buffer wait took.
