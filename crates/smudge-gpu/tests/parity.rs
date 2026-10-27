//! Parity between the CPU and the GPU renderer over the shared feature
//! subset.
//!
//! Every scene is mounted in an app, and one frame of it is rendered through
//! both renderers at scale factors 1 and 2. The CPU surface is unpremultiplied
//! so both images are straight alpha, and each channel is compared by its mean
//! and maximum absolute difference and by the fraction of channel values that
//! differ by more than 8. The two rasterizers anti-alias differently, so byte
//! equality is not the goal; the tolerances were calibrated on the runs
//! recorded in the parity thresholds decision, and a value past one means one
//! renderer changed under the other.
//!
//! The test skips with a printed reason when the machine has no GPU adapter,
//! so a missing adapter can never fail a run. On a violation it writes both
//! images and their difference under `target/smudge-test-failures/parity/`.

use std::collections::HashMap;
use std::path::PathBuf;

use parley::{FontFamily, FontWeight, GenericFamily};
use smudge_core::app::{App, AppConfig, RepaintMode};
use smudge_core::element::Element;
use smudge_core::geometry::{Color, Size};
use smudge_core::renderer::Renderer;
use smudge_core::style::{Align, Edges, Length, px};
use smudge_core::widget::{Panel, panel};
use smudge_cpu::CpuRenderer;
use smudge_gpu::wgpu::{Device, Queue};
use smudge_gpu::{GpuRenderer, wgpu};
use smudge_text::{LabelExt, TextSystem, label};

/// The viewport every scene mounts in, in logical pixels.
const VIEWPORT: Size = Size {
    width: 200.0,
    height: 150.0,
};

/// The scale factors the scenes run at.
const SCALES: [f32; 2] = [1.0, 2.0];

/// The boxes of the grid scene, which wraps them to the viewport width.
const GRID_BOXES: usize = 24;

/// The seed of the grid scene's box colors, fixed so a run is reproducible.
const GRID_SEED: u64 = 0x5EED_0001;

/// The mean, maximum and over-8 fraction one scene and scale may reach.
///
/// The values are the calibration ceiling of the scene and scale they belong
/// to, about 1.5 times the largest value the recorded calibration runs showed.
#[derive(Clone, Copy)]
struct Tolerance {
    /// The scene the tolerance was calibrated on.
    scene: &'static str,
    /// The scale factor the tolerance was calibrated at.
    scale: f32,
    /// The largest mean absolute difference any channel may show.
    mean: f64,
    /// The largest single channel difference any pixel may show.
    max: u32,
    /// The largest fraction of channel values differing by more than 8.
    over_eight: f64,
}

/// The calibrated tolerance of every scene and scale.
///
/// A scene added to the suite needs an entry here, calibrated the same way:
/// run once, read the printed metrics, and record about 1.5 times the observed
/// values, rounded up.
const TOLERANCES: &[Tolerance] = &[
    Tolerance {
        scene: "plain rects",
        scale: 1.0,
        mean: 0.0,
        max: 0,
        over_eight: 0.0,
    },
    Tolerance {
        scene: "plain rects",
        scale: 2.0,
        mean: 0.0,
        max: 0,
        over_eight: 0.0,
    },
    Tolerance {
        scene: "rounded rects",
        scale: 1.0,
        mean: 0.3929,
        max: 87,
        over_eight: 0.0161,
    },
    Tolerance {
        scene: "rounded rects",
        scale: 2.0,
        mean: 0.1754,
        max: 75,
        over_eight: 0.00707,
    },
    Tolerance {
        scene: "borders",
        scale: 1.0,
        mean: 0.1968,
        max: 57,
        over_eight: 0.0086,
    },
    Tolerance {
        scene: "borders",
        scale: 2.0,
        mean: 0.1071,
        max: 87,
        over_eight: 0.0047,
    },
    Tolerance {
        scene: "nested clips",
        scale: 1.0,
        mean: 0.0,
        max: 0,
        over_eight: 0.0,
    },
    Tolerance {
        scene: "nested clips",
        scale: 2.0,
        mean: 0.0,
        max: 0,
        over_eight: 0.0,
    },
    Tolerance {
        scene: "text",
        scale: 1.0,
        mean: 3.584,
        max: 243,
        over_eight: 0.07545,
    },
    Tolerance {
        scene: "text",
        scale: 2.0,
        mean: 1.0266,
        max: 171,
        over_eight: 0.0335,
    },
    Tolerance {
        scene: "box grid",
        scale: 1.0,
        mean: 1.0134,
        max: 56,
        over_eight: 0.04391,
    },
    Tolerance {
        scene: "box grid",
        scale: 2.0,
        mean: 0.7136,
        max: 93,
        over_eight: 0.02633,
    },
];

