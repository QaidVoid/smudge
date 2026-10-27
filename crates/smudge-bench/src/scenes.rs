//! The fixed scenes the benchmark measures.
//!
//! A scene is a tree of panels plus a driver that writes the same state for
//! the same frame index, so every repeat of a run measures identical work.
//! A window run mounts once, so a scene whose state outlives a repeat resets
//! it at the start of the next.
//! The driver's step runs before the frame and receives the app, so the
//! scenes that script input deliver it through the same
//! [`App::handle_event`](smudge_core::app::App::handle_event) path a real
//! event takes. Variation comes from a small xorshift generator seeded per
//! scene, so no random-number dependency is needed.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use smudge_core::app::App;
use smudge_core::element::Element;
use smudge_core::geometry::{Color, Point, Size};
use smudge_core::input::{InputEvent, Key, KeyEvent, Modifiers, PointerButton};
use smudge_core::reactive::{self, prelude::*};
use smudge_core::style::{Align, px};
use smudge_core::widget::{Panel, panel};
use smudge_text::{TextInputExt, label, text_input};
use smudge_widgets::{column, row, scroll, virtual_list};

/// The columns and rows of the box grid the first two scenes share.
const GRID: (usize, usize) = (30, 20);

/// The boxes the grid holds.
const BOXES: usize = GRID.0 * GRID.1;

/// The side of one box in logical pixels.
const BOX_SIZE: f32 = 38.0;

/// The space between adjacent boxes in logical pixels.
const GAP: f32 = 2.0;

/// The width of the box grid in logical pixels, so it wraps after
/// [`GRID`]'s column count whatever the viewport.
const GRID_WIDTH: f32 = GRID.0 as f32 * BOX_SIZE + (GRID.0 - 1) as f32 * GAP;

/// The size of the animated box of the counter scene, in logical pixels.
const COUNTER_SIZE: (f32, f32) = (80.0, 24.0);

/// The variation seed of each scene, reused as the base of its per-frame
/// seeds.
const S1_SEED: u64 = 0x5EED_0001;
const S2_SEED: u64 = 0x5EED_0002;
const S3_SEED: u64 = 0x5EED_0003;
const S5_SEED: u64 = 0x5EED_0005;

/// The viewport of the scenes added for this round, in logical pixels.
const VIEWPORT: (f32, f32) = (1280.0, 800.0);

/// How many labels the text scene lays out and relayouts every frame.
const LABELS: usize = 300;

/// How many rows the scroll scene's content holds.
const SCROLL_ROWS: usize = 1000;

/// How many rows the fixed list scene's list holds.
const LIST_ROWS: usize = 100_000;

/// The height of one row of the scroll scene and one row of the fixed list
/// scene, in logical pixels.
const ROW_HEIGHT: f32 = 24.0;

/// How many frames one direction of the scroll scene's wheel script lasts.
const SWEEP_FRAMES: u64 = 50;

/// How far one wheel event of the scroll scene scrolls, in logical pixels.
const WHEEL_STEP: f32 = 48.0;

/// The size and position of the text field of the input scene.
const FIELD_SIZE: (f32, f32) = (480.0, 36.0);

/// Where the input scene clicks to focus its field, in logical pixels.
const FIELD_POINT: Point = Point { x: 40.0, y: 40.0 };

/// Sets the color of one animated part of a scene.
type SetColor = Box<dyn Fn(Color)>;

/// Sets the text of one animated label of a scene.
type SetText = Box<dyn Fn(String)>;

/// The state one mounted scene drives.
///
/// The setters are the signals' write ends, in the order the scene mounted
/// them. [`Scene::step`] writes one value to each of a scene's setters per
/// frame.
pub(crate) struct Driver {
    /// The write ends of the animated colors, in mount order.
    colors: Vec<SetColor>,
    /// The write ends of the animated texts, in mount order.
    texts: Vec<SetText>,
    /// The clock time the last scripted event was injected at, for the
    /// latency record of the input scene.
    injected_at: Option<Instant>,
}

impl Driver {
    /// Creates an empty driver.
    fn new() -> Driver {
        Driver {
            colors: Vec::new(),
            texts: Vec::new(),
            injected_at: None,
        }
    }

    /// Records the write end of one animated color.
    fn register_color(&mut self, set_color: SetColor) {
        self.colors.push(set_color);
    }

    /// Records the write end of one animated text.
    fn register_text(&mut self, set_text: SetText) {
        self.texts.push(set_text);
    }

    /// Stamps the scripted event the step is about to inject.
    fn note_injection(&mut self) {
        self.injected_at = Some(Instant::now());
    }

