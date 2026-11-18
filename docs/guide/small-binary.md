# Shipping a small binary

<p class="lead">Two levers control how large a Smudge application ships: the profile the release build uses, and the features the application turns off. Neither changes how the application is written.</p>

## The release profile

The default release profile builds quickly and runs quickly. A profile tuned for size builds the same application at under three fifths of the size. Set it through environment variables, without touching any manifest:

```sh
CARGO_PROFILE_RELEASE_OPT_LEVEL=z \
CARGO_PROFILE_RELEASE_LTO=fat \
CARGO_PROFILE_RELEASE_PANIC=abort \
CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
CARGO_PROFILE_RELEASE_STRIP=true \
cargo build --release
```

Or put the same settings in the workspace's `Cargo.toml`:

```toml
[profile.release]
opt-level = "z"
lto = "fat"
panic = "abort"
codegen-units = 1
strip = true
```

| Setting | What it does | What it costs |
|---|---|---|
| `opt-level = "z"` | Optimizes for size instead of speed. | The generated code runs slower than at the default `opt-level = 3`. |
| `lto = "fat"` | Optimizes across crate boundaries at link time, which lets the linker drop code no call path reaches. | Builds take longer. |
| `codegen-units = 1` | Hands the optimizer the whole build in one unit instead of sixteen, so it sees more of the program at once. | Builds take longer. |
| `panic = "abort"` | Ends the process at once when a panic happens, and drops the unwinding landing pads from the application's own code. | A panic cannot be recovered from, and destructors on the panicking call stack do not run. |
| `strip = true` | Removes symbol tables and debug information from the binary. | Backtraces lose their symbol names. |

