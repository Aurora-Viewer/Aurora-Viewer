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
//! back at once. The steps also read the load, which the Legacy Viewer does
//! not: a step that is due waits while objects are still arriving or the
//! last finished frame was too slow, and a frame that stays slow steps the
//! distance back one level. Reaching the user's distance keeps that watch.

use std::time::{Duration, Instant};

/// RenderFarClip set at the start of the steps (gLastDrawDistanceStep).
pub const FIRST_STEP: f32 = 32.0;

/// Geometry and textures still arriving. A step waits above this.
pub const WAITING_LIMIT: u32 = 48;

/// A finished frame at or under this, in milliseconds, may open more distance.
pub const GPU_OK_MS: f32 = 33.0;

/// The shadow-depth pass at or under this, in milliseconds, may open more distance.
pub const SHADOW_OK_MS: f32 = 16.0;

/// A frame slower than this, in milliseconds, can step the distance back.
pub const GPU_HEAVY_MS: f32 = 80.0;

/// A shadow-depth pass slower than this, in milliseconds, can step the distance back.
pub const SHADOW_HEAVY_MS: f32 = 40.0;

/// How long a heavy frame must last before the distance steps back.
pub const HEAVY_HOLD: Duration = Duration::from_millis(500);

/// How soon a refused step is tried again. Two steps that do happen stay an
/// interval apart.
pub const RETRY: Duration = Duration::from_secs(2);

/// What the last finished frame was doing. Absent GPU times do not block:
/// the first frames, and a device without timestamps, still follow the timer
/// once the arrival queue is quiet.
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

    fn blocks_growth(&self) -> bool {
        self.waiting > WAITING_LIMIT || self.slower_than(GPU_OK_MS, SHADOW_OK_MS)
    }

    fn heavy(&self) -> bool {
        self.slower_than(GPU_HEAVY_MS, SHADOW_HEAVY_MS)
    }

    /// The queue is the only reason to wait: the frame itself is cheap.
    fn queue_only(&self) -> bool {
        self.waiting > WAITING_LIMIT && !self.slower_than(GPU_OK_MS, SHADOW_OK_MS)
    }
}

/// The steps under way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stepping {
    /// The user's draw distance, given back at the end (gSavedDrawDistance).
    pub user: f32,
    /// The value last set (gLastDrawDistanceStep): another one in the
    /// setting means the user changed it.
    pub step: f32,
    /// When the next step is due (gTeleportArrivalTimer).
    pub next: Instant,
    /// When the current heavy run began. Cleared by a cheap frame.
    pub heavy_since: Option<Instant>,
    /// Due steps held back only because objects were still arriving. One is
    /// enough: a queue that never drains does not pin the distance when the
    /// frame itself is cheap.
    pub queue_holds: u8,
}

/// Starts the steps (or starts them over, keeping the user's value of steps
/// already under way); returns the draw distance to use.
pub fn start(current: Option<Stepping>, draw_distance: f32, interval: Duration, now: Instant) -> (Stepping, f32) {
    let user = current.map_or(draw_distance, |s| s.user.max(draw_distance));
    let step = FIRST_STEP.min(user);
    (
        Stepping {
            user,
            step,
            next: now + interval,
            heavy_since: None,
            queue_holds: 0,
        },
        step,
    )
}

/// What a frame does with steps under way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tick {
    /// Nothing changes.
    Wait,
    /// The watch changed; the draw distance did not.
    Hold(Stepping),
    /// The draw distance takes this value; the steps go on.
    Step(Stepping, f32),
    /// The steps end; the draw distance takes this value (the user's, or
    /// theirs as they changed it).
    End(f32),
}

