# Shipping a small binary

<p class="lead">Two levers control how large a Smudge application ships: the profile the release build uses, and the features the application turns off. Neither changes how the application is written.</p>

## The release profile

The default release profile builds quickly and runs quickly. A profile tuned for size builds the same application at a little over half the size. Set it through environment variables, without touching any manifest:

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

On the `lists` example (see [Measured sizes](#measured-sizes)), these settings took the stripped binary from 11,566,856 to 6,463,600 bytes.

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
```

With this manifest, `cargo build --release` builds the defaults, `--no-default-features --features bundled-fonts` turns only `a11y` off, and `--no-default-features --features system-fonts` turns both off and finds the machine's fonts instead. A plain `--no-default-features` leaves no font source, so it fails to build on purpose: nothing could draw text.

### `a11y`

The accessibility projection that screen readers read. Without it, the build drops the stack that talks to the platform's accessibility APIs. The feature costs 3,807,672 bytes on the default release profile and 1,510,104 on the size profile, the largest single opt-out. What is lost: a screen reader can no longer see, read or operate the interface. Keyboard navigation still works, without narration. Leave it off only where no assistive tool will ever connect, such as a dedicated appliance build.

### `bundled-fonts`

The three embedded font faces that make text layout produce the same result on every machine. With the feature off, the application needs another font source, which today means the `system-fonts` feature.

Replacing the embedded faces with system font discovery saves 1,148,176 bytes on the default release profile and 1,173,008 on the size profile. What is lost: determinism. Layout wraps and breaks lines for whichever faces the host has, so text settles differently on machines with different fonts, and a machine without a face that covers the application's text draws missing glyphs.

System fonts are found on Linux, FreeBSD, Windows, Android and Apple platforms. Elsewhere, the web included, `system-fonts` finds nothing, so an application there keeps `bundled-fonts` on, and a build that relies on system fonts alone fails.

### `system-fonts`

Discovery for the fonts installed on the machine. Adding it on top of the bundled faces costs 124,656 bytes on the default release profile and 88,816 on the size profile, about a tenth of what the three embedded faces cost. Turn it on together with `bundled-fonts` when the application should use the fonts the user has installed, with the bundled faces as the fallback. Discovery only makes the installed fonts available: text keeps the bundled sans by default, and uses the host's interface font where its style names the `SystemUi` family, as [Text and input](./text-and-input#fonts) describes. Turn it on instead of `bundled-fonts` only when the host is known to carry suitable faces.

## Measured sizes

The numbers above are stripped release binaries of the `lists` example in the Smudge repository, drawn by the CPU renderer. They were measured on 2026-11-13 on Linux on an AMD Ryzen 7 7700, with the stable toolchain the repository pins.

| Feature set | Default release profile | Size profile |
|---|---|---|
| Defaults (`a11y` and `bundled-fonts` on) | 11,566,856 | 6,463,600 |
| `a11y` off | 7,759,184 | 4,953,496 |
| `bundled-fonts` off, `system-fonts` on | 10,418,680 | 5,290,592 |
| Both off, `system-fonts` on | 6,610,888 | 3,780,840 |
| Defaults and `system-fonts` | 11,691,512 | 6,552,416 |

To reproduce a row, build the example in a checkout of the repository with that row's features:

```sh
cargo build --release -p smudge --example lists --no-default-features --features window,bundled-fonts
strip -o target/lists target/release/examples/lists
stat -c %s target/lists
```

The defaults row drops `--no-default-features --features ...`. The release profile strips debug information but keeps the symbol table, so the binary is stripped before it is measured. For the size-profile column, set the environment variables from the top of this page, which include `strip = true`, and measure `target/release/examples/lists` directly. Here `--features` works because the build names the `smudge` package itself, with `-p smudge`.

See [Cargo features](/reference/features) for the complete feature tables of every crate.
