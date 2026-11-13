# Windows and renderers

<p class="lead">`run` opens a window, builds the interface once, and draws frames only when something changed. Two renderers draw the same display list: a CPU renderer that repaints only the damage, which is the default, and a GPU renderer for real graphics adapters that an app asks for.</p>

## Opening a window

```rust
use smudge::prelude::*;

fn main() -> Result<(), PlatformError> {
    run_with(
        WindowOptions {
            title: String::from("Notes"),
            size: Size { width: 640.0, height: 480.0 },
            clear_color: Color::rgb(16, 17, 21),
            ..WindowOptions::default()
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
| `decorations` | Whether the window system draws the title bar and borders. A window that draws its own header turns this off and starts a move itself. |
| `min_size` | The smallest content area the user can resize the window to, in logical pixels. |
| `app_id` | The identifier desktop environments use to group and pin the window. It applies on Linux and the BSDs, as the Wayland app id and the X11 class, and has no effect elsewhere. |
| `stats` | A `PresentStatsHandle` to read what the latest present did. |
| `on_frame` | A hook called after every presented frame with its statistics. |
| `on_event` | A hook that can inject input before every frame but the first. |
| `frame_limit` | Closes the window after this many presented frames. |

The last three exist for measurement and automated runs. An application leaves them at `None`.

## Window commands and cursors

A window without decorations needs a way to be moved, resized and closed. The functions in `smudge::commands` queue one command, which the window system applies against the press that is being handled. The module is the core's `window` module, also reachable as `smudge::core::window`, under a name that does not clash with `smudge::window`, the windowing layer:

| Function | Effect |
|---|---|
| `start_move()` | Start an interactive move, as if the user grabbed the title bar. |
| `start_resize(edge)` | Start an interactive resize from an edge or corner. |
| `toggle_maximize()` | Maximize the window, or restore it. |
| `close()` | Close the window and end the run. |

`maximized()` reads whether the window is maximized, and a binding that reads it reruns when that changes, so a header can swap its maximize icon or hide its resize handles. On the web every command does nothing and `maximized()` stays `false`, as [the web guide](/guide/web) explains.

Call them from a pointer-down handler, because the window system honors a move or a resize only while the press that started it is being handled:

```rust
use smudge::commands::start_move;

let header = panel()
    .height(px(40.0))
    .on_pointer_down(|_| start_move());
```

`focused()` reads whether the window holds the input focus, and a binding or an effect that reads it reruns when the window comes to the front or goes to the background. The keyboard focus inside the window stays where it is either way. An app with no window to report it, such as a headless test, counts as focused.

Any element can set the pointer cursor over itself with `cursor`. `Cursor` is in the prelude:

```rust
let link = label("Open the docs")
    .cursor(Cursor::Pointer)
    .on_press(open_docs);
```

The cursor of the deepest enabled node under the pointer that sets one wins, and the platform default applies when none does. Setting a cursor does not change where a press lands: a press on a node that only sets a cursor reaches the nearest ancestor with a handler. `Cursor::Resize(edge)` names the direction a handle would resize in, with an edge from `smudge::commands::ResizeEdge`, so an edge handle shows the same look the window system uses for that edge.

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

- `RendererChoice::Cpu` always uses the CPU renderer. It is the default, with or without the `gpu` feature.
- `RendererChoice::Auto` uses the GPU renderer when the window's adapter is a real GPU. It stays on the CPU renderer when the only adapter is a software rasterizer, when there is no Vulkan, Metal or DX12 adapter, when the window cannot present GPU frames, while `show_damage` is on, or in a build without the `gpu` feature.
- `RendererChoice::Gpu` always uses the GPU renderer, and fails the run instead of falling back.

Ask for the GPU when most of a large window changes every frame with many shapes, such as an animated chart or canvas. There the CPU renderer's frame grows with the pixels: 600 changing boxes took 3.8 ms at 1280 by 800 and 13.4 ms at 2560 by 1600, against 1.5 and 2.4 ms on the GPU. Most interfaces change a small part of the window per frame, and the CPU renderer draws those faster: a scroll at 2560 by 1600 took 0.5 ms on the CPU renderer and 3.1 ms on the GPU.

```rust
let options = WindowOptions {
    renderer: RendererChoice::Auto,
    ..WindowOptions::default()
};
```

The GPU costs startup time and memory. On a desktop with a GTX 1060, a log viewer reached its first frame after 10 to 20 ms of CPU and peaked at 28 MB on the CPU renderer. On the GPU renderer it took 720 to 1,010 ms and 192 to 195 MB. Most of that memory is the graphics driver, which loads as soon as a device exists.

The `SMUDGE_RENDERER` environment variable, set to `cpu`, `gpu` or `auto`, overrides the choice when the window opens, so a user can switch renderer without a rebuild:

```sh
SMUDGE_RENDERER=auto ./my-app
```

### Backends

The GPU renderer is built with the Vulkan, Metal and DX12 backends only. It needs compute shaders, which OpenGL serves poorly, and leaving OpenGL out saves 670 KB of a stripped binary. On a machine whose only GPU driver is OpenGL, `Auto` uses the CPU renderer and `Gpu` fails the run.

### Drawing into your own device

The window creates its GPU device itself. A program that renders through `smudge::gpu` on a device of its own, with `GpuRenderer::new` or `GpuSurface::new`, should create it the same way:

- `smudge::gpu::instance()` returns the wgpu instance to request the adapter from. It enables the primary backends only (Vulkan, Metal and DX12), and the `WGPU_BACKEND` environment variable still picks the backends when it is set.
- `smudge::gpu::device_limits(&adapter)` returns the limits to request the device with. They raise the storage binding and buffer size limits to what the adapter offers. A device created with the default limits fails validation on a large scene, which wgpu reports as a panic.

`GpuRenderer::headless` does both for an offscreen renderer.

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
