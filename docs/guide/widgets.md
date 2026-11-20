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

A label shows one string, plain or bound to a closure. It wraps to the width its parent gives it. See [Text and input](./text-and-input) for sizes and fonts, and for `rich_text`, which mixes weights, slants, colors and links in one paragraph.

## Text input

```rust
let name = RwSignal::new(String::new());

let field = text_input(name.clone())
    .width(pct(100.0))
    .height(px(32.0))
    .padding_xy(Size { width: 8.0, height: 6.0 })
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

`image(source)` draws one image inside its content box, with the background, border and padding of any panel around it. The source is an `Option<Image>` and can be bound to a signal, so a picture that is still decoding shows nothing until the pixels arrive. With no size set, the node takes the image's pixel size divided by the window's scale factor, so one image pixel covers one logical pixel and a 200 pixel wide image is 100 logical pixels wide at scale 2. With only a width or only a height, it keeps the image's aspect ratio, and with both it stretches. A container that stretches its children fixes that axis too, as a column does across its width by default, so the image scales to it with its aspect ratio kept. Give the container `align_items(Align::Start)`, or any alignment but stretch, to keep the image's own size. `corner_radius` rounds the rectangle the image clips to, and `corner_radii` rounds each of its corners on its own.

Decoding a file is the application's job. `Image::from_straight_rgba(width, height, pixels)` takes the RGBA8 pixels a decoder produces and premultiplies them; `Image::from_premultiplied_rgba` takes pixels whose color channels are already multiplied by alpha, and clamps a color channel above its pixel's alpha down to that alpha, since no multiplication could have produced it. Both return a `Result`, an `ImageError` when the buffer does not hold `width * height * 4` bytes or a side is zero.

## Scroll view

```rust
let log = scroll(
    column().children((0..200).map(|line| label(format!("line {line}")))),
)
.height(px(300.0));
```

`scroll(content)` clips its content to its own box and scrolls it vertically with the wheel or touchpad, and shows a thumb along its right edge while the content does not fit. Dragging the thumb scrolls the view, and a press on the track beside it scrolls one page toward the press. The scrollbar takes presses along the view's right edge, eight logical pixels wide, and leaves the keyboard focus where it was, so dragging it does not take the focus out of a field. Scrolling does not repaint the content: the rows keep their paint and only move, so a scroll stays cheap however many rows it moves.

When Tab, `autofocus()`, a focus restored after a remount, a focus trap handing the focus back, or `App::request_focus` moves the keyboard focus to a node the view hides, the view scrolls just far enough to show that node at its nearest edge, and every scroll view around it does the same. A node taller or wider than the view is shown from its top or its left. A click that focuses a node scrolls nothing, because the node is already under the pointer.

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

While the signal is true, the view shows the end of the content and stays there as lines arrive, which is what an output log wants. Scrolling up writes false, so the reader can look back in peace. Scrolling back to the end or setting the signal to true pins it again. A focus move that scrolls the view up to show the focused node writes false too.

### Scrolling sideways

`ScrollExt::axis` makes a view scroll sideways, for timelines, wide tables or code that does not wrap, or both ways, for a canvas or a grid:

```rust
use smudge::widgets::{ScrollAxis, ScrollExt};

let across = RwSignal::new(0.0_f32);

let timeline = scroll(row().children((0..48).map(|slot| label(format!("Slot {slot}")).width(px(80.0)))))
    .axis(ScrollAxis::Horizontal)
    .horizontal_offset(across.clone())
    .height(px(40.0));
```

A view that scrolls sideways lays its content out at the content's own width, and at least the view's, so narrow content still fills it, and shows a second thumb along its bottom edge that drags and pages like the first. `ScrollAxis::Both` scrolls both ways, and its two scrollbars stop short of the corner where they would meet. `offset` binds the vertical position, as before, and `horizontal_offset` binds the horizontal one. Following the end stays vertical.

A touchpad's sideways swipe scrolls the view, and so does the mouse wheel while Shift is held: while Shift is down, a wheel event with no sideways part reaches every `on_wheel` handler turned sideways. A handler of your own that gives Shift and the wheel another meaning, such as a zoom, reads the sideways part then.

A view decides by the direction a wheel event mostly goes, so the slight sideways drift of a touchpad's vertical swipe counts as vertical. It takes the event only when it can move that way. One that already reached its end, or that does not scroll that way, passes the event to the view around it, so a page keeps scrolling over a sideways strip inside it and past a list scrolled to its end. A focus move reveals the focused node on both axes, and a drag held past any edge a view scrolls toward scrolls it.

## Split panes

`split(direction, first, second)` shows two panes along a row or a column, with a divider between them that a drag moves:

```rust
use smudge::prelude::*;

