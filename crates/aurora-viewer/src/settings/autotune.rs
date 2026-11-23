//! Autotune: the Performance window's target frame rate. While it runs, the
//! viewer lowers the number of avatars drawn in full (the others become
//! impostors) and the draw distance until the frames come at the rate the
//! user asked for, and gives them back when there is room.
//!
//! Port of the Legacy Viewer's Autotune: LLPerfStats::StatsRecorder::
//! updateAvatarParams and LLPerfStats::Tunables (indra/newview/llperfstats.cpp,
//! llperfstats.h), the handlers of llviewercontrol.cpp (handleAutoTune*),
//! the controls of FSFloaterPerformance (fsfloaterperformance.cpp,
//! floater_fs_performance.xml, panel_fs_performance_autotune.xml) and the
//! settings of settings.xml: AutoTuneFPS (off), AutoTuneLock ("Continuous",
//! off), KeepAutoTuneLock (on), TargetFPS (15, 1 to 300), TuningFPSStrategy
//! (1, avatars and scene), AutoTuneImpostorByDistEnabled (off),
//! AutoTuneImpostorFarAwayDistance (64 m, 16 to 256) and
//! AutoTuneRenderFarClipMin (32 m, 32 to 256). Originally LGPL 2.1,
//! Copyright (C) 2022 Linden Research, Inc.
//!
//! As there: off by default; the frame time is the median of the last 50
//! frames, without the frame limiter's wait; a 10 % band around the target;
//! one change at a time, the next one a while after; going back up only
//! after three times that; the draw distance steps by 10 m (DD_STEP) down to
//! the user's minimum; no tuning while the window is in the background, nor
//! for 5 s after (target_fps * 5 frames); a teleport pauses it; the target
//! is held under the frame rate cap; without "Continuous" it stops on its
//! own once the target is held.
//!
//! Differences, all stricter or safer than the Legacy Viewer (TASKS.md):
//! - The avatar lever is the number of avatars drawn in full
//!   (RenderAvatarMaxNonImpostors), not a render time per avatar
//!   (RenderAvatarMaxART): Aurora's renderer does not time each avatar. As
//!   the Legacy Viewer leaves its five nearest avatars untuned
//!   (MIN_NONTUNED_AVS), ours and the five nearest stay in full
//!   ([`AVATAR_FLOOR`]). Avatars the user set to always render fully, and
//!   friends when friends are always shown fully, stay in full too (the
//!   Legacy Viewer's lever spares them: LLVOAvatar::updateTooSlow), see
//!   `scene::Scene::avatar_keep_full`.
//! - It never sets anything costlier than the user's own value: the draw
//!   distance never goes above the user's (the Legacy Viewer climbs to
//!   AutoTuneRenderFarClipTarget, which may be higher), and the distance
//!   mode never draws more avatars in full than the user's limit (the Legacy
//!   Viewer sets the limit to the avatars in range, "unlimited" from 100).
//! - The user's own values are kept apart ([`Hold`]): the settings file
//!   always holds them (`Settings::to_saved`), so a crash or a quit while it
//!   runs leaves them as they were, and turning it off gives them back at
//!   once. The Legacy Viewer writes the tuned values in its settings and
//!   only warns that "all unsaved graphics settings will be lost"
//!   (EnableAutoFPSWarning). Without "Continuous", the values found stay for
//!   the session, with a button that gives the user's back and one that
//!   keeps them as the user's.
//! - A step up that brings the frames back under the target doubles the
//!   wait before the next step up (to two minutes at most), and one that
//!   holds halves it again, so that a scene at the edge is tried again at
//!   most every two minutes instead of going up and down in a loop.
//! - The distance mode has a margin ([`DISTANCE_MARGIN`]): someone standing
//!   at the impostor distance does not flip an avatar between full and
//!   impostor.
//!
//! The decision is a pure function of the frames and the values
//! ([`Tuner::tick`]), run once a frame on a ring of 50 frame times: a few
//! microseconds, no wait on the GPU.

