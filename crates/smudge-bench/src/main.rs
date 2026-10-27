//! Measurement binary for the frame pipeline: scene suite and timing reports.
//!
//! One invocation runs one scene in one repaint mode on one backend, for a
//! fixed number of repeats of warmup and measured frames, and writes one JSON
//! Lines file with an environment header, one line per measured frame, and a
//! percentile summary per repeat and across repeats.
//!
//! See `README.md` in this crate for the scenes and the flags.

mod allocations;
mod process;
mod scenes;
mod stats;

use std::cell::{Cell, RefCell};
use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use smudge_core::app::{App, AppConfig, FrameStats, RepaintMode};
use smudge_core::geometry::Color;
use smudge_core::renderer::Renderer;
use smudge_cpu::CpuRenderer;
use smudge_platform::{
    EventHook, FrameHook, PlatformError, PresentStats, RendererChoice, WindowOptions, run_with,
};

use crate::process::Sample;
use crate::scenes::Scene;
use crate::stats::Percentiles;

/// The color the renderer fills the surface with. It must be opaque, because
/// presentation copies only the color channels.
const CLEAR_COLOR: Color = Color::rgb(16, 17, 21);

/// The text of `--help`.
const USAGE: &str = "usage: smudge-bench --scene s1|s2|s3|s4|s5|s6|s9 [--mode damage|full] \
                      [--backend headless|window] [--renderer cpu|gpu] \
                      [--warmup 30] [--frames 300] [--repeats 3] [--scale 1.0] \
                      [--label <text>] [--out <path>]";

/// What to measure and how.
struct Args {
    /// The scene to run.
    scene: &'static Scene,
    /// How much of the surface each frame repaints.
    mode: RepaintMode,
    /// Where the frames render.
    backend: Backend,
    /// Which renderer draws the frames.
    renderer: RendererArg,
    /// How many frames each repeat runs before the measured frames start.
    warmup: u32,
    /// How many frames each repeat records.
    frames: u32,
    /// How many times the scene runs.
    repeats: u32,
    /// The physical pixels per logical pixel of the headless surface. The
    /// window backend uses the window system's scale instead.
    scale: f32,
    /// A free-form name for the environment, recorded in the header.
    label: String,
    /// Where the report is written.
    out: PathBuf,
}

/// Which renderer a run asks for.
#[derive(Copy, Clone, PartialEq)]
enum RendererArg {
    /// The CPU renderer, the default.
    Cpu,
    /// The GPU renderer. Needs the bench's `gpu` build feature and, on the
    /// headless backend, an adapter.
    Gpu,
}

impl RendererArg {
    /// Returns the name the report records for the chosen renderer, which is
    /// also the default.
    fn name(self) -> &'static str {
        match self {
            RendererArg::Cpu => "cpu",
            RendererArg::Gpu => "gpu",
        }
    }

    /// Returns the window's renderer choice for this argument.
    fn choice(self) -> RendererChoice {
        match self {
            RendererArg::Cpu => RendererChoice::Cpu,
            RendererArg::Gpu => RendererChoice::Gpu,
        }
    }
}

/// Where the frames render.
#[derive(Copy, Clone, PartialEq)]
enum Backend {
    /// An [`App`] and a [`CpuRenderer`], with no window.
    Headless,
    /// A real window through the platform crate.
    Window,
}

impl Backend {
    /// The name the report records.
    fn name(self) -> &'static str {
        match self {
            Backend::Headless => "headless",
            Backend::Window => "window",
        }
    }
}

/// The name the report records for the repaint mode.
fn mode_name(mode: RepaintMode) -> &'static str {
    match mode {
        RepaintMode::Damage => "damage",
        RepaintMode::Full => "full",
    }
}