let ratio = RwSignal::new(0.3);
let editor = split(
    Direction::Row,
    label("Files"),
    split(Direction::Column, label("Editor"), label("Terminal")).min_second(80.0),
)
.ratio(ratio)
.min_first(160.0)
.min_second(240.0);
```

The ratio is the first pane's share of the split's length, a number from 0 to 1. The divider writes it as it moves, so keeping the signal keeps the layout, and writing the signal moves the divider. `min_first` and `min_second` keep each pane at least that long in logical pixels, while dragging and when the window shrinks. With the keyboard focus on the divider, the arrow keys of its axis move it 16 pixels, and Home and End move it to the minimums. A double click on it goes back to the ratio the split was built with, and a screen reader announces it as a splitter named "Resize". Each pane stretches its content to fill it, over the height and growth the content sets itself, so a split nested in a pane fills it. Wrap content that keeps its own size, such as a toolbar, in a `column()`. Content taller than its pane ends at the pane's edge, so a scroll view below a toolbar in a column scrolls instead of running past the pane. The split clips its panes.

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

The list owns its scroll position, so do not call `.offset(...)` on it. A virtual list scrolls up and down only, whatever `axis` it is given.

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

`virtual_list_keyed(items, key, estimate, row)` takes the items themselves and a key per item. Keep the items in a signal of `Rc<[T]>` as above: the list compares the `Rc` it gets back, so a clone of the same `Rc` costs nothing. Every update calls the key closure once per item, so prefer a cheap key such as an integer id; a key that builds a `String` allocates on every call. An update that only adds items after the end skips rebuilding the list's height index, and any other change rebuilds it. The list starts from the estimate and replaces it with each row's measured height, so rows of different heights all work, and a row's measured height is remembered across list updates. Reordering, filtering or inserting does not confuse it: rows are tracked by key, not by position.

Pick the fixed `virtual_list` when every row is the same height and the items are indexed positions; pick `virtual_list_keyed` when heights differ or the items have identities that survive reordering.

Both tell a screen reader that they hold a list, with how many items it has. Content that is not a list, such as the blocks of a long document, wants `virtual_column_keyed`, which takes the same arguments and behaves the same way but reports no list, so a reader reads the content as it is.

## Canvas

`canvas(draw)` draws whatever its closure paints: lines, curves and shapes, rectangles and text, in its own coordinates, with the origin at the top left of its content box. The closure runs again when a signal it read changes, or when the canvas moves or resizes, so a selection box, a grid or a gizmo follows its state. `painter.theme()` gives the window's theme, so a canvas can draw in its tokens' colors, and a canvas that asked for it draws again when the theme changes:

```rust
use smudge::core::path::{Dash, LineCap};

let selection = RwSignal::new(Rect::new(20.0, 16.0, 120.0, 72.0));
let shown = selection.clone();
let view = canvas(move |painter| {
    let size = painter.size();
    for x in (0..size.width as u32).step_by(16) {
        let x = x as f32;
        painter.stroke(
            &Path::line(Point { x, y: 0.0 }, Point { x, y: size.height }),
            Stroke::new(1.0, Color::rgb(60, 64, 72)),
        );
    }
    let area = shown.get();
    painter.fill_rect(area, Corners::ZERO, Color::rgba(80, 150, 255, 40));
    painter.stroke(
        &Path::polygon([
            Point { x: area.x0, y: area.y0 },
            Point { x: area.x1, y: area.y0 },
            Point { x: area.x1, y: area.y1 },
            Point { x: area.x0, y: area.y1 },
        ]),
        Stroke::new(1.5, Color::rgb(80, 150, 255))
            .with_cap(LineCap::Square)
            .with_dash(Dash::new([6.0, 4.0])),
    );
    painter.text("selection", Point { x: area.x0, y: area.y1 + 4.0 }, 12.0, Color::WHITE);
})
.height(px(240.0));
```

A `Path` is built from moves, lines, quadratic and cubic curves, arcs and closes, and `Path::line`, `Path::polyline`, `Path::polygon`, `Path::circle` and `Path::ellipse` build the common ones. `fill` covers it by the nonzero rule, and `draw` with a `PathPaint` picks the even-odd rule, which cuts a hole wherever one subpath lies inside another, or fills and strokes in one call. A `Stroke` has a width, a color, miter, round or bevel joins, butt, round or square caps, a miter limit and an optional dash. A path with a point that is not finite, or more than a million logical pixels from the origin, draws nothing. A dash pattern that would cut its path into more than a hundred thousand pieces draws the stroke solid. Both renderers draw paths, so a canvas looks the same on the CPU and the GPU to within their antialiasing.

## Popups and overlays

`overlay(open, placement, build)` opens the element `build` returns over the whole window while `open` reads `true`. It escapes every clip around the node it belongs to, so a popup inside a scroll view is not cut at the view's edge, and it draws over the window in the popups' layer, the last opened on top. `overlay_in` opens one in another layer: tooltips draw above popups, each dialog above everything opened before it, with its own popups and tooltips above it, and toasts above everything:

```rust
use smudge::core::element::Placement;