use crate::settings::{Settings, draw_stepping};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Frames whose median is the frame time (NUM_PERIODS).
pub const FRAMES: usize = 50;
/// Fresh frames needed after a change or a pause before the next decision:
/// the median must be measured with the new values.
pub const MIN_FRAMES: usize = 30;
/// The band around the target frame time: slower than 110 % of it is too
/// slow, faster than 90 % leaves room (time_buf = target * 0.1).
pub const BAND: f32 = 0.1;
/// The least time between two changes (settingsChangeFrequency: 50 frames
/// or a second's worth, whichever is more).
pub const CHANGE_INTERVAL: Duration = Duration::from_secs(1);
/// A step up, and the end of a run without "Continuous", wait this many
/// change intervals (settingsChangeFrequency * 3).
pub const UP_INTERVALS: u32 = 3;
/// The most a step up may wait after the back-off.
pub const MAX_UP_WAIT: Duration = Duration::from_secs(120);
/// A step up is on trial for this long, and at least until the first
/// measurement made with it: frames too slow meanwhile double the wait of
/// the next one; a step up that holds halves it again.
pub const BOUNCE_WINDOW: Duration = Duration::from_secs(10);
/// The distance mode's margin (m): an avatar raises the count once it is this
/// much inside the impostor distance, and lowers it once it is past it, so
/// that someone standing at the edge does not flip it.
pub const DISTANCE_MARGIN: f32 = 4.0;
/// The wait after a pause (background, teleport, photo, draw distance
/// stepping) before tuning again (target_fps * 5 frames).
pub const RESUME_WAIT: Duration = Duration::from_secs(5);
/// Draw distance step (DD_STEP, m).
pub const DD_STEP: f32 = 10.0;
/// The lowest draw distance Aurora allows (`settings::DRAW_DISTANCE_MIN`).
pub const DD_FLOOR: f32 = crate::settings::DRAW_DISTANCE_MIN;
/// The avatars Autotune keeps in full: ours and the five nearest
/// (MIN_NONTUNED_AVS = 5, llvoavatar.cpp).
pub const AVATAR_FLOOR: u32 = 6;
/// Bounds of the settings (the Legacy Viewer's spinner and sliders).
pub const TARGET_FPS_MIN: u32 = 1;
pub const TARGET_FPS_MAX: u32 = 300;
pub const IMPOSTOR_DISTANCE_MIN: f32 = 16.0;
pub const IMPOSTOR_DISTANCE_MAX: f32 = 256.0;
pub const MIN_DD_MAX: f32 = 256.0;

/// What Autotune may change (TuningFPSStrategy; the Legacy Viewer's window
/// offers these two, its "scene only" is not shown).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Strategy {
    /// The avatars drawn in full only (TUNE_AVATARS_ONLY).
    AvatarsOnly,
    /// The avatars first, then the draw distance (TUNE_SCENE_AND_AVATARS).
    #[default]
    AvatarsAndScene,
}

/// The saved choices (the Performance window).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Config {
    /// TargetFPS: the frame rate to reach (15).
    pub target_fps: u32,
    /// AutoTuneLock: keeps tuning until it is stopped (off). Without it, it
    /// stops on its own once the target is held.
    pub continuous: bool,
    /// KeepAutoTuneLock: "Continuous" starts again at the next login (on).
    pub keep_continuous: bool,
    /// TuningFPSStrategy (avatars and scene).
    pub strategy: Strategy,
    /// AutoTuneImpostorByDistEnabled: while it runs, avatars farther than
    /// `impostor_distance` are drawn as impostors (off).
    pub by_distance: bool,
    /// AutoTuneImpostorFarAwayDistance (64 m, 16 to 256).
    pub impostor_distance: f32,
    /// AutoTuneRenderFarClipMin: the lowest draw distance it may set (32 m,
    /// 32 to 256).
    pub min_draw_distance: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            target_fps: 15,
            continuous: false,
            keep_continuous: true,
            strategy: Strategy::AvatarsAndScene,
            by_distance: false,
            impostor_distance: 64.0,
            min_draw_distance: 32.0,
        }
    }
}

impl Config {
    /// Every value within its bounds, whatever the file said.
    pub fn sanitize(&mut self) {
        self.target_fps = self.target_fps.clamp(TARGET_FPS_MIN, TARGET_FPS_MAX);
        self.impostor_distance = finite_clamp(self.impostor_distance, IMPOSTOR_DISTANCE_MIN, IMPOSTOR_DISTANCE_MAX, 64.0);
        self.min_draw_distance = finite_clamp(self.min_draw_distance, DD_FLOOR, MIN_DD_MAX, 32.0);
    }

