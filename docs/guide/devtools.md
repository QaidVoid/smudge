# Devtools

<p class="lead">With the `devtools` feature, a key opens a panel over your running window. It shows what each frame cost and what it repainted, lets you pick any node to see where it came from and how it was laid out, explains why a binding ran, checks focus and accessibility, follows hot reload, and lets you tune the theme from a file. The app keeps working underneath, and its own frames, numbers and pixels stay its own.</p>

## Turning it on

Add the feature to the `smudge` dependency, for development builds only:

```toml
[dependencies]
smudge = { version = "0.3", features = ["devtools"] }
```

The `dev` feature turns on `devtools` and `hot-reload` together, which is what `cargo smudge dev` wants.

Run the app and press Ctrl+Shift+D, or Cmd+Shift+D on macOS, to show and hide the panel. Two environment variables change how it starts:

- `SMUDGE_DEVTOOLS=1` opens the panel when the window opens.
- `SMUDGE_DEVTOOLS_KEY` picks another key, such as `ctrl+alt+i`: modifiers named `ctrl`, `shift`, `alt` or `cmd`, then one character, joined by `+`.

The panel moves with a drag on its title bar and stays inside the window. Its button collapses it to the title bar. Pointer input reaches the panel only over it, and keys reach it only after you click it, so the app underneath keeps its input.

## Frames

![The frames tab, with the frame timeline and the switches](/devtools-frames.png)

The timeline holds the app's last 240 frames, newest at the right. Each bar stacks the stages of one frame: running the effects, layout, paint, the display list, damage, drawing, waiting for a buffer, and presenting. The line marks the time a frame may take at 60 frames a second, and a gap is a frame whose present was skipped. Hover a bar to read that frame's counts: how many nodes it painted, how many damage rectangles it had and how many pixels they covered.

The switches change what the overlay draws over the app:

- **Damage tint** marks in red the pixels the last frame repainted. It is the run-time form of `WindowOptions::show_damage`, and it works on the GPU renderer too.
- **Paint flashing** flashes green over what each frame repainted and fades it out, so a burst of changes stays visible after it ends.
- **Layout bounds** outlines every node, in a color for its depth. A node a scroll view or a clip hides is outlined too, where layout put it.
- **Focus** outlines every node Tab visits, numbers it in the tab order, and marks the focused node and the trap that holds the keyboard.
- **Hit testing** outlines every node that takes a press, the hovered node and its ancestors, and fills the node a held press began on.
- **Full repaint** makes the app repaint the whole window on every frame, so you can compare the two modes while it runs.
- **Keep tree** applies hot patches without remounting the root.

Below them, the tab lists the last few hot reload steps: a patch being built, a build that failed with the compiler's message, and a patch applied with its time and what the remount kept.

## Inspector

![The inspector tab, with the tree and the details of a node](/devtools-inspector.png)

Press **Pick** and point at the app: the node under the pointer shows its margin, border, padding and content boxes, and a click selects it without the app hearing it. The tree lists every node, hidden ones muted, and pressing a row selects it too.

The details of the selected node name its widget and its accessibility role, the file, line and column that built it, its states, its boxes, what it last painted, its style, and its bindings: how often each ran, the signals it read and where they were created, and the write that woke it last.

**Copy as test** writes the selected node's subtree to `target/smudge-devtools/`: a text file in the format of `smudge-test`'s `layout_dump`, measured from the node's corner, and an image of the node's rectangle at a scale of 1. A harness as large as the node, built from the same element, dumps the same text, and draws the same image as long as nothing behind or over the node paints, such as a card's background, so a bug found by hand becomes a snapshot test.

## Reactive and Access

The **Reactive** tab lists the places whose effects ran most, in runs a second, the cycles a frame cut short with the place of the write that kept waking them, and the owner tree with what each owner holds.

![The access tab, with its warnings and the accessibility tree](/devtools-access.png)

The **Access** tab shows the accessibility tree as a screen reader gets it, and warns about a node that takes the focus but has no name, a node that takes a press but reports no role, and a text input with no name.

## Tuning the theme from a file

`smudge::devtools::watch_theme` reads a theme file whenever it changes and writes its tokens into a theme signal the app binds, so colors and radii change while the app runs:

```rust
fn configure(app: &mut App) {
    let theme = RwSignal::new(Theme::dark());
    let shown = theme.clone();
    app.bind_theme(move || shown.get());
    std::mem::forget(smudge::devtools::watch_theme("theme.txt", theme));
}
```

Pass `configure` to `run_with`, which runs it under the app's owner, where the poll's timer lives. Each line of the file sets one token: a color as `#rrggbb` or `#rrggbbaa`, or a radius as a number of logical pixels. Lines that start with `#` are comments:

```text
# A warmer accent.
accent = #e07a3a
radius_md = 10
```

A line that names no token, or whose value does not parse, is skipped and shown on the frames tab. **Write theme** writes the watched theme in this format to `target/smudge-devtools/theme.txt`, as a starting point.

## What it costs

Nothing, with the feature off: none of this code is compiled in, and no field the devtools read exists. With it on, the panel runs as an app of its own, drawn over the window after the app's frame, and its own time is kept apart from the app's frame and present statistics. While the panel shows on the CPU renderer, each present copies the whole frame, because the panel is composited in the window's buffer alone.
