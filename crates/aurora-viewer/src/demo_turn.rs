//! AURORA_DEMO_TURN=<degrees per second>[,<seconds turning>[,<seconds still>]]:
//! fast camera turns, as a resident looks around in a busy place. Once the
//! loading screen is gone, our avatar (and the camera behind it) turns at
//! that rate for the first duration (1 s by default), then stays still for
//! the second (3 s), and again. What comes into view then has to be synced,
//! listed and textured: read the worst frame of each second (`max=`, `max:`
//! and `ft=` of `perf summary`) with AURORA_PROFILE=1.

use std::time::{Duration, Instant};

/// Not before this frame, nor before this long after the scenario was
/// first driven (the loading screen still covers the view).
const FIRST_FRAME: u64 = 300;
const FIRST_AFTER: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TurnDemo {
    /// Radians a second.
    rate: f32,
    on: f32,
    off: f32,
    armed: Option<Instant>,
    started: Option<Instant>,
    last: Option<Instant>,
}

/// `<degrees per second>[,<seconds turning>[,<seconds still>]]`: 180, 1 and
/// 3 when a part is missing or not a number.
fn parse(value: &str) -> (f32, f32, f32) {
    let mut parts = value.split(',').map(|p| p.trim().parse::<f32>().ok().filter(|v| v.is_finite()));
    let rate = parts.next().flatten().unwrap_or(180.0);
    let on = parts.next().flatten().filter(|v| *v > 0.0).unwrap_or(1.0);
    let off = parts.next().flatten().filter(|v| *v >= 0.0).unwrap_or(3.0);
    (rate, on, off)
}

/// Radians to turn between `from` and `to` seconds after the start: the
/// turning part of each cycle at `rate`, nothing in the still part.
fn turned(rate: f32, on: f32, off: f32, from: f32, to: f32) -> f32 {
    let cycle = on + off;
    // seconds of turning from the start to `t`
    let turning = |t: f32| {
        let whole = (t / cycle).floor();
        whole * on + (t - whole * cycle).min(on)
    };
    rate * (turning(to.max(0.0)) - turning(from.max(0.0)))
}

impl TurnDemo {
    pub fn from_env() -> Option<Self> {
        let (rate, on, off) = parse(&std::env::var("AURORA_DEMO_TURN").ok()?);
        log::info!("demo turn: {rate:.0} degrees a second for {on:.1} s, then still for {off:.1} s");
        Some(TurnDemo {
            rate: rate.to_radians(),
            on,
            off,
            armed: None,
            started: None,
            last: None,
        })
    }

    /// The yaw to add to our avatar this frame (radians).
    pub fn tick(&mut self, frame: u64) -> f32 {
        let now = Instant::now();
        let armed = *self.armed.get_or_insert(now);
        if frame < FIRST_FRAME || now - armed < FIRST_AFTER {
            return 0.0;
        }
        let started = *self.started.get_or_insert(now);
        let last = self.last.replace(now).unwrap_or(now);
        let secs = |t: Instant| (t - started).as_secs_f32();
        turned(self.rate, self.on, self.off, secs(last), secs(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_takes_the_defaults_for_what_is_missing() {
        assert_eq!(parse(""), (180.0, 1.0, 3.0));
        assert_eq!(parse("360"), (360.0, 1.0, 3.0));
        assert_eq!(parse("90,2,0"), (90.0, 2.0, 0.0));
        assert_eq!(parse("x,-1,-2"), (180.0, 1.0, 3.0));
    }

    #[test]
    fn turns_only_in_the_turning_part_of_each_cycle() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
        // 1 s turning, 3 s still: one radian a second
        assert!(close(turned(1.0, 1.0, 3.0, 0.0, 0.5), 0.5));
        assert!(close(turned(1.0, 1.0, 3.0, 0.5, 2.0), 0.5));
        assert!(close(turned(1.0, 1.0, 3.0, 1.0, 4.0), 0.0));
        // across a cycle: the end of one turn and the start of the next
        assert!(close(turned(1.0, 1.0, 3.0, 0.75, 4.25), 0.5));
        // two whole cycles
        assert!(close(turned(1.0, 1.0, 3.0, 0.0, 8.0), 2.0));
        // never still: a steady turn
        assert!(close(turned(2.0, 1.0, 0.0, 0.3, 5.3), 10.0));
    }
}