    /// Whether it runs from the start of a session (initialiseFromSettings:
    /// AutoTuneLock && KeepAutoTuneLock; a lock not kept is dropped).
    pub fn runs_at_login(&self) -> bool {
        self.continuous && self.keep_continuous
    }
}

fn finite_clamp(v: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if v.is_finite() { v.clamp(min, max) } else { fallback }
}

/// The values Autotune changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tuned {
    pub draw_distance: f32,
    /// Avatars drawn in full, ours included (`Settings::max_avatars`).
    pub max_avatars: u32,
}

impl Tuned {
    pub fn read(s: &Settings) -> Self {
        Self {
            draw_distance: s.draw_distance,
            max_avatars: s.max_avatars,
        }
    }
}

/// The user's own values while Autotune holds the live ones (as
/// `draw_stepping::Stepping` and `snapshot::quality::Held` do).
///
/// A hold is only made outside the photo quality and draw distance steps
/// under way (the tuner is paused then), so that the values it takes as the
/// user's are theirs. Steps that reached the user's distance only watch the
/// load: the tuner runs, and its first write ends them. It may last into them: the photo quality then holds the user's
/// values itself (it read them through `Values::read`, which shows the
/// user's), and the steps climb to the tuned draw distance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hold {
    /// The user's values, given back when it stops.
    user: Tuned,
    /// What Autotune last wrote; a value found otherwise was changed by the
    /// user (Preferences, the quick settings, a graphics level, Debug
    /// Settings) and is theirs from then on.
    applied: Tuned,
}

impl Hold {
    pub fn new(s: &Settings) -> Self {
        let now = Tuned::read(s);
        Self { user: now, applied: now }
    }

    /// Whether another hold of the same values is under way: then the live
    /// values are not the user's nor Autotune's. (The steps of the draw
    /// distance only while under the user's: past that they watch, the
    /// live value is the user's.)
    fn others_hold(s: &Settings) -> bool {
        s.photo_held.is_some() || draw_stepping::holds_draw_distance(s)
    }

    /// The user's values, with what they changed since the last write.
    pub fn user(&self, s: &Settings) -> Tuned {
        let mut h = *self;
        h.sync(s);
        h.user
    }

    /// A value the user changed becomes theirs. Not while the photo quality
    /// holds the live values, nor the draw distance while the steps hold it:
    /// theirs are not the user's. (The steps leave the avatars in full alone:
    /// a change of them during the steps is the user's.)
    pub fn sync(&mut self, s: &Settings) {
        if s.photo_held.is_some() {
            return;
        }
        let now = Tuned::read(s);
        if !draw_stepping::holds_draw_distance(s) && now.draw_distance != self.applied.draw_distance {
            self.user.draw_distance = now.draw_distance;
            self.applied.draw_distance = now.draw_distance;
        }
        if now.max_avatars != self.applied.max_avatars {
            self.user.max_avatars = now.max_avatars;
            self.applied.max_avatars = now.max_avatars;
        }
    }

    /// Writes `to`, held within the user's values and Aurora's bounds: never
    /// above the user's, never under the floors. Does nothing while another
    /// hold has the live values.
    pub fn write(&mut self, s: &mut Settings, to: Tuned) {
        self.sync(s);
        if Self::others_hold(s) {
            return;
        }
        let to = self.bounded(to);
        s.draw_distance = to.draw_distance;
        s.max_avatars = to.max_avatars;
        self.applied = to;
    }

    fn bounded(&self, to: Tuned) -> Tuned {
        let user = self.user;
        Tuned {
            draw_distance: finite_clamp(
                to.draw_distance,
                DD_FLOOR.min(user.draw_distance),
                user.draw_distance,
                user.draw_distance,
            ),
            max_avatars: to.max_avatars.clamp(1, user.max_avatars.max(1)),
        }
    }

    /// Gives the user's values back. Under the photo quality nothing is
    /// written: it holds the user's values and gives them back when it ends.
    /// Under draw distance stepping the steps climb to the user's distance.
    pub fn restore(mut self, s: &mut Settings) {
        self.sync(s);
        if s.photo_held.is_some() {
            return;
        }
        s.max_avatars = self.user.max_avatars;
        match &mut s.draw_stepping {
            Some(steps) => steps.user = self.user.draw_distance,
            None => s.draw_distance = self.user.draw_distance,
        }
    }

