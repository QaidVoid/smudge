# Accessibility

<p class="lead">On the desktop, Smudge describes the widget tree to the platform's accessibility API, so screen readers can find, read and operate the interface. Keyboard focus works everywhere.</p>

## What is exposed

Every widget that has a meaning reports a role, and a name or a value:

| Widget | Role | Name |
|---|---|---|
| `button` | Button | Its label text |
| `label`, `rich_text` | Label | Its text, reported as the value and line by line, so a reader can move through it by character, word and line |
| `text_input` | Text input | Its placeholder, with the current text as its value, and line by line with its caret and selection |
| `text_area` | Multi-line text input | Its placeholder, with the current text as its value, and line by line with its caret and selection |
| `scroll` | Scroll view | None by default |
| `virtual_list`, `virtual_list_keyed` | Scroll view holding a list, with how many rows the list has | None by default. Each mounted row is a list item with its place among all the rows. |
| `virtual_column_keyed` | Scroll view | None by default. The rows report only their own content, with no list around them. |
| The window | Window | The window title |

A link inside a `rich_text` paragraph, and so inside markdown, is a link of its own to a screen reader, named by its text, which the reader can move to and press to run the link's handler. Neighbouring spans that share one handler are one link.

A selection made in a [selection scope](./text-and-input#selecting-text) reaches a reader block by block: each label or rich text the selection covers reports the part of its text that is selected. Blocks a virtual list has not mounted report nothing. On Linux the screen reader hears no event as such a selection changes, because the blocks never hold the focus, though it can still ask what is selected.

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

The roles are `Window`, `Group`, `Button`, `Label`, `TextInput`, `MultilineTextInput`, `ScrollView`, `List`, `ListItem`, `Splitter`, `CheckBox`, `Switch`, `ProgressBar`, `Link` and `Heading`.

A heading reports its level with `accessible_level`, from 1 for the top level down, so a screen reader can move between headings. A heading or a link with no `accessible_label` is named by its text. The markdown view reports its headings this way, and a split's divider reports the share its ratio asks the first pane to take as a value from 0 to 100.

A checkbox or a switch of your own reports whether it is checked, and a progress bar how far it has come. Bind both to the state that draws them:

```rust
use smudge::core::semantics::{Checked, ValueRange};

let on = RwSignal::new(false);
let (read, flip) = (on.clone(), on.clone());
let switch = panel()
    .focusable()
    .activate_on_keys()
    .role(Role::Switch)
    .accessible_label("Wi-Fi")
    .accessible_checked(move || Checked::from(read.get()))
    .on_press(move || flip.update(|on| *on = !*on));

let done = RwSignal::new(0.4);
let progress = panel()
    .role(Role::ProgressBar)
    .accessible_label("Upload")
    .accessible_range(move || ValueRange { value: done.get(), min: 0.0, max: 1.0 });
```

A list of your own reports its rows the same way the virtual lists do: give the list `Role::List` and `accessible_set_size(count)`, and each row `Role::ListItem` and `accessible_position(index)`, counted from zero, so a screen reader announces place 39 as "item 40 of 3,000". Both accept a closure, for a count that grows or a row whose place moves.

A screen reader toggles a checkbox or a switch by pressing it, so its `on_press` handler is what flips the state, and one without a handler reads as read only. A progress bar without a range reads as busy, with no known progress, and so does one whose range is not finite or runs backwards. `Checked::Mixed` reports a checkbox that is neither checked nor unchecked, such as one over a list whose boxes are partly checked.

## Announcing what no widget shows

Some news has no widget to read it from, such as a save that finished. An `Announcer` has a screen reader speak it. Attach it to any element of the view, then announce from anywhere, such as a press handler:

```rust
use smudge::core::semantics::{Announcer, Politeness};

let announcer = Announcer::new();
let saved = announcer.clone();
let save = button("Save")
    .announcer(&announcer)
    .on_press(move || saved.announce("Saved", Politeness::Polite));
```

`Politeness::Polite` waits until the reader finishes what it is saying, and `Politeness::Assertive` cuts in, for news that cannot wait. Each message is spoken once, and the messages of one politeness in the order they were announced, even when several arrive in one frame, and a message announced while the element is hidden is dropped. The markdown view announces a streamed reply a block at a time with `announce_blocks`, as the [markdown guide](./markdown#showing-a-document) shows.

## Keyboard

- Tab and Shift+Tab move focus through every `focusable()` node in tree order.
- `activate_on_keys()` makes Enter and Space trigger `on_press`, the way the button does.
- The `focused` style patch applies only while focus last moved by keyboard, so a focus ring shows for keyboard users and stays out of the way of mouse users.
- A press on a node takes the focus when the node is focusable and clears it otherwise. `keeps_focus()` makes a press leave the focus where it is, for a toolbar button that acts on the focused field or a scrollbar of your own, so typing goes on where it was.

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
