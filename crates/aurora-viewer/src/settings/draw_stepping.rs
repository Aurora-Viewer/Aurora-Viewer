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
//! back at once.

use std::time::{Duration, Instant};

/// RenderFarClip set at the start of the steps (gLastDrawDistanceStep).
pub const FIRST_STEP: f32 = 32.0;

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
        },
        step,
    )
}

/// What a frame does with steps under way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tick {
    /// Nothing changes.
    Wait,
    /// The draw distance takes this value; the steps go on.
    Step(Stepping, f32),
    /// The steps end; the draw distance takes this value (the user's, or
    /// theirs as they changed it).
    End(f32),
}

/// One frame of the steps: the user changed the draw distance (`draw_distance`
/// is not the step), the option was turned off, or a step is due. No step
/// while teleporting (`TELEPORT_NONE` in the Legacy Viewer).
pub fn tick(s: Stepping, draw_distance: f32, enabled: bool, teleporting: bool, interval: Duration, now: Instant) -> Tick {
    if draw_distance != s.step {
        return Tick::End(draw_distance);
    }
    if !enabled {
        return Tick::End(s.user);
    }
    if teleporting || now < s.next {
        return Tick::Wait;
    }
    let value = (s.step * 2.0).min(s.user);
    if value >= s.user {
        return Tick::End(s.user);
    }
    Tick::Step(
        Stepping {
            step: value,
            next: now + interval,
            ..s
        },
        value,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTERVAL: Duration = Duration::from_secs(20);

    #[test]
    fn it_doubles_from_32_up_to_the_users_value() {
        let t0 = Instant::now();
        let (mut s, mut dd) = start(None, 256.0, INTERVAL, t0);
        assert_eq!(dd, 32.0);
        let mut seen = vec![dd];
        let mut now = t0;
        loop {
            // nothing before the interval
            assert_eq!(tick(s, dd, true, false, INTERVAL, now), Tick::Wait);
            now += INTERVAL;
            match tick(s, dd, true, false, INTERVAL, now) {
                Tick::Step(next, value) => {
                    s = next;
                    dd = value;
                    seen.push(dd);
                }
                Tick::End(value) => {
                    seen.push(value);
                    break;
                }
                Tick::Wait => panic!("a step was due"),
            }
        }
        assert_eq!(seen, [32.0, 64.0, 128.0, 256.0]);
    }

    #[test]
    fn the_last_step_stops_at_the_users_value() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 96.0, INTERVAL, t0);
        let Tick::Step(s, dd) = tick(s, dd, true, false, INTERVAL, t0 + INTERVAL) else {
            panic!("64 m first")
        };
        assert_eq!(dd, 64.0);
        assert_eq!(tick(s, dd, true, false, INTERVAL, t0 + INTERVAL * 2), Tick::End(96.0));
    }

    #[test]
    fn a_change_by_the_user_ends_the_steps_and_keeps_it() {
        let t0 = Instant::now();
        let (s, _) = start(None, 256.0, INTERVAL, t0);
        assert_eq!(tick(s, 100.0, true, false, INTERVAL, t0), Tick::End(100.0));
    }

    #[test]
    fn turning_it_off_gives_the_users_value_back_at_once() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, INTERVAL, t0);
        assert_eq!(tick(s, dd, false, false, INTERVAL, t0), Tick::End(256.0));
    }

    #[test]
    fn no_step_while_teleporting_and_a_new_start_keeps_the_users_value() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 256.0, INTERVAL, t0);
        assert_eq!(tick(s, dd, true, true, INTERVAL, t0 + INTERVAL * 3), Tick::Wait);
        // a teleport during the steps: back to 32, the user's 256 kept
        let (again, dd) = start(Some(s), dd, INTERVAL, t0 + INTERVAL);
        assert_eq!((again.user, dd), (256.0, 32.0));
    }

    #[test]
    fn a_draw_distance_already_at_32_does_not_step() {
        let t0 = Instant::now();
        let (s, dd) = start(None, 32.0, INTERVAL, t0);
        assert_eq!(dd, 32.0);
        assert_eq!(tick(s, dd, true, false, INTERVAL, t0 + INTERVAL), Tick::End(32.0));
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
