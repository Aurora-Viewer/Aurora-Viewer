//! Region restart alert: the countdown of the agent's region and its rules
//! (port of LLFloaterRegionRestarting, indra/newview/llfloaterregionrestarting.cpp,
//! and of the restart branch of attempt_standard_notification /
//! process_alert_core, llviewermessage.cpp; originally LGPL 2.1).
//!
//! Firestorm opens its box at the first notice of the simulator, resets the
//! count and shakes the view at each one, counts down one second a second,
//! closes on a region change or "Region restart cancelled.". Aurora's alert
//! (`ui/region_restart.rs`, the human's mockup, variant B) keeps those rules
//! and escalates with the time left: amber from 5 minutes, orange from one,
//! red from 30 seconds, the final count from 10. It plays its sound once per
//! step (5 min, 1 min, 30 s) instead of at every notice, and shakes at its
//! opening and at 30 s. Minimized, it is a pill; closed, it comes back as a
//! pill at 30 s.

use aurora_net::RegionHandle;
use serde::{Deserialize, Serialize};

/// The steps of the countdown, by the seconds left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// More than a minute: amber, calm.
    Calm,
    /// One minute or less: orange.
    Near,
    /// 30 seconds or less: red, a second shake.
    Red,
    /// The last 10 seconds.
    Final,
    /// 0: the region restarts.
    Zero,
}

impl Level {
    pub fn of(seconds: i32) -> Level {
        match seconds {
            s if s <= 0 => Level::Zero,
            s if s <= FINAL => Level::Final,
            s if s <= RED => Level::Red,
            s if s <= NEAR => Level::Near,
            _ => Level::Calm,
        }
    }

    /// The steps that play the sound (5 min, 1 min, 30 s).
    fn sounded(self) -> Option<usize> {
        match self {
            Level::Calm => Some(0),
            Level::Near => Some(1),
            Level::Red => Some(2),
            Level::Final | Level::Zero => None,
        }
    }
}

const NEAR: i32 = 60;
const RED: i32 = 30;
const FINAL: i32 = 10;

/// What became of the restart.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// Counting down (or at 0, waiting for the simulator to stop).
    Live,
    /// A manager cancelled it: green, then the alert leaves.
    Cancelled,
    /// We left the region (teleport, crossing): the arrival is told, then
    /// the alert leaves.
    Left { arrival: String },
}

/// How long an ended alert stays, then fades (the mockup's 4.2 s / 3 s and
/// 400 ms).
pub const CANCELLED_STAYS: f64 = 4.2;
pub const LEFT_STAYS: f64 = 3.0;
pub const FADE_OUT: f64 = 0.4;

/// The shake at the opening of the alert (Preferences › Interface › Region
/// restart; FSNoScreenShakeOnRegionRestart, inverted, keeps Firestorm's
/// default: a shake).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Shake {
    /// The alert itself trembles (0.7 s, damped).
    #[default]
    Alert,
    /// The view, like Firestorm (1.8 s).
    View,
    None,
}

/// Preferences › Interface › Region restart (Firestorm's defaults).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// FSUseNewRegionRestartNotification: the alert; off, a toast at each
    /// notice of the simulator.
    pub show_alert: bool,
    pub shake: Shake,
    /// FSReportRegionRestartToChat: whisper `region_restart_in:<s>` on
    /// `channel` (FSRegionRestartAnnounceChannel) for HUDs and scripts.
    pub announce: bool,
    pub channel: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            show_alert: true,
            shake: Shake::Alert,
            announce: false,
            channel: 473_405,
        }
    }
}

/// What a notice or the passing time asks of the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    /// The alert opens: it drops in, the taskbar icon flashes.
    Open,
    /// The restart sound (UISndRestart, or UISndRestartOpenSim).
    Sound,
    /// The shake (of the alert or of the view, by the setting).
    Shake,
    /// A notice of the simulator, with its seconds (the nearby chat line,
    /// the channel announcement).
    Notice(i32),
    /// The step changed (`Level`).
    Step(Level),
    /// A closed alert comes back as a pill (30 s left).
    Back,
}

