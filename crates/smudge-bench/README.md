# smudge-bench

Measurement binary for the frame pipeline. One invocation runs one scene in
one repaint mode on one backend and writes one JSON Lines report: an
environment header, one line per measured frame, a percentile summary per
repeat, and one summary across repeats. The frame and summary lines carry the
phase timings in microseconds, damage rectangles and pixels, allocation
counts, and on the window backend the present's statistics.

## Scenes

| Key | Scene | Viewport | What it exercises |
|---|---|---|---|
| `s1` | 600 rounded, bordered boxes in a 30 by 20 grid, every box changes color every frame | 1280 by 800 | The raster-heavy case, with damage that covers the whole surface |
| `s2` | The same 600 boxes, static, plus one 80 by 24 box that changes color every frame | 1920 by 1080 | The best case for damage: one small rectangle per frame |
| `s3` | The root background changes every frame and covers everything | 1280 by 800 | The worst case for damage: a full-surface repaint either way |

Scene variation comes from an in-crate xorshift generator seeded per scene.
Frame `n` always produces the same colors, so a repeat is reproducible.

## Flags

```sh
smudge-bench --scene s1|s2|s3 [--mode damage|full] [--backend headless|window]
             [--warmup 30] [--frames 300] [--repeats 3] [--scale 1.0]
             [--label <text>] [--out <path>]
```

Defaults are shown in the list above. `--out` defaults to
`<scene>-<mode>-<backend>.jsonl`. The headless backend drives an `App` and a
`CpuRenderer` with no window. The window backend opens a real window, runs
`--warmup + --frames` presents for every repeat in that one window, and closes
itself.

The window backend uses the window's own scale factor and rejects `--scale`.
The window system decides the window's final size, so the surface can differ
from the scene's viewport. Float the window or run headless when a fixed
surface matters. The header records the surface the first measured frame
rendered, and the bench warns when the surface changes size during a run.

## Report shape

Each line is one JSON object, and `type` says which:

- `env`: label, scene, mode, backend, scale (`null` on the window backend),
  the rendered surface size, CPU model, kernel, compiler version,
  display-server presence, timestamp. It is written after the run, because the
  window backend only learns its surface size once it presents.
- `frame`: repeat, frame, `flush_us`, `layout_us`, `paint_us`, `list_us`,
  `damage_us`, `raster_us`, `acquire_us` (the wait for a free window
  buffer), `present_us` (the copy and submission after it),
  `frame_total_us` (the sum of the phases without `acquire_us`, so CPU work
  rather than waiting), damage rectangle and pixel counts
  before and after merging, `raster_pixels`, `bytes_copied`, `allocations`,
  `buffer_age`. Quantities a backend does not produce are `null`.
- `repeat_summary`: percentile objects (`median`, `p95`, `p99`, nearest-rank)
  for the total and each phase, plus the peak RSS and process CPU time at the
  start and end of the repeat, read from `/proc` on Linux and `null`
  elsewhere.
- `summary`: the same percentiles pooled across all repeats.

Allocations are counted by a global counting allocator in this binary only.

## Example

```sh
cargo run --release -p smudge-bench -- --scene s2 --mode damage --backend headless \
    --label my-machine --out s2-damage-headless.jsonl
```