    /// A graphics level or a saved profile the user chose was just written:
    /// its values are the user's, even one equal to a tuned value. Not
    /// under the photo quality, which holds the user's values itself, nor the
    /// draw distance under the steps (they end on the value set during them).
    pub fn chosen(s: &mut Settings) {
        if s.photo_held.is_some() {
            return;
        }
        let stepping = draw_stepping::holds_draw_distance(s);
        let now = Tuned::read(s);
        if let Some(h) = &mut s.autotune_hold {
            if !stepping {
                h.user.draw_distance = now.draw_distance;
                h.applied.draw_distance = now.draw_distance;
            }
            h.user.max_avatars = now.max_avatars;
            h.applied.max_avatars = now.max_avatars;
        }
    }

    /// For `graphics_presets::Values::read`: the user's value of a field
    /// still Autotune's, the live one of a field someone else changed (the
    /// photo quality, the steps, the user). The graphics level shown and a
    /// profile saved are the user's, not the tuned values.
    pub fn shown(&self, s: &Settings) -> Tuned {
        let now = Tuned::read(s);
        Tuned {
            draw_distance: if now.draw_distance == self.applied.draw_distance {
                self.user.draw_distance
            } else {
                now.draw_distance
            },
            max_avatars: if now.max_avatars == self.applied.max_avatars {
                self.user.max_avatars
            } else {
                now.max_avatars
            },
        }
    }
}

/// Where Autotune stands, for the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    /// Not running.
    #[default]
    Off,
    /// Waiting: background, teleport, photo, draw distance stepping, the
    /// seconds after one of them, or frames still being measured.
    Waiting,
    /// Lowering the values: the frames are slower than the target.
    Lowering,
    /// At the target, or giving values back while there is room.
    OnTarget,
    /// Everything is at its floor and the frames are still too slow.
    CannotReach,
    /// Without "Continuous": the target was held, it stopped; the values
    /// found stay for the session.
    Finished,
}

/// What a frame tells the tuner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub now: Instant,
    /// The frame's time without the frame limiter's wait (ms).
    pub work_ms: f32,
    /// The frame rate the loop is held to (frame limiter, vsync), 0 for none.
    pub cap_fps: f32,
    /// No tuning this frame: window in the background or minimized, a
    /// teleport, the photo quality, draw distance stepping, not in world.
    pub paused: bool,
    /// Avatars within the draw distance, ours included.
    pub avatars: u32,
    /// With the distance mode: the avatars around, counted with the current
    /// impostor distance (None until the scene has counted with it).
    pub within_distance: Option<Within>,
}

/// The distance mode's count, ours included, within the draw distance too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Within {
    /// Nearer than the impostor distance.
    pub outer: u32,
    /// Nearer than the impostor distance less [`DISTANCE_MARGIN`].
    pub inner: u32,
}

/// What the tuner decided.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decision {
    Keep,
    /// Write these values (through [`Hold::write`]).
    Set(Tuned),
    /// Without "Continuous": stop, the values stay.
    Finish,
}

/// The tuner's state between frames.
#[derive(Debug, Clone)]
pub struct Tuner {
    frames: VecDeque<f32>,
    /// No decision before this (after a pause or the start).
    resume_at: Option<Instant>,
    /// The frame rate lever's own limit of avatars in full (None: the
    /// user's); the distance mode bounds it further.
    level: Option<u32>,
    /// What the tuner wrote last: a live value found otherwise is the
    /// user's, and the lever starts again from it.
    wrote: Option<Tuned>,
    /// The last change of the frame rate lever, and whether it was a step up.
    last_change: Option<Instant>,
    /// The distance mode's bound of the avatars in full, and its last change.
    distance_cap: Option<u32>,
    last_distance_change: Option<Instant>,
    /// The last step up while it is on trial (see [`BOUNCE_WINDOW`]).
    up_on_trial: Option<Instant>,
    /// Since when the frames have had room.
    room_since: Option<Instant>,
    /// Since when the frames have been at the target or had room, with no
    /// change (for the end of a run without "Continuous").
    held_since: Option<Instant>,
    /// The wait before a step up, doubled after a bounce.
    up_wait: Duration,
    pub status: Status,
}

