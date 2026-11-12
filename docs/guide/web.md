# On the web

<p class="lead">The same application runs in a browser. The window becomes a canvas, the CPU renderer draws into it, and damage tracking works exactly as on the desktop.</p>

## What changes on the web

- The window is a `<canvas>` appended to the page's body. It starts at `WindowOptions::size` and follows any size the page gives it, so a page that sizes the canvas to the viewport gets an application that resizes with the browser window and with page zoom.
- The CPU renderer draws every frame. The `gpu` feature is a desktop feature.
- `run` returns `Ok(())` as soon as the browser's event loop has started, before the first frame. Errors detected before that, such as a translucent clear color, still come back as an `Err`.
- The bundled fonts are the only fonts. `system-fonts` is not available.
- The window commands `start_move`, `start_resize`, `toggle_maximize` and `close` do nothing, because the page owns the canvas, and `maximized` stays `false`. `WindowOptions::decorations` and `app_id` have no effect either, and `min_size` becomes the canvas's minimum size.
- There is no system clipboard, so copy and paste work only inside the application, and no screen reader support yet. See [Known limits](/reference/limits).

## Build

Add the target once, and install the `wasm-bindgen` command line tool:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version <the version in your Cargo.lock>
```

The tool's version must match the `wasm-bindgen` crate in your `Cargo.lock`. `cargo tree -i wasm-bindgen --target wasm32-unknown-unknown` shows which one that is.

Build the application and generate the JavaScript glue:

```sh
cargo build --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir web/pkg \
    target/wasm32-unknown-unknown/release/my-app.wasm
```

## The page

`web/index.html` loads the generated module, which runs your `main`. This one sizes the canvas to the whole viewport:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>My app</title>
    <style>
      html, body { margin: 0; height: 100%; background: #101115; }
      canvas { display: block; width: 100vw !important; height: 100vh !important; }
      canvas:focus { outline: none; }
    </style>
  </head>
  <body>
    <script type="module">
      import init from "./pkg/my-app.js";
      init();
    </script>
  </body>
</html>
```

The `!important` matters: the window sets an inline size on its canvas, and the page's rule overrides it.

Serve the `web` directory with any static file server and open it. Browsers do not load ES modules from `file:` URLs, so opening the file directly does not work.

## Try the example

The repository has a complete web example, a grid of boxes where one box changes per frame and a caption reports what each present copied:

```sh
cargo build --release --target wasm32-unknown-unknown --example web_boxes -p smudge-platform
wasm-bindgen --target web --out-dir crates/smudge-platform/examples/web/pkg \
    target/wasm32-unknown-unknown/release/examples/web_boxes.wasm
cd crates/smudge-platform/examples/web
python3 -m http.server 8000
```

## Size

A release build of the example is about 5.6 MB of wasm, 1.8 MB gzipped, embedded fonts included. The `complex-scripts` feature adds about 2.2 MB gzipped, so leave it off for the web unless the application shows Thai, Lao, Khmer or Burmese.
