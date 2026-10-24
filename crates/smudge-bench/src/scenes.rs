//! The fixed scenes the benchmark measures.
//!
//! A scene is a tree of panels plus a driver that writes the same colors for
//! the same frame index, so every repeat of a run measures identical work.
//! Variation comes from a small xorshift generator seeded per scene, so no
//! random-number dependency is needed.

use std::cell::RefCell;
use std::rc::Rc;

use smudge_core::element::Element;
use smudge_core::geometry::{Color, Size};
use smudge_core::reactive::{self, prelude::*};
use smudge_core::style::{Align, px};
use smudge_core::widget::{Panel, panel};

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

/// Sets the color of one animated part of a scene.
type SetColor = Box<dyn Fn(Color)>;

/// The state one mounted scene drives.
///
/// The setters are the signals' write ends, in the order the scene mounted
/// them. [`Scene::step`] writes one color to each of them per frame.
pub(crate) struct Driver {
    /// The write ends of the animated colors, in mount order.
    setters: Vec<SetColor>,
}

impl Driver {
    /// Creates an empty driver.
    fn new() -> Driver {
        Driver {
            setters: Vec::new(),
        }
    }

    /// Records the write end of one animated color.
    fn register(&mut self, set_color: SetColor) {
        self.setters.push(set_color);
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
    /// Mounts the element tree and records the animated colors.
    mount: fn(&mut Driver) -> Element<Panel>,
    /// Writes the colors of one frame into the driver.
    step: fn(&mut Driver, u64),
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

    /// Writes the colors of `frame` into the scene's signals.
    ///
    /// The colors are a function of the frame index alone, so a run is
    /// reproducible from its seed.
    pub(crate) fn step(&self, driver: &Shared, frame: u64) {
        (self.step)(&mut driver.borrow_mut(), frame);
    }
}

/// The scenes of the suite, keyed as the command line names them.
///
/// The first scene is the raster-heavy case: 600 rounded, bordered boxes in a
/// 30 by 20 grid, and every box changes color every frame. The second is the
/// best case for damage: the same 600 boxes, static, plus one small box that
/// changes color every frame. The third is the worst case for damage: the
/// root background changes every frame and covers everything.
static SCENES: [Scene; 3] = [
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
];

/// Mounts the animated box grid of the first scene.
fn mount_boxes(driver: &mut Driver) -> Element<Panel> {
    let mut rng = Xorshift::new(S1_SEED);
    let mut grid = box_grid();
    for _ in 0..BOXES {
        let (color, set_color) = reactive::signal_local(rng.color());
        driver.register(Box::new(move |color| set_color.set(color)));
        grid = grid.child(bordered_box().background(move || color.get().into()));
    }
    panel().align_items(Align::Start).child(grid)
}

/// Writes one fresh color to every box of the first scene.
fn step_boxes(driver: &mut Driver, frame: u64) {
    let mut rng = Xorshift::new(frame_seed(S1_SEED, frame));
    for set_color in &driver.setters {
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
    driver.register(Box::new(move |color| set_color.set(color)));
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
fn step_counter(driver: &mut Driver, frame: u64) {
    let mut rng = Xorshift::new(frame_seed(S2_SEED, frame));
    if let Some(set_color) = driver.setters.first() {
        set_color(rng.color());
    }
}

/// Mounts the scene whose root background covers everything and changes every
/// frame.
fn mount_full_change(driver: &mut Driver) -> Element<Panel> {
    let (color, set_color) = reactive::signal_local(Xorshift::new(S3_SEED).color());
    driver.register(Box::new(move |color| set_color.set(color)));
    panel().background(move || color.get().into())
}

/// Writes one fresh color to the root background of the third scene.
fn step_full_change(driver: &mut Driver, frame: u64) {
    let mut rng = Xorshift::new(frame_seed(S3_SEED, frame));
    if let Some(set_color) = driver.setters.first() {
        set_color(rng.color());
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
