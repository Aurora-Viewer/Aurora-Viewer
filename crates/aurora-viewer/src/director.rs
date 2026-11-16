//! The Director: a camera track laid out in the world, then played with the
//! interface out of sight, for those who film in Second Life.
//!
//! Aurora's own: the Legacy Viewer has no camera path tool. The track and
//! its curve are in `camera/track.rs`; this is the timeline around it: the
//! playhead, the keyframe being edited, what "Add keyframe" does, and the
//! playback with its start delay and its hold on the last view. The window,
//! the timeline strip and the path drawn in the world are in
//! `ui/director.rs`; the camera and the keys in `app/director.rs`.
//!
//! The lock keeps a target in the centre of the view: a joint of an avatar
//! (ours or another: the neck of a dancer), or an object. Live, it turns
//! the user's own camera wherever they move it, with no keyframe at all; a
//! keyframe taken meanwhile is locked too, and the playback locks the view
//! where its keyframes say so (`Track::lock_weight`).

use crate::camera::track::{self, Pose, Track};
use glam::Vec3;
use std::ops::RangeInclusive;
use uuid::Uuid;

/// Seconds the first view is held before the camera leaves, by default: the
/// time for the recording to start on a clean picture.
pub const DEFAULT_START_DELAY: f32 = 2.0;
pub const START_DELAY_RANGE: RangeInclusive<f32> = 0.0..=10.0;
/// Seconds the last view is held before the interface comes back.
pub const END_HOLD: f32 = 1.0;
/// The lock's smoothing: seconds the view takes to catch up half of a move
/// of its target (0 follows it rigidly, every step of a dance included).
pub const DEFAULT_SMOOTHING: f32 = 0.15;
pub const SMOOTHING_RANGE: RangeInclusive<f32> = 0.0..=1.0;
/// A target that moves further than this in a frame went somewhere else (a
/// teleport, another target): the view goes to it at once.
const FOLLOW_SNAP: f32 = 8.0;

/// The Director's lens when its depth of field is turned on, and the
/// f-numbers it can take (a wide aperture blurs the background most).
pub const DEFAULT_APERTURE: f32 = 2.8;
pub const APERTURE_RANGE: RangeInclusive<f32> = 1.0..=22.0;
/// The distance it is focused at without a target, and its range (m).
pub const DEFAULT_FOCUS: f32 = 5.0;
pub const FOCUS_RANGE: RangeInclusive<f32> = 0.3..=100.0;

/// The green of a green screen (the key color proposed first).
pub const KEY_GREEN: [u8; 3] = [0, 177, 64];

/// An sRGB color as the renderer mixes it: linear RGB, 0 to 1.
pub fn linear(color: [u8; 3]) -> [f32; 3] {
    color.map(|c| {
        let c = f32::from(c) / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    })
}

/// The joint of an avatar the view is locked on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Joint {
    Head,
    #[default]
    Neck,
    Chest,
    Pelvis,
    LeftHand,
    RightHand,
}

impl Joint {
    pub const ALL: [Joint; 6] = [
        Joint::Head,
        Joint::Neck,
        Joint::Chest,
        Joint::Pelvis,
        Joint::LeftHand,
        Joint::RightHand,
    ];

    /// Its name in the avatar's skeleton.
    pub fn bone(self) -> &'static str {
        match self {
            Joint::Head => "mHead",
            Joint::Neck => "mNeck",
            Joint::Chest => "mChest",
            Joint::Pelvis => "mPelvis",
            Joint::LeftHand => "mWristLeft",
            Joint::RightHand => "mWristRight",
        }
    }
}

/// What the view is locked on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Target {
    /// A joint of our own avatar.
    #[default]
    Own,
    /// A joint of another avatar.
    Avatar(Uuid),
    /// An object, at its centre.
    Object(Uuid),
}

/// The lock of the view on a target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lock {
    /// The user's own camera is locked on the target, and so is a keyframe
    /// taken meanwhile.
    pub on: bool,
    pub target: Target,
    pub joint: Joint,
    /// See `DEFAULT_SMOOTHING`.
    pub smoothing: f32,
}

