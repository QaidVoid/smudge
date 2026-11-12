# Introduction

<p class="lead">Smudge is a native Rust GUI library. You describe the interface once as a tree of widgets, keep application state in signals, and the library redraws exactly the pixels a change touched.</p>

## Three ideas

**A retained widget tree.** The closure you hand to `run` builds the interface once. The widgets it returns stay mounted, keep their layout, and keep the paint operations they produced last time. Nothing is rebuilt from scratch on every frame.

**Fine-grained signals.** State lives in signals such as `RwSignal`. When you pass a closure instead of a plain value to a widget property, for example `label(move || format!("{}", count.get()))`, that one property of that one widget binds to the signals the closure reads. A write to `count` reruns that closure and nothing else.

**Damage-tracked repainting.** At the next frame, the updated widgets regenerate their paint operations and the library compares them with what was there before. The difference is the frame's damage: the rectangles of physical pixels that really changed. The renderer repaints those rectangles onto the previous frame's pixels and the window presents only them.

```text
  write a signal ──► bound closures rerun ──► layout (if a size changed)
        │
        ▼
  changed widgets repaint ──► damage = what differs ──► raster and present the damage
```

A frame that changes one small box rasterizes and presents one small box, and an animation frame where nothing changed skips the present.

## Built for machines without a GPU

Many machines that run desktop software have no usable GPU: virtual machines, remote desktops, thin clients and older laptops. There, a renderer that redraws the whole window every frame spends its time drawing pixels that did not change. Smudge's CPU renderer keeps the last frame and only touches the damage, so a typical interaction costs microseconds instead of milliseconds.

When a real GPU is present, the optional GPU renderer takes over automatically, because it is faster when large parts of the window change at once. When the only adapter is a software rasterizer, Smudge stays on the CPU renderer. [Windows and renderers](./rendering) covers the choice.

## What is in it

- Containers (`row`, `column`, `stack`, `spacer`), a button, a label that can end in an ellipsis, an image, a single-line text input, a multi-line text area, a vertical scroll view that can follow its end, and virtual lists for very long lists, with fixed or varying row heights.
- Flexbox and block layout, with lengths in pixels or percent.
- Styling with colors, borders, corner radii, theme tokens, and hover, focus, pressed and disabled variants.
- Keyboard focus with autofocus and focus traps, app-wide key handlers, pointer and wheel input, input methods for composing languages, and the system clipboard on the desktop.
- Messages from other threads, timers, and window commands and cursors for a custom title bar.
- A CPU renderer, and a GPU renderer behind the `gpu` feature.
- Screen reader support on the desktop.
- A web build that runs the CPU renderer in a browser canvas.

Smudge is young. The API will change between minor versions while it settles, and [Known limits](/reference/limits) lists what it does not do yet.

## Where to go next

- [Quick start](./quick-start) builds and runs a first window.
- [Signals](./signals) explains how state reaches the screen.
- [Anatomy of a frame](/internals/frame) follows one frame from a signal write to the present.