    /// Returns the stamp of the injected event, and clears it.
    pub(crate) fn take_injection(&mut self) -> Option<Instant> {
        self.injected_at.take()
    }
}

/// A shared driver, so the app's build closure and the frame tick can reach
/// the same scene state.
pub(crate) type Shared = Rc<RefCell<Driver>>;

/// One fixed scene: how to mount it and how to advance one frame.
pub(crate) struct Scene {
    /// The key the command line selects the scene with.
    pub(crate) key: &'static str,
    /// The viewport of the scene in logical pixels.
    pub(crate) viewport: Size,
    /// Mounts the element tree and records the animated state.
    mount: fn(&mut Driver) -> Element<Panel>,
    /// Advances one frame: writes the animated state and injects any scripted
    /// event into `app`.
    step: fn(&Shared, &mut App, u64),
}

impl Scene {
    /// The scenes of the suite, in the order the protocol runs them.
    pub(crate) fn all() -> &'static [Scene] {
        &SCENES
    }

    /// Returns the scene the command line key selects.
    pub(crate) fn by_key(key: &str) -> Option<&'static Scene> {
        Scene::all().iter().find(|scene| scene.key == key)
    }

    /// Mounts the scene's element tree and returns it with its driver.
    ///
    /// The returned closure creates the scene's signals, so it must run where
    /// the reactive runtime expects a build closure. Pass it to
    /// [`smudge_core::app::App::new`] or the platform's run functions as the
    /// build closure, and give the driver to the frame tick.
    pub(crate) fn mount(&self) -> (impl FnOnce() -> Element<Panel>, Shared) {
        let driver: Shared = Rc::new(RefCell::new(Driver::new()));
        let fill = Rc::clone(&driver);
        let mount = self.mount;
        let build = move || (mount)(&mut fill.borrow_mut());
        (build, driver)
    }

    /// Writes the state of `frame` into the scene and injects its scripted
    /// event into `app`.
    ///
    /// The values are a function of the frame index alone, so a run is
    /// reproducible from its seed. The step runs before the frame, so the
    /// frame it precedes shows what it wrote.
    pub(crate) fn step(&self, driver: &Shared, app: &mut App, frame: u64) {
        (self.step)(driver, app, frame);
    }
}

/// The scenes of the suite, keyed as the command line names them.
///
/// The first scene is the raster-heavy case: 600 rounded, bordered boxes in a
/// 30 by 20 grid, and every box changes color every frame. The second is the
/// best case for damage: the same 600 boxes, static, plus one small box that
/// changes color every frame. The third is the worst case for damage: the
/// root background changes every frame and covers everything. The fourth
/// lays out and repaints 300 labels whose text changes every frame. The
/// fifth and sixth scroll long content with scripted wheel events: a
/// thousand mixed rows, and a virtual list of a hundred thousand rows. The
/// last types into a focused text field, one keystroke per frame.
static SCENES: [Scene; 7] = [
    Scene {
        key: "s1",
        viewport: Size {
            width: 1280.0,
            height: 800.0,
        },
        mount: mount_boxes,
        step: step_boxes,
    },
    Scene {
        key: "s2",
        viewport: Size {
            width: 1920.0,
            height: 1080.0,
        },
        mount: mount_counter,
        step: step_counter,
    },
    Scene {
        key: "s3",
        viewport: Size {
            width: 1280.0,
            height: 800.0,
        },
        mount: mount_full_change,
        step: step_full_change,
    },
    Scene {
        key: "s4",
        viewport: Size {
            width: VIEWPORT.0,
            height: VIEWPORT.1,
        },
        mount: mount_labels,
        step: step_labels,
    },
    Scene {
        key: "s5",
        viewport: Size {
            width: VIEWPORT.0,
            height: VIEWPORT.1,
        },
        mount: mount_scroll,
        step: step_scroll,
    },
    Scene {
        key: "s6",
        viewport: Size {
            width: VIEWPORT.0,
            height: VIEWPORT.1,
        },
        mount: mount_fixed_list,
        step: step_fixed_list,
    },
    Scene {
        key: "s9",
        viewport: Size {
            width: VIEWPORT.0,
            height: VIEWPORT.1,
        },
        mount: mount_input,
        step: step_input,
    },
];

/// Mounts the animated box grid of the first scene.
fn mount_boxes(driver: &mut Driver) -> Element<Panel> {
    let mut rng = Xorshift::new(S1_SEED);
    let mut grid = box_grid();
    for _ in 0..BOXES {
        let (color, set_color) = reactive::signal_local(rng.color());
        driver.register_color(Box::new(move |color| set_color.set(color)));
        grid = grid.child(bordered_box().background(move || color.get().into()));
    }
    panel().align_items(Align::Start).child(grid)
}

