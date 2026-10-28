# Signals

<p class="lead">State lives in signals. A widget property given a closure binds to the signals that closure reads, and a write to one of them updates exactly the properties that depend on it.</p>

## Creating and using a signal

```rust
use smudge::prelude::*;

let count = RwSignal::new(0_u32);

count.set(5);                       // replace the value
count.update(|n| *n += 1);          // change it in place
let now = count.get();              // read a copy (needs Clone)
let len = count.with(|n| n.to_string().len()); // read by reference
```

`get`, `set`, `update` and `with` come from the traits in the prelude, so `use smudge::prelude::*` brings them into scope.

A signal handle is cheap to clone, and every clone names the same signal. Clone one for each closure that needs it, because each `move` closure takes its own handle:

```rust
let count = RwSignal::new(0_u32);
let press = count.clone();
let shown = count.clone();

let ui = column()
    .child(button("Add").on_press(move || press.update(|n| *n += 1)))
    .child(label(move || format!("{}", shown.get())));
```

`signal_local` splits a signal into a `ReadSignal` and a `WriteSignal` when you want to hand out only one side.

## Binding a property

Almost every widget property accepts either a plain value or a closure. A plain value never changes. A closure is a binding: Smudge runs it once when the widget mounts, records which signals it read, and runs it again after any of them changes.

```rust
let selected = RwSignal::new(false);
let shown = selected.clone();

let chip = panel()
    .size(Size { width: 80.0, height: 28.0 })
    .corner_radius(8.0)
    .background(move || {
        if shown.get() { Color::rgb(212, 255, 63) } else { Color::rgb(42, 46, 55) }.into()
    })
    .on_press(move || selected.update(|on| *on = !*on));
```

Color closures end in `.into()`, because a color property also accepts theme tokens and the closure returns that wider type.

What a binding costs depends on what it changes. A color change repaints the node and skips layout. A text or size change lays the affected part of the tree out again, then repaints what moved.

## When updates happen

Writes do not redraw anything by themselves. A write marks the bindings that read the signal and asks the window for a frame. At the start of that frame, the marked bindings run, in the order they were woken, and writes made while they run are picked up by the same frame. Then layout, paint and the present follow.

Several writes before a frame cost one frame:

```rust
first.set(String::from("Ada"));
last.set(String::from("Lovelace"));
// One frame later, a label that reads both shows "Ada Lovelace" once.
```

## Derived values with `Memo`

A `Memo` caches a value computed from other signals. It recomputes when read after a source changed, and it stops the update from spreading when the new value equals the old one.

```rust
let filter = RwSignal::new(String::new());
let needle = filter.clone();
let matches = Memo::new(move || {
    let needle = needle.get();
    (0..100_000)
        .filter(|index| format!("Item {index:05}").contains(needle.as_str()))
        .count()
});
let counted = matches.clone();
let summary = label(move || format!("{} matches", counted.get()));
```

Use a memo when several bindings read the same derived value, or when an expensive computation should only run when its inputs really changed.

## Effects

A `RenderEffect` runs a closure once, then again after any signal it read changes. Use it for side effects that are not widget properties, such as writing a log line or syncing a value to storage.

```rust
let saved = count.clone();
let _effect = RenderEffect::new(move || {
    println!("count is now {}", saved.get());
});
```

An effect created while the interface is being built belongs to the widget being built, and stops when that widget is removed. An effect created outside any owner lives as long as the handle you keep.

## Ownership and cleanup

Signals, memos and effects belong to an `Owner`. The widget tree creates an owner for every mounted subtree, so when `show` or `for_each` removes a widget, the signals and bindings created for it are freed with it. You do not clean anything up by hand.

## Context

`provide_context` makes a value available to everything built below the current owner, and `use_context` reads it. The text style uses this: `provide_text_style` sets the default font size and color for every label and input underneath.

```rust
use smudge::core::reactive::{provide_context, use_context};

#[derive(Clone)]
struct Settings { compact: bool }

provide_context(Settings { compact: true });
// Further down the tree:
let compact = use_context::<Settings>().map_or(false, |s| s.compact);
```

## Rules of thumb

- Read a signal inside the closure that should rerun, not outside it. A value read outside a closure is captured once and never updates.
- Keep bindings small. A closure per property updates less than one closure that rebuilds a whole subtree.
- Write signals from the thread that runs the window. Signals are single-threaded.