/// The mean, maximum and over-8 difference of one channel over one image.
#[derive(Clone, Copy)]
struct Metrics {
    /// The absolute differences averaged over all pixels.
    mean: f64,
    /// The largest single difference.
    max: u32,
    /// The fraction of channel values differing by more than 8.
    over_eight: f64,
}

/// One scene: its name and how to mount it.
type Scene = (&'static str, fn() -> Element<Panel>);

#[test]
fn both_renderers_draw_the_feature_subset_within_the_calibrated_tolerances() {
    let Some((device, queue, adapter)) = headless_device() else {
        eprintln!("skipping: no GPU adapter available for the headless renderer");
        return;
    };
    eprintln!("parity adapter: {adapter}");
    TextSystem::bundled_only().install();

    let mut gpu_renderers: HashMap<(u32, u32), GpuRenderer> = HashMap::new();
    let mut violations = Vec::new();
    for (name, build) in scenes() {
        for scale in SCALES {
            let mut app = App::new(
                AppConfig {
                    viewport: VIEWPORT,
                    scale,
                    clear_color: Color::WHITE,
                    repaint: RepaintMode::Full,
                },
                build,
            );
            let (width, height) = app.surface_size();
            let frame = app.frame();
            let mut cpu = CpuRenderer::new(width, height);
            cpu.render(&frame);
            let gpu = gpu_renderers
                .entry((width, height))
                .or_insert_with(|| GpuRenderer::new(device.clone(), queue.clone(), width, height));
            gpu.render(&frame);

            let mut cpu_pixels = cpu.pixels().to_vec();
            unpremultiply(&mut cpu_pixels);
            let gpu_pixels = gpu.read_pixels();
            let metrics = compare(&cpu_pixels, &gpu_pixels);
            print_metrics(name, scale, &metrics);

            let tolerance = TOLERANCES
                .iter()
                .find(|tolerance| tolerance.scene == *name && tolerance.scale == scale);
            let Some(tolerance) = tolerance else {
                violations.push(format!("{name} at scale {scale} has no recorded tolerance"));
                continue;
            };
            if let Some(report) = exceed(&metrics, tolerance) {
                write_failure_images(name, scale, width, height, &cpu_pixels, &gpu_pixels);
                violations.push(format!("{name} at scale {scale} {report}"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "the renderers disagree past the calibrated tolerances:\n{}",
        violations.join("\n")
    );
}

/// Requests an adapter the way [`GpuRenderer::headless`] does and returns its
/// device, queue and a readable name, or `None` when no adapter works.
fn headless_device() -> Option<(Device, Queue, String)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = futures_executor::block_on(
        instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )
    .ok()?;
    let info = adapter.get_info();
    let name = format!("{} on {:?}", info.name, info.backend);
    let (device, queue) =
        futures_executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .ok()?;
    Some((device, queue, name))
}

/// Converts premultiplied RGBA8 pixels to straight alpha in place.
fn unpremultiply(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u32::from(pixel[3]);
        if alpha == 0 {
            pixel[..3].fill(0);
        } else if alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha) as u8;
            }
        }
    }
}

/// Compares both images per channel, which must be straight alpha throughout.
fn compare(cpu: &[u8], gpu: &[u8]) -> [Metrics; 4] {
    let pixels = (cpu.len() / 4) as f64;
    let mut sums = [0u64; 4];
    let mut maxs = [0u32; 4];
    let mut over_eight = [0u64; 4];
    for (cpu, gpu) in cpu.as_chunks::<4>().0.iter().zip(gpu.as_chunks::<4>().0) {
        for channel in 0..4 {
            let difference = u8::abs_diff(cpu[channel], gpu[channel]);
            sums[channel] += u64::from(difference);
            maxs[channel] = maxs[channel].max(u32::from(difference));
            if difference > 8 {
                over_eight[channel] += 1;
            }
        }
    }
    std::array::from_fn(|channel| Metrics {
        mean: sums[channel] as f64 / pixels,
        max: maxs[channel],
        over_eight: over_eight[channel] as f64 / pixels,
    })
}

/// Prints one line per channel with the three metrics of the scene and scale.
fn print_metrics(name: &str, scale: f32, metrics: &[Metrics; 4]) {
    let channels = ["r", "g", "b", "a"];
    let columns: Vec<String> = metrics
        .iter()
        .zip(channels)
        .map(|(metrics, channel)| {
            format!(
                "{channel} mean {:.4} max {} over8 {:.5}",
                metrics.mean, metrics.max, metrics.over_eight
            )
        })
        .collect();
    eprintln!("{name} s{}: {}", scale, columns.join("  "));
}

