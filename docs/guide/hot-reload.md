# Hot reload

<p class="lead">`cargo smudge dev` builds and starts your app, watches its sources, and patches the running window within about a second of each save. The state you pass to the window survives every patch, and so does most of the state the view itself created. Hot reload runs on x86-64 Linux.</p>

## Setting it up

Install the subcommand once:

```sh
cargo install cargo-smudge
```

A GitHub release also carries a static build for x86-64 Linux, `cargo-smudge-x86_64-unknown-linux-musl.tar.xz`, which runs on any distribution. Put the `cargo-smudge` inside it on your `PATH`, and cargo finds it as the `smudge` subcommand.

### A new app

`cargo smudge new` makes a package that hot-reloads from the start, and `cargo smudge init` makes one in an existing directory. Both run cargo's own `new` or `init` and pass `--name`, `--vcs` and `--edition` through to it:

```sh
cargo smudge new counter
cd counter
cargo smudge dev
```

The package depends on the `smudge` version of the `cargo-smudge` that made it, pinned exactly, since the two speak to each other while the app runs. Its manifest declares a `hot-reload` feature, which `cargo smudge dev` turns on by itself, so `cargo run` and release builds carry nothing of hot reload. Its `src/main.rs` is a counter whose count lives in the state the window keeps:

```rust
//! A counter: a button and a label over a count that lives in the window's
//! state, so `cargo smudge dev` keeps it across every hot patch.

use smudge::prelude::*;

/// What the window keeps while the app runs, across every hot patch.
struct State {
    /// How many times the button was pressed.
    count: RwSignal<u32>,
}

/// Builds the window's contents from the state. Edit it while
/// `cargo smudge dev` runs, and the window follows each save.
fn view(state: &State) -> AnyElement {
    let count = state.count.clone();
    let shown = state.count.clone();
    column()
        .padding(px(24.0))
        .gap(Size {
            width: 0.0,
            height: 12.0,
        })
        .align_items(Align::Center)
        .child(
            button("Count up")
                .primary()
                .on_press(move || count.update(|count| *count += 1)),
        )
        .child(label(move || format!("Clicked {} times", shown.get())))
        .into_any()
}

fn main() -> Result<(), PlatformError> {
    run_view(
        WindowOptions {
            title: String::from("Counter"),
            size: Size {
                width: 320.0,
                height: 160.0,
            },
            ..WindowOptions::default()
        },
        State {
            count: RwSignal::new(0),
        },
        view,
    )
}
```

`init` refuses to run where a `Cargo.toml`, a `src/main.rs` or a `main.rs` already exists, and names the file.

### An existing app

Give the app a feature that turns on Smudge's `hot-reload` feature, so release builds never carry it:

```toml
[features]
hot-reload = ["smudge/hot-reload"]
```

Start the app through `run_view`, as the template does, or through `run_view_with` when it needs a configure step. The view is a plain function that builds the interface from the state, and the state is the type the window keeps.

### Running it

Run the app under the builder instead of `cargo run`:

```sh
cargo smudge dev
```

A package that declares a `hot-reload` feature gets it on. A package whose feature has another name passes it with `--features NAME`. The command also takes `--bin NAME` or `--example NAME`, `-p NAME` and `--profile NAME`, as cargo does. The profile must keep the debug information in the objects and link without LTO: `split-debuginfo` must be `"off"` and `lto` must be off, which the default `dev` profile already is. The builder links with the linker cargo is configured to use, or `cc`. While it runs, type `r` and Enter to rebuild and restart the app, and `q` and Enter to quit. Each save prints one line that says whether the patch was applied, how long it took, and what it left out.

Without the feature, `run_view` and `run_view_with` are plain runs, and nothing of hot reload is compiled in. With the feature on but outside `cargo smudge dev`, the app runs as usual and never looks for a builder.

The `hot` example of the `smudge` crate is a short tour. Run it with `cargo smudge dev --example hot -p smudge`, then edit its helper, its handler and its timer, and type in its notes, while it runs.

## What a patch changes

A save replays the compile of the crates it touched, links the new code into a library, and hands the running app a table that sends every changed function to its new version. The app then remounts its root: it builds the interface again through the patched view, so new widgets, changed layouts and changed styles all show.

The crates of your workspace are patched, the app crate and its library crates alike, apart from the library crates the limits below name. Closures the interface stores, such as handlers, bindings and the builders of lists, run the patched code too, and so do the timers and hooks they register.

## What keeps its state

