# Accessibility

<p class="lead">On the desktop, Smudge describes the widget tree to the platform's accessibility API, so screen readers can find, read and operate the interface. Keyboard focus works everywhere.</p>

## What is exposed

Every widget that has a meaning reports a role, and a name or a value:

| Widget | Role | Name |
|---|---|---|
| `button` | Button | Its label text |
| `label` | Label | Its text, reported as the value |
| `text_input` | Text input | Its placeholder, with the current text as its value |
| `text_area` | Multi-line text input | Its placeholder, with the current text as its value |
| `scroll` | Scroll view | None by default |
| The window | Window | The window title |

Bounds, keyboard focus and the press action are reported too, and each frame sends only what changed since the last one.

The projection is on by default through the `a11y` feature of `smudge`, which forwards to `smudge-platform` and reaches the platform accessibility APIs on Windows, macOS and Linux. Building with the feature off drops the projection, and the screen reader sees nothing. It has no effect on the web: see [Known limits](/reference/limits).

## Naming your own elements

A panel you make interactive has no role or name of its own. Give it both:

```rust
use smudge::prelude::*;
use smudge::core::semantics::Role;

let close = panel()
    .size(Size { width: 28.0, height: 28.0 })
    .focusable()
    .activate_on_keys()
    .role(Role::Button)
    .accessible_label("Close")
    .on_press(|| println!("closed"));
```

`accessible_label` also accepts a closure, so a name can follow your state:

```rust
let muted = RwSignal::new(false);
let named = muted.clone();

let toggle = panel()
    .role(Role::Button)
    .accessible_label(move || if named.get() { "Unmute" } else { "Mute" }.to_owned());
```

The roles are `Window`, `Group`, `Button`, `Label`, `TextInput`, `MultilineTextInput`, `ScrollView`, `List` and `ListItem`.

## Keyboard

- Tab and Shift+Tab move focus through every `focusable()` node in tree order.
- `activate_on_keys()` makes Enter and Space trigger `on_press`, the way the button does.
- The `focused` style patch applies only while focus last moved by keyboard, so a focus ring shows for keyboard users and stays out of the way of mouse users.

```rust
let item = panel()
    .focusable()
    .activate_on_keys()
    .border(2.0)
    .border_color(Color::TRANSPARENT)
    .focused(|p| p.border_color(ColorToken::FOCUS_RING));
```

## Testing with a screen reader

Turn on your platform's screen reader, run one of the repository's examples, and move through it with Tab. The screen reader should announce each button and input by name as focus arrives.