/// Returns a report of every metric past its tolerance, or `None` when all
/// four channels stay within them.
fn exceed(metrics: &[Metrics; 4], tolerance: &Tolerance) -> Option<String> {
    let channels = ["r", "g", "b", "a"];
    let mut reports = Vec::new();
    for (metrics, channel) in metrics.iter().zip(channels) {
        if metrics.mean > tolerance.mean {
            reports.push(format!(
                "{channel} mean {:.4} past {:.4}",
                metrics.mean, tolerance.mean
            ));
        }
        if metrics.max > tolerance.max {
            reports.push(format!(
                "{channel} max {} past {}",
                metrics.max, tolerance.max
            ));
        }
        if metrics.over_eight > tolerance.over_eight {
            reports.push(format!(
                "{channel} over8 {:.5} past {:.5}",
                metrics.over_eight, tolerance.over_eight
            ));
        }
    }
    if reports.is_empty() {
        None
    } else {
        Some(reports.join(", "))
    }
}

/// Writes the CPU and GPU images and their per-channel difference under
/// `target/smudge-test-failures/parity/`, named after the scene and scale.
fn write_failure_images(name: &str, scale: f32, width: u32, height: u32, cpu: &[u8], gpu: &[u8]) {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("smudge-test-failures")
        .join("parity");
    if let Err(error) = std::fs::create_dir_all(&directory) {
        eprintln!("could not create {}: {error}", directory.display());
        return;
    }
    let tag = format!("{}-s{}", name.replace(' ', "-"), scale);
    let mut difference = Vec::with_capacity(cpu.len());
    for (cpu, gpu) in cpu.as_chunks::<4>().0.iter().zip(gpu.as_chunks::<4>().0) {
        difference.extend([
            u8::abs_diff(cpu[0], gpu[0]),
            u8::abs_diff(cpu[1], gpu[1]),
            u8::abs_diff(cpu[2], gpu[2]),
            255,
        ]);
    }
    for (suffix, pixels) in [
        ("cpu.png", cpu),
        ("gpu.png", gpu),
        ("diff.png", &difference),
    ] {
        let path = directory.join(format!("{tag}-{suffix}"));
        match png(pixels, width, height) {
            Some(bytes) => {
                if let Err(error) = std::fs::write(&path, bytes) {
                    eprintln!("could not write {}: {error}", path.display());
                }
            }
            None => eprintln!("could not encode {}", path.display()),
        }
    }
}

/// Encodes straight-alpha RGBA8 pixels as a PNG image.
fn png(pixels: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    let data = pixmap.data_mut();
    for (pixel, source) in data
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(pixels.as_chunks::<4>().0)
    {
        let alpha = u32::from(source[3]);
        for (target, channel) in pixel[..3].iter_mut().zip(source[..3].iter()) {
            *target = ((u32::from(*channel) * alpha + 127) / 255) as u8;
        }
        pixel[3] = source[3];
    }
    pixmap.encode_png().ok()
}

/// A fixed length at `left`, `top` with `width` and `height`, out of the flow.
fn at(left: f32, top: f32, width: f32, height: f32) -> Element<Panel> {
    panel()
        .absolute()
        .width(px(width))
        .height(px(height))
        .inset(Edges {
            left: Length::px(left),
            top: Length::px(top),
            right: Length::auto(),
            bottom: Length::auto(),
        })
}

/// Opaque rectangles over one another, one translucent, one past each surface
/// edge and one with no coverage at all.
fn plain_rects() -> Element<Panel> {
    panel()
        .child(at(8.0, 8.0, 60.0, 40.0).background(Color::rgb(40, 60, 180)))
        .child(at(40.0, 30.0, 80.0, 50.0).background(Color::rgba(200, 40, 40, 128)))
        .child(at(150.0, 100.0, 80.0, 60.0).background(Color::rgb(30, 140, 60)))
        .child(at(-12.0, 60.0, 30.0, 30.0).background(Color::rgb(230, 200, 40)))
        .child(at(120.0, 20.0, 40.0, 30.0).background(Color::rgba(90, 90, 90, 0)))
}

/// Rounded rectangles at rising radii, one past the fittable radius, and one
/// translucent over an opaque one.
fn rounded_rects() -> Element<Panel> {
    panel()
        .child(
            at(6.0, 10.0, 50.0, 40.0)
                .corner_radius(2.0)
                .background(Color::rgb(150, 40, 90)),
        )
        .child(
            at(70.0, 10.0, 50.0, 40.0)
                .corner_radius(6.0)
                .background(Color::rgb(40, 120, 150)),
        )
        .child(
            at(134.0, 10.0, 50.0, 40.0)
                .corner_radius(20.0)
                .background(Color::rgb(120, 150, 30)),
        )
        .child(
            at(20.0, 70.0, 50.0, 40.0)
                .corner_radius(30.0)
                .background(Color::rgb(90, 60, 160)),
        )
        .child(
            at(60.0, 90.0, 90.0, 50.0)
                .corner_radius(10.0)
                .background(Color::rgba(30, 30, 140, 140)),
        )
}

