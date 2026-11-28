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
//! stepped value to RenderFarClip and keeps the user's aside in
//! FSSavedRenderFarClip until the steps end. Turning the option off during
//! the steps gives the user's value back at once. The steps follow the
//! load, where the Legacy Viewer only uses its timer ([`tick`]): the
//! distance doubles every 2 s while the arrival is quiet and the frames are
//! fast, keeps the Legacy pace (the interval) with one of the two busy, and
//! the floor pace ([`FLOOR_INTERVALS`]) with both busy, and a frame that
//! stays heavy steps it back one level. The point is to spare the GPU the
//! burst of a whole crowd arriving at once after a teleport (the shadow maps
//! of a busy region drawn in one frame). As in the Legacy Viewer, the steps
//! end once they reach the user's distance: what happens after the arrival
//! is Autotune's, which the steps pause while they last.

use crate::settings::Settings;
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

/// The floor pace, in intervals: with the arrival and the frames both busy,
/// or a heavy frame at 32 m (where nothing steps back), the distance still
/// doubles every this many intervals, so that it always reaches the user's
/// unless a step back is pending. With the 20 s default: 40 s a step, 32 m
/// to 256 m in 2 minutes at worst, twice the Legacy Viewer's fixed 60 s;
/// one interval would be the Legacy pace itself, leaving a busy arrival
/// nothing to spare.
pub const FLOOR_INTERVALS: u32 = 2;

/// After a step back, the distance that was too heavy is not tried again
/// for this long (no up and down every few seconds). Each new step back
/// from that same distance doubles it, up to [`CEILING_HOLD_MAX`].
pub const CEILING_HOLD: Duration = Duration::from_secs(60);

/// The longest a distance found too heavy waits: 16 minutes (60 s doubled
/// four times), so that a place heavy at the user's distance settles a
/// level under it and still tries it again now and then.
pub const CEILING_HOLD_MAX: Duration = Duration::from_secs(16 * 60);

/// What the last finished frame was doing. Absent GPU times count as fast:
/// the first frames, a photo being drawn, a frame with no new measure
/// ([`Readback`]) and a device without timestamps follow the arrival queue
/// alone; they neither start nor end a heavy run.
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

    fn measured(&self) -> bool {
        self.gpu_ms.is_some() || self.shadow_ms.is_some()
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

/// The renderer's last GPU measure, as the steps read it. The renderer
/// gives its last measure again each frame until a new one comes back from
/// the GPU (app/busy_pause.rs filters it the same way): a heavy frame seen
/// again is not a heavy run.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Readback {
    last: Option<(Option<f32>, Option<f32>)>,
}

impl Readback {
    /// The GPU times of this frame for [`Load`]: none when they are the
    /// last ones again, or while a photo is taken (its frames, drawn larger
    /// and at the photo's quality, are not the world's load).
    pub fn fresh(&mut self, gpu_ms: Option<f32>, shadow_ms: Option<f32>, photo: bool) -> (Option<f32>, Option<f32>) {
        let measure = (gpu_ms, shadow_ms);
        let new = self.last != Some(measure);
        if gpu_ms.is_some() || shadow_ms.is_some() {
            self.last = Some(measure);
        }
        if photo || !new { (None, None) } else { measure }
    }
}

/// A distance found too heavy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ceiling {
    /// The draw distance the step back left.
    pub level: f32,
    /// Not tried again before this.
    pub until: Instant,
    /// How long it waited this time; doubled by the next step back from
    /// the same level.
    pub hold: Duration,
}

/// The steps under way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stepping {
    /// The user's draw distance, given back at the end (it plays
    /// FSSavedRenderFarClip's role).
    pub user: f32,
    /// The draw distance of the current step.
    pub step: f32,
    /// The last change of the draw distance, or the arrival
    /// (gTeleportArrivalTimer).
    pub changed: Instant,
    /// When the current heavy run began. Cleared by a measured frame that
    /// is not heavy.
    pub heavy_since: Option<Instant>,
    /// The last distance found too heavy.
    pub ceiling: Option<Ceiling>,
}