impl Default for Lock {
    fn default() -> Self {
        Lock {
            on: false,
            target: Target::Own,
            joint: Joint::Neck,
            smoothing: DEFAULT_SMOOTHING,
        }
    }
}

/// Where a view that follows its target smoothly looks now: from where it
/// looked (`from`) towards where the target is (`to`), half of the way each
/// `half_life` seconds.
pub fn follow(from: Option<Vec3>, to: Vec3, half_life: f32, dt: f32) -> Vec3 {
    match from {
        Some(from) if half_life > 0.0 && dt.is_finite() && from.distance(to) < FOLLOW_SNAP => {
            from + (to - from) * (1.0 - (-dt.max(0.0) * std::f32::consts::LN_2 / half_life).exp())
        }
        _ => to,
    }
}

/// The proportions of the picture: the window's own, or a frame laid in it
/// (bars hide the rest while the track plays, guides show it meanwhile).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    #[default]
    Window,
    Wide,
    Flat,
    Univisium,
    Scope,
    Classic,
    Square,
    Vertical,
}

impl Format {
    pub const ALL: [Format; 8] = [
        Format::Window,
        Format::Wide,
        Format::Flat,
        Format::Univisium,
        Format::Scope,
        Format::Classic,
        Format::Square,
        Format::Vertical,
    ];

    /// Width over height; `None` for the window's own.
    pub fn aspect(self) -> Option<f32> {
        Some(match self {
            Format::Window => return None,
            Format::Wide => 16.0 / 9.0,
            Format::Flat => 1.85,
            Format::Univisium => 2.0,
            Format::Scope => 2.39,
            Format::Classic => 4.0 / 3.0,
            Format::Square => 1.0,
            Format::Vertical => 9.0 / 16.0,
        })
    }

    /// How the trade writes it; `None` for the window's own, which has a
    /// name in the interface's language.
    pub fn ratio(self) -> Option<&'static str> {
        Some(match self {
            Format::Window => return None,
            Format::Wide => "16:9",
            Format::Flat => "1.85:1",
            Format::Univisium => "2:1",
            Format::Scope => "2.39:1",
            Format::Classic => "4:3",
            Format::Square => "1:1",
            Format::Vertical => "9:16",
        })
    }

    /// The size of the largest picture of these proportions in a window of
    /// `width` by `height`.
    pub fn fit(self, width: f32, height: f32) -> (f32, f32) {
        match self.aspect() {
            Some(aspect) if width > 0.0 && height > 0.0 => {
                if width / height > aspect {
                    (height * aspect, height)
                } else {
                    (width, width / aspect)
                }
            }
            _ => (width, height),
        }
    }
}

