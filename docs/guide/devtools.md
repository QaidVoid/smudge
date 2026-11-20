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

The panel moves with a drag on its title bar, and **Hide** folds it to the bar. It stays inside the window. Pointer input reaches the panel only over it, and keys reach it only after you click it, so the app underneath keeps its input. The title bar shows what the app's frames cost on average, in red once that is over the budget of 60 frames a second.

The panel has six tabs, and each starts with a line that says what it is for.

## Frames

![The frames tab, with the frame timeline and a frame broken down by stage](/devtools-frames.png)

The panel records the frames the app draws while the panel is open. Frames that only redraw the panel are left out, so a still app records none, and the tab says so until you hover, type or scroll in the app. The tiles at the top sum up the last 240 frames: the newest, the average, the slowest, and how many took longer than 16.7 ms, the budget of a frame at 60 frames a second.

The timeline below them draws a bar per frame, newest at the right. Each bar stacks the stages of its frame in their colors, and the dashed line marks the budget, so a bar that crosses it missed 60 frames a second. A gap is a frame that presented nothing.

The card under the timeline is its legend. It names each stage in its color with what it took on average: running the effects, layout, paint, the display list, damage, drawing, waiting for a buffer, and presenting. Hover a bar, and the card breaks down that frame instead, with how many nodes it painted and how much damage it had.

## Overlays

![The overlays tab, with a switch for each mark](/devtools-overlays.png)

Each switch draws something over the app, under the panel, in the color its swatch shows:

- **Damage tint** tints in red what the app's last frame repainted, and the tint stays until the app's next frame. It is the run-time form of `WindowOptions::show_damage`, and it works on the GPU renderer too.
- **Paint flashing** flashes green over what each frame repainted and fades it out, so a burst of changes stays visible after it ends.
- **Layout bounds** outlines every node, in a color for its depth. A node a scroll view or a clip hides is outlined too, where layout put it.
- **Focus** outlines every node Tab visits, numbers it in the tab order, and marks the focused node and the trap that holds the keyboard.
- **Hit testing** outlines every node that takes a press, the hovered node and its ancestors, and fills the node a held press began on.

Under **Rendering**, **Full repaint** makes the app repaint the whole window on every frame, so the frames tab shows what damage tracking saves.

## Inspector

![The inspector tab, with the tree and the details of a node](/devtools-inspector.png)

Press **Pick** and point at the app: the node under the pointer shows its margin, border, padding and content boxes, and a click selects it without the app hearing it. The tree lists every node with its size, hidden ones muted, and pressing a row selects it too.

The details of the selected node name its widget and its accessibility role, and give the file, line and column that built it. Below them:

- **Box model** draws the node's margin, border and padding as nested bands around its content, with the width of each side, in the colors the overlay outlines them in.
- **State** gives where the node is, its size, what it last painted, its interaction states and what is out of date.
- **Bindings** lists each binding with how often it ran, the signals it read and where they were created, and the write that woke it last.
- **Style** lists the properties the node was built with that differ from the default, and those its state changes.

**Copy as test** writes the selected node's subtree to `target/smudge-devtools/`: a text file in the format of `smudge-test`'s `layout_dump`, measured from the node's corner, and an image of the node's rectangle at a scale of 1. A harness as large as the node, built from the same element, dumps the same text, and draws the same image as long as nothing behind or over the node paints, such as a card's background, so a bug found by hand becomes a snapshot test.

## Reactive

![The reactive tab, with the busiest effects](/devtools-reactive.png)

**Busiest effects** lists the places whose effects ran most, in runs a second, each with a bar for its share of the busiest. **Cycles** shows only when a frame cut a cycle short, with where its effect was created and the write that kept waking it. **Owners** sums up what the app's owners hold and lists them as a tree.

## Access

![The access tab, with its warnings and the accessibility tree](/devtools-access.png)

The access tab counts the warnings and the nodes, then explains each warning and how to fix it: a node that takes the focus but has no name, a node that takes a press but reports no role, and a text input with no name. Below them, the tree shows what assistive technology reads, in order, with each node's role, name and value. Press a warning or a row to see its node in the inspector.

## Live

![The live tab, with hot reload and the theme file](/devtools-live.png)

**Hot reload** lists the last few steps of `cargo smudge dev`, newest first: a patch being built, a build that failed with the compiler's message, and a patch applied with its time and what the remount kept. **Keep tree** applies patches without remounting the root, so all state stays. An edit to what the build creates shows at the next remount, and a closure created after the first patch keeps that patch's code.

**Theme file** says which file `watch_theme` watches, and lists the lines its last read skipped.

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

A line that names no token, or whose value does not parse, is skipped and shown on the live tab. **Write theme** writes the watched theme in this format to `target/smudge-devtools/theme.txt`, as a starting point.

## What it costs

Nothing, with the feature off: none of this code is compiled in, and no field the devtools read exists. With it on, the panel runs as an app of its own, drawn over the window after the app's frame, and its own time is kept apart from the app's frame and present statistics. While the panel shows on the CPU renderer, each present copies the whole frame, because the panel is composited in the window's buffer alone.
