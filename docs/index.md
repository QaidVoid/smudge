---
layout: page
sidebar: false
title: Smudge
titleTemplate: A Rust GUI library that repaints only what changed
---

<Home />

<div class="sm-home-code vp-doc">

## The whole program

A button and a label bound to one signal. Each press writes the signal, the label's text closure runs again, and the next frame repaints the label and the pressed button. Nothing else in the window is touched.

```rust
use smudge::prelude::*;

fn main() -> Result<(), PlatformError> {
    let count = RwSignal::new(0_u32);
    let press = count.clone();
    let shown = count.clone();

    run(
        WindowOptions {
            title: String::from("Counter"),
            size: Size { width: 280.0, height: 140.0 },
            clear_color: Color::rgb(16, 17, 21),
            repaint: Default::default(),
            show_damage: false,
            renderer: RendererChoice::default(),
            stats: None,
            on_frame: None,
            on_event: None,
            frame_limit: None,
        },
        move || {
            column()
                .padding(px(24.0))
                .gap(Size { width: 0.0, height: 12.0 })
                .align_items(Align::Center)
                .child(
                    button("Count up")
                        .primary()
                        .on_press(move || press.update(|count| *count += 1)),
                )
                .child(label(move || format!("Clicked {} times", shown.get())))
        },
    )
}
```

<div class="sm-home-next">
  <a href="/guide/quick-start">Build it yourself <span>→</span></a>
  <a href="/internals/frame">See what a frame does <span>→</span></a>
</div>

</div>