/// What the timeline is doing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// The user's own camera; the track is drawn in the world.
    Editing,
    /// The playhead is held: the view follows it along the track.
    Scrubbing,
    /// The track plays, the interface hidden. `wait`: seconds left on the
    /// first view; `hold`: seconds spent on the last one, once there.
    Playing { wait: f32, hold: Option<f32> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Director {
    pub track: Track,
    /// The playhead, seconds from the start of the track.
    pub time: f32,
    pub mode: Mode,
    /// The keyframe the window edits.
    pub selected: Option<usize>,
    pub looping: bool,
    /// The HUDs go with the interface while the track plays.
    pub hide_huds: bool,
    /// The path and its cameras are drawn in the world.
    pub show_path: bool,
    /// Seconds on the first view before the camera leaves.
    pub start_delay: f32,
    /// The mouse pointer goes with the interface while the track plays.
    pub hide_pointer: bool,
    pub lock: Lock,
    pub format: Format,
    /// The rule of thirds drawn over the picture while the track is edited.
    pub thirds: bool,
    /// The color (sRGB) the isolation's empty background shows, to film
    /// against and key out; `None`: left empty.
    pub key_color: Option<[u8; 3]>,
    /// A hand-held camera while the track plays: 0 (a steady one) to 1.
    pub shake: f32,
    /// The depth of field of the Director's lens: its f-number; `None`:
    /// the viewer's own depth of field, as outside the Director.
    pub dof: Option<f32>,
    /// The distance (m) that lens is focused at when the view is not locked
    /// on a target (locked, it focuses on the target).
    pub focus: f32,
}

impl Default for Director {
    fn default() -> Self {
        Director {
            track: Track::default(),
            time: 0.0,
            mode: Mode::Editing,
            selected: None,
            looping: false,
            hide_huds: true,
            show_path: true,
            start_delay: DEFAULT_START_DELAY,
            hide_pointer: true,
            lock: Lock::default(),
            format: Format::Window,
            thirds: false,
            key_color: None,
            shake: 0.0,
            dof: None,
            focus: DEFAULT_FOCUS,
        }
    }
}

impl Director {
    pub fn playing(&self) -> bool {
        matches!(self.mode, Mode::Playing { .. })
    }

    /// Seconds left before the camera leaves the first view, while it waits.
    pub fn countdown(&self) -> Option<f32> {
        match self.mode {
            Mode::Playing { wait, .. } if wait > 0.0 => Some(wait),
            _ => None,
        }
    }

    /// A track needs two keyframes to go anywhere.
    pub fn can_play(&self) -> bool {
        self.track.len() >= 2
    }

    /// "Add keyframe" with the view `pose`: between two keyframes when the
    /// playhead is there, past the end when it was put there, else after
    /// the last one (`track::DEFAULT_GAP` later). A keyframe already under
    /// the playhead is never replaced: that is "Update". The new keyframe
    /// is selected and the playhead goes to it; taken while the view is
    /// locked on the target, it is a locked keyframe. `None` when the track
    /// is full.
    pub fn add_key(&mut self, pose: Pose) -> Option<usize> {
        let on_key = self.track.nearest(self.time, track::MIN_GAP).is_some();
        let index = if !self.track.is_empty() && !on_key && self.time > 0.0 && self.time <= track::MAX_TIME {
            self.track.insert(self.time, pose)
        } else {
            self.track.append(pose)
        }?;
        self.track.set_locked(index, self.lock.on);
        self.selected = Some(index);
        self.time = self.track.keys()[index].time;
        Some(index)
    }

    /// Remove a keyframe: the selection follows the keyframes that stay, the
    /// playhead stays within the track.
    pub fn remove(&mut self, index: usize) {
        if index >= self.track.len() {
            return;
        }
        // the first keyframe gone, the times start again from the next one
        let before = self.track.keys().get(1).map_or(0.0, |k| k.time);
        self.track.remove(index);
        if index == 0 {
            self.time = (self.time - before).max(0.0);
        }
        self.selected = match self.selected {
            Some(s) if s == index => None,
            Some(s) if s > index => Some(s - 1),
            other => other,
        };
        self.time = self.time.min(self.track.duration());
    }

    pub fn clear(&mut self) {
        self.track.clear();
        self.selected = None;
        self.time = 0.0;
        self.mode = Mode::Editing;
    }

    /// Select a keyframe and put the playhead on it.
    pub fn select(&mut self, index: usize) {
        if let Some(key) = self.track.keys().get(index) {
            self.selected = Some(index);
            self.time = key.time;
        }
    }

    /// Start the playback: from the playhead, or from the start when it is
    /// at the end (or past it). False when there is nothing to play.
    pub fn play(&mut self) -> bool {
        if !self.can_play() {
            return false;
        }
        if self.time >= self.track.duration() - 1e-3 {
            self.time = 0.0;
        }
        let wait = self.start_delay.clamp(*START_DELAY_RANGE.start(), *START_DELAY_RANGE.end());
        self.mode = Mode::Playing { wait, hold: None };
        true
    }

    /// Stop the playback or the scrub: the user's camera again.
    pub fn stop(&mut self) {
        self.mode = Mode::Editing;
    }

    /// The playhead held at `time`: the view follows it.
    pub fn scrub(&mut self, time: f32) {
        if self.playing() {
            return;
        }
        self.time = if time.is_finite() { time.clamp(0.0, track::MAX_TIME) } else { 0.0 };
        self.mode = Mode::Scrubbing;
    }

    pub fn end_scrub(&mut self) {
        if self.mode == Mode::Scrubbing {
            self.mode = Mode::Editing;
        }
    }

    /// `dt` seconds of a frame: the playback waits on the first view, runs,
    /// holds the last view, then ends (a loop never does).
    pub fn tick(&mut self, dt: f32) {
        let Mode::Playing { wait, hold } = self.mode else {
            return;
        };
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        if !self.can_play() {
            self.mode = Mode::Editing;
        } else if wait > 0.0 {
            self.mode = Mode::Playing { wait: wait - dt, hold };
        } else if let Some(held) = hold {
            self.mode = if held + dt >= END_HOLD {
                Mode::Editing
            } else {
                Mode::Playing {
                    wait,
                    hold: Some(held + dt),
                }
            };
        } else {
            let (time, done) = track::advance(self.time, dt, self.track.duration(), self.looping);
            self.time = time;
            if done {
                self.mode = Mode::Playing { wait, hold: Some(0.0) };
            }
        }
    }

    /// The view the track gives the camera now: while it plays or the
    /// playhead is held. `None`: the user's own camera. With the lock's
    /// `target` (where it is now), the view turns to it as much as the
    /// keyframes around the playhead are locked.
    pub fn view(&self, target: Option<Vec3>) -> Option<Pose> {
        let pose = match self.mode {
            Mode::Editing => return None,
            Mode::Scrubbing | Mode::Playing { .. } => self.track.sample(self.time)?,
        };
        let pose = match target {
            Some(target) => pose.aimed(target, self.track.lock_weight(self.time)),
            None => pose,
        };
        Some(track::hand_held(pose, self.time, self.shake))
    }

    /// Replace the track by one loaded from a file: nothing selected, the
    /// playhead at its start, whatever played or was held stopped.
    pub fn load(&mut self, track: Track) {
        self.track = track;
        self.selected = None;
        self.time = 0.0;
        self.mode = Mode::Editing;
    }

    /// The proportions the picture has in a window of `aspect` (width over
    /// height): the chosen format's, or the window's own.
    pub fn picture_aspect(&self, aspect: f32) -> f32 {
        self.format.aspect().unwrap_or(aspect)
    }
}

/// How many changes of the track can be undone.
pub const HISTORY: usize = 100;

/// The track as it was, with the keyframe then selected and the playhead.
#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    track: Track,
    selected: Option<usize>,
    time: f32,
}

