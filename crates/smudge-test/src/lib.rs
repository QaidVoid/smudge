//! Headless test harness, damage equivalence checks and snapshot helpers.
//!
//! Tests drive the same frame pipeline a window does, so behavior observed here
//! matches what a running application shows. [`Harness`] runs an [`App`] and a
//! [`CpuRenderer`] together without a window.

#![forbid(unsafe_code)]

use std::cell::Cell;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use web_time::Instant;

use smudge_core::app::{App, AppConfig, FrameOutput, RepaintMode};
use smudge_core::element::IntoElement;
use smudge_core::geometry::{Color, PixelRect, Point, Size};
use smudge_core::input::{InputEvent, Key, KeyEvent, Modifiers, PointerButton};
use smudge_core::paint::DisplayList;
use smudge_core::timer::VirtualClock;
use smudge_cpu::CpuRenderer;

/// The color the harness clears the surface with before it replays a frame.
const CLEAR_COLOR: Color = Color::WHITE;

/// The most frames [`Harness::settle`] runs before it gives up on the app
/// settling.
const SETTLE_FRAMES: u32 = 8;

/// What one frame did, copied out of the app so the harness stays usable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameReport {
    /// The physical rectangles the frame repainted.
    pub damage: Vec<PixelRect>,
    /// Whether the frame repainted the whole surface.
    pub full: bool,
    /// How many nodes the paint pass regenerated.
    pub nodes_painted: u32,
    /// Whether the layout pass ran the layout engine.
    pub layout_ran: bool,
}

/// An app and a CPU renderer driven together without a window.
///
/// Every frame runs [`App::frame`] and then hands its output to the renderer,
/// so the surface holds what a window would show. The harness keeps the last
/// display list and its settings so that
/// [`Harness::assert_matches_full_repaint`] can compare the damaged surface
/// with a full repaint.
///
/// # Examples
///
/// ```
/// use smudge_core::geometry::Color;
/// use smudge_core::reactive::{prelude::*, signal_local};
/// use smudge_core::style::px;
/// use smudge_core::widget::panel;
/// use smudge_test::Harness;
///
/// let (color, set_color) = signal_local(Color::BLACK);
/// let mut harness = Harness::new(120.0, 80.0, 1.0, move || {
///     panel()
///         .width(px(20.0))
///         .height(px(10.0))
///         .background(move || color.get().into())
/// });
///
/// assert!(harness.frame().full, "the first frame repaints everything");
///
/// set_color.set(Color::WHITE);
/// let changed = harness.frame();
/// assert_eq!(changed.nodes_painted, 1);
///
/// harness.assert_matches_full_repaint();
/// ```
pub struct Harness {
    /// The app whose frames the harness runs.
    app: App,
    /// The renderer that accumulates the damaged rectangles.
    renderer: CpuRenderer,
    /// Whether a frame has run, so there is a display list to compare.
    framed: bool,
    /// The display list of the last frame, kept for the comparison.
    list: DisplayList,
    /// The damage of the last frame, kept for the failure message.
    damage: Vec<PixelRect>,
    /// The scale of the last frame.
    scale: f32,
    /// The clear color of the last frame.
    clear_color: Color,
    /// Whether a signal write woke the app since the last frame began.
    woken: Rc<Cell<bool>>,
    /// The clock the app's timers read, which only
    /// [`Harness::advance`] moves.
    clock: VirtualClock,
}

impl Harness {
    /// Creates a harness for a surface of `width` by `height` logical pixels at
    /// `scale` physical pixels per logical pixel.
    ///
    /// `build` runs once, exactly as it does in [`App::new`], and the first
    /// call to [`Harness::frame`] lays the tree out and repaints the whole
    /// surface. The app runs on a virtual clock that only
    /// [`Harness::advance`] moves, including for timers `build` creates.
    pub fn new<E: IntoElement>(
        width: f32,
        height: f32,
        scale: f32,
        build: impl FnOnce() -> E,
    ) -> Harness {
        let clock = VirtualClock::new(Instant::now());
        let app = App::with_clock(
            AppConfig {
                viewport: Size { width, height },
                scale,
                clear_color: CLEAR_COLOR,
                repaint: RepaintMode::Damage,
            },
            clock.clone().into(),
            build,
        );
        let woken = Rc::new(Cell::new(false));
        app.set_wake_callback({
            let woken = Rc::clone(&woken);
            move || woken.set(true)
        });
        let (surface_width, surface_height) = app.surface_size();
        Harness {
            renderer: CpuRenderer::new(surface_width, surface_height),
            app,
            framed: false,
            list: DisplayList::new(),
            damage: Vec::new(),
            scale,
            clear_color: CLEAR_COLOR,
            woken,
            clock,
        }
    }