/// Writes one fresh color to every box of the first scene.
fn step_boxes(driver: &Shared, _app: &mut App, frame: u64) {
    let mut rng = Xorshift::new(frame_seed(S1_SEED, frame));
    let driver = &mut *driver.borrow_mut();
    for set_color in &driver.colors {
        set_color(rng.color());
    }
}

/// Mounts the static box grid and the animated counter box of the second
/// scene.
fn mount_counter(driver: &mut Driver) -> Element<Panel> {
    let mut rng = Xorshift::new(S2_SEED);
    let mut grid = box_grid();
    for _ in 0..BOXES {
        grid = grid.child(bordered_box().background(rng.color()));
    }
    let (color, set_color) = reactive::signal_local(rng.color());
    driver.register_color(Box::new(move |color| set_color.set(color)));
    let counter = bordered_box()
        .width(px(COUNTER_SIZE.0))
        .height(px(COUNTER_SIZE.1))
        .background(move || color.get().into());
    panel()
        .align_items(Align::Start)
        .column()
        .gap(Size {
            width: 0.0,
            height: 16.0,
        })
        .child(counter)
        .child(grid)
}

/// Writes one fresh color to the counter box of the second scene.
fn step_counter(driver: &Shared, _app: &mut App, frame: u64) {
    let mut rng = Xorshift::new(frame_seed(S2_SEED, frame));
    let driver = driver.borrow_mut();
    if let Some(set_color) = driver.colors.first() {
        set_color(rng.color());
    }
}

/// Mounts the scene whose root background covers everything and changes every
/// frame.
fn mount_full_change(driver: &mut Driver) -> Element<Panel> {
    let (color, set_color) = reactive::signal_local(Xorshift::new(S3_SEED).color());
    driver.register_color(Box::new(move |color| set_color.set(color)));
    panel().background(move || color.get().into())
}

/// Writes one fresh color to the root background of the third scene.
fn step_full_change(driver: &Shared, _app: &mut App, frame: u64) {
    let mut rng = Xorshift::new(frame_seed(S3_SEED, frame));
    let driver = driver.borrow_mut();
    if let Some(set_color) = driver.colors.first() {
        set_color(rng.color());
    }
}

/// Mounts the wrapped label grid of the text scene.
fn mount_labels(driver: &mut Driver) -> Element<Panel> {
    let mut grid = box_grid();
    for _ in 0..LABELS {
        let (text, set_text) = reactive::signal_local(String::new());
        driver.register_text(Box::new(move |text| set_text.set(text)));
        grid = grid.child(label(move || text.get()));
    }
    panel().align_items(Align::Start).child(grid)
}

/// Writes one fresh text into every label of the text scene.
fn step_labels(driver: &Shared, _app: &mut App, frame: u64) {
    let driver = &mut *driver.borrow_mut();
    for (index, set_text) in driver.texts.iter().enumerate() {
        set_text(label_text(index, frame));
    }
}

/// Returns the text of one label of the text scene at one frame.
fn label_text(index: usize, frame: u64) -> String {
    format!("Label {index:03} frame {frame:05}")
}

/// Mounts the scroll scene: a thousand mixed rows in a scroll view.
fn mount_scroll(_driver: &mut Driver) -> Element<Panel> {
    let mut rng = Xorshift::new(S5_SEED);
    let mut list = column();
    for index in 0..SCROLL_ROWS {
        list = list.child(
            row()
                .gap(Size {
                    width: 12.0,
                    height: 0.0,
                })
                .height(px(ROW_HEIGHT))
                .align_items(Align::Center)
                .child(
                    panel()
                        .width(px(24.0))
                        .height(px(24.0))
                        .corner_radius(6.0)
                        .background(rng.color()),
                )
                .child(label(format!("Row {index}"))),
        );
    }
    panel().child(scroll(list).width(px(VIEWPORT.0)).height(px(VIEWPORT.1)))
}

/// Scrolls the scroll scene with one scripted wheel event per frame.
///
/// The script scrolls down for [`SWEEP_FRAMES`] frames and back up for the
/// same, from the top at the start of every repeat.
fn step_scroll(driver: &Shared, app: &mut App, frame: u64) {
    if frame == 0 {
        start_scroll_repeat(app, SCROLL_ROWS);
    }
    let down = (frame / SWEEP_FRAMES).is_multiple_of(2);
    let delta = if down { WHEEL_STEP } else { -WHEEL_STEP };
    app.handle_event(InputEvent::Wheel {
        delta: Point { x: 0.0, y: delta },
    });
    driver.borrow_mut().note_injection();
}

