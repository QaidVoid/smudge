# Performance

<p class="lead">Smudge is measured with a fixed set of scenes, on a desktop and in a virtual machine without a GPU, with damage tracking on and off. These are the v0.1 numbers.</p>

## The scenes

| Scene | What changes every frame |
|---|---|
| S1 | 600 rounded, bordered boxes in a grid, every box changes color. The raster-heavy case. |
| S2 | The same 600 boxes, static, plus one small box that changes color. The best case for damage. |
| S3 | The root background, which covers everything. The worst case for damage. |
| S4 | 300 labels whose text changes. Layout-heavy. |
| S5 | A scroll through a thousand mixed rows, one wheel step per frame. |
| S6 | A scroll through a virtual list of a hundred thousand rows. |
| S9 | One keystroke per frame into a focused text field. |

Each scene runs in two modes. **Damage** is the default: repaint only what changed. **Full** repaints the whole surface every frame, which is what a renderer without damage tracking does. Each run is 30 warm-up frames and 300 measured frames, repeated three times, and the tables give the median frame time.

## On a desktop, CPU renderer

A Ryzen 7 7700 on Linux, one thread doing the work. Headless frames are 1280 by 800 (S2: 1920 by 1080) and include everything up to finished pixels. Window frames add the copy to the window buffer and were drawn in a tiled window of 1270 by 1388.

| Scene | Damage, headless | Full, headless | Damage, window | Full, window |
|---|---|---|---|---|
| S1 | 3.80 ms | 3.75 ms | 4.23 ms | 4.61 ms |
| S2 | **24 µs** | 3.73 ms | **49 µs** | 4.46 ms |
| S3 | 101 µs | 101 µs | 1.16 ms | 1.10 ms |
| S4 | 3.50 ms | 3.44 ms | 4.32 ms | 4.55 ms |
| S5 | 310 µs | 283 µs | 509 µs | 1.37 ms |
| S6 | 125 µs | 155 µs | 294 µs | 1.24 ms |
| S9 | **37 µs** | 87 µs | **75 µs** | 1.06 ms |

Where little changes, damage tracking turns milliseconds into microseconds: S2 is 150 times cheaper than a full repaint headless and 90 times cheaper in a window. Where everything changes (S1, S3), damage costs the same as a full repaint, so tracking it is free.

## On a desktop, GPU renderer

The same machine with the `gpu` feature. Headless frames drew on a GTX 1060, window frames on the processor's integrated GPU. The GPU renderer draws the whole frame either way.

| Scene | Headless | Window | CPU renderer, window, damage |
|---|---|---|---|
| S1 | 1.55 ms | **1.24 ms** | 4.23 ms |
| S2 | 1.56 ms | 1.09 ms | **49 µs** |
| S3 | 582 µs | **466 µs** | 1.16 ms |
| S4 | 4.41 ms | **4.10 ms** | 4.32 ms |
| S5 | 2.35 ms | 1.91 ms | **509 µs** |
| S6 | 686 µs | 518 µs | **294 µs** |
| S9 | 593 µs | 467 µs | **75 µs** |

On window frames, the GPU wins when most of the window changes (S1, S3) and loses when little does, because it redraws everything. `RendererChoice::Auto` still picks the GPU on a real adapter: at high scale factors, where every frame of a full-window change is millions of pixels, it keeps those frames fast, and a small update stays at about a millisecond or less on either renderer.

## In a virtual machine without a GPU

A virtual machine with 4 virtual CPUs and no GPU passthrough, running a compositor that renders in software. This is where a CPU renderer matters. The GPU renderer here runs on a software rasterizer, which is what an application gets when it asks for a GPU on such a machine.

| Scene | CPU renderer, window, damage | Software GPU, window |
|---|---|---|
| S1 | 5.01 ms | 12.5 ms |
| S2 | **75 µs** | 15.8 ms |
| S3 | 801 µs | 7.74 ms |
| S4 | 4.89 ms | 24.1 ms |
| S5 | 808 µs | 16.1 ms |
| S6 | 448 µs | 7.25 ms |
| S9 | **105 µs** | 5.96 ms |

The CPU renderer is faster in every scene, between 2.5 and over 200 times, which is why `RendererChoice::Auto` never picks a software adapter.

## How to reproduce

The benchmark tool is the `smudge-bench` crate in the repository. One run:

```sh
cargo run --release -p smudge-bench -- \
    --scene s2 --mode damage --backend headless \
    --warmup 30 --frames 300 --repeats 3 --out s2.jsonl
```

Each report holds every frame's timings by phase (flush, layout, paint, display list, damage, raster, present), the process and machine CPU load during the run, and the adapter for GPU runs. Add `--renderer gpu` with the bench's `gpu` feature to measure the GPU renderer.