    /// Runs one frame and repaints its damage into the surface.
    ///
    /// The renderer is resized first when the app's surface no longer matches
    /// it, which happens after [`Harness::app`] changes the viewport. A
    /// viewport change forces a full frame, so the resized surface is repainted
    /// in full.
    ///
    /// # Panics
    ///
    /// Panics in debug builds when the update queue does not settle, or when a
    /// binding keeps waking itself. See [`App::frame`].
    pub fn frame(&mut self) -> FrameReport {
        let Harness {
            app,
            renderer,
            framed,
            list,
            damage,
            scale,
            clear_color,
            woken,
            ..
        } = self;
        woken.set(false);
        let (width, height) = app.surface_size();
        if (renderer.width(), renderer.height()) != (width, height) {
            renderer.resize(width, height);
        }
        let full = {
            let output = app.frame();
            renderer.render(&output);
            *list = output.list.clone();
            *damage = output.damage.to_vec();
            *scale = output.scale;
            *clear_color = output.clear_color;
            *framed = true;
            output.full
        };

        let stats = app.stats();
        FrameReport {
            damage: damage.clone(),
            full,
            nodes_painted: stats.nodes_painted,
            layout_ran: stats.layout_ran,
        }
    }

    /// Returns whether a signal write woke the app since the last frame
    /// began, which is how a window learns that it needs another frame.
    ///
    /// A write between frames wakes the app, and so does a write made while
    /// a frame paints, for example a size a widget reports, which the next
    /// frame applies. A write the frame applies itself, from a timer, the
    /// tick or a bind function, wakes nothing, as
    /// [`App::set_wake_callback`] describes. The harness installs the app's
    /// wake callback
    /// itself, so a test that replaces it through [`Harness::app`] turns this
    /// off.
    pub fn woken(&self) -> bool {
        self.woken.get()
    }

    /// Runs frames until the app asks for no more, as a window would, and
    /// returns how many ran.
    ///
    /// The app asks for a frame when a signal write woke it, see
    /// [`Harness::woken`], or when [`App::needs_frame`] reports work.
    ///
    /// # Panics
    ///
    /// Panics when the app still asks for a frame after eight frames, which
    /// means something wakes it on every frame.
    pub fn settle(&mut self) -> u32 {
        let mut frames = 0;
        while self.woken() || self.app.needs_frame() {
            assert!(
                frames < SETTLE_FRAMES,
                "the app still asked for a frame after {SETTLE_FRAMES} frames, \
                 so something wakes it on every frame"
            );
            self.frame();
            frames += 1;
        }
        frames
    }

    /// Returns the app, so a test can change the viewport, the repaint mode or
    /// the tick callback.
    pub fn app(&mut self) -> &mut App {
        &mut self.app
    }

    /// Injects one input event into the app, between frames.
    ///
    /// The event goes through [`App::handle_event`], exactly as a window
    /// delivers it. A handler's change to the tree becomes visible in the
    /// next call to [`Harness::frame`], which a test calls itself.
    pub fn event(&mut self, event: InputEvent) {
        self.app.handle_event(event);
    }

    /// Injects a pointer move to `position`, in logical pixels.
    pub fn move_to(&mut self, position: Point) {
        self.event(InputEvent::PointerMoved { position });
    }

    /// Injects a primary-button click at `position`, in logical pixels.
    ///
    /// The pointer moves there first, then the button goes down and up at
    /// that position, the three events a real click delivers in order.
    pub fn click(&mut self, position: Point) {
        self.move_to(position);
        self.event(InputEvent::PointerDown {
            position,
            button: PointerButton::Primary,
        });
        self.event(InputEvent::PointerUp {
            position,
            button: PointerButton::Primary,
        });
    }

    /// Injects a press and release of `key`, without modifiers or repeat.
    pub fn press_key(&mut self, key: Key) {
        self.press_key_with(key, Modifiers::default());
    }

    /// Injects a press and release of `key` with `modifiers` held, without
    /// repeat.
    pub fn press_key_with(&mut self, key: Key, modifiers: Modifiers) {
        self.event(InputEvent::Key(KeyEvent {
            key: key.clone(),
            pressed: true,
            repeat: false,
            modifiers,
            text: None,
        }));
        self.event(InputEvent::Key(KeyEvent {
            key,
            pressed: false,
            repeat: false,
            modifiers,
            text: None,
        }));
    }