/// One measured frame.
struct FrameRecord {
    /// The repeat the frame belongs to, from zero.
    repeat: u32,
    /// The frame within its repeat's measured span, from zero.
    frame: u32,
    /// The frame pipeline's timings and counts.
    stats: FrameStats,
    /// How long the renderer took, in microseconds.
    raster_us: f64,
    /// What the present did, on the window backend.
    present: Option<PresentStats>,
    /// How long the scripted event took from its injection to the end of the
    /// frame, when the scene injected one: to the end of the raster on the
    /// headless backend, and to the end of the present on the window backend.
    latency_us: Option<f64>,
    /// The width and height of the rendered surface in physical pixels.
    surface: (u32, u32),
    /// How many allocations the frame made.
    allocations: u64,
}

impl FrameRecord {
    /// Returns the total CPU time of the frame's measured phases, in
    /// microseconds.
    ///
    /// The total is the sum of the recorded phases, so it counts the time the
    /// frame spent working and not the time it spent waiting. The wait for a
    /// free window buffer is recorded on its own and left out.
    fn total_us(&self) -> f64 {
        let s = &self.stats;
        let present_us = self.present.map_or(0.0, |p| us(p.present));
        us(s.flush)
            + us(s.layout)
            + us(s.paint)
            + us(s.list)
            + us(s.damage)
            + self.raster_us
            + present_us
    }
}