/// The countdown of one restart.
#[derive(Debug, Clone)]
pub struct Restart {
    pub region: String,
    pub handle: Option<RegionHandle>,
    pub opensim: bool,
    /// Seconds left (fractional: the count goes on between two frames).
    remaining: f64,
    /// The seconds of the first notice: the bar empties over them.
    pub total: f64,
    pub phase: Phase,
    /// Seconds since the phase ended (cancelled, left).
    pub ended_for: f64,
    /// Reduced to a pill; `closed`: gone from the view (still in the bell).
    pub minimized: bool,
    pub closed: bool,
    sounded: [bool; 3],
}

impl Restart {
    /// The seconds shown: whole, rounded up (Firestorm shows the time the
    /// simulator gave, then one less each second).
    pub fn seconds(&self) -> i32 {
        self.remaining.ceil().max(0.0) as i32
    }

    pub fn level(&self) -> Level {
        Level::of(self.seconds())
    }

    /// Still counting (not cancelled nor left, not yet at 0).
    pub fn live(&self) -> bool {
        self.phase == Phase::Live && self.level() != Level::Zero
    }

    /// What is left of the bar (0..1).
    pub fn fraction(&self) -> f32 {
        (self.remaining / self.total.max(1.0)).clamp(0.0, 1.0) as f32
    }

    /// Gone for good: ended and faded out.
    fn finished(&self) -> bool {
        let stays = match self.phase {
            Phase::Live => return false,
            Phase::Cancelled => CANCELLED_STAYS,
            Phase::Left { .. } => LEFT_STAYS,
        };
        self.ended_for >= stays + FADE_OUT
    }

    /// The fade out of an ended alert (0 shown, 1 gone).
    pub fn fade(&self) -> f32 {
        let stays = match self.phase {
            Phase::Live => return 0.0,
            Phase::Cancelled => CANCELLED_STAYS,
            Phase::Left { .. } => LEFT_STAYS,
        };
        (((self.ended_for - stays) / FADE_OUT).clamp(0.0, 1.0)) as f32
    }

    /// The sound of the step entered, once per step.
    fn sound_for(&mut self, level: Level, out: &mut Vec<Cue>) {
        if let Some(i) = level.sounded()
            && !self.sounded[i]
        {
            self.sounded[i] = true;
            out.push(Cue::Sound);
        }
    }

    /// Entering `level` from a higher one: its sound, its shake.
    fn enter(&mut self, from: Level, to: Level, out: &mut Vec<Cue>) {
        if to <= from {
            return;
        }
        out.push(Cue::Step(to));
        self.sound_for(to, out);
        if from < Level::Red && to >= Level::Red && to != Level::Zero {
            // a closed alert comes back as a pill (the mockup's choice)
            if self.closed {
                self.closed = false;
                self.minimized = true;
                out.push(Cue::Back);
            }
            out.push(Cue::Shake);
        }
    }
}

/// The restart of the agent's region, if one is announced.
#[derive(Debug, Default)]
pub struct Tracker {
    pub alert: Option<Restart>,
}