/// Mounts the fixed list scene: a hundred thousand rows of one height.
fn mount_fixed_list(_driver: &mut Driver) -> Element<Panel> {
    panel().child(
        virtual_list(
            move || LIST_ROWS,
            ROW_HEIGHT,
            |index| label(format!("Item {index:06}")),
        )
        .width(px(VIEWPORT.0))
        .height(px(VIEWPORT.1)),
    )
}

/// Scrolls the fixed list scene one row height per frame.
fn step_fixed_list(_driver: &Shared, app: &mut App, frame: u64) {
    if frame == 0 {
        start_scroll_repeat(app, LIST_ROWS);
    }
    app.handle_event(InputEvent::Wheel {
        delta: Point {
            x: 0.0,
            y: ROW_HEIGHT,
        },
    });
}

/// Starts one repeat of a scrolling scene whose content holds `rows` rows.
///
/// It places the pointer over the view, because a wheel event goes to the
/// chain under the pointer, and scrolls back past the top. A window run
/// mounts once, so without the rewind a repeat would start where the last
/// one ended and show other rows.
fn start_scroll_repeat(app: &mut App, rows: usize) {
    app.handle_event(InputEvent::PointerMoved {
        position: Point {
            x: VIEWPORT.0 / 2.0,
            y: VIEWPORT.1 / 2.0,
        },
    });
    app.handle_event(InputEvent::Wheel {
        delta: Point {
            x: 0.0,
            y: -(rows as f32 * ROW_HEIGHT),
        },
    });
}

/// Mounts the input scene: one text field with room for its caret.
fn mount_input(driver: &mut Driver) -> Element<Panel> {
    let value = reactive::RwSignal::new(String::new());
    let writer = value.clone();
    driver.register_text(Box::new(move |text| writer.set(text)));
    panel().padding(px(24.0)).child(
        text_input(value)
            .width(px(FIELD_SIZE.0))
            .height(px(FIELD_SIZE.1))
            .placeholder(String::from("Type here")),
    )
}

/// Types into the input scene's field, one keystroke per frame.
///
/// The first frame empties the field and clicks it, which is how a user's
/// focus arrives, and every later frame presses one letter. Emptying it makes
/// every repeat of a window run type into the same field a fresh headless
/// repeat does. The events go through the app's event path; the press alone
/// stands for a keystroke, since a handler acts on the press and ignores the
/// release.
fn step_input(driver: &Shared, app: &mut App, frame: u64) {
    if frame == 0 {
        if let Some(set_text) = driver.borrow().texts.first() {
            set_text(String::new());
        }
        app.handle_event(InputEvent::PointerMoved {
            position: FIELD_POINT,
        });
        app.handle_event(InputEvent::PointerDown {
            position: FIELD_POINT,
            button: PointerButton::Primary,
        });
        app.handle_event(InputEvent::PointerUp {
            position: FIELD_POINT,
            button: PointerButton::Primary,
        });
    } else {
        app.handle_event(InputEvent::Key(pressed_key(frame)));
    }
    driver.borrow_mut().note_injection();
}

/// Returns the key press of one frame: the letters in cycling order.
fn pressed_key(frame: u64) -> KeyEvent {
    let character = (b'a' + (frame % 26) as u8) as char;
    let text = character.to_string();
    KeyEvent {
        key: Key::Character(text.clone()),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::default(),
        text: Some(text),
    }
}

/// Creates the empty grid that holds the boxes of the first two scenes.
fn box_grid() -> Element<Panel> {
    panel().row().wrap(true).width(px(GRID_WIDTH)).gap(Size {
        width: GAP,
        height: GAP,
    })
}

/// Creates one rounded, bordered box of the box grid.
fn bordered_box() -> Element<Panel> {
    panel()
        .width(px(BOX_SIZE))
        .height(px(BOX_SIZE))
        .corner_radius(8.0)
        .border(2.0)
        .border_color(Color::rgb(70, 74, 88))
}

/// A small deterministic generator for the scenes' color variation.
struct Xorshift(u64);

impl Xorshift {
    /// Creates a generator from a seed, which may be zero.
    fn new(seed: u64) -> Xorshift {
        Xorshift(seed | 1)
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

/// Derives the generator seed of one frame from the scene's seed.
///
/// The mix is the splitmix64 finalizer, so consecutive frame indices give
/// unrelated seeds.
fn frame_seed(scene_seed: u64, frame: u64) -> u64 {
    let mut z = frame.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    scene_seed ^ (z ^ (z >> 31))
}