    /// Injects `text` as typed input, one key press and release per character.
    ///
    /// Every press carries its character as the event's text, so an editor
    /// can insert it without reading the key name.
    pub fn type_text(&mut self, text: &str) {
        for character in text.chars() {
            let key = Key::Character(character.to_string());
            self.event(InputEvent::Key(KeyEvent {
                key: key.clone(),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::default(),
                text: Some(character.to_string()),
            }));
            self.event(InputEvent::Key(KeyEvent {
                key,
                pressed: false,
                repeat: false,
                modifiers: Modifiers::default(),
                text: None,
            }));
        }
    }

    /// Injects a primary-button press at `position`, in logical pixels.
    ///
    /// The pointer moves there first. Pair with
    /// [`Harness::button_up`](Self::button_up) and
    /// [`Harness::move_to`](Self::move_to) to drag.
    pub fn button_down(&mut self, position: Point) {
        self.move_to(position);
        self.event(InputEvent::PointerDown {
            position,
            button: PointerButton::Primary,
        });
    }

    /// Injects the release of a primary-button press at `position`, in
    /// logical pixels.
    pub fn button_up(&mut self, position: Point) {
        self.event(InputEvent::PointerUp {
            position,
            button: PointerButton::Primary,
        });
    }

    /// Moves the app's clock forward by `by` and runs one frame, which is
    /// when timers due in that time run.
    ///
    /// The harness keeps the app on a virtual clock, so timers, caret
    /// blinking and other timing behave exactly as their durations say,
    /// without the test sleeping.
    ///
    /// # Panics
    ///
    /// Panics under the conditions of [`App::frame`].
    pub fn advance(&mut self, by: Duration) -> FrameReport {
        self.clock.advance(by);
        self.frame()
    }

    /// Encodes the current surface as a PNG image.
    ///
    /// # Panics
    ///
    /// Panics when the PNG encoder fails, which it cannot when encoding into
    /// memory. See [`CpuRenderer::encode_png`].
    pub fn png(&self) -> Vec<u8> {
        self.renderer.encode_png()
    }

    /// Asserts that the surface equals a full repaint of the last frame.
    ///
    /// Call it after at least one call to [`Harness::frame`], because it
    /// compares against the display list and settings that frame left. A fresh
    /// renderer replays that list over the whole surface, and its bytes must
    /// match the surface the damaged frames produced.
    ///
    /// # Panics
    ///
    /// Panics at the first pixel whose bytes differ, naming the pixel, both
    /// values and the damage of the last frame. It also writes the damaged and
    /// the full image under `target/smudge-test-failures/<test name>/`, where
    /// the test name is the name of the thread that runs the test.
    ///
    /// Also panics when no frame has run yet, because there is nothing to
    /// compare.
    pub fn assert_matches_full_repaint(&self) {
        assert!(
            self.framed,
            "assert_matches_full_repaint compares against the last frame, so call \
             Harness::frame first"
        );
        let width = self.renderer.width();
        let height = self.renderer.height();
        let mut full = CpuRenderer::new(width, height);
        full.render(&FrameOutput {
            list: &self.list,
            damage: &[PixelRect {
                x: 0,
                y: 0,
                width,
                height,
            }],
            full: true,
            clear_color: self.clear_color,
            scale: self.scale,
        });
        assert_surfaces_equal(
            &self.renderer,
            &full,
            &self.damage,
            "the damaged surface differs from a full repaint",
            "full.png",
        );
    }

    /// Asserts that the surface equals the surface of `expected`, byte for
    /// byte.
    ///
    /// Both harnesses must have run at least one frame on surfaces of one size,
    /// which a fresh mount of the same scene at the same viewport and scale
    /// gives. Where [`Harness::assert_matches_full_repaint`] repaints the
    /// display list the frame left and so cannot see a stale list, this
    /// comparison fails whenever the two trees disagree about what the screen
    /// shows.
    ///
    /// # Panics
    ///
    /// Panics at the first pixel whose bytes differ, naming the pixel, both
    /// values and the damage of the last frame of `self`. It also writes both
    /// images under `target/smudge-test-failures/<test name>/`, where the test
    /// name is the name of the thread that runs the test.
    ///
    /// Also panics when either harness has not run a frame yet, because there
    /// is nothing to compare, or when the two surfaces differ in size, because
    /// the frames were made for different viewports.
    pub fn assert_matches(&self, expected: &Harness) {
        assert!(
            self.framed && expected.framed,
            "assert_matches compares two surfaces, so call Harness::frame on \
             both harnesses first"
        );
        assert_eq!(
            (self.renderer.width(), self.renderer.height()),
            (expected.renderer.width(), expected.renderer.height()),
            "assert_matches compares surfaces of one size, so both harnesses \
             must run at the same viewport and scale"
        );
        assert_surfaces_equal(
            &self.renderer,
            &expected.renderer,
            &self.damage,
            "the surface differs from the expected surface",
            "expected.png",
        );
    }