impl Snapshot {
    fn of(d: &Director) -> Snapshot {
        Snapshot {
            track: d.track.clone(),
            selected: d.selected,
            time: d.time,
        }
    }

    fn restore(&self, d: &mut Director) {
        d.track = self.track.clone();
        d.selected = self.selected.filter(|index| *index < d.track.len());
        d.time = self.time;
    }
}

/// The changes of the track, to undo and redo them (Ctrl+Z, Ctrl+Y). A
/// change is noticed, not announced: whatever edits the track (the timeline,
/// the window, a key) is seen by `observe`, once the mouse is released so
/// that a whole drag is one step. The keyframes, their times and views, the
/// curve and its options are the track; the playback's options, the lock's
/// target and the picture's format are not undone.
#[derive(Debug, Clone, Default)]
pub struct History {
    /// The track as it was last seen at rest.
    settled: Option<Snapshot>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl History {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Each frame, after the interface; `at_rest`: no mouse button is held.
    pub fn observe(&mut self, d: &Director, at_rest: bool) {
        let Some(settled) = &mut self.settled else {
            self.settled = Some(Snapshot::of(d));
            return;
        };
        if settled.track == d.track {
            // the selection and the playhead follow without making a step
            settled.selected = d.selected;
            settled.time = d.time;
            return;
        }
        if !at_rest {
            return;
        }
        self.undo.push(std::mem::replace(settled, Snapshot::of(d)));
        if self.undo.len() > HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// The track as it was before its last change. False with nothing to undo.
    pub fn undo(&mut self, d: &mut Director) -> bool {
        // a change not seen yet (the key pressed in the frame of the change)
        // is a step of its own first
        self.observe(d, true);
        let Some(back) = self.undo.pop() else {
            return false;
        };
        self.redo.push(Snapshot::of(d));
        back.restore(d);
        self.settled = Some(back);
        true
    }

    /// The change last undone, done again. False with nothing to redo.
    pub fn redo(&mut self, d: &mut Director) -> bool {
        self.observe(d, true);
        let Some(again) = self.redo.pop() else {
            return false;
        };
        self.undo.push(Snapshot::of(d));
        again.restore(d);
        self.settled = Some(again);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn at(x: f32) -> Pose {
        Pose::from_view(Vec3::new(x, 0.0, 20.0), Vec3::X, Vec3::Z, 1.0)
    }

    fn three() -> Director {
        let mut d = Director::default();
        for x in [0.0, 10.0, 20.0] {
            d.add_key(at(x));
        }
        d
    }

    fn times(d: &Director) -> Vec<f32> {
        d.track.keys().iter().map(|k| k.time).collect()
    }

    /// The track gives the view of the keyframe at `x`.
    fn views(d: &Director, x: f32) -> bool {
        d.view(None).is_some_and(|pose| pose.position.distance(at(x).position) < 1e-3)
    }

    #[test]
    fn keyframes_added_in_a_row_follow_each_other() {
        let d = three();
        assert_eq!(times(&d), [0.0, 3.0, 6.0]);
        // the last one added is selected, the playhead on it
        assert_eq!(d.selected, Some(2));
        assert_eq!(d.time, 6.0);
    }

    #[test]
    fn a_keyframe_goes_where_the_playhead_is() {
        let mut d = three();
        // between two keyframes
        d.time = 4.0;
        assert_eq!(d.add_key(at(15.0)), Some(2));
        assert_eq!(times(&d), [0.0, 3.0, 4.0, 6.0]);
        // past the end: the gap the user chose
        d.time = 16.0;
        assert_eq!(d.add_key(at(30.0)), Some(4));
        assert_eq!(d.track.duration(), 16.0);
        // on a keyframe: never replaced, the new one goes after the last
        d.select(1);
        assert_eq!(d.add_key(at(40.0)), Some(5));
        assert_eq!(d.track.duration(), 19.0);
        assert_eq!(d.track.keys()[1].pose, at(10.0));
    }

    #[test]
    fn removing_a_keyframe_keeps_the_selection_on_the_same_one() {
        let mut d = three();
        d.select(2);
        d.remove(0);
        assert_eq!(d.selected, Some(1));
        assert_eq!(times(&d), [0.0, 3.0]);
        // the playhead followed its keyframe
        assert_eq!(d.time, 3.0);
        d.remove(1);
        assert_eq!(d.selected, None);
        assert_eq!(d.time, 0.0);
        d.remove(5);
        assert_eq!(d.track.len(), 1);
    }

    #[test]
    fn a_track_of_one_keyframe_does_not_play() {
        let mut d = Director::default();
        d.add_key(at(0.0));
        assert!(!d.play());
        assert_eq!(d.mode, Mode::Editing);
        assert_eq!(d.view(None), None);
    }

    #[test]
    fn the_playback_waits_runs_holds_then_ends() {
        let mut d = three();
        d.start_delay = 1.0;
        // the playhead at the end: it plays from the start
        assert!(d.play());
        assert_eq!(d.time, 0.0);
        assert_eq!(d.countdown(), Some(1.0));
        d.tick(0.6);
        assert_eq!(d.time, 0.0, "the first view is held");
        assert!(views(&d, 0.0));
        d.tick(0.6);
        assert_eq!(d.countdown(), None);
        d.tick(2.0);
        assert_eq!(d.time, 2.0);
        d.tick(10.0);
        assert_eq!(d.time, 6.0);
        // the last view stays a moment before the interface comes back
        assert!(d.playing());
        d.tick(END_HOLD * 0.6);
        assert!(d.playing());
        d.tick(END_HOLD * 0.6);
        assert_eq!(d.mode, Mode::Editing);
        assert_eq!(d.view(None), None);
    }

    #[test]
    fn a_loop_plays_until_it_is_stopped() {
        let mut d = three();
        d.start_delay = 0.0;
        d.looping = true;
        d.time = 5.0;
        assert!(d.play());
        d.tick(2.0);
        assert!((d.time - 1.0).abs() < 1e-4);
        assert!(d.playing());
        d.stop();
        assert_eq!(d.mode, Mode::Editing);
    }

    #[test]
    fn a_held_playhead_gives_the_view_then_gives_it_back() {
        let mut d = three();
        d.scrub(3.0);
        assert!(views(&d, 10.0));
        d.end_scrub();
        assert_eq!(d.view(None), None);
        assert_eq!(d.time, 3.0);
        // not while the track plays
        d.start_delay = 0.0;
        d.play();
        d.scrub(5.0);
        assert!(d.playing());
        assert_eq!(d.time, 3.0);
        d.end_scrub();
        assert!(d.playing());
    }

    #[test]
    fn clearing_the_track_ends_everything() {
        let mut d = three();
        d.play();
        d.clear();
        assert!(d.track.is_empty());
        assert_eq!((d.selected, d.time, d.mode), (None, 0.0, Mode::Editing));
    }

    #[test]
    fn a_keyframe_taken_while_the_view_is_locked_is_a_locked_keyframe() {
        let mut d = Director::default();
        d.add_key(at(0.0));
        d.lock.on = true;
        d.add_key(at(10.0));
        d.lock.on = false;
        d.add_key(at(20.0));
        let locked: Vec<bool> = d.track.keys().iter().map(|k| k.locked).collect();
        assert_eq!(locked, [false, true, false]);
        // played, the view turns to the target where the keyframes say so
        let target = Vec3::new(10.0, 50.0, 20.0);
        let towards = |d: &Director| {
            let pose = d.view(Some(target)).expect("view");
            pose.forward().dot((target - pose.position).normalize())
        };
        d.scrub(0.0);
        assert!(towards(&d) < 0.3, "free at the first keyframe");
        d.scrub(3.0);
        assert!(towards(&d) > 0.999, "locked at the second");
        d.scrub(1.5);
        assert!((0.3..0.999).contains(&towards(&d)), "turning to it in between");
        // without a target to find (the avatar left), the track's own view
        assert!(d.view(None).expect("view").forward().dot(Vec3::X) > 0.999);
    }

    #[test]
    fn a_smooth_lock_catches_up_half_of_the_way_each_half_life() {
        let (from, to) = (Vec3::ZERO, Vec3::X);
        assert_eq!(follow(None, to, 0.2, 0.016), to);
        assert_eq!(follow(Some(from), to, 0.0, 0.016), to, "rigid");
        assert!((follow(Some(from), to, 0.2, 0.2).x - 0.5).abs() < 1e-4);
        // the same whatever the frame rate
        let mut at = from;
        for _ in 0..20 {
            at = follow(Some(at), to, 0.2, 0.01);
        }
        assert!((at.x - 0.5).abs() < 1e-3);
        // a target that jumps away is another target: no sweep across the scene
        let far = Vec3::X * 50.0;
        assert_eq!(follow(Some(from), far, 0.2, 0.016), far);
        assert_eq!(follow(Some(from), to, 0.2, f32::NAN), to);
    }

    #[test]
    fn a_change_of_the_track_is_undone_and_done_again() {
        let mut d = Director::default();
        let mut history = History::default();
        history.observe(&d, true);
        assert!(!history.can_undo() && !history.undo(&mut d));
        for x in [0.0, 10.0, 20.0] {
            d.add_key(at(x));
            history.observe(&d, true);
        }
        // selecting and moving the playhead are no steps
        d.select(0);
        history.observe(&d, true);
        d.remove(1);
        history.observe(&d, true);
        assert_eq!(times(&d), [0.0, 6.0]);
        assert!(history.undo(&mut d));
        assert_eq!(times(&d), [0.0, 3.0, 6.0]);
        // the keyframe that was selected before the change is selected again
        assert_eq!((d.selected, d.time), (Some(0), 0.0));
        assert!(history.undo(&mut d));
        assert_eq!(times(&d), [0.0, 3.0]);
        assert!(history.redo(&mut d) && history.redo(&mut d));
        assert_eq!(times(&d), [0.0, 6.0]);
        assert!(!history.redo(&mut d));
        // a new change after an undo forgets what was undone
        history.undo(&mut d);
        d.track.set_span(1, 1.0);
        history.observe(&d, true);
        assert!(!history.can_redo());
        assert_eq!(times(&d), [0.0, 1.0, 4.0]);
        // clearing the track is a step like another
        d.clear();
        history.observe(&d, true);
        assert!(history.undo(&mut d));
        assert_eq!(d.track.len(), 3);
    }

    #[test]
    fn a_whole_drag_is_one_step() {
        let mut d = three();
        let mut history = History::default();
        history.observe(&d, true);
        // the mouse held: the keyframe moves frame after frame
        for time in [3.5, 4.0, 4.5] {
            d.track.set_time(1, time);
            history.observe(&d, false);
        }
        assert!(!history.can_undo());
        history.observe(&d, true);
        assert!(history.can_undo());
        assert!(history.undo(&mut d));
        assert_eq!(times(&d), [0.0, 3.0, 6.0]);
        assert!(!history.can_undo());
        // undone in the very frame of a change: that change first
        d.track.set_time(1, 5.0);
        assert!(history.undo(&mut d));
        assert_eq!(times(&d), [0.0, 3.0, 6.0]);
        assert!(history.redo(&mut d));
        assert_eq!(times(&d), [0.0, 5.0, 6.0]);
    }

    #[test]
    fn the_history_keeps_a_hundred_steps() {
        let mut d = three();
        let mut history = History::default();
        history.observe(&d, true);
        for step in 0..HISTORY + 20 {
            d.track.set_time(2, 7.0 + step as f32);
            history.observe(&d, true);
        }
        let mut undone = 0;
        while history.undo(&mut d) {
            undone += 1;
        }
        assert_eq!(undone, HISTORY);
    }

    #[test]
    fn a_key_color_is_given_to_the_renderer_in_linear_light() {
        assert_eq!(linear([0, 0, 0]), [0.0; 3]);
        let white = linear([255, 255, 255]);
        assert!(white.iter().all(|c| (c - 1.0).abs() < 1e-5));
        // middle grey of the screen is about a fifth of the light
        assert!((linear([128, 128, 128])[0] - 0.2158).abs() < 1e-3);
        let green = linear(KEY_GREEN);
        assert!(green[1] > green[2] && green[2] > green[0]);
        assert_eq!(Director::default().key_color, None);
    }

    #[test]
    fn a_loaded_track_replaces_the_one_being_edited() {
        let mut d = three();
        d.play();
        let mut other = Director::default();
        other.add_key(at(50.0));
        other.add_key(at(60.0));
        d.load(other.track.clone());
        assert_eq!(d.track, other.track);
        assert_eq!((d.selected, d.time, d.mode), (None, 0.0, Mode::Editing));
        // a hand-held camera moves the played view, not the keyframes
        d.shake = 1.0;
        d.scrub(1.0);
        let held = d.view(None).expect("view");
        d.shake = 0.0;
        let steady = d.view(None).expect("view");
        assert!(held.position.distance(steady.position) > 1e-4);
        assert_eq!(d.track, other.track);
    }

    #[test]
    fn a_format_fits_its_picture_in_the_window() {
        // a scope picture in a 16:9 window: bars above and below
        let (w, h) = Format::Scope.fit(1920.0, 1080.0);
        assert_eq!(w, 1920.0);
        assert!((h - 1920.0 / 2.39).abs() < 0.01);
        // a vertical picture: bars on the sides
        let (w, h) = Format::Vertical.fit(1920.0, 1080.0);
        assert_eq!(h, 1080.0);
        assert!((w - 607.5).abs() < 0.01);
        assert_eq!(Format::Window.fit(800.0, 600.0), (800.0, 600.0));
        assert_eq!(Format::Wide.fit(0.0, 0.0), (0.0, 0.0));
        for format in Format::ALL {
            assert_eq!(format.aspect().is_some(), format.ratio().is_some());
        }
        let d = Director {
            format: Format::Square,
            ..Default::default()
        };
        assert_eq!(d.picture_aspect(1.78), 1.0);
        assert_eq!(Director::default().picture_aspect(1.78), 1.78);
        // every joint has a bone of the avatar's skeleton
        for joint in Joint::ALL {
            assert!(joint.bone().starts_with('m'));
        }
    }
}
