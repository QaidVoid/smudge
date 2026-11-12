# Widgets

<p class="lead">Smudge ships a small set of widgets, each built from the same panel and label every application can use. They are styled with theme tokens, so a theme switch repaints them with no extra code.</p>

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

The input is bound both ways: typing writes the signal, and writing the signal from elsewhere replaces the text. A multi-line editor exists too: see [Text and input](./text-and-input) for `text_area`.

## Image

```rust
use smudge::core::image::Image;

let source = RwSignal::new(None);
let picture = image(move || source.get()).size(Size { width: 96.0, height: 96.0 });
```

`image(source)` draws one image inside its content box, with the background, border and padding of any panel around it. The source is an `Option<Image>` and can be bound to a signal, so a picture that is still decoding shows nothing until the pixels arrive. With no size set, the node takes the image's size; with only a width or only a height, it keeps the image's aspect ratio. `corner_radius` rounds the rectangle the image clips to.

Decoding a file is the application's job. `Image::from_straight_rgba(width, height, pixels)` takes the RGBA8 pixels a decoder produces and premultiplies them; `Image::from_premultiplied_rgba` takes pixels whose color channels are already multiplied by alpha. Both return a `Result`, an `ImageError` when the buffer does not hold `width * height * 4` bytes or a side is zero.

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

For a view that keeps up with content growing underneath it, `ScrollExt::follow_end` pins the view to the end:

```rust
use smudge::widgets::ScrollExt;

let follow = RwSignal::new(true);

let log = scroll(output_lines).follow_end(follow).height(px(300.0));
```

While the signal is true, the view shows the end of the content and stays there as lines arrive, which is what an output log wants. Scrolling up writes false, so the reader can look back in peace; scrolling back to the end or setting the signal to true pins it again.

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

### Rows of their own height

Rows that grow with their content, as a log line does, want `virtual_list_keyed`:

```rust
use std::rc::Rc;

use smudge::widgets::virtual_list_keyed;

let items: RwSignal<Rc<[u32]>> = RwSignal::new(Rc::from([12, 40, 8, 24]));
let source = items.clone();

let list = virtual_list_keyed(
    move || source.get(),
    |item| *item,
    24.0,
    |item| label(format!("Item {item}")),
)
.height(px(420.0));
```

`virtual_list_keyed(items, key, estimate, row)` takes the items themselves and a key per item. Keep the items in a signal of `Rc<[T]>` as above: the list compares the `Rc` it gets back, so a clone of the same `Rc` costs nothing, and appending to the end costs only the new rows. The list starts from the estimate and replaces it with each row's measured height, so rows of different heights all work, and a row's measured height is remembered across list updates. Reordering, filtering or inserting does not confuse it: rows are tracked by key, not by position.

Pick the fixed `virtual_list` when every row is the same height and the items are indexed positions; pick `virtual_list_keyed` when heights differ or the items have identities that survive reordering.

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