fn main() -> ExitCode {
    let args = match Args::parse() {
        Ok(Some(args)) => args,
        Ok(None) => return ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("smudge-bench: {message}");
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    let file = match File::create(&args.out) {
        Ok(file) => file,
        Err(error) => {
            eprintln!(
                "smudge-bench: cannot create {}: {error}",
                args.out.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let mut out = BufWriter::new(file);

    let repeat_runs = match args.backend {
        Backend::Headless => {
            let mut runs = Vec::with_capacity(args.repeats as usize);
            for repeat in 0..args.repeats {
                let start = process::sample();
                match run_headless(&args, repeat) {
                    Ok(records) => runs.push(RepeatRun {
                        records,
                        start,
                        end: process::sample(),
                    }),
                    Err(message) => {
                        eprintln!("smudge-bench: {message}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            runs
        }
        Backend::Window => match run_window(&args) {
            Ok(runs) => runs,
            Err(error) => {
                eprintln!("smudge-bench: the run failed: {error}");
                return ExitCode::FAILURE;
            }
        },
    };

    let surface = repeat_runs
        .iter()
        .find_map(|run| run.records.first())
        .map(|record| record.surface);
    let renderer = repeat_runs
        .iter()
        .flat_map(|run| &run.records)
        .find_map(|record| record.present.as_ref().map(|p| p.renderer))
        .unwrap_or_else(|| match args.backend {
            Backend::Headless => args.renderer.name(),
            Backend::Window => "none",
        });
    let resized = repeat_runs
        .iter()
        .flat_map(|run| &run.records)
        .any(|record| Some(record.surface) != surface);
    if resized {
        eprintln!(
            "smudge-bench: the surface changed size during the run, so its frames are not comparable"
        );
    }
    if let Err(error) = writeln!(out, "{}", env_line(&args, surface, renderer)) {
        eprintln!("smudge-bench: cannot write the header: {error}");
        return ExitCode::FAILURE;
    }

    let mut all_records = Vec::with_capacity(args.repeats as usize);
    for (repeat, run) in repeat_runs.into_iter().enumerate() {
        let repeat = repeat as u32;
        if run.records.len() != args.frames as usize {
            eprintln!(
                "smudge-bench: repeat {repeat} recorded {} frames instead of {}",
                run.records.len(),
                args.frames,
            );
        }
        for record in &run.records {
            if let Err(error) = writeln!(out, "{}", frame_line(record)) {
                eprintln!("smudge-bench: cannot write a frame: {error}");
                return ExitCode::FAILURE;
            }
        }
        let line = repeat_summary_line(repeat, &run.records, run.start, run.end);
        if let Err(error) = writeln!(out, "{line}") {
            eprintln!("smudge-bench: cannot write a repeat summary: {error}");
            return ExitCode::FAILURE;
        }
        all_records.push(run.records);
    }

    let pooled: Vec<FrameRecord> = all_records.into_iter().flatten().collect();
    if let Err(error) = writeln!(out, "{}", pooled_summary_line(&pooled)) {
        eprintln!("smudge-bench: cannot write the summary: {error}");
        return ExitCode::FAILURE;
    }
    if let Err(error) = out.flush() {
        eprintln!("smudge-bench: cannot finish the report: {error}");
        return ExitCode::FAILURE;
    }

    eprintln!(
        "wrote {} frames of scene {} to {}",
        pooled.len(),
        args.scene.key,
        args.out.display(),
    );
    ExitCode::SUCCESS
}

impl Args {
    /// Parses the command line, or returns `None` when `--help` was asked
    /// for, or the reason the line is wrong.
    fn parse() -> Result<Option<Args>, String> {
        let mut scene = None;
        let mut mode = RepaintMode::Damage;
        let mut backend = Backend::Headless;
        let mut renderer = RendererArg::Cpu;
        let mut warmup = 30;
        let mut frames = 300;
        let mut repeats = 3;
        let mut scale: f32 = 1.0;
        let mut scale_given = false;
        let mut label = String::new();
        let mut out = None;

        let mut arguments = env::args().skip(1);
        while let Some(flag) = arguments.next() {
            let mut value = || {
                arguments
                    .next()
                    .ok_or_else(|| format!("the flag {flag} needs a value"))
            };
            match flag.as_str() {
                "--scene" => {
                    let key = value()?;
                    scene = Some(Scene::by_key(&key).ok_or_else(|| {
                        format!("unknown scene {key}, want s1, s2, s3, s4, s5, s6 or s9")
                    })?);
                }
                "--mode" => {
                    mode = match value()?.as_str() {
                        "damage" => RepaintMode::Damage,
                        "full" => RepaintMode::Full,
                        other => return Err(format!("unknown mode {other}, want damage or full")),
                    };
                }
                "--backend" => {
                    backend = match value()?.as_str() {
                        "headless" => Backend::Headless,
                        "window" => Backend::Window,
                        other => {
                            return Err(format!(
                                "unknown backend {other}, want headless or window"
                            ));
                        }
                    };
                }
                "--renderer" => {
                    renderer = match value()?.as_str() {
                        "cpu" => RendererArg::Cpu,
                        "gpu" => RendererArg::Gpu,
                        other => {
                            return Err(format!("unknown renderer {other}, want cpu or gpu"));
                        }
                    };
                }
                "--warmup" => warmup = parse_count(&flag, &value()?)?,
                "--frames" => frames = parse_count(&flag, &value()?)?,
                "--repeats" => repeats = parse_count(&flag, &value()?)?,
                "--scale" => {
                    scale = value()?.parse().map_err(|_| {
                        "the flag --scale needs a finite positive number".to_string()
                    })?;
                    if !scale.is_finite() || scale <= 0.0 {
                        return Err("the flag --scale needs a finite positive number".to_string());
                    }
                    scale_given = true;
                }
                "--label" => label = value()?,
                "--out" => out = Some(PathBuf::from(value()?)),
                "--help" | "-h" => {
                    // The usage text is the program's documented answer to
                    // --help, which belongs on standard output.
                    #[allow(clippy::print_stdout)]
                    {
                        println!("{USAGE}");
                    }
                    return Ok(None);
                }
                other => return Err(format!("unknown flag {other}")),
            }
        }

        let scene = scene.ok_or("the flag --scene is required")?;
        if frames == 0 {
            return Err("the flag --frames needs at least 1".to_string());
        }
        if repeats == 0 {
            return Err("the flag --repeats needs at least 1".to_string());
        }
        if scale_given && backend == Backend::Window {
            return Err(
                "the flag --scale applies to headless runs only, because the window \
                 system sets the window's scale"
                    .to_string(),
            );
        }
        let out = out.unwrap_or_else(|| {
            PathBuf::from(format!(
                "{}-{}-{}.jsonl",
                scene.key,
                mode_name(mode),
                backend.name()
            ))
        });
        Ok(Some(Args {
            scene,
            mode,
            backend,
            renderer,
            warmup,
            frames,
            repeats,
            scale,
            label,
            out,
        }))
    }
}

/// Parses a non-negative frame count.
fn parse_count(flag: &str, value: &str) -> Result<u32, String> {
    value
        .parse()
        .map_err(|_| format!("the flag {flag} needs a whole number, got {value:?}"))
}

/// One repeat's measured frames and the process metrics around it.
struct RepeatRun {
    /// The repeat's measured frames.
    records: Vec<FrameRecord>,
    /// The process metrics when the repeat began.
    start: Sample,
    /// The process metrics when the repeat ended.
    end: Sample,
}

/// Runs one repeat of the headless backend: an [`App`] and a renderer, stepped
/// by hand.
///
/// The scene's step runs before the frame and injects its scripted events
/// through the app, exactly as the window backend does.
fn run_headless(args: &Args, repeat: u32) -> Result<Vec<FrameRecord>, String> {
    let scene = args.scene;
    let (build, driver) = scene.mount();
    let mut app = App::new(app_config(args), build);
    let (width, height) = app.surface_size();
    let mut renderer = headless_renderer(args, width, height)?;
    // The first frame mounts and lays the tree out, so the events the scene
    // scripts land on laid out rectangles, as they do between the frames of a
    // real window. Its full repaint also fills the renderer's pixmap.
    let output = app.frame();
    renderer.render(&output);
    let warmup = u64::from(args.warmup);
    let mut records = Vec::with_capacity(args.frames as usize);
    for index in 0..warmup + u64::from(args.frames) {
        scene.step(&driver, &mut app, index);
        let allocations_before = allocations::count();
        let output = app.frame();
        let raster_start = Instant::now();
        renderer.render(&output);
        let latency_us = driver
            .borrow_mut()
            .take_injection()
            .map(|injected| us(injected.elapsed()));
        let raster_us = us(raster_start.elapsed());
        let stats = app.stats().clone();
        let allocations = allocations::count() - allocations_before;
        if index >= warmup {
            records.push(FrameRecord {
                repeat,
                frame: (index - warmup) as u32,
                stats,
                raster_us,
                present: None,
                latency_us,
                surface: (width, height),
                allocations,
            });
        }
    }
    Ok(records)
}

/// Builds the renderer the headless backend draws with.
///
/// A GPU renderer needs the bench's `gpu` build feature and an adapter to
/// draw on. A headless GPU frame is a texture the machine never shows, so any
/// adapter the environment resolves to will do, software ones included.
fn headless_renderer(args: &Args, width: u32, height: u32) -> Result<Box<dyn Renderer>, String> {
    match args.renderer {
        RendererArg::Cpu => Ok(Box::new(CpuRenderer::new(width, height))),
        #[cfg(not(feature = "gpu"))]
        RendererArg::Gpu => Err(String::from(
            "the renderer gpu needs a build with this crate's gpu feature",
        )),
        #[cfg(feature = "gpu")]
        RendererArg::Gpu => match smudge_gpu::GpuRenderer::headless(width, height) {
            Some(renderer) => Ok(Box::new(AwaitedGpu(renderer))),
            None => Err(String::from(
                "no GPU adapter is available for the headless renderer",
            )),
        },
    }
}

/// A GPU renderer whose render returns once the GPU has executed the frame,
/// so a headless frame's raster time and latency cover the GPU's work and not
/// only its submission.
#[cfg(feature = "gpu")]
struct AwaitedGpu(smudge_gpu::GpuRenderer);

#[cfg(feature = "gpu")]
impl Renderer for AwaitedGpu {
    fn render(&mut self, frame: &smudge_core::app::FrameOutput<'_>) {
        self.0.render(frame);
        self.0.wait();
    }
}

/// Runs every repeat of the window backend in one window, because the window
/// system's event loop cannot be recreated in one process.
///
/// The bench installs its driver and frame limit through the platform's
/// measurement hooks. The event hook drives the scene before every frame, so
/// a scripted event takes the same [`App::handle_event`] path a real one
/// takes, and the frame hook records the frames the run presents. Both take
/// their frame index modulo one repeat's length, so every repeat drives the
/// same sequence, and the process samples are taken at the repeat boundaries
/// the frame hook observes.
fn run_window(args: &Args) -> Result<Vec<RepeatRun>, PlatformError> {
    let scene = args.scene;
    let per_repeat = u64::from(args.warmup) + u64::from(args.frames);
    let repeats = u64::from(args.repeats);
    let (build, driver) = scene.mount();

    let records = Rc::new(RefCell::new(Vec::with_capacity(
        args.frames as usize * args.repeats as usize,
    )));
    let starts = Rc::new(RefCell::new(vec![None; args.repeats as usize]));
    let ends = Rc::new(RefCell::new(vec![None; args.repeats as usize]));
    let warmup = args.warmup;

    let on_frame: FrameHook = {
        let records = Rc::clone(&records);
        let starts = Rc::clone(&starts);
        let ends = Rc::clone(&ends);
        let driver = Rc::clone(&driver);
        let presented = Cell::new(0u64);
        let baseline = Cell::new(allocations::count());
        Rc::new(move |stats, present: PresentStats| {
            let frame_allocations = allocations::count() - baseline.get();
            let latency_us = driver
                .borrow_mut()
                .take_injection()
                .map(|injected| us(injected.elapsed()));
            let index = presented.get();
            presented.set(index + 1);
            let position = index % per_repeat;
            let repeat = (index / per_repeat) as usize;
            if position == 0
                && let Some(slot) = starts.borrow_mut().get_mut(repeat)
            {
                *slot = Some(process::sample());
            }
            if position == per_repeat - 1
                && let Some(slot) = ends.borrow_mut().get_mut(repeat)
            {
                *slot = Some(process::sample());
            }
            if position >= u64::from(warmup) {
                records.borrow_mut().push(FrameRecord {
                    repeat: repeat as u32,
                    frame: (position - u64::from(warmup)) as u32,
                    stats,
                    raster_us: us(present.raster),
                    present: Some(present),
                    latency_us,
                    surface: (present.surface_width, present.surface_height),
                    allocations: frame_allocations,
                });
            }
            baseline.set(allocations::count());
        })
    };

    let on_event: EventHook = {
        let driver = Rc::clone(&driver);
        let position = Cell::new(0u64);
        Rc::new(move |app| {
            let frame = position.get();
            position.set((frame + 1) % per_repeat);
            scene.step(&driver, app, frame);
        })
    };

    run_with(
        WindowOptions {
            title: format!(
                "smudge-bench {} {} {}",
                scene.key,
                mode_name(args.mode),
                args.backend.name(),
            ),
            size: scene.viewport,
            clear_color: CLEAR_COLOR,
            repaint: args.mode,
            show_damage: false,
            renderer: args.renderer.choice(),
            stats: None,
            on_frame: Some(on_frame),
            on_event: Some(on_event),
            frame_limit: Some(per_repeat * repeats),
        },
        build,
        |_| {},
    )?;

    let records = records.take();
    let starts = starts.take();
    let ends = ends.take();

    let mut runs: Vec<Vec<FrameRecord>> = Vec::with_capacity(args.repeats as usize);
    runs.resize_with(args.repeats as usize, Vec::new);
    for record in records {
        runs[record.repeat as usize].push(record);
    }
    let sample = |slots: Vec<Option<Sample>>| {
        slots
            .into_iter()
            .map(|slot| slot.unwrap_or_default())
            .collect::<Vec<_>>()
    };
    let starts = sample(starts);
    let ends = sample(ends);
    Ok(runs
        .into_iter()
        .zip(starts)
        .zip(ends)
        .map(|((records, start), end)| RepeatRun {
            records,
            start,
            end,
        })
        .collect())
}

/// Builds the app settings of a run.
fn app_config(args: &Args) -> AppConfig {
    AppConfig {
        viewport: args.scene.viewport,
        scale: args.scale,
        clear_color: CLEAR_COLOR,
        repaint: args.mode,
    }
}

/// Builds the environment header line.
///
/// `surface` is the size of the surface the first measured frame rendered,
/// or `None` when no frame was measured. The scale is `null` on the window
/// backend, because the window system chooses it. `renderer` names the
/// renderer that drew the frames: the choice asked for on the headless
/// backend, and the presenter the window actually used on the window backend,
/// which is how a GPU run proves it did not fall back to the CPU.
fn env_line(args: &Args, surface: Option<(u32, u32)>, renderer: &str) -> String {
    let scale = match args.backend {
        Backend::Headless => format!("{:.3}", args.scale),
        Backend::Window => String::from("null"),
    };
    format!(
        concat!(
            r#"{{"type":"env","label":{},"scene":{},"mode":{},"backend":{},"renderer":{},"#,
            r#""scale":{},"surface_width":{},"surface_height":{},"#,
            r#""cpu_model":{},"kernel":{},"rustc":{},"#,
            r#""wayland_display":{},"display":{},"timestamp":{}}}"#
        ),
        json_string(&args.label),
        json_string(args.scene.key),
        json_string(mode_name(args.mode)),
        json_string(args.backend.name()),
        json_string(renderer),
        scale,
        json_opt_u64(surface.map(|(width, _)| u64::from(width))),
        json_opt_u64(surface.map(|(_, height)| u64::from(height))),
        json_opt_string(&process::cpu_model()),
        json_opt_string(&process::kernel_release()),
        json_opt_string(&rustc_version()),
        env::var_os("WAYLAND_DISPLAY").is_some(),
        env::var_os("DISPLAY").is_some(),
        json_string(&utc_timestamp()),
    )
}

/// Builds one measured frame's line.
fn frame_line(record: &FrameRecord) -> String {
    let s = &record.stats;
    let present_us = record.present.map(|p| us(p.present));
    let acquire_us = record.present.map(|p| us(p.acquire));
    let bytes_copied = record.present.map(|p| p.bytes_copied);
    let buffer_age = record.present.map(|p| u64::from(p.buffer_age));
    format!(
        concat!(
            r#"{{"type":"frame","repeat":{},"frame":{},"#,
            r#""flush_us":{:.3},"layout_us":{:.3},"paint_us":{:.3},"list_us":{:.3},"#,
            r#""damage_us":{:.3},"raster_us":{:.3},"acquire_us":{},"present_us":{},"frame_total_us":{:.3},"#,
            r#""latency_us":{},"layout_ran":{},"nodes_painted":{},"flush_rounds":{},"#,
            r#""damage_rects":{},"damage_pixels":{},"damage_pixels_before_merge":{},"#,
            r#""raster_pixels":{},"bytes_copied":{},"allocations":{},"buffer_age":{}}}"#
        ),
        record.repeat,
        record.frame,
        us(s.flush),
        us(s.layout),
        us(s.paint),
        us(s.list),
        us(s.damage),
        record.raster_us,
        json_opt_f64(acquire_us),
        json_opt_f64(present_us),
        record.total_us(),
        json_opt_f64(record.latency_us),
        s.layout_ran,
        s.nodes_painted,
        s.flush_rounds,
        s.damage_rects,
        s.damage_pixels,
        s.damage_pixels_before_merge,
        s.damage_pixels,
        json_opt_u64(bytes_copied),
        record.allocations,
        json_opt_u64(buffer_age),
    )
}

/// Builds one repeat's summary line.
fn repeat_summary_line(repeat: u32, records: &[FrameRecord], start: Sample, end: Sample) -> String {
    format!(
        concat!(
            r#"{{"type":"repeat_summary","repeat":{},"frames":{},"#,
            r#""peak_rss_kb_start":{},"peak_rss_kb_end":{},"#,
            r#""cpu_time_s_start":{},"cpu_time_s_end":{},{}}}"#
        ),
        repeat,
        records.len(),
        json_opt_u64(start.peak_rss_kb),
        json_opt_u64(end.peak_rss_kb),
        json_opt_f64(start.cpu_time_s),
        json_opt_f64(end.cpu_time_s),
        percentile_columns(records),
    )
}

/// Builds the summary line across all repeats.
fn pooled_summary_line(records: &[FrameRecord]) -> String {
    format!(
        r#"{{"type":"summary","frames":{},{}}}"#,
        records.len(),
        percentile_columns(records),
    )
}

/// Builds the percentile columns that both summary lines share.
fn percentile_columns(records: &[FrameRecord]) -> String {
    let totals: Vec<f64> = records.iter().map(FrameRecord::total_us).collect();
    let mut columns: Vec<(&str, Vec<f64>)> = vec![
        ("frame_total_us", totals),
        ("flush_us", phase_values(records, |s| us(s.flush))),
        ("layout_us", phase_values(records, |s| us(s.layout))),
        ("paint_us", phase_values(records, |s| us(s.paint))),
        ("list_us", phase_values(records, |s| us(s.list))),
        ("damage_us", phase_values(records, |s| us(s.damage))),
        ("raster_us", record_values(records, |r| r.raster_us)),
    ];
    let with_present = records
        .first()
        .is_some_and(|record| record.present.is_some());
    if with_present {
        columns.push((
            "acquire_us",
            record_values(records, |r| r.present.map_or(0.0, |p| us(p.acquire))),
        ));
        columns.push((
            "present_us",
            record_values(records, |r| r.present.map_or(0.0, |p| us(p.present))),
        ));
    } else {
        columns.push(("acquire_us", Vec::new()));
        columns.push(("present_us", Vec::new()));
    }
    columns.push((
        "latency_us",
        records
            .iter()
            .filter_map(|record| record.latency_us)
            .collect(),
    ));

    columns
        .iter()
        .map(|(name, values)| {
            let values_json = if values.is_empty() {
                String::from("null")
            } else {
                percentiles_object(values)
            };
            format!(r#""{name}":{values_json}"#)
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Collects one frame-pipeline phase of every record, in microseconds.
fn phase_values(records: &[FrameRecord], get: impl Fn(&FrameStats) -> f64) -> Vec<f64> {
    records.iter().map(|record| get(&record.stats)).collect()
}

/// Collects one value of every record.
fn record_values(records: &[FrameRecord], get: impl Fn(&FrameRecord) -> f64) -> Vec<f64> {
    records.iter().map(get).collect()
}

/// Formats one quantity's percentiles.
fn percentiles_object(values: &[f64]) -> String {
    let p = Percentiles::of(values);
    format!(
        r#"{{"median":{:.3},"p95":{:.3},"p99":{:.3}}}"#,
        p.median, p.p95, p.p99,
    )
}

/// Returns the version line of the compiler that is on the path.
fn rustc_version() -> Option<String> {
    let output = std::process::Command::new("rustc")
        .arg("-V")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8(output.stdout).ok()?;
    let version = version.trim();
    (!version.is_empty()).then(|| version.to_string())
}

/// Formats a string as a JSON string, escaping what JSON reserves.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            control if (control as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Formats an optional string as a JSON string or `null`.
fn json_opt_string(value: &Option<String>) -> String {
    value
        .as_deref()
        .map_or_else(|| "null".to_string(), json_string)
}

/// Formats a whole number or `null`.
fn json_opt_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "null".to_string(), |v| v.to_string())
}

/// Formats a number or `null`.
fn json_opt_f64(value: Option<f64>) -> String {
    value.map_or_else(|| "null".to_string(), |v| format!("{v:.3}"))
}

/// Returns the current time as an ISO 8601 timestamp in UTC.
fn utc_timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    let (hour, minute, second) = (rest / 3_600, rest % 3_600 / 60, rest % 60);

    // The civil date comes from days since the epoch by the standard
    // days-to-year-month-day conversion.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_pointer = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_pointer + 2) / 5 + 1;
    let month = if month_pointer < 10 {
        month_pointer + 3
    } else {
        month_pointer - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Converts a duration to microseconds.
fn us(duration: Duration) -> f64 {
    duration.as_nanos() as f64 / 1000.0
}