let open = RwSignal::new(false);
let toggle = open.clone();
let picker = button("Colors")
    .on_press(move || toggle.update(|open| *open = !*open))
    .overlay(open, Placement::Below, || {
        column()
            .padding(px(8.0))
            .background(ColorToken::SURFACE)
            .border(1.0)
            .border_color(ColorToken::BORDER)
            .child(label("Red"))
            .child(label("Green"))
    });
```

`Placement::Below` and `Placement::Above` open against the node's rectangle, `Placement::After` to its right, as a submenu does, and `Placement::At(point)` at a point inside it, as a context menu opens at the pointer. An overlay flips to the other side where the window has no room, shifts to stay inside the window, and follows its node as a scroll moves it. Its size comes from its content alone.

Escape and a press outside the overlay write `false` to `open`, so it closes. A press on the node an overlay opens against is not outside it, so the button above closes its popup with its own press. A press over an overlay reaches the overlay and never the window under it, and keys inside it bubble on to the node that opened it and that node's ancestors, so the shortcuts around it keep working. An overlay that takes the keyboard calls `trap_focus()`, and `autofocus()` on the node that should take the focus first: Tab then stays inside it, and the focus goes back to the node that opened it when it closes, or where it was when that node cannot take the focus.

## Menus

`menu_bar()` builds a bar of menus, `menu(title, items)` a button that opens one, and `context_menu(items)` opens one at the pointer on a right press. Each takes a closure that returns the items, which runs every time the menu opens, so the items follow the app's state:

```rust
use smudge::widgets::{
    ContextMenuExt, menu_bar, menu_check_item, menu_item, menu_separator, submenu,
};

let grid = RwSignal::new(true);
let shown = grid.clone();
let bar = menu_bar()
    .menu("File", || {
        vec![
            menu_item("Open", || println!("open")).shortcut("Ctrl+O"),
            submenu("Recent", || vec![menu_item("map.map", || {})]),
            menu_separator(),
            menu_item("Quit", || {}).disabled(true),
        ]
    })
    .menu("View", move || vec![menu_check_item("Grid", shown.clone())]);

let canvas_area = panel()
    .grow(1.0)
    .focusable()
    .context_menu(|| vec![menu_item("Paste here", || {})]);
```

`menu_item(label, on_choose)` runs its closure after the menus close, under the owner the menu's opener was built in, so the signals and timers it creates outlive the menu, `menu_check_item(label, signal)` toggles a signal and shows a check mark while it is on, `submenu(label, items)` opens another menu beside it, and `menu_separator()` draws a line between groups. `.shortcut(text)` shows a shortcut at the right of an item. It only tells the user, and the app handles the keys itself. `.disabled(value)` greys an item out.

The keyboard walks every menu: Up and Down step through the items, a letter goes to the next item that starts with it, Enter and Space choose, Right opens a submenu and Left closes it, Escape closes the menu that is open, and in a menu bar Left and Right move to the next menu. A context menu also opens with the menu key, or Shift+F10, at the top left of the focused node. Screen readers hear the menu bar, menu, menu item and checked item roles, a lone `menu` as a button, and whether each title or submenu is open.

Under them is `navigate_with_arrows(direction)`, which any container can use, so the arrow keys move the focus among the focusable nodes inside it, as a list or a toolbar does.

## Tooltips and dropdowns

`tooltip_text(text)` shows a short text below any element once the pointer has rested on it for a moment, and hides it when the pointer leaves, on a press, and on Escape. The pointer passes through a tooltip to whatever lies under it, and a screen reader hears the text as the element's description. `tooltip(build)`, on any element, shows an element of your own the same way.

`dropdown(choices, selected)` shows the choice `selected` picks and opens the list of choices below it:

```rust
use smudge::widgets::{TooltipExt, dropdown};

let brush = RwSignal::new(1_usize);
let picker = row()
    .child(
        dropdown(
            || vec![String::from("Small"), String::from("Medium"), String::from("Large")],
            brush,
        )
        .accessible_label("Brush size"),
    )
    .child(button("Clear").tooltip_text("Clear the canvas"));
