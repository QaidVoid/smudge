# Dynamic content

<p class="lead">The tree is built once, but parts of it can come and go. `show` mounts a subtree while a condition holds, `for_each` keeps one child per item of a list, and `virtual_list` handles lists too long to mount at all.</p>

## Conditional content with `show`

```rust
use smudge::prelude::*;

let signed_in = RwSignal::new(false);
let when = signed_in.clone();
let toggle = signed_in.clone();

let page = column()
    .child(button("Sign in or out").on_press(move || toggle.update(|on| *on = !*on)))
    .child(show(move || when.get(), || label("Welcome back")));
```

`show(when, build)` mounts what `build` returns while `when` is true and removes it while it is false. Only `when` is a binding. `build` runs once each time the child mounts, under an owner of its own, so any signal it creates lives exactly as long as the child and is freed when the child is removed.

To hide something without unmounting it, which keeps its state and is cheaper to toggle often, bind `hidden` instead:

```rust
let details = column().hidden(move || !expanded.get());
```

## Lists with `for_each`

```rust
#[derive(Clone)]
struct Task { id: u32, title: String }

let tasks = RwSignal::new(vec![
    Task { id: 1, title: String::from("Write the docs") },
    Task { id: 2, title: String::from("Ship v0.1") },
]);
let listed = tasks.clone();

let list = column().child(for_each(
    move || listed.get(),
    |task| task.id,
    |task| label(task.title).padding(px(6.0)),
));
```

`for_each(items, key, row)` keeps one child per key:

- A key that appears mounts a new child, built by `row`.
- A key that disappears removes that child and frees everything it created.
- A key that moves keeps its child, which is reordered, not rebuilt, and keeps its state.

Each row is built once per key. If an item's content can change while its key stays, the row must read that content from a signal. A common shape is a list of keys plus a signal per item:

```rust
#[derive(Clone)]
struct Row { id: u32, done: RwSignal<bool> }

let row = |item: Row| {
    let done = item.done.clone();
    label(move || if done.get() { "done" } else { "open" }.to_owned())
};
```

Keys must be unique within the list. Debug builds check this.

The returned slot manages its own children, so do not add other children to the same element beside it. Wrap it in a `column` or `row` as above.

## Very long lists

`for_each` mounts every item. For thousands of rows of one height, use [`virtual_list`](./widgets#virtual-list), which mounts only the rows near the viewport.
