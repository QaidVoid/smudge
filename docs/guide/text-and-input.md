# Text and input

<p class="lead">Labels and the text input share one text system: shaped text, font fallback from the bundled fonts, and an editor that understands graphemes, words and input methods.</p>

## Labels

```rust
use smudge::prelude::*;

let heading = label("Account").text_size(22.0);
let hint = label("Changes save automatically").text_color(ColorToken::TEXT_MUTED);
```

| Method | Effect |
|---|---|
| `text_size(f32)` | Font size in logical pixels. Accepts a closure. |
| `text_color(...)` | Glyph color, a `Color`, a token or a closure. |
| `weight(FontWeight)` | Font weight, for example `FontWeight::BOLD`. |
| `family(FontFamily)` | Font family, named or generic. |

`weight` and `family` take `FontWeight` and `FontFamily` from the `parley` crate, version 0.12, which Smudge lays text out with. Add it to your dependencies to name them.

A label wraps at the width its parent gives it, and its height follows the number of lines.

### Default style

`provide_text_style` sets the size, color, family, weight and line height for every label and input built after it, below the current owner:

```rust
provide_text_style(TextStyle {
    size: 15.0,
    line_height: 1.4,
    ..current_text_style()
});
```

## Fonts

The default build embeds Noto Sans, Noto Sans Bold and Noto Sans Mono, so text looks the same on every machine and nothing is read from the system. Enable the `system-fonts` feature of `smudge-text` to add the fonts installed on the machine, for scripts and families the bundled fonts do not cover. It is not available on the web.

### Scripts without spaces

Chinese and Japanese wrap between characters out of the box. Thai, Lao, Khmer and Burmese write words without spaces between them, so finding a place to wrap needs a dictionary. Turn on the `complex-scripts` feature of `smudge` to compile the dictionaries in:

```toml
[dependencies]
smudge = { version = "0.1", features = ["complex-scripts"] }
```

It also makes double-click and word movement select whole words in Chinese and Japanese. It adds about 3 MB to every binary and about 5 percent to the layout time of other text, which is why it is off by default.

## Text input

```rust
let email = RwSignal::new(String::new());
let preview = email.clone();

let form = column()
    .gap(Size { width: 0.0, height: 8.0 })
    .child(
        text_input(email)
            .width(px(280.0))
            .height(px(32.0))
            .placeholder("you@example.com")
            .border(1.0)
            .border_color(ColorToken::BORDER),
    )
    .child(label(move || format!("Sending to {}", preview.get())));
```

`text_input(signal)` edits one line of text bound to a `RwSignal<String>`. Every edit writes the signal, and a write from elsewhere replaces the text and moves the caret to the end. `placeholder` shows a hint while the text is empty.

### Editing

| Input | Action |
|---|---|
| Left, Right | Move by grapheme, or by word with Ctrl or Alt held. Shift extends the selection. |
| Home, End | Jump to the start or end of the text. Shift extends the selection. |
| Backspace, Delete | Delete by grapheme, or by word with the word modifier. |
| Click, drag, double click | Place the caret, select, select a word. |
| Ctrl or Cmd with A, C, X, V | Select all, copy, cut, paste. |
| Ctrl or Cmd with Z | Undo, a typed word at a time. Add Shift to redo, or use Ctrl with Y outside macOS. |

The caret is its own node, so a blink repaints the caret's thin rectangle and nothing else.

### Input methods

While an input method composes, for example typing pinyin for Chinese or romaji for Japanese, the composition is drawn underlined in the field and does not reach the signal. The candidate window is placed at the caret. Committing writes the chosen text, and Escape or a click elsewhere drops the composition and leaves the value untouched.

### Clipboard

Copy, cut and paste use the system clipboard on the desktop. On Wayland that needs a compositor with the data-control protocol, or Xwayland; without either, the input falls back to an in-memory clipboard inside the application. The web build has no system clipboard yet: copy and paste work only inside the application.

## Pointer and keyboard events

Any element can handle input:

| Handler | Called with |
|---|---|
| `on_press(f)` | A completed press: pointer down and up over the node, or Enter or Space when `activate_on_keys` is set. |
| `on_hover(f)` | `true` when the pointer enters, `false` when it leaves. |
| `on_pointer_down(f)`, `on_pointer_move(f)`, `on_pointer_up(f)` | The position relative to the node's top left corner. |
| `on_press_outside(f)` | A press anywhere outside the node, for closing popovers. |
| `on_wheel(f)` | The scroll delta. |
| `on_key(f)` | A `KeyEvent`. Return `Handled::Yes` to stop it, `Handled::No` to let it bubble. |
| `on_focus(f)` | `true` when the node gains keyboard focus, `false` when it loses it. |

`focusable()` puts a node in the Tab order. A node that captures the pointer with a press keeps receiving moves and the release even when the pointer leaves it, and positions keep following the node if it scrolls.

```rust
use smudge::core::input::{Handled, Key};

let canvas = panel()
    .size(Size { width: 320.0, height: 200.0 })
    .focusable()
    .on_pointer_down(|at| println!("pressed at {}, {}", at.x, at.y))
    .on_key(|event| {
        if event.pressed && event.key == Key::Escape {
            println!("cancel");
            return Handled::Yes;
        }
        Handled::No
    });
```