    /// Returns one line per visible node: its depth as indentation, the name of
    /// its widget type without the path or generic arguments, and its absolute
    /// rectangle.
    ///
    /// The root is at depth zero and each level indents by two spaces. The
    /// rectangle is printed as its two corners, `x0, y0, x1, y1`, in logical
    /// pixels. A hidden node and its subtree are left out, because they are
    /// laid out as empty and paint nothing.
    pub fn layout_dump(&self) -> String {
        let mut dump = String::new();
        self.app.visit_nodes(|depth, type_name, rect| {
            let path = type_name.split('<').next().unwrap_or(type_name);
            let name = path.rsplit("::").next().unwrap_or(path);
            let _ = writeln!(
                dump,
                "{:indent$}{name} [{}, {}, {}, {}]",
                "",
                rect.x0,
                rect.y0,
                rect.x1,
                rect.y1,
                indent = depth * 2
            );
        });
        dump
    }
}

/// Panics when the two surfaces differ, at the first pixel that does.
///
/// The panic names the pixel, both values and `damage`, which is the damage of
/// the frame that produced `damaged`, and writes both images under
/// `target/smudge-test-failures/<test name>/` with the damaged image called
/// `damaged.png` and the expected one `expected_name`.
fn assert_surfaces_equal(
    damaged: &CpuRenderer,
    expected: &CpuRenderer,
    damage: &[PixelRect],
    note: &str,
    expected_name: &str,
) {
    let width = damaged.width();
    let damaged_bytes = damaged.pixels();
    let expected_bytes = expected.pixels();
    let (damaged_pixels, _) = damaged_bytes.as_chunks::<4>();
    let (expected_pixels, _) = expected_bytes.as_chunks::<4>();
    let Some(pixel) = damaged_pixels
        .iter()
        .zip(expected_pixels)
        .position(|(damaged, expected)| damaged != expected)
    else {
        return;
    };
    let x = pixel % width as usize;
    let y = pixel / width as usize;
    let damaged_pixel = damaged_pixels[pixel];
    let expected_pixel = expected_pixels[pixel];
    let written = write_failure_images(damaged, expected, expected_name);
    panic!(
        "{note} at pixel ({x}, {y}): damaged {damaged_pixel:?}, expected \
         {expected_pixel:?}. The damage of the last frame was {damage:?}. \
         {written}"
    );
}

/// Writes both surfaces under `target/smudge-test-failures/<test name>/` and
/// returns a sentence describing the result, for the panic message.
fn write_failure_images(
    damaged: &CpuRenderer,
    expected: &CpuRenderer,
    expected_name: &str,
) -> String {
    let directory = failure_directory();
    if let Err(error) = std::fs::create_dir_all(&directory) {
        return format!("Could not create {}: {error}.", directory.display());
    }
    let directory = std::fs::canonicalize(&directory).unwrap_or(directory);
    for (name, renderer) in [("damaged.png", damaged), (expected_name, expected)] {
        let path = directory.join(name);
        if let Err(error) = std::fs::write(&path, renderer.encode_png()) {
            return format!("Could not write {}: {error}.", path.display());
        }
    }
    format!("Wrote both images to {}.", directory.display())
}

/// Returns the directory that holds the images of a failed comparison for the
/// test that is running.
///
/// The directory sits in the workspace's `target` directory, which the build
/// keeps out of version control.
fn failure_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("smudge-test-failures")
        .join(current_test_name())
}

/// Returns the name of the running test as one path component.
///
/// The test harness names each test's thread after the test, and a test name
/// can hold characters that a path cannot.
fn current_test_name() -> String {
    let name = std::thread::current()
        .name()
        .unwrap_or("unknown")
        .to_owned();
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect()
}