/// Starts the steps (or starts them over, keeping the user's value of steps
/// under way): the draw distance becomes 32 m, or stays lower. A draw
/// distance that is not the current step was just set by the user (the
/// slider in the frame of a teleport): it is theirs.
pub fn start(current: Option<Stepping>, draw_distance: f32, now: Instant) -> (Stepping, f32) {
    let user = match current {
        Some(s) if draw_distance == s.step => s.user,
        _ => draw_distance,
    };
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

/// The login's first arrival, or a teleport to another region finished:
/// starts the steps in `s` when the option is on. Not under the photo
/// quality, which holds the draw distance itself: a step written under it
/// would become the user's value when the photo ends.
pub fn begin(s: &mut Settings, now: Instant) -> Option<Stepping> {
    if !s.draw_distance_stepping || s.photo_held.is_some() {
        return None;
    }
    let (steps, value) = start(s.draw_stepping, s.draw_distance, now);
    s.draw_stepping = Some(steps);
    s.draw_distance = value;
    Some(steps)
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
/// - the step is the user's distance (a draw distance of 32 m or less at
///   the start): the steps end, as display() does once RenderFarClip
///   reaches gSavedDrawDistance;
/// - nothing while teleporting (`TELEPORT_NONE` in the Legacy Viewer);
/// - a frame heavy for [`HEAVY_HOLD`] halves the distance (not under 32 m),
///   and that distance waits [`CEILING_HOLD`] before it is tried again,
///   twice as long after each new step back from it ([`CEILING_HOLD_MAX`]);
/// - the distance doubles, up to the user's, at most every [`MIN_GAP`] while
///   the arrival is quiet and the frames are fast; with one of the two busy
///   (objects still arriving, or a slow but not heavy frame), at most every
///   `interval` (FSRenderFarClipSteppingInterval, the Legacy Viewer's fixed
///   pace); with both busy, or a heavy frame at 32 m, at the floor pace
///   ([`FLOOR_INTERVALS`]); not at all while a heavy run may step back;
///   the step that reaches the user's distance ends the steps.
pub fn tick(s: Stepping, draw_distance: f32, enabled: bool, teleporting: bool, load: Load, interval: Duration, now: Instant) -> Tick {
    if draw_distance != s.step {
        return Tick::End(draw_distance);
    }
    if !enabled || s.step >= s.user {
        return Tick::End(s.user);
    }
    if teleporting {
        return Tick::Wait;
    }
    let heavy = load.heavy();
    if heavy && s.step > FIRST_STEP {
        let since = s.heavy_since.unwrap_or(now);
        if now.saturating_duration_since(since) >= HEAVY_HOLD {
            let value = (s.step * 0.5).max(FIRST_STEP);
            let hold = match s.ceiling {
                Some(c) if c.level == s.step => (c.hold * 2).min(CEILING_HOLD_MAX),
                _ => CEILING_HOLD,
            };
            return Tick::Step(
                Stepping {
                    step: value,
                    changed: now,
                    heavy_since: None,
                    ceiling: Some(Ceiling {
                        level: s.step,
                        until: now + hold,
                        hold,
                    }),
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
        // a measured frame that is not heavy ends the run; a frame with no
        // new measure leaves it pending, and nothing grows meanwhile
        return if load.measured() && !heavy {
            Tick::Hold(Stepping { heavy_since: None, ..s })
        } else {
            Tick::Wait
        };
    }
    let value = (s.step * 2.0).min(s.user);
    if s.ceiling.is_some_and(|c| value >= c.level && now < c.until) {
        return Tick::Wait;
    }
    let paced = interval.max(MIN_GAP);
    let floor = paced * FLOOR_INTERVALS;
    let gap = if heavy {
        floor
    } else {
        match (load.arrival_quiet(), load.fast()) {
            (true, true) => MIN_GAP,
            (false, false) => floor,
            _ => paced,
        }
    };
    if now.saturating_duration_since(s.changed) < gap {
        return Tick::Wait;
    }
    if value >= s.user {
        return Tick::End(s.user);
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
    const FRAME: Duration = Duration::from_millis(100);

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
    /// the steps and the distance at `until`, or at their end (the steps
    /// then sit at the user's distance and are not run further).
    fn run(mut s: Stepping, mut dd: f32, load: impl Fn(Duration) -> Load, t0: Instant, until: Duration) -> (Stepping, f32) {
        let mut t = Duration::ZERO;
        while t <= until {
            match at(s, dd, true, false, load(t), t0 + t) {
                Tick::Wait => {}
                Tick::Hold(next) => s = next,
                Tick::Step(next, value) => (s, dd) = (next, value),
                Tick::End(value) => {
                    assert_eq!(value, s.user, "only the user ends the steps elsewhere");
                    return (Stepping { step: value, ..s }, value);
                }
            }
            t += FRAME;
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
        let (s, dd) = run(s, dd, fast, t0, MIN_GAP * 2);
        assert_eq!(dd, 128.0);
        // the step to the user's distance, in about 6 s, not 60, ends them
        assert_eq!(at(s, dd, true, false, load(0, 8.0, 2.0), t0 + MIN_GAP * 3), Tick::End(256.0));
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
    fn one_busy_reason_keeps_the_legacy_pace_and_two_the_floor_pace() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, t0);
        // objects still arriving, frames fast: one step per interval
        let arriving = |_| load(WAITING_LIMIT + 1, 8.0, 2.0);
        assert_eq!(run(s, dd, arriving, t0, INTERVAL - Duration::from_millis(200)).1, 32.0);
        assert_eq!(run(s, dd, arriving, t0, INTERVAL).1, 64.0);
        // a slow but not heavy frame, arrival quiet: the same pace
        let slow = |_| load(0, 50.0, 4.0);
        assert_eq!(run(s, dd, slow, t0, INTERVAL).1, 64.0);
        // both busy: the floor pace (no burst on top of a burst), and still
        // the user's distance in bounded time
        let both = |_| load(WAITING_LIMIT + 1, 50.0, 20.0);
        let floor = INTERVAL * FLOOR_INTERVALS;
        assert_eq!(run(s, dd, both, t0, floor - Duration::from_millis(200)).1, 32.0);
        assert_eq!(run(s, dd, both, t0, floor).1, 64.0);
        assert_eq!(run(s, dd, both, t0, floor * 3).1, 256.0, "32 to 256 m in 3 floor steps");
    }

    #[test]
    fn a_heavy_frame_at_32_still_grows_at_the_floor_pace() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, t0);
        // heavy at 32 m: nothing lower, and no step back pending
        let heavy = |_| load(0, 200.0, 100.0);
        assert_eq!(run(s, dd, heavy, t0, INTERVAL).1, 32.0);
        let (s, dd) = run(s, dd, heavy, t0, INTERVAL * FLOOR_INTERVALS);
        assert_eq!(dd, 64.0);
        // heavy at 64 m: back to 32 after HEAVY_HOLD
        let (_, dd) = run(s, dd, heavy, t0 + INTERVAL * FLOOR_INTERVALS, HEAVY_HOLD + FRAME);
        assert_eq!(dd, 32.0);
    }

    #[test]
    fn a_pending_heavy_run_holds_the_growth() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, t0);
        let (s, dd) = run(s, dd, |_| quiet(), t0, MIN_GAP * 2);
        assert_eq!(dd, 128.0);
        // one heavy frame, then no new measure for a long time (a hidden
        // window): the run stays pending, nothing grows, nothing steps back
        let t1 = t0 + MIN_GAP * 2;
        let Tick::Hold(s) = at(s, dd, true, false, load(0, 120.0, 4.0), t1) else {
            panic!("a heavy run starts");
        };
        let (s, dd) = run(s, dd, |_| quiet(), t1, INTERVAL * FLOOR_INTERVALS * 2);
        assert_eq!((dd, s.heavy_since.is_some()), (128.0, true));
        // a measured frame that is not heavy ends it
        let t2 = t1 + INTERVAL * FLOOR_INTERVALS * 2;
        let Tick::Hold(s) = at(s, dd, true, false, load(0, 8.0, 2.0), t2) else {
            panic!("the run ends");
        };
        assert_eq!(s.heavy_since, None);
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
        assert_eq!(s.ceiling.map(|c| (c.level, c.hold)), Some((128.0, CEILING_HOLD)));
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
    fn a_distance_heavy_on_the_way_waits_longer_each_time() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, t0);
        // heavy from 128 m, fast under it
        let scene = |dd: f32| if dd >= 128.0 { load(0, 120.0, 60.0) } else { load(0, 8.0, 2.0) };
        let (mut s, mut dd) = (s, dd);
        let mut holds = Vec::new();
        let mut t = Duration::ZERO;
        while t <= Duration::from_secs(3600) {
            match at(s, dd, true, false, scene(dd), t0 + t) {
                Tick::Wait => {}
                Tick::Hold(next) => s = next,
                Tick::Step(next, value) => {
                    if value < s.step {
                        holds.push(next.ceiling.expect("a ceiling").hold);
                    }
                    (s, dd) = (next, value);
                }
                Tick::End(_) => panic!("never reaches the user's distance"),
            }
            t += FRAME;
        }
        // each step back from 128 m doubles the wait, up to its cap: 7 tries
        // in an hour, where a fixed 60 s would make about 60
        let min = Duration::from_secs(60);
        assert_eq!(
            holds,
            [min, min * 2, min * 4, min * 8, min * 16, CEILING_HOLD_MAX, CEILING_HOLD_MAX]
        );
        assert_eq!(CEILING_HOLD_MAX, min * 16);
        assert_eq!(dd, 64.0, "a level under the heavy one");
    }

    #[test]
    fn the_steps_end_at_the_users_distance_even_where_it_is_heavy() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, t0);
        // heavy at 256 m only: the steps reach it and end there, as the
        // Legacy Viewer's do; what comes after the arrival is Autotune's
        let scene = |dd: f32| if dd >= 256.0 { load(0, 120.0, 60.0) } else { load(0, 8.0, 2.0) };
        let (mut s, mut dd) = (s, dd);
        let mut t = Duration::ZERO;
        let end = loop {
            assert!(t <= Duration::from_secs(10), "the steps end in about 6 s");
            match at(s, dd, true, false, scene(dd), t0 + t) {
                Tick::Wait => {}
                Tick::Hold(next) => s = next,
                Tick::Step(next, value) => (s, dd) = (next, value),
                Tick::End(value) => break value,
            }
            t += FRAME;
        };
        assert_eq!(end, 256.0);
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
    fn a_slider_change_in_the_frame_of_a_teleport_is_the_users() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, t0);
        let (s, dd) = run(s, dd, |_| quiet(), t0, MIN_GAP);
        assert_eq!(dd, 64.0);
        // the user sets 200 m, and the teleport finishes before the next tick
        let (again, dd) = start(Some(s), 200.0, t0 + MIN_GAP);
        assert_eq!((again.user, dd), (200.0, 32.0));
        // lower than the steps' user value too
        let (again, _) = start(Some(s), 48.0, t0 + MIN_GAP);
        assert_eq!(again.user, 48.0);
    }

    #[test]
    fn a_draw_distance_already_at_32_ends_at_once() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 32.0, t0);
        assert_eq!(dd, 32.0);
        assert_eq!(at(s, dd, true, false, quiet(), t0), Tick::End(32.0));
        let (s, dd) = start(None, 20.0, t0);
        assert_eq!((s.step, dd), (20.0, 20.0));
        assert_eq!(at(s, dd, true, false, quiet(), t0), Tick::End(20.0));
    }

    #[test]
    fn reaching_the_users_distance_lets_go_of_the_draw_distance() {
        let t0 = Instant::now();
        let mut settings = Settings {
            draw_distance: 256.0,
            draw_distance_stepping: true,
            ..Settings::default()
        };
        begin(&mut settings, t0).expect("on");
        let mut t = Duration::ZERO;
        while let Some(steps) = settings.draw_stepping {
            assert!(t <= Duration::from_secs(10), "the steps end in about 6 s");
            match at(steps, settings.draw_distance, true, false, quiet(), t0 + t) {
                Tick::Wait => {}
                Tick::Hold(next) => settings.draw_stepping = Some(next),
                Tick::Step(next, value) => {
                    settings.draw_stepping = Some(next);
                    settings.draw_distance = value;
                }
                Tick::End(value) => {
                    settings.draw_stepping = None;
                    settings.draw_distance = value;
                }
            }
            t += FRAME;
        }
        // nothing holds it any more (Autotune pauses on `draw_stepping`)
        assert_eq!(settings.draw_distance, 256.0);
        assert_eq!(settings.to_saved().draw_distance, 256.0);
    }

    #[test]
    fn a_stale_or_photo_readback_does_not_count() {
        let mut r = Readback::default();
        assert_eq!(r.fresh(Some(120.0), Some(4.0), false), (Some(120.0), Some(4.0)));
        // the same measure given again: no new frame measured
        assert_eq!(r.fresh(Some(120.0), Some(4.0), false), (None, None));
        assert_eq!(r.fresh(Some(121.0), Some(4.0), false), (Some(121.0), Some(4.0)));
        // a photo's frames are not the world's load
        assert_eq!(r.fresh(Some(300.0), Some(90.0), true), (None, None));
        // a device without timestamps
        assert_eq!(Readback::default().fresh(None, None, false), (None, None));
        // a heavy readback seen again and again never steps back
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, t0);
        let (s, dd) = run(s, dd, |_| quiet(), t0, MIN_GAP * 2);
        assert_eq!(dd, 128.0);
        let mut r = Readback::default();
        let (mut s, mut dd) = (s, dd);
        let t1 = t0 + MIN_GAP * 2;
        let mut t = Duration::ZERO;
        while t <= Duration::from_secs(5) {
            let (gpu_ms, shadow_ms) = r.fresh(Some(120.0), Some(60.0), false);
            let stale = Load {
                waiting: 0,
                gpu_ms,
                shadow_ms,
            };
            match at(s, dd, true, false, stale, t1 + t) {
                Tick::Hold(next) => s = next,
                Tick::Step(next, value) => (s, dd) = (next, value),
                _ => {}
            }
            t += FRAME;
        }
        assert_eq!(dd, 128.0, "one heavy frame, seen 50 times");
    }

    #[test]
    fn no_step_under_32() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, t0);
        assert_eq!(run(s, dd, |_| load(0, 200.0, 100.0), t0, Duration::from_secs(3)).1, 32.0);
    }

    /// The saved draw distance: the user's while a step holds it, but one
    /// the user set since the last step (a graphics level, the slider) is
    /// theirs and saved as it is, though the steps only end at the next
    /// tick.
    #[test]
    fn a_distance_set_since_the_last_step_is_the_one_saved() {
        let t0 = Instant::now();
        let stepping = |step: f32| Stepping {
            user: 256.0,
            step,
            changed: t0,
            heavy_since: None,
            ceiling: None,
        };
        let stepped = Settings {
            draw_distance: 64.0,
            draw_stepping: Some(stepping(64.0)),
            ..Settings::default()
        };
        assert_eq!(stepped.to_saved().draw_distance, 256.0);
        let set_since = Settings {
            draw_distance: 128.0,
            draw_stepping: Some(stepping(64.0)),
            ..Settings::default()
        };
        assert_eq!(set_since.to_saved().draw_distance, 128.0);
    }

    #[test]
    fn no_start_under_the_photo_quality() {
        let mut s = Settings {
            draw_distance: 256.0,
            draw_distance_stepping: true,
            ..Settings::default()
        };
        s.photo_held = Some(crate::snapshot::quality::boost(&mut s));
        let boosted = s.draw_distance;
        assert_eq!(begin(&mut s, Instant::now()), None);
        assert_eq!((s.draw_stepping, s.draw_distance), (None, boosted));
        // off: no steps either
        let mut off = Settings::default();
        assert_eq!(begin(&mut off, Instant::now()), None);
    }

    #[test]
    fn a_photo_after_a_step_back_keeps_the_users_distance() {
        let t0 = Instant::now();
        let mut s = Settings {
            draw_distance: 256.0,
            draw_distance_stepping: true,
            ..Settings::default()
        };
        let steps = begin(&mut s, t0).expect("on");
        let (steps, dd) = run(steps, s.draw_distance, |_| quiet(), t0, MIN_GAP * 2);
        assert_eq!(dd, 128.0);
        let (steps, dd) = run(steps, dd, |_| load(0, 120.0, 4.0), t0 + MIN_GAP * 2, HEAVY_HOLD + FRAME);
        assert_eq!((dd, steps.user), (64.0, 256.0));
        s.draw_stepping = Some(steps);
        s.draw_distance = dd;
        // the photo quality: its user value is 256 m, not the step's 64
        let held = crate::snapshot::quality::boost(&mut s);
        assert_eq!(held.user().draw_distance, 256.0);
        assert_eq!(s.draw_stepping, None, "the photo ends the steps");
        s.photo_held = Some(held);
        assert_eq!(s.to_saved().draw_distance, 256.0, "saved during the photo");
        let held = s.photo_held.take().expect("held");
        held.restore(&mut s);
        assert_eq!(s.draw_distance, 256.0, "and after it");
        assert_eq!(s.to_saved().draw_distance, 256.0);
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