/// One frame of the steps: the user changed the draw distance (`draw_distance`
/// is not the step), the option was turned off, the frame stayed too heavy,
/// or a step is due and the load allows it. No step while teleporting
/// (`TELEPORT_NONE` in the Legacy Viewer).
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
                    next: now + interval,
                    heavy_since: None,
                    queue_holds: 0,
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
    if now < s.next {
        return Tick::Wait;
    }
    // A queue that already held one step does not pin a cheap frame.
    if load.blocks_growth() && !(load.queue_only() && s.queue_holds >= 1) {
        return Tick::Hold(Stepping {
            next: now + RETRY,
            queue_holds: if load.queue_only() { s.queue_holds.saturating_add(1) } else { 0 },
            ..s
        });
    }
    let value = (s.step * 2.0).min(s.user);
    let next = Stepping {
        step: value,
        next: now + interval,
        heavy_since: None,
        queue_holds: 0,
        ..s
    };
    if value == s.step {
        return Tick::Hold(next);
    }
    Tick::Step(next, value)
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

    #[test]
    fn it_doubles_from_32_up_to_the_users_value() {
        let t0 = Instant::now();
        let (mut s, mut dd) = start(None, 256.0, INTERVAL, t0);
        assert_eq!(dd, 32.0);
        let mut seen = vec![dd];
        let mut now = t0;
        loop {
            // nothing before the interval
            assert_eq!(at(s, dd, true, false, quiet(), now), Tick::Wait);
            now += INTERVAL;
            match at(s, dd, true, false, quiet(), now) {
                Tick::Step(next, value) => {
                    s = next;
                    dd = value;
                    seen.push(dd);
                    if dd >= s.user {
                        break;
                    }
                }
                Tick::End(value) => {
                    seen.push(value);
                    break;
                }
                Tick::Wait | Tick::Hold(_) => panic!("a step was due"),
            }
        }
        assert_eq!(seen, [32.0, 64.0, 128.0, 256.0]);
        // the saved distance stays under watch, so a later heavy frame can step back
        assert!(matches!(at(s, dd, true, false, quiet(), now + INTERVAL), Tick::Hold(_)));
    }

    #[test]
    fn the_last_step_stops_at_the_users_value() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 96.0, INTERVAL, t0);
        let Tick::Step(s, dd) = at(s, dd, true, false, quiet(), t0 + INTERVAL) else {
            panic!("64 m first")
        };
        assert_eq!(dd, 64.0);
        let Tick::Step(s, dd) = at(s, dd, true, false, quiet(), t0 + INTERVAL * 2) else {
            panic!("the user's 96 m")
        };
        assert_eq!(dd, 96.0);
        assert!(matches!(at(s, dd, true, false, quiet(), t0 + INTERVAL * 3), Tick::Hold(_)));
    }

    #[test]
    fn a_change_by_the_user_ends_the_steps_and_keeps_it() {
        let t0 = Instant::now();
        let (s, _) = start(None, 256.0, INTERVAL, t0);
        assert_eq!(at(s, 100.0, true, false, quiet(), t0), Tick::End(100.0));
    }

    #[test]
    fn turning_it_off_gives_the_users_value_back_at_once() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, INTERVAL, t0);
        assert_eq!(at(s, dd, false, false, quiet(), t0), Tick::End(256.0));
    }

    #[test]
    fn no_step_while_teleporting_and_a_new_start_keeps_the_users_value() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, INTERVAL, t0);
        assert_eq!(at(s, dd, true, true, quiet(), t0 + INTERVAL * 3), Tick::Wait);
        // a teleport during the steps: back to 32, the user's 256 kept
        let (again, dd) = start(Some(s), dd, INTERVAL, t0 + INTERVAL);
        assert_eq!((again.user, dd), (256.0, 32.0));
    }

    #[test]
    fn a_draw_distance_already_at_32_does_not_step() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 32.0, INTERVAL, t0);
        assert_eq!(dd, 32.0);
        assert!(matches!(at(s, dd, true, false, quiet(), t0 + INTERVAL), Tick::Hold(_)));
    }

    #[test]
    fn a_due_step_waits_while_the_frame_or_the_queue_is_busy() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, INTERVAL, t0);
        let slow = Load {
            waiting: 0,
            gpu_ms: Some(40.0),
            shadow_ms: Some(4.0),
        };
        let Tick::Hold(held) = at(s, dd, true, false, slow, t0 + INTERVAL) else {
            panic!("a slow frame holds the step")
        };
        assert_eq!(held.step, 32.0);
        let queued = Load {
            waiting: WAITING_LIMIT + 1,
            gpu_ms: Some(10.0),
            shadow_ms: Some(2.0),
        };
        let Tick::Hold(held) = at(s, dd, true, false, queued, t0 + INTERVAL) else {
            panic!("a full queue holds the first step")
        };
        assert_eq!(held.queue_holds, 1);
        // the same queue, once it has already held a step, lets a cheap frame grow
        let Tick::Step(_, value) = at(held, dd, true, false, queued, t0 + INTERVAL + RETRY) else {
            panic!("one hold is enough")
        };
        assert_eq!(value, 64.0);
    }

    #[test]
    fn a_heavy_frame_steps_back_one_level() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, INTERVAL, t0);
        let Tick::Step(at_64, dd) = at(s, dd, true, false, quiet(), t0 + INTERVAL) else {
            panic!("64 m")
        };
        let heavy = Load {
            waiting: 0,
            gpu_ms: Some(120.0),
            shadow_ms: Some(90.0),
        };
        let began = t0 + INTERVAL + Duration::from_millis(10);
        let Tick::Hold(latched) = at(at_64, dd, true, false, heavy, began) else {
            panic!("the heavy run starts")
        };
        assert_eq!(at(latched, dd, true, false, heavy, began + Duration::from_millis(200)), Tick::Wait);
        let Tick::Step(_, value) = at(latched, dd, true, false, heavy, began + HEAVY_HOLD) else {
            panic!("step back")
        };
        assert_eq!(value, 32.0);
        // a cheap frame clears a run that has not lasted
        let Tick::Hold(cleared) = at(latched, dd, true, false, quiet(), began + Duration::from_millis(20)) else {
            panic!("cleared")
        };
        assert!(cleared.heavy_since.is_none());
    }

    #[test]
    fn the_saved_file_keeps_the_users_draw_distance() {
        let mut settings = crate::settings::Settings {
            draw_distance: 256.0,
            ..crate::settings::Settings::default()
        };
        let (steps, value) = start(None, settings.draw_distance, INTERVAL, Instant::now());
        settings.draw_stepping = Some(steps);
        settings.draw_distance = value;
        assert_eq!(settings.to_saved().draw_distance, 256.0);
        assert_eq!(settings.draw_distance, 32.0);
    }
}