- **The state you passed to `run_view`.** It is built once by `main`, and every remount hands it to the patched view again.
- **Signals the view creates.** The remount after a patch gives each signal the new build creates the value of the old build's signal at the same place: under the same owner, created by the same source file, as the same nth signal of its type there. A line added above a signal keeps its value. Two signals of one type that swap places in one file swap their values. A list's rows go by their keys, so a row finds its values wherever the rows moved. Typed text, scroll positions and the rows of a keyed list carry over this way. A text field keeps only its text, not its caret, its selection or its undo history.
- **Statics.** A `pub static` of a library crate keeps its state across patches, because the patched code uses the running app's copy. A private static of the app crate starts from its initial value in the patched code, and the builder names it once per session. The app crate's own `thread_local!` values start fresh in the patched code too, without a note. Keep state in the state type, in signals, or in a library's `pub static`.

A signal's value is carried only when the builder could check that its type kept its layout. Its value starts fresh when the patch changed the type, when the type holds a signal, an owner, an effect, a timer, a channel or a trait object, or when the app first used the type after it started. A remount the app asks for itself, to reset its interface, starts every signal fresh.

## What needs a restart

Some changes cannot reach a running app. The builder refuses some of them and names the reason, others it restarts the app for, and the rest simply have no effect until you restart with `r`:

- **`main` and `WindowOptions`.** They ran before the first patch, so the patch applies but the change shows only after `r`.
- **`Cargo.toml` and the build's configuration.** The builder sees the change and rebuilds and restarts the app on its own.
- **Build scripts.** A build script that appears or goes away restarts the app on its own. An edit to a script or to a file it reads cannot be replayed, so the builder refuses every patch after it until you restart with `r`.
- **Crates from a registry.** They are neither watched nor patched.
- **A changed state type.** The running app holds its state in the old layout, so the builder refuses every patch, naming the type, until you undo the change or restart. That includes the types the state reaches through signals, `Rc`, `Box` and collections. A type the state holds only behind a trait object, such as a `Box<dyn Any>`, cannot be checked, so restart after changing one.
- **A changed signature of the view.** The builder refuses that patch, and the window keeps its view until a restart.
- **Values created in `main` or in the configure step.** The closures there run the patched code, but the values they captured keep the layout they were built with. Nothing checks them, so restart after changing their types.

## Threads and the window's hooks

A thread your app spawned keeps running the code it was started with, since nothing rebuilds it. Run each step of its loop through `hot::call`, and the next step runs the newest version of the body, with every capture the loop holds:

```rust
use smudge::hot;

fn work_once() {}

fn spawn_worker() {
    std::thread::spawn(|| loop {
        hot::call(work_once);
        std::thread::sleep(std::time::Duration::from_millis(100));
    });
}
```

A worker that should start over with the new code instead registers a hook. `on_hot_patch` runs after every patch, once the app remounted, on the thread that runs the window:

```rust
use smudge::hot;

fn restart_workers() {}

fn watch_patches() {
    hot::on_hot_patch(restart_workers);
}
```

A hook registered inside the view goes with the build and comes back with each remount, so it runs once per patch. Register long-lived hooks in the configure step of `run_view_with`.

The `on_frame` and `on_event` hooks of `WindowOptions` are not patched either. Their bodies can call `hot::call` the same way.

## Limits

- **Closures a patch created go stale on the next patch.** A closure the patched code made runs its own version, which the next patch does not replace. The remount after each patch rebuilds the ones the interface holds, so this only reaches closures kept outside the build.
- **A closure whose captures changed keeps its old code.** The patch line counts the functions it left out for changed captures and names the closures. Closures made before the patch keep their old code until their captures match again or the app restarts.
- **Closures that move inside a function.** A closure inserted before another in the same function, or removed from before it, renumbers the closures after it, and the patch leaves them out, which the patch line counts as renumbered closures: the ones the app made before keep their old code until a restart. Two closures that swap places, or one removed while another is added, keep the count, and when their captures have the same types they can exchange bodies. This too only reaches closures kept outside the build.
- **Library crates with private state.** A library crate with a writable private static or a thread-local of its own refuses every patch that replays it, for as long as it keeps them, so every edit to it needs a restart. Logging macros that cache their call sites create such a static at each call site.
- **Another crate's thread-local.** When the app's code reaches a thread-local of another crate through a generic or inlined function, the builder refuses every patch, and a restart does not change that while the code uses it.
- **Platforms.** Hot reload runs on x86-64 Linux. Elsewhere `cargo smudge dev` says so, and `run_view` runs the app as it is.
