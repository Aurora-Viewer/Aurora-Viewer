//! "Jiggly name plates" (Preferences › Just for fun, off by default,
//! #1112): a plate is a little jelly the pointer can play with. Each time
//! the pointer reaches it, and a little when it leaves, the plate wobbles
//! at once, with no linger nor wait: the kicks add up, so the faster one
//! fidgets the more it shakes, up to a bound. A plate that grows (a pill
//! expanding into a full plate) overshoots its size and settles, and a
//! click presses it in (the click combo, `combo.rs`). Aurora's own: the
//! Legacy Viewer's tags do not move.
//!
//! Two damped springs make the wobble: the size (both sides out, then in)
//! and the squash (wider and flatter, then narrower and taller). As with
//! the hover bounce, only the surface moves: the text and the picture stay
//! sharp.

use egui::Vec2;

/// Angular frequency of the springs (about 4 Hz) and their damping ratio:
/// a few visible swings, gone in about a second.
const OMEGA: f32 = 26.0;
const ZETA: f32 = 0.2;
/// Speeds (points per second) given by the pointer coming in, going out,
/// a click and a plate starting to grow.
const ENTER_KICK: f32 = 130.0;
const LEAVE_KICK: f32 = 55.0;
const PRESS_KICK: f32 = 80.0;
const EXPAND_KICK: f32 = 75.0;
/// Bound of a spring's speed: fidgeting does not tear the plate apart.
const MAX_SPEED: f32 = 280.0;
/// Points the surface goes out on each side at most; it nears this bound
/// smoothly instead of stopping on it.
const MAX_STRETCH: f32 = 10.0;
/// The height gives back this part of what the width takes in a squash.
const SQUASH_HEIGHT: f32 = 0.55;
/// Below this, a spring is still.
const REST: f32 = 1e-3;

/// Seconds of a jiggly plate's change of level: a little longer than
/// `LEVEL_EASE`, for the overshoot to show.
pub const EASE: f64 = 0.34;
/// How far `overshoot` goes past its end (the "back" constant of the usual
/// ease-out-back curve, 1.70158; more here: about 14 %).
const OVERSHOOT: f32 = 2.4;

/// A damped spring around 0: where it is and how fast it goes.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Spring {
    pub x: f32,
    pub v: f32,
}

impl Spring {
    /// `dt` seconds later, at `omega` rad/s with a damping ratio `zeta`,
    /// in steps of at most 1/240 s: a slow frame does not make it explode.
    pub fn step(&mut self, dt: f32, omega: f32, zeta: f32) {
        if self.still() || dt <= 0.0 {
            return;
        }
        let n = (dt * 240.0).ceil().clamp(1.0, 64.0);
        let h = dt / n;
        for _ in 0..n as u32 {
            let a = -omega * omega * self.x - 2.0 * zeta * omega * self.v;
            self.v += a * h;
            self.x += self.v * h;
        }
        if self.x.abs() < REST && self.v.abs() < REST * omega {
            *self = Spring::default();
        }
    }

    pub fn still(&self) -> bool {
        self.x == 0.0 && self.v == 0.0
    }

    /// Add `speed` (signed), within `max`.
    pub fn kick(&mut self, speed: f32, max: f32) {
        self.v = (self.v + speed).clamp(-max, max);
    }

    /// Add `speed` the way the spring already goes, as one pumps a swing:
    /// kicks in a row add up instead of cancelling each other.
    pub fn pump(&mut self, speed: f32, max: f32) {
        let way = if self.v.abs() > 1.0 {
            self.v.signum()
        } else if self.x.abs() > 0.1 {
            -self.x.signum()
        } else {
            1.0
        };
        self.kick(way * speed, max);
    }
}

/// 0 to 1 past 1 and back: the ease-out-back curve, for a plate that grows.
pub fn overshoot(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let u = t - 1.0;
    1.0 + (OVERSHOOT + 1.0) * u * u * u + OVERSHOOT * u * u
}

/// `x` brought within ±`max` without a hard stop.
fn soft(x: f32, max: f32) -> f32 {
    max * (x / max).tanh()
}

/// The wobble of one plate.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Wobble {
    size: Spring,
    squash: Spring,
}

impl Wobble {
    pub fn step(&mut self, dt: f32) {
        self.size.step(dt, OMEGA, ZETA);
        // a little slower: the two swings do not stay in step
        self.squash.step(dt, OMEGA * 0.85, ZETA);
    }

    /// The pointer reaches the plate.
    pub fn enter(&mut self) {
        self.squash.pump(ENTER_KICK, MAX_SPEED);
        self.size.pump(ENTER_KICK * 0.35, MAX_SPEED);
    }

    /// The pointer leaves it.
    pub fn leave(&mut self) {
        self.squash.pump(LEAVE_KICK, MAX_SPEED);
    }

    /// A click presses it in; it springs back.
    pub fn press(&mut self) {
        self.size.kick(-PRESS_KICK, MAX_SPEED);
        self.squash.kick(PRESS_KICK * 0.3, MAX_SPEED);
    }

    /// It starts growing to a larger level.
    pub fn expand(&mut self) {
        self.squash.kick(EXPAND_KICK, MAX_SPEED);
    }

    pub fn still(&self) -> bool {
        self.size.still() && self.squash.still()
    }

    /// Points the surface grows on each side, across and up.
    pub fn grow(&self) -> Vec2 {
        let size = soft(self.size.x, MAX_STRETCH);
        let squash = soft(self.squash.x, MAX_STRETCH);
        Vec2::new(size + squash, size - squash * SQUASH_HEIGHT)
    }
}