impl Tracker {
    /// A notice of the simulator. `region` names it when the simulator did
    /// not (OpenSim: the agent's region, LLViewerRegion::getName); `classic`:
    /// Firestorm's box, which plays its sound and shakes at every notice
    /// (LLFloaterRegionRestarting::updateTime).
    pub fn notice(
        &mut self,
        notice: &aurora_net::region_restart::Notice,
        region: &str,
        handle: Option<RegionHandle>,
        classic: bool,
    ) -> Vec<Cue> {
        let mut out = vec![Cue::Notice(notice.seconds)];
        let name = notice.region.clone().unwrap_or_else(|| region.to_owned());
        let seconds = f64::from(notice.seconds.max(0));
        let fresh = !self.alert.as_ref().is_some_and(|a| a.phase == Phase::Live);
        if fresh {
            let mut a = Restart {
                region: name,
                handle,
                opensim: notice.opensim,
                remaining: seconds,
                total: seconds.max(1.0),
                phase: Phase::Live,
                ended_for: 0.0,
                minimized: false,
                closed: false,
                sounded: [false; 3],
            };
            out.push(Cue::Open);
            out.push(Cue::Shake);
            out.push(Cue::Step(a.level()));
            // the first notice always rings, whatever its step
            let level = a.level();
            a.sound_for(level, &mut out);
            if !out.contains(&Cue::Sound) {
                out.push(Cue::Sound);
            }
            self.alert = Some(a);
        } else if let Some(a) = self.alert.as_mut() {
            let from = a.level();
            a.remaining = seconds;
            a.opensim |= notice.opensim;
            if notice.region.is_some() {
                a.region = name;
            }
            let to = a.level();
            a.enter(from, to, &mut out);
            if classic {
                if !out.contains(&Cue::Sound) {
                    out.push(Cue::Sound);
                }
                if !out.contains(&Cue::Shake) {
                    out.push(Cue::Shake);
                }
            }
        }
        out
    }

    /// The count goes on by `dt` seconds (`real`: the wall clock, for the
    /// fade of an ended alert; the demo plays the count faster).
    pub fn advance(&mut self, dt: f64, real: f64) -> Vec<Cue> {
        let mut out = Vec::new();
        let Some(a) = self.alert.as_mut() else {
            return out;
        };
        if a.phase == Phase::Live {
            let from = a.level();
            a.remaining = (a.remaining - dt.max(0.0)).max(0.0);
            let to = a.level();
            a.enter(from, to, &mut out);
        } else {
            a.ended_for += real.max(0.0);
            if a.finished() {
                self.alert = None;
            }
        }
        out
    }

    /// "Region restart cancelled.": the alert turns green, then leaves.
    /// Whether there was one to cancel.
    pub fn cancelled(&mut self) -> bool {
        match self.alert.as_mut() {
            Some(a) if a.phase == Phase::Live => {
                a.phase = Phase::Cancelled;
                a.ended_for = 0.0;
                a.minimized = false;
                a.closed = false;
                true
            }
            _ => false,
        }
    }

    /// The agent's region is now `handle` (LLAgent's region changed
    /// callback): leaving the restarting region ends the alert.
    pub fn region_changed(&mut self, handle: RegionHandle, arrival: &str) {
        if let Some(a) = self.alert.as_mut()
            && a.phase == Phase::Live
            && a.handle.is_some_and(|h| h != handle)
        {
            a.phase = Phase::Left {
                arrival: arrival.to_owned(),
            };
            a.ended_for = 0.0;
            a.minimized = false;
            a.closed = false;
        }
    }

    pub fn minimize(&mut self, on: bool) {
        if let Some(a) = self.alert.as_mut() {
            a.minimized = on && a.live();
            if !on {
                a.closed = false;
            }
        }
    }

    /// The close button: an alert still counting stays in the bell (and
    /// comes back at 30 s); an ended one goes.
    pub fn close(&mut self) {
        match self.alert.as_mut() {
            Some(a) if a.live() => {
                a.closed = true;
                a.minimized = false;
            }
            _ => self.alert = None,
        }
    }

    /// "Show the alert" (the bell's card).
    pub fn show(&mut self) {
        if let Some(a) = self.alert.as_mut() {
            a.closed = false;
            a.minimized = false;
        }
    }
}

