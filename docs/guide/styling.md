# Styling and themes

<p class="lead">Style is set with builder methods on the element. Colors and radii can be fixed values, theme tokens, or closures over signals, and interaction states are written as small patches.</p>

## Paint properties

```rust
use smudge::prelude::*;

let card = panel()
    .padding(px(16.0))
    .background(Color::rgb(28, 31, 37))
    .border(1.0)
    .border_color(Color::rgb(42, 46, 55))
    .corner_radius(10.0)
    .child(label("Synced 2 minutes ago").text_color(Color::rgb(236, 238, 243)));
```

| Method | Takes |
|---|---|
| `background` | A `Color`, a `ColorToken`, or a closure returning either (with `.into()`). |
| `border` | The border width in logical pixels, the same on all four sides. |
| `border_edges` | An `Edges<f32>` with the width of each side, such as a bottom border alone under a header. |
| `border_color` | Same as `background`. |
| `corner_radius` | An `f32`, a `RadiusToken`, or a closure, the same on all four corners. |
| `corner_radii` | A `Corners` of `f32` radii, a `CornerRadii` with an `f32` or a `RadiusToken` per corner, or a closure returning a `CornerRadii`, such as a card whose top corners are square under a header. |
| `text_color` | The color of the node's own text, on a label or an input. A container ignores it; use `style_children` or `provide_text_style` to color the text below it. |

`Color::rgb(r, g, b)` and `Color::rgba(r, g, b, a)` take 8-bit sRGB channels. `Color::WHITE`, `Color::BLACK` and `Color::TRANSPARENT` are predefined.

Where two sides of different widths meet at a rounded corner, the border's inner edge curves as a quarter ellipse whose radii are the corner radius less each side's width, so the band narrows smoothly along the curve from the wider side to the narrower one.

The border widths take space from the content, so changing them lays the node out again. None of the other properties affect size, so changing them repaints the node without laying anything out again.

## Theme tokens

A token names a role instead of a color. It resolves against the current theme when the node paints, so switching the theme repaints every node that uses a token and nothing else.

```rust
let surface = panel()
    .background(ColorToken::SURFACE)
    .border(1.0)
    .border_color(ColorToken::BORDER)
    .corner_radius(RadiusToken::MEDIUM)
    .child(label("Theme-aware").text_color(ColorToken::TEXT));
```

| Color tokens | Radius tokens |
|---|---|
| `SURFACE`, `SURFACE_HOVER`, `SURFACE_PRESSED` | `SMALL` |
| `TEXT`, `TEXT_MUTED` | `MEDIUM` |
| `ACCENT`, `ACCENT_HOVER`, `ACCENT_PRESSED`, `ACCENT_TEXT` | `LARGE` |
| `BORDER`, `FOCUS_RING` | |
| `BACKDROP` | |

The built-in widgets use tokens throughout, so they follow the theme with no extra code. `Theme::light()` and `Theme::dark()` are the two built-in themes.

### Switching the theme

`run_with` gives you the `App` once before the first frame. Bind the theme to a signal there:

```rust
let dark = RwSignal::new(true);
let toggle = dark.clone();

run_with(
    options,
    move || {
        column()
            .background(ColorToken::SURFACE)
            .child(button("Toggle theme").on_press(move || toggle.update(|d| *d = !*d)))
    },
    move |app| {
        app.bind_theme(move || if dark.get() { Theme::dark() } else { Theme::light() });
    },
)?;
```

## Interaction states

`hover`, `pressed`, `focused` and `disabled_style` take a patch: a few properties that apply only while the node is in that state.

```rust
let row_item = panel()
    .padding(px(8.0))
    .corner_radius(6.0)
    .background(Color::TRANSPARENT)
    .hover(|p| p.background(ColorToken::SURFACE_HOVER))
    .pressed(|p| p.background(ColorToken::SURFACE_PRESSED))
    .on_press(|| println!("opened"));
```

Patches apply in a fixed order: base, hovered, focused, pressed, then disabled. A pressed button that is also hovered therefore shows its pressed color. A patch overrides only the properties it sets, and a state change re-resolves the node's style alone, repainting it when the patch sets paint properties. The `focused` patch applies only while focus last moved by keyboard, so a focus ring follows Tab and not a mouse click.

Patches hold fixed values. For a value that changes with your own state, bind the base property to a closure and let the patches override it while their state is active.

`disabled(true)`, or a closure, takes the node out of hit testing, so its pointer handlers do not run, and applies its `disabled_style` patch.

## Opacity

`opacity` draws a node and everything under it as one group, blended at a value from 0.0 to 1.0:

```rust
let ghost = panel()
    .padding(px(8.0))
    .background(ColorToken::SURFACE)
    .opacity(0.5)
    .child(label("Archived"));
```

The children cover each other as usual, and only the result shows through, so overlapping children never show through each other. A node at 0.0 draws nothing, while it still takes part in layout and hit testing. Changing the opacity repaints no child: the group is blended again over the pixels it covers.

## Transitions

`transition` moves a node's background, border color, text color, corner radius and opacity under a spring whenever their values change, instead of switching them at once. The change can come from a state patch, a bound closure or a theme switch.

```rust
let row_item = panel()
    .padding(px(8.0))
    .corner_radius(6.0)
    .background(Color::TRANSPARENT)
    .hover(|p| p.background(ColorToken::SURFACE_HOVER))
    .transition(Spring::SNAPPY);
```

`Spring::SNAPPY` suits state changes, `Spring::SMOOTH` arrives without overshooting, and `Spring::BOUNCY` swings past its value and back. A `Spring` of your own sets its stiffness, damping and mass. Colors move in linear sRGB with premultiplied alpha, so a fade never passes through a darker or lighter color than its ends, and a background or border that appears or goes away fades in or out. A text color that becomes set or unset switches at once, since an unset text color is the widget's own. A new value that arrives mid-motion continues from where the motion is, at the speed it has, so a hover that ends halfway turns back without a jump. The node's first paint shows its values as they are.

A moving node repaints alone, with no layout. The window keeps drawing frames while anything moves and goes idle once everything has arrived. When the system asks for reduced motion, every new value shows at once. The window reads that setting from the desktop's settings portal on Linux and the `prefers-reduced-motion` media query on the web, following their changes, and from the system's accessibility settings on Windows and macOS, when it starts and whenever it regains the focus. `App::set_reduced_motion` overrides it for your app. Layout properties never move.

## Styling children

`style_children` applies one style function to every direct child, which keeps repeated styling in one place:

```rust
let tags = row()
    .gap(Size { width: 6.0, height: 0.0 })
    .style_children(|style| {
        style.layout.padding = Edges::all(px(4.0));
        style.paint.background = Some(ColorToken::SURFACE_HOVER.into());
    })
    .children(["rust", "gui", "cpu"].map(label));
```

## Text style

Labels and inputs read a text style from their ancestors when they mount. `provide_text_style` sets it for everything built below, so call it before building the widgets it should reach:

```rust
provide_text_style(TextStyle { size: 15.0, ..current_text_style() });
```

Individual labels override it with `.text_size(18.0)`. See [Text and input](./text-and-input).
