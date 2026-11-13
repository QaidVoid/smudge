# Elements and layout

<p class="lead">An interface is a tree of elements. Each element wraps a widget and carries its style, its children and its event handlers. Layout is flexbox.</p>

## Building a tree

Every widget constructor returns an `Element`, and every builder method returns it again, so a tree reads top to bottom:

```rust
use smudge::prelude::*;

let toolbar = row()
    .padding(px(8.0))
    .gap(Size { width: 8.0, height: 0.0 })
    .align_items(Align::Center)
    .child(button("Open"))
    .child(button("Save").primary())
    .child(spacer())
    .child(label("Ready"));
```

`panel()` is the plain box every other widget builds on: it has a style and children and draws nothing but its background and border. `column()` and `row()` are panels with a direction, `stack()` layers its children on top of each other, and `spacer()` grows to fill the space between its neighbors.

`child` adds one element, and `children` adds any iterator of them:

```rust
let list = column().children(["Red", "Green", "Blue"].map(label));
```

## Lengths

Sizes and spacing take a `Length`:

| Builder | Meaning |
|---|---|
| `px(24.0)` | Logical pixels, scaled by the display's scale factor. |
| `pct(50.0)` | Percent of the parent's content box. |
| `Length::Auto` | Sized by the content or the flex rules. The default. |

```rust
let sidebar = column()
    .width(px(240.0))
    .height(pct(100.0))
    .min_width(px(180.0))
    .padding(px(12.0));
```

`size` sets both dimensions at once in pixels, `padding_xy` sets horizontal and vertical padding separately, and `margin` adds space outside the border.

## Flex layout

A panel lays its children out along its direction, which is a row unless you call `.column()` or use `column()`.

| Method | What it controls |
|---|---|
| `direction(Direction::Row)`, `.row()`, `.column()` | The main axis. |
| `gap(Size { width, height })` | Space between adjacent children. |
| `align_items(Align::...)` | Cross-axis placement: `Start`, `Center`, `End` or `Stretch`. |
| `justify_content(Justify::...)` | Main-axis placement: `Start`, `Center`, `End` or `SpaceBetween`. |
| `wrap(true)` | Lets children wrap onto further lines. |
| `grow(1.0)`, `shrink(0.0)`, `basis(px(120.0))` | How a child takes or gives up space. |

```rust
let header = row()
    .justify_content(Justify::SpaceBetween)
    .align_items(Align::Center)
    .padding_xy(Size { width: 16.0, height: 8.0 })
    .child(label("Inbox"))
    .child(button("Compose").primary());
```

## Absolute placement

`absolute()` takes a child out of the flow and places it by `inset` against its parent. Use it for badges, overlays inside a panel, and anything that should not push its siblings.

```rust
use smudge::core::style::Edges;

let badge = panel()
    .absolute()
    .inset(Edges { top: px(-4.0), right: px(-4.0), bottom: Length::Auto, left: Length::Auto })
    .size(Size { width: 12.0, height: 12.0 })
    .corner_radius(6.0)
    .background(Color::rgb(255, 64, 64));
```

## Pinning a child

`pin_child` keeps one flowing child at a fixed place in its parent, whatever the heights of the children before it. The closure returns the child's index and how far below the top of the content box its top sits, or `None` to leave the children alone. Every layout pass then moves the children together, outside the layout engine: the ones before the pinned child stack upward from it, the ones after it stack downward, edge to edge, so a child that grows pushes its neighbours away in the same frame. This is what keeps the keyed virtual list's anchored row still while rows above it change height.

```rust
let anchor = RwSignal::new(Some((1, 100.0)));
let pinned = anchor.clone();

let rows = column()
    .pin_child(move || pinned.get())
    .child(label("Grows as it wraps"))
    .child(label("Stays 100 pixels down"));
```

The pin moves the children the way a scroll offset does, so do not give the same node a scroll offset as well. A change of the pin alone moves the children without a layout pass.

## Clipping and hiding

`clip(true)` clips children to the panel's padding box. `overflow(Overflow::Scroll)` clips the same way and also makes the panel a scroll container: as a flex item it shrinks to the space it is given instead of growing to its content, which is what the scroll view and the virtual lists use, so a list with `grow(1.0)` in a column fills what is left of it. `hidden(true)` removes a node from layout and paint while keeping it mounted, so its state survives; bind it to a signal to toggle visibility cheaply. To mount and unmount instead, use [`show`](./dynamic-content).

## What layout costs

Layout runs only in frames where something that affects size changed: text, a length, a child added or removed. A change of color, border color or radius repaints the node and skips layout entirely. When layout does run, only the parts of the tree whose size inputs changed are measured again.