```

A press, Enter, Space or Down opens the list on the picked choice, or on the first one when `selected` names none. Up and Down walk it, a letter goes to the next choice that starts with it, and Enter or a press picks one and closes the list, while Escape or a press outside closes it unchanged. An empty list does not open, and a list whose choices change while it is open closes. A screen reader hears a combo box whose value is the picked choice, and whether its list is open.

## Dialogs and toasts

`modal(open, content)` shows the element `content` builds as a dialog, centered over the window on a dimmed backdrop, while `open` reads `true`:

```rust
use smudge::widgets::modal;

let open = RwSignal::new(false);
let (ask, close) = (open.clone(), open.clone());

let page = column()
    .child(button("Delete").on_press(move || ask.set(true)))
    .child(modal(open, move || {
        let close = close.clone();
        column()
            .padding(px(16.0))
            .background(ColorToken::SURFACE)
            .accessible_label("Delete the map?")
            .child(label("Delete the map?"))
            .child(button("Cancel").on_press(move || close.set(false)))
    }));
```

The dialog covers the whole window wherever it sits in the tree, so one opened from inside a scroll view is never cut at the view's edge. While it shows, the backdrop takes every press and the dialog holds the keyboard: Tab stays inside it, and its first focusable node takes the focus as it opens, unless a node inside asks for it with `autofocus()`. Escape closes it by writing `false` to `open`, and the focus goes back to the node that held it before. A press on the backdrop does not close it, since a dialog often holds work in progress. The content's own element is the dialog: give it a look, a surface and padding, and a name with `accessible_label`. A screen reader hears a modal dialog and reaches nothing behind it. `.fade(spring)` fades the dialog and its backdrop in and out.

`Toasts` shows short notices in the window's bottom right corner, which `toast_host` draws. Mount the host once and keep a clone of the handle wherever a notice starts:

```rust
use smudge::widgets::{Toasts, toast_host};

let toasts = Toasts::new();
let saved = toasts.clone();

let page = column()
    .child(button("Save").on_press(move || saved.show("Saved")))
    .child(toast_host(&toasts));
```

A toast shows for four seconds, `show_for` sets another time, and a press closes it sooner. Its timer pauses while the pointer is over it. At most four show at once, the newest at the bottom, and a fifth closes the oldest. A toast never takes the focus: a screen reader hears it through the host's announcer, politely, or at once with `show_urgent`. Call `show` on the window's thread, from a handler or from the handler of a message a worker sent.

Every popup, menu, tooltip, dialog and toast draws in a layer of its own kind: the app's popups and then its tooltips, then each dialog in the order it opened, with what opens from inside it above it, and the toasts above everything. Losing the window's input focus closes popups and tooltips, and leaves dialogs and toasts open.

## Form controls

`checkbox`, `switch`, `slider` and `number_field` bind to signals, draw in the theme's colors, and work from the keyboard:

```rust
use smudge::widgets::{checkbox, number_field, slider, switch};

let grid = RwSignal::new(true);
let snap = RwSignal::new(false);
let opacity = RwSignal::new(0.8);
let width = RwSignal::new(12.0);
let form = column()
    .child(checkbox("Show the grid", grid))
    .child(switch("Snap to grid", snap))
    .child(slider(opacity, 0.0..=1.0, 0.05).accessible_label("Opacity"))
    .child(number_field("Width", width, 1.0..=512.0, 1.0));
```

A checkbox and a switch toggle on a press, Enter or Space. A slider sets the number under the pointer as it is pressed and dragged, and the arrow keys, Page Up, Page Down, Home and End move it. A number field reads what was typed when Enter is pressed and when the focus leaves it after an edit, moving it onto a step of the range, and Up, Down, the wheel while it holds the focus, and the arrows at its right step it. Its first argument is the name a screen reader hears. Every number a slider or a number field sets lies on a step and inside the range, counted from the range's start, and a screen reader hears each control's role, state and number, and can step a slider or a number field up and down.

A control of your own that checks its input when the user moves on uses `on_focus_within`, which hears the focus enter the node or anything inside it, and leave them all.

## Tabs

`tabs(selected)` builds a row of tabs over their panels, and shows the panel of the tab `selected` names by index:

```rust
use smudge::widgets::tabs;

let shown = RwSignal::new(0_usize);
let side = tabs(shown)
    .tab("Entities", || label("Every entity of the map"))
    .tab("Textures", || label("The texture browser"));
```

A press on a tab shows its panel, and Left and Right move along the tabs and show each panel as they reach it. Every panel is built once and stays mounted, hidden while another shows, so what it holds, such as the text of a field or where a list was scrolled to, is there again when its tab comes back. Screen readers hear a tab list, its tabs with the shown one selected, and the panels.

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
