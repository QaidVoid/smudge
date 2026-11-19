//! Springs that move a value toward a target and can be given a new target
//! at any moment.
//!
//! A spring is a damped harmonic oscillator: the value accelerates toward
//! its target in proportion to how far away it is, and slows in proportion
//! to its speed. [`SpringState`] evaluates it in closed form from the
//! moment it last changed target, so a step costs the same however long the
//! spring has run, and allocates nothing. A new target keeps the value and
//! its velocity where they are, so the motion bends toward the new target
//! instead of jumping.
//!
//! ```
//! use smudge_core::animation::{Spring, SpringState};
//! use std::time::Duration;
//! use web_time::Instant;
//!
//! let start = Instant::now();
//! let mut state = SpringState::new(Spring::SMOOTH, 0.0, start);
//! state.step(1.0, start);
//! let (value, _, settled) = state.step(1.0, start + Duration::from_millis(100));
//! assert!(value > 0.0 && value < 1.0 && !settled);
//! let (value, velocity, settled) = state.step(1.0, start + Duration::from_secs(5));
//! assert!(settled && value == 1.0 && velocity == 0.0);
//! ```

use web_time::Instant;

use crate::geometry::Color;
use crate::style::Style;
use crate::theme::{ColorValue, RadiusValue, Theme};

/// How far from its target a value may be and still count as arrived.
const SETTLED_DISTANCE: f64 = 1e-3;

/// How fast a value may still move, per second, and count as arrived.
const SETTLED_SPEED: f64 = 1e-2;

/// How close to 1 a damping ratio counts as critical, where the formulas
/// for the other two regimes divide by a number near zero.
const CRITICAL_BAND: f64 = 1e-4;

/// The physical parameters of a spring.
///
/// A higher `stiffness` pulls harder toward the target, a higher `damping`
/// slows the motion more, and a higher `mass` makes both act more slowly.
/// Their ratio decides the character of the motion: with little damping the
/// value overshoots and swings back, and with much it creeps in without
/// overshooting. A spring whose parameters are not all finite and positive
/// moves nothing: its value jumps to the target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    /// How hard the spring pulls toward its target, per unit of distance.
    pub stiffness: f32,
    /// How much the spring resists motion, per unit of speed.
    pub damping: f32,
    /// How heavy the moving value is.
    pub mass: f32,
}

impl Spring {
    /// A quick spring that gets 95 percent of the way in about a fifth of a
    /// second, with a slight overshoot, for state changes such as a hover.
    pub const SNAPPY: Spring = Spring {
        stiffness: 400.0,
        damping: 30.0,
        mass: 1.0,
    };

    /// A critically damped spring that gets 95 percent of the way in about
    /// a third of a second, without overshooting.
    pub const SMOOTH: Spring = Spring {
        stiffness: 169.0,
        damping: 26.0,
        mass: 1.0,
    };

    /// A lightly damped spring that overshoots by about two fifths of the
    /// distance and swings back a few times.
    pub const BOUNCY: Spring = Spring {
        stiffness: 300.0,
        damping: 10.0,
        mass: 1.0,
    };

    /// Whether every parameter is finite and positive, so the spring moves.
    fn moves(&self) -> bool {
        [self.stiffness, self.damping, self.mass]
            .iter()
            .all(|value| value.is_finite() && *value > 0.0)
    }

    /// The four coefficients that carry a displacement and a velocity from
    /// one moment to `elapsed` seconds later: the displacement becomes
    /// `a * x + b * v` and the velocity `c * x + d * v`.
    fn response(&self, elapsed: f64) -> [f64; 4] {
        let stiffness = f64::from(self.stiffness);
        let damping = f64::from(self.damping);
        let mass = f64::from(self.mass);
        let natural = (stiffness / mass).sqrt();
        let ratio = damping / (2.0 * (stiffness * mass).sqrt());
        let t = elapsed;
        if (ratio - 1.0).abs() < CRITICAL_BAND {
            let decay = (-natural * t).exp();
            [
                decay * (1.0 + natural * t),
                decay * t,
                -decay * natural * natural * t,
                decay * (1.0 - natural * t),
            ]
        } else if ratio < 1.0 {
            let damped = natural * (1.0 - ratio * ratio).sqrt();
            let decay = (-ratio * natural * t).exp();
            let (sin, cos) = (damped * t).sin_cos();
            let lean = ratio * natural / damped;
            [
                decay * (cos + lean * sin),
                decay * sin / damped,
                -decay * natural * natural / damped * sin,
                decay * (cos - lean * sin),
            ]
        } else {
            let root = (ratio * ratio - 1.0).sqrt();
            // The slow rate is computed from the sum, which keeps its
            // precision where the difference of two close numbers would not.
            let slow = -natural / (ratio + root);
            let fast = -natural * (ratio + root);
            let (slow_decay, fast_decay) = ((slow * t).exp(), (fast * t).exp());
            let spread = slow - fast;
            [
                (slow * fast_decay - fast * slow_decay) / spread,
                (slow_decay - fast_decay) / spread,
                slow * fast * (fast_decay - slow_decay) / spread,
                (slow * slow_decay - fast * fast_decay) / spread,
            ]
        }
    }
}

