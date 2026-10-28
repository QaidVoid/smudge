# Widgets

<p class="lead">v0.1 ships a small set of widgets, each built from the same panel and label every application can use. They are styled with theme tokens, so a theme switch repaints them with no extra code.</p>

## Containers

| Widget | What it does |
|---|---|
| `column()` | Lays children out top to bottom. |
| `row()` | Lays children out left to right. |
| `stack()` | Layers children on top of each other in the same box. |
| `spacer()` | Takes the leftover space on the main axis, pushing its neighbors apart. |
| `panel()` | The plain box: style, children, no behavior. |

All of them are elements, so every builder from [Elements and layout](./layout) and [Styling](./styling) applies.

## Button

```rust
use smudge::prelude::*;

let saves = RwSignal::new(0_u32);
let save = saves.clone();

let actions = row()
    .gap(Size { width: 8.0, height: 0.0 })
    .child(button("Cancel"))
    .child(button("Save").primary().on_press(move || save.update(|n| *n += 1)));
```

`button(text)` is a focusable panel with a label, hover and pressed colors, and the button role for screen readers. `.primary()` switches it to the accent colors. It activates on a pointer press, and on Enter or Space while it has keyboard focus.

## Label

```rust
let title = label("Settings").text_size(22.0);
let status = label(move || format!("{} unread", unread.get()));
```

A label shows one string, plain or bound to a closure. It wraps to the width its parent gives it. See [Text and input](./text-and-input) for sizes and fonts.

## Text input

```rust
let name = RwSignal::new(String::new());

let field = text_input(name.clone())
    .width(pct(100.0))
    .height(px(32.0))
    .placeholder("Your name")
    .border(1.0)
    .border_color(ColorToken::BORDER);
```

The input is bound both ways: typing writes the signal, and writing the signal from elsewhere replaces the text. Details are in [Text and input](./text-and-input).

## Scroll view

```rust
let log = scroll(
    column().children((0..200).map(|line| label(format!("line {line}")))),
)
.height(px(300.0));
```

`scroll(content)` clips its content to its own box and scrolls it vertically with the wheel or touchpad, showing a thumb as a position indicator. Scrolling does not repaint the content: the rows keep their paint and only move, so a scroll frame through a thousand rows stays well under a millisecond on the CPU renderer.

To control the position from your own state, bind it to a signal with `ScrollExt::offset`:

```rust
use smudge::widgets::ScrollExt;

let offset = RwSignal::new(0.0_f32);
let to_top = offset.clone();

let view = scroll(long_content)
    .offset(offset.clone())
    .height(px(300.0));
let top = button("Back to top").on_press(move || to_top.set(0.0));
```

The view writes the clamped position back to the signal after it paints, so the signal always reads what is on screen.

## Virtual list

```rust
let rows = RwSignal::new(100_000_usize);
let count = rows.clone();

let list = virtual_list(
    move || count.get(),
    28.0,
    |index| label(format!("Row {index}")).height(px(28.0)).padding(px(4.0)),
)
.height(px(420.0));
```

`virtual_list(count, row_height, row)` mounts only the rows near the viewport, so a hundred thousand rows cost the tree and the layout about as much as the twenty on screen. `row` receives the row's index and builds that row the first time it scrolls into view; the row stays mounted, with its state, until the view moves past it. Content that changes while a row is visible must come from signals the row reads. Every row must be exactly `row_height` tall. The count is a binding: when the signals it reads change, the list updates its window of rows.

The list owns its scroll position, so do not call `.offset(...)` on it.

## Writing your own

The button, the containers, the scroll view and the virtual list are composed from `panel`, `label` and the public element builders. A component of your own is the same thing, a function that returns an element:

```rust
fn stat(title: &'static str, value: RwSignal<u32>) -> Element<Panel> {
    column()
        .padding(px(12.0))
        .corner_radius(RadiusToken::MEDIUM)
        .background(ColorToken::SURFACE)
        .child(label(title).text_color(ColorToken::TEXT_MUTED))
        .child(label(move || value.get().to_string()).text_size(24.0))
}
```

A widget that paints something new, as the label and the text input do, implements the `Widget` trait from `smudge::core::widget`: it measures itself, paints into a list of operations, and describes itself to screen readers, while the element around it carries its style and event handlers. The [API docs](https://docs.rs/smudge-core/latest/smudge_core/widget/trait.Widget.html) describe the trait.
