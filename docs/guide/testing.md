# Testing headless

<p class="lead">The window is optional. An `App` runs the same interface without one: you advance it frame by frame, feed it input, and check what it did, all inside an ordinary `cargo test`.</p>

## Driving an app

```rust
use smudge::core::app::{App, AppConfig, RepaintMode};
use smudge::core::input::{InputEvent, PointerButton};
use smudge::prelude::*;

#[test]
fn a_press_repaints_the_button_and_the_label_only() {
    TextSystem::bundled_only().install();

    let count = RwSignal::new(0_u32);
    let press = count.clone();
    let shown = count.clone();

    let mut app = App::new(
        AppConfig {
            viewport: Size { width: 280.0, height: 140.0 },
            scale: 1.0,
            clear_color: Color::rgb(16, 17, 21),
            repaint: RepaintMode::Damage,
        },
        move || {
            column()
                .padding(px(24.0))
                .child(button("Count up").on_press(move || press.update(|n| *n += 1)))
                .child(label(move || format!("Clicked {} times", shown.get())))
        },
    );
    app.set_theme(Theme::dark());
    app.frame();

    let at = Point { x: 40.0, y: 36.0 };
    app.handle_event(InputEvent::PointerDown { position: at, button: PointerButton::Primary });
    app.frame();
    // The pressed color repaints the button alone, with no layout.
    assert!(!app.stats().layout_ran);
    assert_eq!(app.stats().nodes_painted, 1);

    app.handle_event(InputEvent::PointerUp { position: at, button: PointerButton::Primary });
    app.frame();
    // The release counts, and the new text lays out and repaints the label.
    assert_eq!(count.get(), 1);
    assert_eq!(app.stats().nodes_painted, 2);
}
```

`TextSystem::bundled_only().install()` makes text use only the embedded fonts, so a test lays text out the same way on every machine.

`app.frame()` runs one whole frame and returns its `FrameOutput`: the display list, the damage rectangles and the scale. `app.stats()` describes the frame that just ran.

## Frame statistics

| Field | Meaning |
|---|---|
| `layout_ran` | Whether the frame laid anything out. |
| `nodes_painted` | How many nodes regenerated their paint. |
| `damage_rects` | How many rectangles the damage was merged into. |
| `damage_pixels` | How many physical pixels the frame repaints. |
| `flush`, `layout`, `paint`, `list`, `damage` | How long each phase took. |

These make performance part of a test. A test can state that a color change skips layout, or that typing a character repaints the input and nothing else, and fail when a change makes the frame do more work than it should.

## Checking pixels

To check the rendered image, add `smudge-cpu` as a dev-dependency and render each frame's output:

```rust
use smudge_cpu::CpuRenderer;

let mut renderer = CpuRenderer::new(280, 140);
let output = app.frame();
renderer.render(&output);

let pixels = renderer.pixels();      // premultiplied RGBA8, row-major
let png = renderer.encode_png();     // for snapshot files
```

The renderer repaints only each frame's damage onto the previous pixels, exactly like the window does, so rendering every frame in order gives the same image a window would show. The surface size is in physical pixels: multiply the viewport by the scale.

## Animation and time

`app.set_tick(|now| ...)` installs a closure that runs at the start of every frame with the frame's time, the same hook a window drives once per display refresh. In a test, call `app.frame()` as many times as the animation needs.
