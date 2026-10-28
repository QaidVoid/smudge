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
| `s4` | 300 labels in a wrapped grid, every label's text changes every frame | 1280 by 800 | The layout-heavy case: a text layout for every label, every frame |
| `s5` | 1,000 rows of a box and a label in a scroll view, scrolled by scripted wheel events | 1280 by 800 | Scroll without layout: the content moves and the display list rebuilds |
| `s6` | A 100,000-row virtual list, scrolled one row height per frame | 1280 by 800 | The mounted window of a fixed-height list moving through a long list |
| `s9` | One text field, emptied and focused by a scripted click at the start of each repeat, given one keystroke per frame | 1280 by 800 | Input latency: the time from an event to the pixel it changes |

Scene variation comes from an in-crate xorshift generator seeded per scene.
Frame `n` always produces the same state, so a repeat is reproducible. The
scenes that script input deliver every event through
`App::handle_event`, the path a real event takes, and mark when they injected
it, which is what `latency_us` measures.

## Flags

```sh
smudge-bench --scene s1|s2|s3|s4|s5|s6|s9 [--mode damage|full] [--backend headless|window]
             [--renderer cpu|gpu]
             [--warmup 30] [--frames 300] [--repeats 3] [--scale 1.0]
             [--label <text>] [--out <path>]
```

Defaults are shown in the list above, with `--renderer cpu`. `--out` defaults
to `<scene>-<mode>-<backend>.jsonl`. The headless backend drives an `App` and
a renderer with no window: the CPU renderer draws into a pixmap, and, with
the crate's `gpu` build feature and `--renderer gpu`, the GPU renderer draws
into an offscreen texture that nothing shows. The window backend opens a real
window, runs `--warmup + --frames` presents for every repeat in that one
window, and closes itself.

`--renderer gpu` needs the `gpu` build feature and an adapter. It never falls
back to the CPU renderer: without an adapter the run fails. A headless GPU
frame waits for the GPU to finish, so its `raster_us` covers the GPU's work.
A window GPU frame does not wait, so its `raster_us` is the CPU side of the
draw and its submission.

The window backend uses the window's own scale factor and rejects `--scale`.
The window system decides the window's final size, so the surface can differ
from the scene's viewport. Float the window or run headless when a fixed
surface matters. The header records the surface the first measured frame
rendered, and the bench warns when the surface changes size during a run.

## Report shape

Each line is one JSON object, and `type` says which:

- `env`: label, scene, mode, backend, the renderer that drew the frames
  (`cpu`, or `gpu` for a GPU run: on the window backend the presenter the
  window actually used, which is how a GPU run proves it did not fall back),
  scale (`null` on the window backend), the rendered surface size, CPU model,
  kernel, compiler version, the GPU adapter the frames were drawn on as
  `adapter_name`, `adapter_backend` and `adapter_type` (`null` for CPU
  runs), display-server presence, timestamp. It is written after the run,
  because the window backend only learns its surface size once it presents.
- `frame`: repeat, frame, `flush_us`, `layout_us`, `paint_us`, `list_us`,
  `damage_us`, `raster_us`, `acquire_us` (the wait for a free window
  buffer), `present_us` (the copy and submission after it),
  `frame_total_us` (the sum of the phases without `acquire_us`, so CPU work
  rather than waiting), `latency_us` (from a scripted event to the end of the
  frame that shows it, on the scenes that inject one), damage rectangle and
  pixel counts before and after merging, `raster_pixels`, `bytes_copied`,
  `allocations`, `buffer_age`. Quantities a backend or scene does not produce
  are `null`.
- `repeat_summary`: percentile objects (`median`, `p95`, `p99`, nearest-rank)
  for the total, each phase, and the latency when the scene measures it,
  plus the peak RSS and process CPU time just before the repeat's first
  warmup frame and just after its last frame, read from `/proc` on Linux and
  `null` elsewhere. A headless repeat creates its renderer before the first
  sample, so a GPU renderer's pipeline compilation is not charged to the
  frames. A window repeat's start sample follows its first warmup frame,
  so it counts one frame less than a headless repeat. The same boundaries carry
  the machine's one-minute load average (`load1_start`, `load1_end`). Across
  the repeat, `busy_share` is the share of processor time the whole machine
  spent busy, and `other_busy_share` the share it spent busy on everything
  but this process, which is the one that shows a contaminated run. Both
  come from `/proc/stat` with busy counted as everything but idle and
  iowait, and all of them are `null` where `/proc` is missing.
- `package_energy_uj` and `energy_per_frame_uj` sit in the repeat summary
  too, read from the kernel's RAPL powercap zone for the whole processor
  package at the same two boundaries as the process samples. The energy is
  the whole package over the span the samples cover: the integrated GPU of
  the package and every other process included, so the machine's idle draw
  is in the number. A discrete GPU and the monitor are not. Attributing energy to the frames
  means subtracting the idle power times the span's wall time, and checking
  `other_busy_share` first. `energy_per_frame_uj` divides the span's energy
  by the frames it covers, warmup included, which is one frame less than a
  headless repeat's frames plus warmup on the window backend. The counter
  wraps at the range the kernel reports beside it, which the reader handles
  once; a wrap that crosses the range twice cannot be detected and would
  underreport. Both fields are `null` where the machine exposes no readable
  package counter.
- `summary`: the same percentiles pooled across all repeats.

Allocations are counted by a global counting allocator in this binary only.

## Example

```sh
cargo run --release -p smudge-bench -- --scene s2 --mode damage --backend headless \
    --label my-machine --out s2-damage-headless.jsonl
```