/// Borders of rising widths, a rounded border, a translucent border and a
/// border thicker than its box, which fills it whole.
fn borders() -> Element<Panel> {
    panel()
        .child(
            at(8.0, 10.0, 50.0, 40.0)
                .border(1.0)
                .border_color(Color::rgb(180, 30, 30)),
        )
        .child(
            at(74.0, 10.0, 50.0, 40.0)
                .border(2.0)
                .border_color(Color::rgb(30, 130, 60))
                .background(Color::rgb(240, 240, 220)),
        )
        .child(
            at(140.0, 10.0, 50.0, 40.0)
                .border(8.0)
                .border_color(Color::rgb(60, 60, 190)),
        )
        .child(
            at(20.0, 70.0, 44.0, 36.0)
                .corner_radius(8.0)
                .border(3.0)
                .border_color(Color::rgb(150, 80, 20))
                .background(Color::rgb(250, 210, 120)),
        )
        .child(
            at(90.0, 70.0, 10.0, 10.0)
                .border(8.0)
                .border_color(Color::rgb(20, 20, 20)),
        )
        .child(
            at(120.0, 70.0, 60.0, 44.0)
                .corner_radius(12.0)
                .border(6.0)
                .border_color(Color::rgba(120, 20, 150, 160)),
        )
}

/// Clips nested two deep, with content that overflows every clip edge, plus a
/// rounded clipping node whose clip stays rectangular on both renderers.
fn nested_clips() -> Element<Panel> {
    panel()
        .child(
            at(10.0, 10.0, 120.0, 90.0)
                .clip(true)
                .background(Color::rgb(225, 230, 240))
                .child(
                    at(60.0, 30.0, 100.0, 80.0)
                        .clip(true)
                        .background(Color::rgb(200, 220, 200))
                        .child(at(70.0, 40.0, 80.0, 80.0).background(Color::rgb(200, 80, 30))),
                )
                .child(at(-20.0, 50.0, 40.0, 30.0).background(Color::rgb(120, 40, 160))),
        )
        .child(
            at(150.0, 20.0, 40.0, 40.0)
                .clip(true)
                .corner_radius(10.0)
                .background(Color::rgb(250, 240, 200))
                .child(at(-10.0, -10.0, 60.0, 60.0).background(Color::rgb(40, 90, 170))),
        )
}

/// Three lines in three faces and three colors, so a wrong glyph color, a
/// wrong face or a font blob mixed up with another diverges per channel.
fn text_lines() -> Element<Panel> {
    panel()
        .column()
        .padding(px(14.0))
        .gap(Size {
            width: 0.0,
            height: 10.0,
        })
        .child(
            label("Parity smudge 12")
                .text_size(16.0)
                .text_color(Color::rgb(20, 60, 200)),
        )
        .child(
            label("mono {glyphs} 07")
                .text_size(13.0)
                .family(FontFamily::from(GenericFamily::Monospace))
                .text_color(Color::rgb(160, 30, 30)),
        )
        .child(
            label("Bold parity")
                .text_size(14.0)
                .weight(FontWeight::BOLD)
                .text_color(Color::rgb(10, 110, 70)),
        )
}

/// The box grid of the benchmark's first scene at the test viewport: rounded,
/// bordered boxes wrapping over the width, each an opaque color from a fixed
/// seed.
fn box_grid() -> Element<Panel> {
    let mut colors = Rng::new(GRID_SEED);
    let mut grid = panel()
        .row()
        .wrap(true)
        .width(px(VIEWPORT.width))
        .gap(Size {
            width: 2.0,
            height: 2.0,
        });
    for _ in 0..GRID_BOXES {
        grid = grid.child(
            panel()
                .width(px(38.0))
                .height(px(38.0))
                .corner_radius(8.0)
                .border(2.0)
                .border_color(Color::rgb(70, 74, 88))
                .background(colors.color()),
        );
    }
    panel().align_items(Align::Start).child(grid)
}

/// The scenes of the subset, in the order they run.
fn scenes() -> &'static [Scene] {
    &[
        ("plain rects", plain_rects),
        ("rounded rects", rounded_rects),
        ("borders", borders),
        ("nested clips", nested_clips),
        ("text", text_lines),
        ("box grid", box_grid),
    ]
}

/// A small deterministic generator for the grid scene's box colors.
struct Rng(u64);

impl Rng {
    /// Creates a generator from a seed, which may be zero.
    fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }

    /// Returns the next value.
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Returns the next value as an opaque sRGB color.
    fn color(&mut self) -> Color {
        let value = self.next();
        Color::rgb(value as u8, (value >> 8) as u8, (value >> 16) as u8)
    }
}
