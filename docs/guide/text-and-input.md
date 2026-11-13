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
| `max_lines(Option<u32>)` | The most lines shown, `Some(n)` or `None`; the last one ends in an ellipsis. Accepts a closure. |

`weight` takes `FontWeight`, which the prelude re-exports. `family` takes `FontFamily`, which comes from `smudge_text`: `use smudge::text::FontFamily;`. No other crate is needed to name them.

A label wraps at the width its parent gives it, and its height follows the number of lines. A label with `max_lines` keeps its first lines and ends the last one with an ellipsis, cut back so the whole fits the width, and reports the kept lines' height. The accessible value keeps the full text, so a screen reader reads what was cut. A width below the ellipsis's own advance leaves it standing alone and past the width.

```rust
use smudge::prelude::*;

let name = label("Long department name").max_lines(Some(2));
let open = label("Open-ended").max_lines(None);
```

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

The default build embeds Noto Sans, Noto Sans Bold and Noto Sans Mono, so text looks the same on every machine and nothing is read from the system. Enable the `system-fonts` feature of `smudge` to add the fonts installed on the machine, for scripts and families the bundled fonts do not cover. It is not available on the web. The generic families resolve to the system's choices first and keep the bundled faces as the fallback, so a machine without usable fonts still lays text out.

### Scripts without spaces

Chinese and Japanese wrap between characters out of the box. Thai, Lao, Khmer and Burmese write words without spaces between them, so finding a place to wrap needs a dictionary. Turn on the `complex-scripts` feature of `smudge` to compile the dictionaries in:

```toml
[dependencies]
smudge = { version = "0.2", features = ["complex-scripts"] }
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

A single-line input flattens line breaks into spaces, so multi-line clipboard content, an input method commit or a write to the signal stays one line, and the input writes that line back to the signal. A paste into a text area keeps the breaks.

## Text area

```rust
let draft = RwSignal::new(String::from("Dear reader,\n"));

let field = text_area(draft.clone())
    .width(px(360.0))
    .min_lines(4)
    .placeholder("Write your reply")
    .border(1.0)
    .border_color(ColorToken::BORDER);
```

`text_area(signal)` edits multi-line text bound to a `RwSignal<String>`, through the same editing machinery as the single-line input: the same undo, IME composition, clipboard and pointer behavior, now across paragraphs. Enter and Shift+Enter insert a line break and are handled, so they do not reach a parent's key handler; Enter with Ctrl, Cmd or Alt bubbles, so a parent can make it a submit shortcut. Paste keeps line breaks, and Windows line breaks become plain ones.

The area is at least one line tall, and `min_lines(n)` raises that floor to `n` lines of the ambient text style, so a field of a few lines holds its size while the text is short and grows with it. Without an explicit height the area measures its whole text; with one it clips and scrolls, and the wheel scrolls it without moving the caret. A wheel the area cannot use, because its text fits or is already scrolled to its end in that direction, passes on to the page around it.

### Area keys

| Input | Action |
|---|---|
| Up, Down | Move one visual line, keeping the caret's horizontal position across lines. Shift extends the selection. |
| Page Up, Page Down | Move several lines at a time. |
| Enter | Insert a line break. |
| Home, End | Move to the start or end of the visual line. Shift extends the selection. |
| Ctrl or Cmd with Home, End | Move to the start or end of the whole text. |
| Left, Right, Backspace, Delete | As in the single-line input. Left and Right cross line breaks. |
| Ctrl or Cmd with A, C, X, V, Z | Select all, copy, cut, paste, undo. |

Text wider than the area wraps at the width its parent gives it. When the text is taller than the area, the view scrolls just enough to keep the caret visible after every edit and caret move.

## Pointer and keyboard events

Any element can handle input:

| Handler | Called with |
|---|---|
| `on_press(f)` | A completed press: pointer down and up over the node, or Enter or Space when `activate_on_keys` is set. |
| `on_hover(f)` | `true` when the pointer enters, `false` when it leaves. |
| `on_pointer_down(f)`, `on_pointer_move(f)`, `on_pointer_up(f)` | The position relative to the node's top left corner. |
| `on_press_outside(f)` | A press anywhere outside the node, for closing popovers. |
| `on_wheel(f)` | The scroll delta. Return `Handled::Yes` to take it, `Handled::No` to let it bubble to an outer handler. |
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

### Keyboard focus

Three builders cover the focus an application drives itself:

- `autofocus()` gives the node the keyboard focus on the first frame it is laid out visible, so a form's first field is ready to type into, and a field inside a hidden panel takes the focus when the panel shows.
- `trap_focus()` keeps Tab and Shift+Tab inside the node's subtree, which is how a modal dialog holds the keyboard while it is up. While a trap is active, a key pressed with the focus outside it, or with no focus, starts at the trap, so the dialog hears it first. The trap mounted last wins, and removing it restores the trap mounted before it and gives the focus back to the node that held it when the trap mounted.
- A key that no node handles bubbles to the application's top element, so an `on_key` there is the application-wide shortcut handler, for example an Escape that closes a dialog.

```rust
use smudge::core::input::{Handled, Key};

let dialog = panel()
    .autofocus()
    .trap_focus()
    .on_key(|event| {
        if event.key == Key::Escape {
            return Handled::Yes;
        }
        Handled::No
    });
```
