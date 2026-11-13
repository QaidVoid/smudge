# Cargo features

<p class="lead">The defaults give a desktop window with the CPU renderer, screen reader support and embedded fonts. Everything else is opt-in.</p>

## `smudge`

| Feature | Default | What it adds |
|---|---|---|
| `window` | yes | Windows, input and presentation through `smudge-platform`, and `run`. Turn it off for a headless build that only uses `App`. |
| `gpu` | no | The GPU renderer and its window path. Implies `window`. With it, `RendererChoice::Auto` becomes the default. |
| `complex-scripts` | no | Word and line break dictionaries for Chinese, Japanese, Thai, Lao, Khmer and Burmese. About 3 MB per binary and about 5 percent slower layout of other text. |
| `system-fonts` | no | Also finds the fonts installed on the machine. Desktop only. The same feature of `smudge-text`, forwarded, so applications can turn it on without adding `smudge-text` themselves. |

```toml
[dependencies]
smudge = { version = "0.2", features = ["gpu"] }
```

## `smudge-text`

| Feature | Default | What it adds |
|---|---|---|
| `bundled-fonts` | yes | Embeds Noto Sans, Noto Sans Bold and Noto Sans Mono, so text never depends on the machine. |
| `system-fonts` | no | Also finds the fonts installed on the machine. Desktop only. On Linux, fontconfig is loaded at run time, so building needs no fontconfig development files. |
| `complex-scripts` | no | The break dictionaries, as above. |

An application that depends on `smudge` turns these on through the `smudge` features of the same name:

```toml
[dependencies]
smudge = { version = "0.2", features = ["system-fonts"] }
```

## `smudge-platform`

| Feature | Default | What it adds |
|---|---|---|
| `a11y` | yes | The accessibility projection for screen readers on the desktop. It has no effect on the web. |
| `gpu` | no | The GPU renderer. The `smudge` feature of the same name turns this on. |

## Runtime switches

| Variable | Values | Effect |
|---|---|---|
| `SMUDGE_RENDERER` | `cpu`, `gpu`, `auto` | Overrides `WindowOptions::renderer` when the window opens. |
