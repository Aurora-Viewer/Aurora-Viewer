//! Progressive draw distance: after the login and after a teleport to
//! another region, the draw distance starts at 32 m and doubles every few
//! seconds back up to the user's, so that the region sends the nearby
//! objects first (Preferences › Graphics).
//!
//! Port of the draw distance stepping of the Legacy Viewer, after Henri
//! Beauchamp's SpeedRez: LLAgent::teleportCore and LLStartUp (the start),
//! display() in llviewerdisplay.cpp (the steps), settings
//! FSRenderFarClipStepping (off) and FSRenderFarClipSteppingInterval (20 s,
//! 5 to 30), originally LGPL 2.1.
//!
//! As there, the stepped value is the draw distance itself, so everything
//! that reads it follows: what is drawn, picking, and the far distance sent
//! to the region in each AgentUpdate. A change of the draw distance by the
//! user during the steps ends them and keeps that value.
//!
//! Differences: the user's value is never lost. The settings file always
//! holds it (`Settings::to_saved`), where the Legacy Viewer writes the
//! stepped value and keeps the user's in FSSavedRenderFarClip for the next
//! login. Turning the option off during the steps gives the user's value
//! back at once. The steps follow the load, where the Legacy Viewer only
//! uses its timer ([`tick`]): the distance doubles every 2 s while the
//! arrival is quiet and the frames are fast, keeps the Legacy pace (the
//! interval) with one of the two busy, does not grow with both busy, and a
//! frame that stays heavy steps it back one level. The point is to spare
//! the GPU the burst of a whole crowd arriving at once after a teleport
//! (the shadow maps of a busy region drawn in one frame). Reaching the
//! user's distance keeps that watch.

use std::time::{Duration, Instant};

/// RenderFarClip set at the start of the steps (gLastDrawDistanceStep).
pub const FIRST_STEP: f32 = 32.0;

/// Geometry and textures still arriving. Above this the arrival is busy.
pub const WAITING_LIMIT: u32 = 48;

/// A finished frame at or under this, in milliseconds, is fast.
pub const GPU_OK_MS: f32 = 33.0;

/// The shadow-depth pass at or under this, in milliseconds, is fast.
pub const SHADOW_OK_MS: f32 = 16.0;

/// A frame slower than this, in milliseconds, is heavy: no growth, and the
/// distance steps back if it lasts.
pub const GPU_HEAVY_MS: f32 = 80.0;

/// A shadow-depth pass slower than this, in milliseconds, is heavy.
pub const SHADOW_HEAVY_MS: f32 = 40.0;

/// How long a heavy frame must last before the distance steps back.
pub const HEAVY_HOLD: Duration = Duration::from_millis(500);

/// The shortest time between two changes: with the arrival quiet and the
/// frames fast, 32 m reach 256 m in about 6 s.
pub const MIN_GAP: Duration = Duration::from_secs(2);

/// After a step back, the distance that was too heavy is not tried again
/// for this long (no up and down every few seconds).
pub const CEILING_HOLD: Duration = Duration::from_secs(60);

/// What the last finished frame was doing. Absent GPU times count as fast:
/// the first frames, a photo being drawn, and a device without timestamps
/// follow the arrival queue alone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Load {
    /// Textures fetching, decoding or waiting to upload, plus geometry not
    /// built yet.
    pub waiting: u32,
    pub gpu_ms: Option<f32>,
    pub shadow_ms: Option<f32>,
}

impl Load {
    fn slower_than(&self, gpu: f32, shadow: f32) -> bool {
        self.gpu_ms.is_some_and(|ms| ms > gpu) || self.shadow_ms.is_some_and(|ms| ms > shadow)
    }

    fn arrival_quiet(&self) -> bool {
        self.waiting <= WAITING_LIMIT
    }

    fn fast(&self) -> bool {
        !self.slower_than(GPU_OK_MS, SHADOW_OK_MS)
    }

    fn heavy(&self) -> bool {
        self.slower_than(GPU_HEAVY_MS, SHADOW_HEAVY_MS)
    }
}

/// The steps under way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stepping {
    /// The user's draw distance, given back at the end
    /// (FSSavedRenderFarClip).
    pub user: f32,
    /// The draw distance of the current step.
    pub step: f32,
    /// The last change of the draw distance, or the arrival
    /// (gTeleportArrivalTimer).
    pub changed: Instant,
    /// When the current heavy run began. Cleared by a frame that is not heavy.
    pub heavy_since: Option<Instant>,
    /// A distance found too heavy, and until when it is not tried again.
    pub ceiling: Option<(f32, Instant)>,
}

/// Starts the steps (or starts them over, keeping the user's value of steps
/// under way): the draw distance becomes 32 m, or stays lower.
pub fn start(current: Option<Stepping>, draw_distance: f32, now: Instant) -> (Stepping, f32) {
    let user = current.map_or(draw_distance, |s| s.user);
    let step = FIRST_STEP.min(user);
    (
        Stepping {
            user,
            step,
            changed: now,
            heavy_since: None,
            ceiling: None,
        },
        step,
    )
}

