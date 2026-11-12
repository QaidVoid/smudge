# Quick start

<p class="lead">Create a project, add Smudge, and open a window with a counter in it.</p>

## Requirements

- Rust 1.89 or newer.
- On Linux, a Wayland or X11 desktop session. Building needs no system development packages: the default build embeds its fonts and loads nothing from the system at compile time.

## Create the project

```sh
cargo new hello-smudge
cd hello-smudge
cargo add smudge
```

The default features give you a window and the CPU renderer. Add `--features gpu` for the GPU renderer as well, see [Cargo features](/reference/features).

## Write the counter

Replace `src/main.rs` with:

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

Run it:

```sh
cargo run --release
```

## What happens

1. `RwSignal::new(0)` creates the state. Cloning a signal handle is cheap and every clone names the same signal, so one clone moves into the button's handler and one into the label.
2. `run_with` opens the window with these options and calls the build closure once. The `column` and its children are mounted and stay mounted. The last closure sets the app up before the first frame; here it picks the dark theme, so the text contrasts with the dark clear color. `run` is the shorter form with default options and no setup.
3. The label gets a closure instead of a string. Smudge runs it once to get the first text, and records that it read `shown`.
4. A press calls `press.update(...)`, which changes the value and marks the label's closure to run again. Nothing is drawn yet.
5. The window schedules a frame. The closure runs, the label lays out its new text, and only the label's rectangle and the button's pressed state are repainted and presented.

::: tip See the damage
Set `show_damage: true` and every present tints the pixels that frame repainted. Click the button and watch only the label and the button light up. The overlay works on the CPU renderer.
:::

## The `WindowOptions` fields

A window opens with `WindowOptions::default()`, and a sample spells out only the fields it changes, with `..WindowOptions::default()` for the rest. [Windows and renderers](./rendering) describes them all. The ones you will change most:

| Field | What it does |
|---|---|
| `title` | The title bar text. |
| `size` | The content area in logical pixels. |
| `clear_color` | The background behind everything. Must be opaque. |
| `renderer` | `RendererChoice::Cpu` by default, `Auto` in builds with the `gpu` feature: the GPU when it pays, the CPU otherwise. |
| `show_damage` | Tints what each frame repainted. |

## Examples in the repository

The repository's examples are complete programs:

```sh
git clone https://github.com/QaidVoid/smudge
cd smudge
cargo run --release --example counter -p smudge
cargo run --release --example form -p smudge
cargo run --release --example lists -p smudge
```

`form` binds two text inputs to signals and shows a live preview. `lists` filters a 100,000-row virtual list as you type.

Next, [Signals](./signals) explains how state reaches the screen in more detail.