/// The motion of `N` values that move together under one spring, such as
/// the channels of a color.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Motion<const N: usize> {
    /// The spring that moves the values.
    spring: Spring,
    /// When the values last changed target.
    origin: Instant,
    /// How far each value was from its target at `origin`.
    distance: [f64; N],
    /// How fast each value was moving at `origin`, per second.
    velocity: [f64; N],
    /// Where the values are going.
    target: [f64; N],
    /// Whether the values have arrived and stopped.
    settled: bool,
}

impl<const N: usize> Motion<N> {
    /// Values at rest at `at`, from `now`.
    pub(crate) fn rest(spring: Spring, at: [f64; N], now: Instant) -> Motion<N> {
        Motion {
            spring,
            origin: now,
            distance: [0.0; N],
            velocity: [0.0; N],
            target: at,
            settled: true,
        }
    }

    /// Where the values are going.
    fn target(&self) -> [f64; N] {
        self.target
    }

    /// The values and their velocities at `now`.
    fn at(&self, now: Instant) -> ([f64; N], [f64; N]) {
        if self.settled || !self.spring.moves() {
            return (self.target, [0.0; N]);
        }
        let elapsed = now.saturating_duration_since(self.origin).as_secs_f64();
        let [a, b, c, d] = self.spring.response(elapsed);
        let mut value = [0.0; N];
        let mut velocity = [0.0; N];
        for index in 0..N {
            let (x, v) = (self.distance[index], self.velocity[index]);
            value[index] = self.target[index] + a * x + b * v;
            velocity[index] = c * x + d * v;
        }
        (value, velocity)
    }

    /// Moves the values toward `target` and returns them and their
    /// velocities at `now`, and whether they have arrived.
    ///
    /// A target that differs from the last one is taken from `now`, with the
    /// values and velocities they have then, so the motion stays
    /// continuous. Values that arrive snap to the target and stop. With
    /// `jump` set, the values are at the target at once.
    pub(crate) fn step(
        &mut self,
        target: [f64; N],
        now: Instant,
        jump: bool,
    ) -> ([f64; N], [f64; N], bool) {
        if jump || !self.spring.moves() {
            *self = Motion::rest(self.spring, target, now);
            return (target, [0.0; N], true);
        }
        if target != self.target {
            let (value, velocity) = self.at(now);
            self.origin = now;
            self.target = target;
            self.velocity = velocity;
            for index in 0..N {
                self.distance[index] = value[index] - target[index];
            }
            self.settled = false;
        }
        let (value, velocity) = self.at(now);
        let arrived = (0..N).all(|index| {
            (value[index] - target[index]).abs() < SETTLED_DISTANCE
                && velocity[index].abs() < SETTLED_SPEED
        });
        if arrived && !self.settled {
            *self = Motion::rest(self.spring, target, now);
        }
        if self.settled {
            return (target, [0.0; N], true);
        }
        (value, velocity, false)
    }
}

/// One value moving toward a target under a spring.
///
/// The state starts at rest. Each [`step`](SpringState::step) names the
/// target and the time, and answers the value, its velocity and whether it
/// has arrived. Time is read from the caller, so an app steps its springs
/// with its own clock, which a test can drive.
#[derive(Clone, Copy, Debug)]
pub struct SpringState {
    /// The motion of the one value.
    motion: Motion<1>,
}

impl SpringState {
    /// A value at rest at `value`, moved by `spring` from `now` on.
    pub fn new(spring: Spring, value: f32, now: Instant) -> SpringState {
        SpringState {
            motion: Motion::rest(spring, [f64::from(value)], now),
        }
    }

    /// Moves the value toward `target` and returns its value and velocity
    /// at `now`, and whether it has arrived.
    ///
    /// A target that differs from the last step's takes effect at `now`,
    /// from the value and the velocity the value has then. Once the value
    /// is close enough to the target and slow enough, it snaps to the
    /// target, its velocity becomes zero, and the step reports it arrived.
    /// A spring whose parameters are not finite and positive arrives at
    /// once.
    pub fn step(&mut self, target: f32, now: Instant) -> (f32, f32, bool) {
        let ([value], [velocity], settled) = self.motion.step([f64::from(target)], now, false);
        (narrow(value), narrow(velocity), settled)
    }
}

/// `value` as an `f32`, held at the largest finite magnitude, since a stiff
/// and light spring can move faster than an `f32` counts.
fn narrow(value: f64) -> f32 {
    (value as f32).clamp(-f32::MAX, f32::MAX)
}

