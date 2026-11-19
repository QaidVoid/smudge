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

#[cfg(test)]
#[path = "animation_tests.rs"]
mod tests;
