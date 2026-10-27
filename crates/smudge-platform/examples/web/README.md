# Smudge boxes on the web

The `web_boxes` example runs the CPU renderer with damage-tracked repainting
in a browser. The window appends its canvas to the page, and `index.html`
sizes it to fill the viewport, so resizing the browser window or zooming the
page resizes the app. One box changes color per frame, and the caption
reports what each present copied.

## Build and run

From the repository root:

```sh
cargo build --release --target wasm32-unknown-unknown --example web_boxes -p smudge-platform
wasm-bindgen --target web --out-dir crates/smudge-platform/examples/web/pkg \
    target/wasm32-unknown-unknown/release/examples/web_boxes.wasm
```

`wasm-bindgen` is the command line tool, installed separately, and its version
must match the `wasm-bindgen` version in `Cargo.lock`. Check both with
`wasm-bindgen --version`.

Then serve this directory (`examples/web`) with any static file server and
open the page:

```sh
cd crates/smudge-platform/examples/web
python3 -m http.server 8000
```

Point the browser at <http://localhost:8000>. The page needs a static server
because browsers do not load ES modules from `file:` URLs.

`pkg/` holds generated output and is not checked in. `index.html` loads
`pkg/web_boxes.js`, which is the name `wasm-bindgen` derives from the example.