impl Default for Tuner {
    fn default() -> Self {
        Self {
            frames: VecDeque::with_capacity(FRAMES),
            resume_at: None,
            level: None,
            wrote: None,
            last_change: None,
            distance_cap: None,
            last_distance_change: None,
            up_on_trial: None,
            room_since: None,
            held_since: None,
            up_wait: UP_WAIT,
            status: Status::Off,
        }
    }
}

/// The wait before a step up, before any back-off.
const UP_WAIT: Duration = Duration::from_secs(UP_INTERVALS as u64 * CHANGE_INTERVAL.as_secs());

/// The median of the frame times (updateMeanFrameTime).
fn median(frames: &VecDeque<f32>) -> f32 {
    let mut buf = [0.0f32; FRAMES];
    let n = frames.len().min(FRAMES);
    for (to, from) in buf.iter_mut().zip(frames.iter()) {
        *to = *from;
    }
    let v = &mut buf[..n];
    v.sort_unstable_by(f32::total_cmp);
    if n == 0 {
        0.0
    } else if n.is_multiple_of(2) {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    } else {
        v[n / 2]
    }
}

/// The target frame rate, under the cap (with vsync the Legacy Viewer takes
/// min(refresh rate, target)).
pub fn effective_target(config: &Config, cap_fps: f32) -> f32 {
    let target = config.target_fps.clamp(TARGET_FPS_MIN, TARGET_FPS_MAX) as f32;
    if cap_fps >= 1.0 { target.min(cap_fps) } else { target }
}

impl Tuner {
    /// A new run, from the user's values, after the resume wait.
    pub fn start(now: Instant) -> Self {
        Self {
            resume_at: Some(now + RESUME_WAIT),
            status: Status::Waiting,
            ..Self::default()
        }
    }

    /// The median frame time measured (ms), once there are enough frames.
    pub fn frame_ms(&self) -> Option<f32> {
        (self.frames.len() >= MIN_FRAMES).then(|| median(&self.frames))
    }

    fn pause(&mut self, now: Instant) {
        self.frames.clear();
        self.resume_at = Some(now + RESUME_WAIT);
        self.room_since = None;
        self.held_since = None;
        if self.status != Status::Finished {
            self.status = Status::Waiting;
        }
    }