/// The paint props of one node that move under a spring when their
/// resolved values change, and the style the node paints with meanwhile.
///
/// A node whose element called
/// [`Element::transition`](crate::element::Element::transition) owns one.
/// Each paint resolves the node's props against the theme, so tokens and a
/// theme switch move too, and the props it hands the widget hold the
/// values the springs have reached.
#[derive(Debug)]
pub(crate) struct Transition {
    /// The spring every prop moves under.
    spring: Spring,
    /// The fill behind the node.
    background: ColorTrack,
    /// The color of the node's border.
    border: ColorTrack,
    /// The color the node's text paints with.
    text: ColorTrack,
    /// The radius of the node's corners, once the node has painted.
    radius: Option<Motion<1>>,
    /// The style the node paints with, its resolved style with the moving
    /// props in place.
    animated: Style,
}

impl Transition {
    /// A transition under `spring` for a node that has not painted yet.
    pub(crate) fn new(spring: Spring) -> Transition {
        Transition {
            spring,
            background: ColorTrack::default(),
            border: ColorTrack::default(),
            text: ColorTrack::default(),
            radius: None,
            animated: Style::default(),
        }
    }

    /// Moves every prop toward its value in `resolved` against `theme`, at
    /// `now`, and answers whether any of them is still moving. With `jump`
    /// set, every prop is at its value at once.
    ///
    /// A node's first paint shows its values without motion.
    pub(crate) fn update(
        &mut self,
        resolved: &Style,
        theme: &Theme,
        now: Instant,
        jump: bool,
    ) -> bool {
        let paint = &resolved.paint;
        let target = |value: &Option<ColorValue>| value.as_ref().map(|value| value.resolve(theme));
        let spring = self.spring;
        let (background, moving_background) =
            self.background
                .update(target(&paint.background), spring, now, jump);
        let (border, moving_border) =
            self.border
                .update(target(&paint.border_color), spring, now, jump);
        let (text, moving_text) = self
            .text
            .update(target(&paint.text_color), spring, now, jump);
        let radius_target = [f64::from(paint.corner_radius.resolve(theme))];
        let radius = self
            .radius
            .get_or_insert_with(|| Motion::rest(spring, radius_target, now));
        let ([corner], _, radius_settled) = radius.step(radius_target, now, jump);

        self.animated.clone_from(resolved);
        let animated = &mut self.animated.paint;
        animated.background = background.map(ColorValue::Literal);
        animated.border_color = border.map(ColorValue::Literal);
        animated.text_color = text.map(ColorValue::Literal);
        animated.corner_radius = RadiusValue::Literal(corner.max(0.0) as f32);
        moving_background || moving_border || moving_text || !radius_settled
    }

    /// The style the node paints with, as the last update left it.
    pub(crate) fn animated(&self) -> &Style {
        &self.animated
    }
}

/// One color prop under a spring, in linear sRGB with straight alpha.
#[derive(Debug, Default)]
struct ColorTrack {
    /// Whether the node has painted, so a color that appears later fades in
    /// rather than the first paint fading up from nothing.
    seen: bool,
    /// The motion of the four channels, once the prop has held a color.
    motion: Option<Motion<4>>,
}

impl ColorTrack {
    /// Moves the color toward `target` and returns the color to paint and
    /// whether it is still moving.
    ///
    /// A color that goes away fades out through its alpha and paints
    /// nothing once it arrives, and one that appears fades in from its own
    /// color at no alpha.
    fn update(
        &mut self,
        target: Option<Color>,
        spring: Spring,
        now: Instant,
        jump: bool,
    ) -> (Option<Color>, bool) {
        let first = !self.seen;
        self.seen = true;
        let goal = match (target, &self.motion) {
            (Some(color), _) => linear(color),
            (None, Some(motion)) => {
                let [r, g, b, _] = motion.target();
                [r, g, b, 0.0]
            }
            (None, None) => return (None, false),
        };
        let motion = self.motion.get_or_insert_with(|| {
            let start = if first {
                goal
            } else {
                [goal[0], goal[1], goal[2], 0.0]
            };
            Motion::rest(spring, start, now)
        });
        let (value, _, settled) = motion.step(goal, now, jump);
        let shown = match target {
            None if settled => None,
            _ => Some(srgb(value)),
        };
        (shown, !settled)
    }
}

/// `color` as linear red, green and blue, with its alpha, each from 0 to 1.
fn linear(color: Color) -> [f64; 4] {
    let channel = |value: u8| {
        let value = f64::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    [
        channel(color.r),
        channel(color.g),
        channel(color.b),
        f64::from(color.a) / 255.0,
    ]
}

/// The sRGB color of linear red, green, blue and alpha, each clamped to the
/// range a color holds, since a spring can overshoot it.
fn srgb([r, g, b, a]: [f64; 4]) -> Color {
    let channel = |value: f64| {
        let value = value.clamp(0.0, 1.0);
        let encoded = if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        };
        (encoded * 255.0).round() as u8
    };
    Color::rgba(
        channel(r),
        channel(g),
        channel(b),
        (a.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

#[cfg(test)]
#[path = "animation_tests.rs"]
mod tests;