/// Firestorm's view shake (LLFloaterRegionRestarting::draw): every 25 ms
/// the camera is panned left, up, right then down, the horizontal pans a
/// quarter of the vertical ones, for 1.8 s (the length of the sound), the
/// magnitude falling with the square of the time left. A pan key moves the
/// camera by `magnitude × PAN_RATE / fps` metres for one frame
/// (LLAgentCamera::updateCamera, PAN_RATE 5 m/s); Aurora takes 60 frames a
/// second for it, so that the shake does not shrink at a high frame rate.
/// Returns the offset (right, up) in metres `t` seconds after the start.
pub fn view_shake(t: f64) -> Option<(f32, f32)> {
    const INTERVAL: f64 = 0.025;
    const DURATION: f64 = 1.8;
    const MAGNITUDE: f64 = 1.5;
    const HORIZONTAL: f64 = 0.25;
    const METRES: f64 = 5.0 / 60.0;
    if !(0.0..DURATION).contains(&t) {
        return None;
    }
    let step = (t / INTERVAL) as u64;
    let cycle = step / 4;
    let shaking = (cycle * 4) as f64 * INTERVAL;
    let left = (DURATION - shaking) / DURATION;
    let m = MAGNITUDE * left * left * METRES;
    // where the four pans leave the camera within a cycle
    let (x, y) = match step % 4 {
        0 => (-m * HORIZONTAL, 0.0),
        1 => (-m * HORIZONTAL, m),
        2 => (0.0, m),
        _ => (0.0, 0.0),
    };
    Some((x as f32, y as f32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::region_restart::Notice;

    fn notice(seconds: i32) -> Notice {
        Notice {
            seconds,
            region: Some("Aurora Demo".into()),
            opensim: false,
        }
    }

    fn sounds(cues: &[Cue]) -> usize {
        cues.iter().filter(|c| **c == Cue::Sound).count()
    }

    #[test]
    fn the_steps_follow_the_time_left() {
        assert_eq!(Level::of(300), Level::Calm);
        assert_eq!(Level::of(61), Level::Calm);
        assert_eq!(Level::of(60), Level::Near);
        assert_eq!(Level::of(31), Level::Near);
        assert_eq!(Level::of(30), Level::Red);
        assert_eq!(Level::of(11), Level::Red);
        assert_eq!(Level::of(10), Level::Final);
        assert_eq!(Level::of(1), Level::Final);
        assert_eq!(Level::of(0), Level::Zero);
    }

    #[test]
    fn the_sound_rings_once_per_step_even_when_the_simulator_repeats() {
        let mut t = Tracker::default();
        let first = t.notice(&notice(300), "", Some(1), false);
        assert!(first.contains(&Cue::Open) && first.contains(&Cue::Shake));
        assert_eq!(sounds(&first), 1);
        // the simulator repeats at 4 and 2 minutes: no sound, no shake
        assert_eq!(sounds(&t.notice(&notice(240), "", Some(1), false)), 0);
        assert!(!t.notice(&notice(120), "", Some(1), false).contains(&Cue::Shake));
        // the count crosses 1 minute, then 30 s
        let near = t.advance(60.5, 60.5);
        assert_eq!((sounds(&near), near.contains(&Cue::Shake)), (1, false));
        assert_eq!(sounds(&t.notice(&notice(60), "", Some(1), false)), 0, "1 minute rang already");
        let red = t.advance(30.0, 30.0);
        assert_eq!((sounds(&red), red.contains(&Cue::Shake)), (1, true));
        // the final count and 0 are silent
        assert_eq!(sounds(&t.advance(25.0, 25.0)), 0);
        let zero = t.advance(10.0, 10.0);
        assert!(zero.contains(&Cue::Step(Level::Zero)) && sounds(&zero) == 0);
        let a = t.alert.as_ref().expect("still shown at 0");
        assert_eq!((a.seconds(), a.live()), (0, false));
    }

    #[test]
    fn firestorms_box_rings_and_shakes_at_every_notice() {
        let mut t = Tracker::default();
        t.notice(&notice(300), "", Some(1), true);
        let again = t.notice(&notice(240), "", Some(1), true);
        assert_eq!((sounds(&again), again.contains(&Cue::Shake)), (1, true));
        assert_eq!(t.alert.as_ref().map(Restart::seconds), Some(240), "the count follows the simulator");
    }

    #[test]
    fn a_first_notice_late_in_the_count_still_rings() {
        let mut t = Tracker::default();
        let cues = t.notice(&notice(8), "", Some(1), false);
        assert_eq!(sounds(&cues), 1);
        assert_eq!(t.alert.as_ref().map(Restart::level), Some(Level::Final));
    }

    #[test]
    fn minimized_closed_and_back_at_thirty_seconds() {
        let mut t = Tracker::default();
        t.notice(&notice(90), "", Some(1), false);
        t.minimize(true);
        assert!(t.alert.as_ref().is_some_and(|a| a.minimized));
        t.close();
        let a = t.alert.as_ref().expect("closed, still in the bell");
        assert!(a.closed && !a.minimized);
        let cues = t.advance(60.0, 60.0);
        assert!(cues.contains(&Cue::Back));
        let a = t.alert.as_ref().expect("back");
        assert!(!a.closed && a.minimized, "it comes back as a pill");
        // closed after 30 s, it stays closed
        t.close();
        assert!(!t.advance(5.0, 5.0).contains(&Cue::Back));
        t.show();
        let a = t.alert.as_ref().expect("shown");
        assert!(!a.closed && !a.minimized);
    }

    #[test]
    fn cancelled_or_left_it_says_so_then_leaves() {
        let mut t = Tracker::default();
        t.notice(&notice(120), "", Some(1), false);
        assert!(t.cancelled());
        assert!(!t.cancelled(), "only once");
        t.advance(10.0, CANCELLED_STAYS);
        assert!(t.alert.is_some());
        t.advance(10.0, FADE_OUT + 0.01);
        assert!(t.alert.is_none());

        t.notice(&notice(120), "", Some(1), false);
        t.region_changed(1, "Aurora Demo");
        assert_eq!(t.alert.as_ref().map(|a| a.phase.clone()), Some(Phase::Live), "same region");
        t.region_changed(2, "Iris");
        assert_eq!(
            t.alert.as_ref().map(|a| a.phase.clone()),
            Some(Phase::Left { arrival: "Iris".into() })
        );
        t.advance(0.0, LEFT_STAYS + FADE_OUT);
        assert!(t.alert.is_none());
    }

    #[test]
    fn a_notice_after_a_cancel_opens_a_new_alert() {
        let mut t = Tracker::default();
        t.notice(&notice(300), "", Some(1), false);
        t.cancelled();
        let cues = t.notice(&notice(120), "", Some(1), false);
        assert!(cues.contains(&Cue::Open));
        assert_eq!(t.alert.as_ref().map(|a| a.total), Some(120.0));
    }

    #[test]
    fn opensim_notices_name_the_agents_region() {
        let mut t = Tracker::default();
        let n = Notice {
            seconds: 120,
            region: None,
            opensim: true,
        };
        t.notice(&n, "Sandbox", Some(1), false);
        let a = t.alert.as_ref().expect("alert");
        assert_eq!((a.region.as_str(), a.opensim), ("Sandbox", true));
    }

    #[test]
    fn the_view_shake_is_mostly_vertical_and_dies_out() {
        assert_eq!(view_shake(-0.1), None);
        assert_eq!(view_shake(1.8), None);
        let (x, y) = view_shake(0.03).expect("up");
        assert!(y > 0.0 && x < 0.0 && x.abs() < y);
        assert_eq!(view_shake(0.09), Some((0.0, 0.0)), "back after the four pans");
        let early = view_shake(0.03).map(|v| v.1).unwrap_or_default();
        let late = view_shake(1.63).map(|v| v.1).unwrap_or_default();
        assert!(late < early * 0.1);
    }
}
