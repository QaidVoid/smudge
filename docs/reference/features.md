# Cargo features

<p class="lead">The defaults give a desktop window with the CPU renderer, screen reader support and embedded fonts. Everything else is opt-in. [Shipping a small binary](/guide/small-binary) shows what each default costs and how to build without it.</p>

## `smudge`

| Feature | Default | What it adds |
|---|---|---|
| `window` | yes | Windows, input and presentation through `smudge-platform`, and `run`. A headless build that only uses `App` leaves it out and keeps a font feature: `default-features = false, features = ["bundled-fonts"]`. |
| `a11y` | yes | The accessibility projection that screen readers read, forwarded to the `a11y` feature of `smudge-platform`. Implies `window`. It has no effect on the web. Building without it drops the whole accessibility stack. New in 0.3: before, `window` alone brought it in, so a build that lists features after `default-features = false` must now name `a11y` to keep screen reader support. |
| `bundled-fonts` | yes | Embeds Noto Sans, Noto Sans Bold and Noto Sans Mono, so text never depends on the machine. Forwarded to the same feature of `smudge-text` and `smudge-widgets`. On wasm this is the only font source. |
| `gpu` | no | The GPU renderer and its window path, with the Vulkan, Metal and DX12 backends. Implies `window`. Windows still start on the CPU renderer unless `WindowOptions::renderer` or `SMUDGE_RENDERER` asks for `Auto` or `Gpu`. New in 0.3: before, `Auto` was the default with this feature. |
| `complex-scripts` | no | Word and line break dictionaries for Chinese, Japanese, Thai, Lao, Khmer and Burmese. About 3 MB per binary and about 5 percent slower layout of other text. |
| `system-fonts` | no | Also finds the fonts installed on the machine, on Linux, FreeBSD, Windows, Android and Apple platforms. The same feature of `smudge-text`, forwarded, so applications can turn it on without adding `smudge-text` themselves. |
| `hot-reload` | no | Hot reload for applications started with `run_view` or `run_view_with`, on Linux. Under a patch builder, the application applies each patch the builder sends and remounts its root through the patched view, keeping the state passed to the entry point. Implies `window`. Elsewhere, and without a builder, both are plain runs. Meant for development builds. With it off, no patching code is compiled in. |

```toml
[dependencies]
smudge = { version = "0.2", features = ["gpu"] }
```

## `smudge-widgets`

| Feature | Default | What it adds |
|---|---|---|
| `bundled-fonts` | yes | Embeds the fonts, as the same feature of `smudge-text` does, so widgets that lay out text find a font even when no other dependency provides one. |

## `smudge-text`

| Feature | Default | What it adds |
|---|---|---|
| `bundled-fonts` | yes | Embeds Noto Sans, Noto Sans Bold and Noto Sans Mono, so text never depends on the machine. |
| `system-fonts` | no | Also finds the fonts installed on the machine, on Linux, FreeBSD, Windows, Android and Apple platforms. On Linux, fontconfig is loaded at run time, so building needs no fontconfig development files. Elsewhere, the web included, it finds nothing, so it does not count as a font source there. |
| `complex-scripts` | no | The break dictionaries, as above. |

An application that depends on `smudge` turns these on through the `smudge` features of the same name. It can also turn the bundled fonts off and rely on the system's faces:

```toml
[dependencies]
smudge = { version = "0.3", default-features = false, features = ["window", "a11y", "system-fonts"] }
```

One font source must reach `smudge-text`, through whichever dependency carries the feature. Only `bundled-fonts` and `system-fonts` count as sources, so system font discovery is turned on through `system-fonts`, not through the font crates underneath. A build in which none does fails to compile, because nothing could draw text. The check sees cargo's unified features, so a `smudge` with its defaults off builds when another dependency in the graph enables a font feature on `smudge-text`.

A library built on Smudge leaves the font choice to the application the way `smudge-widgets` does: it depends on Smudge with `default-features = false` and has its own default `bundled-fonts` feature that forwards to Smudge's, which an application turns off when it brings its own font source:

```toml
[dependencies]
smudge = { version = "0.3", default-features = false }

[features]
default = ["bundled-fonts"]
bundled-fonts = ["smudge/bundled-fonts"]
```

## `smudge-platform`

| Feature | Default | What it adds |
|---|---|---|
| `a11y` | yes | The accessibility projection for screen readers on the desktop. It has no effect on the web. |
| `gpu` | no | The GPU renderer. The `smudge` feature of the same name turns this on. |
| `hot-reload` | no | The hot reload receiver behind `run_view` and `run_view_with`, on Linux. The `smudge` feature of the same name turns this on. |

## Runtime switches

| Variable | Values | Effect |
|---|---|---|
| `SMUDGE_RENDERER` | `cpu`, `gpu`, `auto` | Overrides `WindowOptions::renderer` when the window opens. |
| `SMUDGE_HOT_SOCKET` | a socket path | Set by a patch builder. With the `hot-reload` feature on Linux, an application started with `run_view` or `run_view_with` connects to it and applies the patches it receives. |
| `SMUDGE_HOT_BUILD` | any id | Set by a patch builder next to `SMUDGE_HOT_SOCKET`. The application sends it back unchanged, so the builder knows which build connected. |