    /// One frame: decides from the frames, the values live (`applied`, what
    /// the settings hold now) and the user's own (`user`).
    pub fn tick(&mut self, config: &Config, frame: Frame, applied: Tuned, user: Tuned) -> Decision {
        if frame.paused || !frame.work_ms.is_finite() || frame.work_ms < 0.0 {
            self.pause(frame.now);
            return Decision::Keep;
        }
        if self.status == Status::Finished {
            return Decision::Keep;
        }
        if self.resume_at.is_some_and(|t| frame.now < t) {
            self.status = Status::Waiting;
            return Decision::Keep;
        }
        self.resume_at = None;
        if self.frames.len() == FRAMES {
            self.frames.pop_front();
        }
        self.frames.push_back(frame.work_ms);

        // the user changed the avatars in full since the last write: the
        // lever starts again from their value
        if self.wrote.is_some_and(|w| w.max_avatars != applied.max_avatars) {
            self.level = None;
        }
        let user_max = user.max_avatars.max(1);
        let floor = AVATAR_FLOOR.min(user_max);
        let mut level = self.level.unwrap_or(user_max).clamp(floor.min(user_max), user_max);

        // the distance mode bounds the avatars in full whatever the frame
        // rate (updateAvatarParams, userImpostorDistanceTuningEnabled):
        // never above the user's limit nor under the floor. It goes down as
        // soon as someone is past the distance, up once someone is
        // DISTANCE_MARGIN inside it and a change interval after the last
        // change, so that someone at the edge does not flip it; until the
        // scene has counted with the current distance it stays as it is.
        let cap = if config.by_distance {
            let was = self.distance_cap.map(|c| c.clamp(floor, user_max));
            let cap = match (frame.within_distance, was) {
                (Some(w), was) => {
                    let outer = w.outer.clamp(floor, user_max);
                    let inner = w.inner.clamp(floor, user_max).min(outer);
                    match was {
                        None => outer,
                        Some(c) if outer < c => outer,
                        Some(c) if inner > c && self.last_distance_change.is_none_or(|t| frame.now >= t + CHANGE_INTERVAL) => inner,
                        Some(c) => c,
                    }
                }
                (None, was) => was.unwrap_or(user_max),
            };
            self.distance_cap = Some(cap);
            cap
        } else {
            self.distance_cap = None;
            user_max
        };
        let want = Tuned {
            max_avatars: level.min(cap),
            ..applied
        };
        if want.max_avatars != applied.max_avatars {
            // a change of the avatars drawn: the frame rate lever measures
            // again with it
            self.last_distance_change = Some(frame.now);
            self.level = Some(level);
            self.frames.clear();
            self.room_since = None;
            self.held_since = None;
            return self.set(want);
        }

        let Some(frame_ms) = self.frame_ms() else {
            self.status = Status::Waiting;
            return Decision::Keep;
        };
        if self.last_change.is_some_and(|t| frame.now < t + CHANGE_INTERVAL) {
            return Decision::Keep;
        }
        let target_ms = 1000.0 / effective_target(config, frame.cap_fps);
        let dd_floor = config.min_draw_distance.max(DD_FLOOR).min(user.draw_distance);
        // avatars in full now (a step only changes something when more are around)
        let shown = applied.max_avatars.min(frame.avatars.max(1));

        if frame_ms > target_ms * (1.0 + BAND) {
            // too slow: one step down, the avatars first
            self.room_since = None;
            self.held_since = None;
            if self.up_on_trial.take().is_some() {
                // the last step up brought it back under the target
                self.up_wait = (self.up_wait * 2).min(MAX_UP_WAIT);
            }
            if shown > floor {
                level = shown - 1;
                self.level = Some(level);
                self.status = Status::Lowering;
                return self.change(
                    frame.now,
                    false,
                    Tuned {
                        max_avatars: level.min(cap),
                        ..applied
                    },
                );
            }
            if config.strategy == Strategy::AvatarsAndScene && applied.draw_distance > dd_floor {
                self.status = Status::Lowering;
                let draw_distance = (applied.draw_distance - DD_STEP).max(dd_floor);
                return self.change(frame.now, false, Tuned { draw_distance, ..applied });
            }
            self.status = Status::CannotReach;
            return Decision::Keep;
        }

        self.status = Status::OnTarget;
        if self.up_on_trial.is_some_and(|t| frame.now >= t + BOUNCE_WINDOW) {
            // the last step up held: the wait comes back down
            self.up_on_trial = None;
            self.up_wait = (self.up_wait / 2).max(UP_WAIT);
        }
        let held = *self.held_since.get_or_insert(frame.now);
        if frame_ms < target_ms * (1.0 - BAND) {
            // room: give back after a longer wait, the avatars first
            let since = *self.room_since.get_or_insert(frame.now);
            if frame.now >= since + self.up_wait {
                if level < user_max && level < cap && frame.avatars > applied.max_avatars {
                    self.level = Some(level + 1);
                    return self.change(
                        frame.now,
                        true,
                        Tuned {
                            max_avatars: (level + 1).min(cap),
                            ..applied
                        },
                    );
                }
                if config.strategy == Strategy::AvatarsAndScene && applied.draw_distance < user.draw_distance {
                    let draw_distance = (applied.draw_distance + DD_STEP).min(user.draw_distance);
                    return self.change(frame.now, true, Tuned { draw_distance, ..applied });
                }
            } else {
                // still giving back: not held yet
                return Decision::Keep;
            }
        } else {
            self.room_since = None;
        }
        // at the target long enough without a change: without
        // "Continuous", done (updateAvatarParams: !userAutoTuneLock)
        if !config.continuous && frame.now >= held + CHANGE_INTERVAL * UP_INTERVALS {
            self.status = Status::Finished;
            return Decision::Finish;
        }
        Decision::Keep
    }

    fn set(&mut self, to: Tuned) -> Decision {
        self.wrote = Some(to);
        Decision::Set(to)
    }

    /// A step of the frame rate lever: the next measurement starts afresh.
    fn change(&mut self, now: Instant, up: bool, to: Tuned) -> Decision {
        self.last_change = Some(now);
        self.up_on_trial = up.then_some(now);
        self.frames.clear();
        self.room_since = None;
        self.held_since = None;
        self.set(to)
    }

    /// The values written were bounded by the hold (`Hold::write`): the
    /// tuner keeps what is live as its own write.
    pub fn written(&mut self, live: Tuned) {
        self.wrote = Some(live);
    }
}

#[cfg(test)]
mod tests;
