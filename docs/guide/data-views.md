# Data views

<p class="lead">Some content outgrows a column of children. A form aligns labels with fields across rows, a settings page folds into sections, a file picker walks a tree, and a table sorts and selects a hundred thousand rows. The views here hold the mechanics, and the data stays yours.</p>

## Grid layout

`grid(columns)` is a panel laid out on a grid. Each track sizes one column, and children flow into the cells in order, two per row here:

```rust
use smudge::core::style::Track;
use smudge::prelude::*;

let name = RwSignal::new(String::new());
let email = RwSignal::new(String::new());
let name_field = name.clone();
let email_field = email.clone();

let form = grid([Track::Px(110.0), Track::Fr(1.0)])
    .gap(Size { width: 12.0, height: 8.0 })
    .child(label("Name"))
    .child(text_input(name_field))
    .child(label("Email"))
    .child(text_input(email_field));
```

A track is one sizing function. `Track::Px` fixes the column, `Track::Fr` shares the space that is left among the fr tracks, and `Auto`, `MinContent` and `MaxContent` fit the content. `Track::MinMax` pairs a minimum with a maximum, which is how a column stays useful across widths:

```rust
let content = grid([
    Track::MinMax(TrackSize::Px(160.0), TrackSize::Fr(1.0)),
    Track::Fr(3.0),
]);
```

A child spans more than one column or row with `column_span` and `row_span`. Rows a template does not name size to their content, and both templates are bindings, so a template can come from a signal like any layout property:

```rust
let form = grid([Track::Px(110.0), Track::Fr(1.0)])
    .child(label("Account").column_span(2))
    .child(label("Name"))
    .child(text_input(name));
```

## Collapsible sections

`collapsible(open, header, content)` folds its content away under a header. The header is a row with a disclosure triangle in front of the header you give it, and a press, Enter or Space toggles `open`; Right opens and Left closes. While `open` holds, the content mounts below the header, and a closed section holds no content at all, so it costs nothing:

```rust
let advanced = RwSignal::new(false);
let section = collapsible(
    advanced,
    label("Advanced"),
    || column()
        .child(label("Expert settings live here"))
        .child(label("Each section folds on its own")),
);
```

A screen reader hears the header as a button whose expanded state follows `open`.

## Trees

`tree_view(roots, children, key, estimate, row)` shows a tree of nodes as rows in a virtual list, so a tree of a hundred thousand nodes mounts only the rows near the viewport. `row` builds a row from its node and its depth, which an application uses to indent:

```rust
use std::rc::Rc;
use smudge::prelude::*;

#[derive(Clone)]
struct Node {
    id: u64,
    name: String,
    children: Vec<Node>,
}

let tree: RwSignal<Rc<[Node]>> = RwSignal::new(Rc::from(vec![Node {
    id: 1,
    name: String::from("src"),
    children: vec![Node {
        id: 2,
        name: String::from("main.rs"),
        children: Vec::new(),
    }],
}]));

let roots = tree.clone();
let view = tree_view(
    move || roots.get().clone(),
    |node| Rc::from(node.children.as_slice()),
    |node| node.id,
    24.0,
    |node, depth| label(format!("{}{}", "  ".repeat(depth), node.name)),
)
.height(px(320.0));
```

The nodes of a closed branch are never asked for: `children` runs for a node only while the node is open, so a lazy model loads a branch when the user unfolds it and pays for nothing before that. The rows come out in depth-first order, and the flattening is a memo, so a fold re-walks the tree once.

The view keeps its own state and hands it back: `view.expanded()` holds the keys of the open nodes, and `view.selected()` the key of the picked one.

```rust
let opened = view.expanded();
let picked = view.selected();
```

A press on a row picks it. Right opens a closed row or moves to its first child, Left closes an open row or moves to its parent, and Up, Down, Home and End walk the mounted rows. The whole tree is one Tab stop, which lands on the row with the roving focus, or the picked one, or the first.

## Tables

`table(columns, rows, key, cell)` shows rows of data as a scrollable table under a header. Each `Column` names its title, its starting width, its minimum width, its alignment, and whether pressing its header may sort:

```rust
use std::rc::Rc;
use smudge::prelude::*;

#[derive(Clone)]
struct File {
    name: String,
    size: u64,
}

let files: RwSignal<Rc<[File]>> = RwSignal::new(Rc::from(vec![
    File { name: String::from("main.rs"), size: 4096 },
    File { name: String::from("guide.md"), size: 512 },
]));

let listed = files.clone();
let view = table(
    [
        Column::new("Name", 220.0, 80.0, Align::Start, true),
        Column::new("Size", 90.0, 60.0, Align::End, false),
    ],
    move || listed.get().clone(),
    |file| file.name.clone(),
    |file, column_index| {
        if column_index == 0 {
            label(file.name.clone()).max_lines(Some(1))
        } else {
            label(format!("{} kB", file.size / 1024))
        }
    },
)
.height(px(400.0));
```

The rows are keyed exactly as a keyed virtual list keys them, and only the rows near the viewport mount, so the table scrolls a hundred thousand rows as easily as twenty. Every cell truncates to one line, so all rows measure the same.

A drag on a header cell's right edge resizes its column, never below its minimum. Pressing a sortable header cycles its sort through ascending, descending and none. The table never reorders the rows: it writes the column and the direction to `view.sort()`, and the application sorts its rows to match before the next frame. `view.selection()` holds the keys of the selected rows, and `view.widths()` holds the widths the user dragged, which an application can persist:

```rust
let by = view.sort();
let picked = view.selection();
let widths = view.widths();
```

A press selects its row. With `.multiple(true)`, Ctrl or Cmd adds or removes one row, and Shift extends the selection from the last row a press or the keyboard started from. Without it, a press replaces the selection with its row.

The keyboard moves the focus: Up and Down walk the rows, Page Up and Page Down a page, Home and End the ends, and Space toggles the focused row, with Shift extending the selection as it moves. A focused row outside the viewport is revealed, and the whole table is one Tab stop.