/// What a frame of the steps does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tick {
    /// Nothing changes.
    Wait,
    /// The watch changed; the draw distance did not.
    Hold(Stepping),
    /// The draw distance takes this value; the steps go on.
    Step(Stepping, f32),
    /// The steps end; the draw distance takes this value (the user's, or
    /// the one the user just set).
    End(f32),
}

/// One frame of the steps, driven by the load rather than by a timer:
/// - the user changed the draw distance (`draw_distance` is not the step):
///   the steps end and keep it; the option turned off: the user's value;
/// - nothing while teleporting (`TELEPORT_NONE` in the Legacy Viewer);
/// - a frame heavy for [`HEAVY_HOLD`] halves the distance (not under 32 m),
///   and that distance waits [`CEILING_HOLD`] before it is tried again;
/// - the distance doubles, up to the user's, at most every [`MIN_GAP`] while
///   the arrival is quiet and the frames are fast; with one of the two busy
///   (objects still arriving, or a slow but not heavy frame), at most every
///   `interval` (FSRenderFarClipSteppingInterval, the Legacy Viewer's fixed
///   pace); with both busy, or a heavy frame, not at all.
///
/// Reaching the user's distance keeps the watch, so a later heavy frame can
/// still step back.
pub fn tick(s: Stepping, draw_distance: f32, enabled: bool, teleporting: bool, load: Load, interval: Duration, now: Instant) -> Tick {
    if draw_distance != s.step {
        return Tick::End(draw_distance);
    }
    if !enabled {
        return Tick::End(s.user);
    }
    if teleporting {
        return Tick::Wait;
    }
    if load.heavy() {
        let since = s.heavy_since.unwrap_or(now);
        if s.step > FIRST_STEP && now.saturating_duration_since(since) >= HEAVY_HOLD {
            let value = (s.step * 0.5).max(FIRST_STEP);
            return Tick::Step(
                Stepping {
                    step: value,
                    changed: now,
                    heavy_since: None,
                    ceiling: Some((s.step, now + CEILING_HOLD)),
                    ..s
                },
                value,
            );
        }
        if s.heavy_since.is_none() {
            return Tick::Hold(Stepping {
                heavy_since: Some(since),
                ..s
            });
        }
        return Tick::Wait;
    }
    if s.heavy_since.is_some() {
        return Tick::Hold(Stepping { heavy_since: None, ..s });
    }
    let value = (s.step * 2.0).min(s.user);
    if value <= s.step {
        return Tick::Wait;
    }
    if s.ceiling.is_some_and(|(ceiling, until)| value >= ceiling && now < until) {
        return Tick::Wait;
    }
    let waited = now.saturating_duration_since(s.changed);
    let gap = match (load.arrival_quiet(), load.fast()) {
        (true, true) => MIN_GAP,
        (true, false) | (false, true) => interval.max(MIN_GAP),
        (false, false) => return Tick::Wait,
    };
    if waited < gap {
        return Tick::Wait;
    }
    Tick::Step(
        Stepping {
            step: value,
            changed: now,
            heavy_since: None,
            ..s
        },
        value,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTERVAL: Duration = Duration::from_secs(20);

    fn quiet() -> Load {
        Load {
            waiting: 0,
            gpu_ms: None,
            shadow_ms: None,
        }
    }

    fn at(s: Stepping, dd: f32, enabled: bool, teleporting: bool, load: Load, now: Instant) -> Tick {
        tick(s, dd, enabled, teleporting, load, INTERVAL, now)
    }

    fn load(waiting: u32, gpu: f32, shadow: f32) -> Load {
        Load {
            waiting,
            gpu_ms: Some(gpu),
            shadow_ms: Some(shadow),
        }
    }

    /// Runs the steps frame by frame (every 100 ms) under `load` and returns
    /// the distance at `until`.
    fn run(mut s: Stepping, mut dd: f32, load: impl Fn(Duration) -> Load, t0: Instant, until: Duration) -> (Stepping, f32) {
        let mut t = Duration::ZERO;
        while t <= until {
            match at(s, dd, true, false, load(t), t0 + t) {
                Tick::Wait => {}
                Tick::Hold(next) => s = next,
                Tick::Step(next, value) => (s, dd) = (next, value),
                Tick::End(_) => panic!("only the user ends the steps"),
            }
            t += Duration::from_millis(100);
        }
        (s, dd)
    }

    #[test]
    fn a_quiet_arrival_and_fast_frames_double_every_two_seconds() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, t0);
        assert_eq!(dd, 32.0);
        // nothing before the gap
        assert_eq!(at(s, dd, true, false, quiet(), t0 + MIN_GAP / 2), Tick::Wait);
        let fast = |_| load(0, 8.0, 2.0);
        assert_eq!(run(s, dd, fast, t0, MIN_GAP).1, 64.0);
        assert_eq!(run(s, dd, fast, t0, MIN_GAP * 2).1, 128.0);
        let (s, dd) = run(s, dd, fast, t0, MIN_GAP * 3);
        assert_eq!(dd, 256.0, "the user's distance in about 6 s, not 60");
        // and it stays there, still watched
        assert_eq!(run(s, dd, fast, t0 + MIN_GAP * 3, INTERVAL).1, 256.0);
    }

    #[test]
    fn the_last_step_stops_at_the_users_value() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 96.0, t0);
        let (s, dd) = run(s, dd, |_| quiet(), t0, MIN_GAP);
        assert_eq!(dd, 64.0);
        let (_, dd) = run(s, dd, |_| quiet(), t0 + MIN_GAP, MIN_GAP * 3);
        assert_eq!(dd, 96.0);
    }

    #[test]
    fn one_busy_reason_keeps_the_legacy_pace_and_two_hold_the_step() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, t0);
        // objects still arriving, frames fast: one step per interval
        let arriving = |_| load(WAITING_LIMIT + 1, 8.0, 2.0);
        assert_eq!(run(s, dd, arriving, t0, INTERVAL - Duration::from_millis(200)).1, 32.0);
        assert_eq!(run(s, dd, arriving, t0, INTERVAL).1, 64.0);
        // a slow but not heavy frame, arrival quiet: the same pace
        let slow = |_| load(0, 50.0, 4.0);
        assert_eq!(run(s, dd, slow, t0, INTERVAL).1, 64.0);
        // both busy: no growth at all (no burst on top of a burst)
        let both = |_| load(WAITING_LIMIT + 1, 50.0, 20.0);
        assert_eq!(run(s, dd, both, t0, INTERVAL * 3).1, 32.0);
    }

    #[test]
    fn a_heavy_frame_steps_back_and_that_distance_waits() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, t0);
        let (s, dd) = run(s, dd, |_| quiet(), t0, MIN_GAP * 2);
        assert_eq!(dd, 128.0);
        let t1 = t0 + MIN_GAP * 2;
        // a heavy shadow pass: halved after HEAVY_HOLD, not before
        let heavy = |_| load(0, 30.0, 120.0);
        assert_eq!(run(s, dd, heavy, t1, HEAVY_HOLD - Duration::from_millis(200)).1, 128.0);
        let (s, dd) = run(s, dd, heavy, t1, HEAVY_HOLD);
        assert_eq!(dd, 64.0);
        assert_eq!(s.ceiling.map(|c| c.0), Some(128.0));
        // fast again: 128 m is not tried before CEILING_HOLD
        let t2 = t1 + HEAVY_HOLD;
        assert_eq!(run(s, dd, |_| quiet(), t2, CEILING_HOLD - Duration::from_secs(1)).1, 64.0);
        assert_eq!(run(s, dd, |_| quiet(), t2, CEILING_HOLD + Duration::from_millis(500)).1, 128.0);
        // a short heavy run is forgotten
        let (fresh, dd) = start(None, 256.0, t0);
        let blip = |t: Duration| {
            if t < Duration::from_millis(300) {
                load(0, 120.0, 4.0)
            } else {
                quiet()
            }
        };
        assert_eq!(run(fresh, dd, blip, t0, Duration::from_millis(900)).1, 32.0);
    }

    #[test]
    fn no_step_under_32_and_a_photo_does_not_count() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, t0);
        // heavy at 32 m: nothing lower
        assert_eq!(run(s, dd, |_| load(0, 200.0, 100.0), t0, Duration::from_secs(3)).1, 32.0);
        // a photo's frames come with no GPU time (app/draw_stepping.rs): the queue alone
        assert_eq!(run(s, dd, |_| quiet(), t0, MIN_GAP).1, 64.0);
    }

    #[test]
    fn a_change_by_the_user_ends_the_steps_and_keeps_it() {
        let t0 = Instant::now();
        let (s, _) = start(None, 256.0, t0);
        assert_eq!(at(s, 100.0, true, false, quiet(), t0), Tick::End(100.0));
    }

    #[test]
    fn turning_it_off_gives_the_users_value_back_at_once() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, t0);
        assert_eq!(at(s, dd, false, false, quiet(), t0), Tick::End(256.0));
    }

    #[test]
    fn no_step_while_teleporting_and_a_new_start_keeps_the_users_value() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, t0);
        assert_eq!(at(s, dd, true, true, quiet(), t0 + INTERVAL * 3), Tick::Wait);
        // a teleport during the steps: back to 32, the user's 256 kept
        let (again, dd) = start(Some(s), dd, t0 + INTERVAL);
        assert_eq!((again.user, dd), (256.0, 32.0));
    }

    #[test]
    fn a_draw_distance_already_at_32_does_not_step() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 32.0, t0);
        assert_eq!(dd, 32.0);
        assert_eq!(at(s, dd, true, false, quiet(), t0 + INTERVAL), Tick::Wait);
    }

    #[test]
    fn the_saved_file_keeps_the_users_draw_distance() {
        let mut settings = crate::settings::Settings {
            draw_distance: 256.0,
            ..crate::settings::Settings::default()
        };
        let (steps, value) = start(None, settings.draw_distance, Instant::now());
        settings.draw_stepping = Some(steps);
        settings.draw_distance = value;
        assert_eq!(settings.to_saved().draw_distance, 256.0);
        assert_eq!(settings.draw_distance, 32.0);
    }
}