On the `lists` example (see [Measured sizes](#measured-sizes)), these settings took the stripped binary from 12,035,352 to 7,043,688 bytes.

## The opt-out features

Besides `window`, two features are on by default, and both exist mostly so an application can leave them out. They first ship in Smudge 0.3. The feature set lives in the application's dependency on `smudge`, because a command line flag can turn a dependency's features on but cannot turn its default features off. The four configurations are:

```toml
# The defaults.
smudge = { version = "0.3" }
```

```toml
# a11y off.
smudge = { version = "0.3", default-features = false, features = ["window", "bundled-fonts"] }
```

```toml
# bundled-fonts off, system-fonts on.
smudge = { version = "0.3", default-features = false, features = ["window", "a11y", "system-fonts"] }
```

```toml
# Both off, system-fonts on.
smudge = { version = "0.3", default-features = false, features = ["window", "system-fonts"] }
```

An application that ships its own fonts can use them as its only source, with `app-fonts` in place of `system-fonts` in either of the last two, as [`app-fonts`](#app-fonts) describes:

```toml
# bundled-fonts off, the application's own fonts on.
smudge = { version = "0.3", default-features = false, features = ["window", "a11y", "app-fonts"] }
```

A headless application, one that drives `App` without a window, leaves out `window` and keeps a font feature:

```toml
smudge = { version = "0.3", default-features = false, features = ["bundled-fonts"] }
```

An application that wants these switchable at build time forwards the features through its own:

```toml
[dependencies]
smudge = { version = "0.3", default-features = false, features = ["window"] }

[features]
default = ["a11y", "bundled-fonts"]
a11y = ["smudge/a11y"]
bundled-fonts = ["smudge/bundled-fonts"]
system-fonts = ["smudge/system-fonts"]
app-fonts = ["smudge/app-fonts"]
```

With this manifest, `cargo build --release` builds the defaults, `--no-default-features --features bundled-fonts` turns only `a11y` off, `--no-default-features --features system-fonts` turns both off and finds the machine's fonts instead, and `--no-default-features --features app-fonts` turns both off and draws with the fonts the application registers. A plain `--no-default-features` leaves no font source, so it fails to build on purpose: nothing could draw text.

### `a11y`

The accessibility projection that screen readers read. Without it, the build drops the stack that talks to the platform's accessibility APIs. The feature costs 3,802,320 bytes on the default release profile and 1,509,168 on the size profile, the largest single opt-out. What is lost: a screen reader can no longer see, read or operate the interface. Keyboard navigation still works, without narration. Leave it off only where no assistive tool will ever connect, such as a dedicated appliance build.

### `bundled-fonts`

The five embedded font faces, Noto Sans in regular, bold, italic and bold italic, and Noto Sans Mono, that make text layout produce the same result on every machine. With the feature off, the application needs another font source: the machine's fonts through `system-fonts`, or its own through `app-fonts`.

Replacing the embedded faces with system font discovery saves 1,827,728 bytes on the default release profile and 1,848,720 on the size profile. What is lost: determinism. Layout wraps and breaks lines for whichever faces the host has, so text settles differently on machines with different fonts, and a machine without a face that covers the application's text draws missing glyphs.

System fonts are found on Linux, FreeBSD, Windows, Android and Apple platforms. Elsewhere, the web included, `system-fonts` finds nothing, so an application there keeps `bundled-fonts` or `app-fonts` on, and a build that relies on system fonts alone fails.

### `system-fonts`

Discovery for the fonts installed on the machine. Adding it on top of the bundled faces costs 126,704 bytes on the default release profile and 88,688 on the size profile, about a fifteenth of what the five embedded faces cost on the default release profile and a twenty-second on the size profile. Turn it on together with `bundled-fonts` when the application should use the fonts the user has installed, with the bundled faces as the fallback. Discovery only makes the installed fonts available: text keeps the bundled sans by default, and uses the host's interface font where its style names the `SystemUi` family, as [Text and input](./text-and-input#fonts) describes. Turn it on instead of `bundled-fonts` only when the host is known to carry suitable faces.

### `app-fonts`

Makes the fonts the application registers itself, as [Text and input](./text-and-input#your-own-fonts) describes, its font source, so the build needs neither the bundled faces nor the machine's. The feature adds no code: the binary carries the font files the application embeds instead of the five bundled faces, which suits the web best, where it is the only alternative to them.

The first family the application registers becomes the font of the sans serif, system UI and monospace families, so text in the default style and monospace text draw in it without naming it. Text draws nothing until a font is registered, so register the fonts before the window opens. What is lost: the bundled faces' coverage. Characters the application's fonts have no glyph for draw as missing.

## Measured sizes

The numbers above are stripped release binaries of the `lists` example in the Smudge repository, drawn by the CPU renderer. They were measured on 2026-11-17 on Linux on an AMD Ryzen 7 7700, with Rust 1.99.0.

| Feature set | Default release profile | Size profile |
|---|---|---|
| Defaults (`a11y` and `bundled-fonts` on) | 12,035,352 | 7,043,688 |
| `a11y` off | 8,233,032 | 5,534,520 |
| `bundled-fonts` off, `system-fonts` on | 10,207,624 | 5,194,968 |
| Both off, `system-fonts` on | 6,404,824 | 3,686,232 |
| Defaults and `system-fonts` | 12,162,056 | 7,132,376 |

To reproduce a row, build the example in a checkout of the repository with that row's features:

```sh
cargo build --release -p smudge --example lists --no-default-features --features window,bundled-fonts
strip -o target/lists target/release/examples/lists
stat -c %s target/lists
```

The defaults row drops `--no-default-features --features ...`. The release profile strips debug information but keeps the symbol table, so the binary is stripped before it is measured. For the size-profile column, set the environment variables from the top of this page, which include `strip = true`, and measure `target/release/examples/lists` directly. Here `--features` works because the build names the `smudge` package itself, with `-p smudge`.

## Where the bytes go

The defaults row splits into these areas, in bytes of the stripped binary. Each symbol counts toward the area of the crate its name belongs to, and generic code compiled for a crate counts toward that crate. The clipboard row also takes Smudge's own Wayland clipboard client and the data-control protocol code it uses. The unwind tables, relocations and unnamed data are read from the binary's sections.

| Area | Default release profile | Size profile |
|---|---|---|
| Accessibility: the projection and the bus client screen readers talk to | 2,785,145 | 1,106,273 |
| The five bundled font faces | 1,928,320 | 1,928,320 |
| Windowing: the window, its Wayland and X11 clients and software presentation | 1,600,704 | 807,651 |
| Text shaping, font reading and line breaking | 1,559,244 | 847,427 |
| Unwind tables | 1,269,120 | 613,656 |
| Smudge itself, with its layout and rasterizer | 840,616 | 557,054 |
| The standard library | 571,384 | 397,386 |
| Relocations and dynamic linking | 496,397 | 235,025 |
| Clipboard | 117,896 | 60,261 |
| Everything else: string literals, unnamed constants, C symbols and headers | 866,526 | 490,635 |
| Total | 12,035,352 | 7,043,688 |

The unwind tables follow the application's panic setting and the toolchain's standard library, not anything Smudge chooses. The clipboard costs more than its row shows, because its share of the Wayland and X11 client code, the unwind tables and the relocations is counted in those rows. Measured on 2026-11-13, removing it altogether saved 368,896 bytes on the default release profile and 200,704 on the size profile. Of that, the Wayland path accounts for 158,336 bytes on the default release profile and 72,448 on the size profile, and the X11 path for the rest.

The font faces are embedded byte for byte, so they cost the same on both profiles. Their glyph names are already removed, because nothing reads them. Measured on 2026-11-13, trimming the three faces bundled then down to fewer characters would have saved about 330 KB of outlines, but it would drop the phonetic letters, the symbols and the alternate glyphs they carry, so the faces keep their full coverage.

See [Cargo features](/reference/features) for the complete feature tables of every crate.
