//! When the teleport screen lets the new region show, and how its bar
//! reaches the end.
//!
//! The Legacy Viewer keeps the screen for a fixed `TeleportArrivalDelay`
//! (2 s) after AgentMovementComplete (TELEPORT_START_ARRIVAL, then
//! TELEPORT_ARRIVING in llviewerdisplay.cpp), whatever has loaded. Aurora
//! leaves as soon as the ground is there and a short settling time has
//! passed, then finishes the bar in a quick glide instead of easing slowly
//! towards it: the region shows about a second sooner, never before its
//! terrain.

use std::time::Duration;

/// Shortest wait after the arrival: the first object updates of the region
/// and the first update of our avatar come about 0.3 s after
/// AgentMovementComplete on Agni.
pub const SETTLE: Duration = Duration::from_millis(400);
/// Longest wait for the terrain before showing the region anyway.
pub const GIVE_UP: Duration = Duration::from_secs(10);
/// Share of the terrain patches received that counts as "the ground is
/// there".
pub const TERRAIN_READY: f32 = 0.97;
/// How long the bar takes to finish once the region is ready.
pub const FINISH: Duration = Duration::from_millis(150);

/// The screen's behaviour before 2026-10-17, kept to compare
/// (`AURORA_TP_SCREEN=legacy`, `alt` one teleport in two).
const OLD_SETTLE: Duration = Duration::from_millis(800);
const OLD_RATE: f32 = 2.6;
const OLD_DONE: f32 = 0.985;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Quick,
    /// Wait 0.8 s, then ease towards the end at 2.6 / s until 98.5 %.
    Old,
}

impl Mode {
    /// The screen of the next teleport: `AURORA_TP_SCREEN=legacy` the old
    /// one, `alt` the old one every other teleport (starting with it),
    /// otherwise quick.
    pub fn for_next_teleport() -> Mode {
        static TELEPORTS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let count = TELEPORTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Mode::from_setting(std::env::var("AURORA_TP_SCREEN").ok().as_deref(), count)
    }

    fn from_setting(value: Option<&str>, count: u32) -> Mode {
        match value {
            Some("legacy") => Mode::Old,
            Some("alt") if count.is_multiple_of(2) => Mode::Old,
            _ => Mode::Quick,
        }
    }
}

/// Whether the new region may show `since_arrival` after
/// AgentMovementComplete, with `terrain` of its patches received.
pub fn ready(mode: Mode, since_arrival: Duration, terrain: f32) -> bool {
    let settle = match mode {
        Mode::Quick => SETTLE,
        Mode::Old => OLD_SETTLE,
    };
    (terrain > TERRAIN_READY && since_arrival > settle) || since_arrival > GIVE_UP
}

/// The bar after `dt` seconds, gliding from `shown` towards `target`, never
/// backwards. Once the target is the end, the quick screen finishes in
/// `FINISH`.
pub fn glide(mode: Mode, shown: f32, target: f32, dt: f32) -> f32 {
    let next = match mode {
        Mode::Quick if target >= 1.0 => shown + dt / FINISH.as_secs_f32(),
        _ => shown + (target - shown) * (1.0 - (-OLD_RATE * dt).exp()),
    };
    next.max(shown).min(1.0)
}

/// Whether the bar has reached the end and the screen may fade out.
pub fn done(mode: Mode, shown: f32, target: f32) -> bool {
    target >= 1.0
        && match mode {
            Mode::Quick => shown >= 1.0,
            Mode::Old => shown > OLD_DONE,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frames of `dt` from `shown` to the end, ready from the first frame.
    fn seconds_to_finish(mode: Mode, mut shown: f32, dt: f32) -> f32 {
        let mut t = 0.0;
        while !done(mode, shown, 1.0) {
            shown = glide(mode, shown, 1.0, dt);
            t += dt;
            assert!(t < 10.0, "never finished");
        }
        t
    }

    #[test]
    fn the_region_waits_for_its_ground_and_a_short_settling_time() {
        let ms = Duration::from_millis;
        assert!(!ready(Mode::Quick, ms(300), 1.0), "too soon after the arrival");
        assert!(!ready(Mode::Quick, ms(2000), 0.5), "half the terrain");
        assert!(ready(Mode::Quick, ms(450), 0.98));
        assert!(ready(Mode::Quick, Duration::from_secs(11), 0.0), "gives up on the terrain");
        assert!(!ready(Mode::Old, ms(700), 1.0), "the old screen waited 0.8 s");
        assert!(ready(Mode::Old, ms(850), 1.0));
    }

    #[test]
    fn the_bar_finishes_in_a_fraction_of_a_second_once_ready() {
        let dt = 1.0 / 144.0;
        let quick = seconds_to_finish(Mode::Quick, 0.6, dt);
        let old = seconds_to_finish(Mode::Old, 0.6, dt);
        assert!(quick < 0.2, "quick glide {quick} s");
        assert!(old > 1.0, "old glide {old} s");
        // from a bar that was already almost full
        assert!(seconds_to_finish(Mode::Quick, 0.95, dt) < 0.02);
    }

    #[test]
    fn the_bar_never_goes_backwards_nor_past_the_end() {
        for mode in [Mode::Quick, Mode::Old] {
            assert_eq!(glide(mode, 0.7, 0.4, 0.016), 0.7);
            assert!(glide(mode, 0.99, 1.0, 1.0) <= 1.0);
        }
        assert_eq!(glide(Mode::Quick, 0.99, 1.0, 1.0), 1.0);
        // before the arrival the quick screen eases like before
        let eased = glide(Mode::Quick, 0.2, 0.5, 0.1);
        assert!(eased > 0.2 && eased < 0.5);
    }

    #[test]
    fn the_switch_picks_the_old_screen_or_alternates() {
        assert_eq!(Mode::from_setting(None, 0), Mode::Quick);
        assert_eq!(Mode::from_setting(Some("legacy"), 3), Mode::Old);
        assert_eq!(Mode::from_setting(Some("alt"), 0), Mode::Old);
        assert_eq!(Mode::from_setting(Some("alt"), 1), Mode::Quick);
    }
}
