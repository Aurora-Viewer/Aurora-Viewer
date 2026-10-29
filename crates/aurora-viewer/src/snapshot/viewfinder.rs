//! The photo viewfinder's logic (the interface is `ui::viewfinder`, the
//! frames `app::viewfinder`): how much of the overlay shows, the
//! autofocus that hunts past the subject, comes back and locks like a
//! camera's contrast detection, the zone the lens keeps sharp, the
//! distance and exposure scales, and the settings kept between sessions.
//! A camera's behaviour, not the Legacy Viewer's: its snapshot floater has
//! no viewfinder.

use serde::{Deserialize, Serialize};

/// The closest a lens focuses (the photo window's manual distance starts
/// there too): nearer, the autofocus fails.
pub const MIN_FOCUS: f32 = 0.3;
/// The distance taken when nothing is under the AF point (the sky): far
/// enough to be infinity for any lens.
pub const FAR: f32 = 1000.0;
/// Distances from here are shown as infinity.
pub const INFINITY: f32 = 100.0;
/// The apertures the dial steps through (thirds of a stop, like a lens).
pub const STOPS: [f32; 16] = [1.4, 1.8, 2.0, 2.2, 2.8, 3.5, 4.0, 4.5, 5.6, 6.3, 8.0, 9.0, 11.0, 13.0, 16.0, 22.0];
/// The focal lengths of the keys 1 to 5 (mm, full frame).
pub const FOCAL_PRESETS: [f32; 5] = [24.0, 35.0, 50.0, 85.0, 135.0];
/// The exposure scale's reach, in stops either side of Preferences'.
pub const EV_RANGE: f32 = 2.0;
/// A circle of confusion up to this many pixels of the window still looks
/// sharp: the depth of field shown on the scale.
pub const SHARP_COC: f32 = 1.5;
/// Bins of the luminance histogram (`aurora_render::HISTOGRAM_BINS`).
pub const HISTOGRAM_BINS: usize = aurora_render::HISTOGRAM_BINS;

/// How much of the overlay shows (a camera's DISP button).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Display {
    /// Everything: the top bar, the light, the histogram, the level.
    #[default]
    Full,
    /// The bottom bar, the AF point and the dock.
    Essential,
    /// The picture alone; the AF point only while it focuses.
    Hidden,
}

impl Display {
    pub fn next(self) -> Display {
        match self {
            Display::Full => Display::Essential,
            Display::Essential => Display::Hidden,
            Display::Hidden => Display::Full,
        }
    }
}

/// Where the autofocus looks: the face of the avatar under the AF point,
/// or the point itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AfArea {
    #[default]
    Avatar,
    Point,
}

/// The colour of the sharp edges in manual focus (focus peaking).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Peaking {
    #[default]
    White,
    Red,
    Yellow,
    Off,
}

impl Peaking {
    pub const ALL: [Peaking; 4] = [Peaking::White, Peaking::Red, Peaking::Yellow, Peaking::Off];
}

/// The sounds of the viewfinder: the focus beep and the shutter
/// (`Settings::sound`, PlayModeUISndSnapshot) are switched together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sounds {
    BeepAndShutter,
    Shutter,
    None,
}

impl Sounds {
    pub const ALL: [Sounds; 3] = [Sounds::BeepAndShutter, Sounds::Shutter, Sounds::None];

    pub fn of(beep: bool, shutter: bool) -> Sounds {
        match (beep, shutter) {
            (_, false) => Sounds::None,
            (true, true) => Sounds::BeepAndShutter,
            (false, true) => Sounds::Shutter,
        }
    }

    /// (beep, shutter)
    pub fn split(self) -> (bool, bool) {
        match self {
            Sounds::BeepAndShutter => (true, true),
            Sounds::Shutter => (false, true),
            Sounds::None => (false, false),
        }
    }
}

/// When the on-screen controls show (the lens rings, the AF choices, the
/// camera pad): for the people who would rather not learn the shortcuts,
/// which stay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OnScreen {
    #[default]
    Always,
    /// When the pointer comes near the bottom of the screen or the pad.
    Hover,
    /// The keyboard and the wheel only.
    Hidden,
}

impl OnScreen {
    pub const ALL: [OnScreen; 3] = [OnScreen::Always, OnScreen::Hover, OnScreen::Hidden];
}

/// The self-timer's delays (seconds; 0: off).
pub const TIMERS: [u8; 3] = [0, 2, 10];

/// The tilt's reach either side (degrees): the camera turns upside down.
pub const ROLL_MAX: f32 = 180.0;
/// The tilt's detents (degrees) and how far they hold: straight, and a
/// quarter turn either way.
const ROLL_DETENTS: [(f32, f32); 3] = [(0.0, 1.2), (-90.0, 2.0), (90.0, 2.0)];
/// The tilt slider's slope at its middle, as a share of a straight one: a
/// few degrees stay easy to set, and the ends still reach `ROLL_MAX`.
const ROLL_FINE: f32 = 0.3;

/// The tilt at `t` of the slider (−1 to 1): finer near the middle, held at
/// the detents, by half degrees.
pub fn roll_of_slider(t: f32) -> f32 {
    let t = t.clamp(-1.0, 1.0);
    let r = ROLL_MAX * t * (ROLL_FINE + (1.0 - ROLL_FINE) * t.abs());
    let r = ROLL_DETENTS.iter().find(|(d, hold)| (r - d).abs() < *hold).map_or(r, |(d, _)| *d);
    (r * 2.0).round() / 2.0
}

/// Where `roll` sits on the slider (−1 to 1), the inverse of
/// `roll_of_slider`.
pub fn slider_of_roll(roll: f32) -> f32 {
    let x = (roll.abs() / ROLL_MAX).min(1.0);
    let a = 1.0 - ROLL_FINE;
    let t = (-ROLL_FINE + (ROLL_FINE * ROLL_FINE + 4.0 * a * x).sqrt()) / (2.0 * a);
    t.copysign(roll)
}

/// The tilt after a notch of the wheel (Ctrl + Shift): a degree.
pub fn step_roll(roll: f32, dir: i32) -> f32 {
    (roll.round() + dir.signum() as f32).clamp(-ROLL_MAX, ROLL_MAX)
}

/// The viewfinder's settings kept between sessions (the rest of the photo
/// is `snapshot::Settings`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub display: Display,
    pub area: AfArea,
    pub peaking: Peaking,
    /// The two beeps when the focus locks.
    pub beep: bool,
    /// The list of the controls, under the light (display Full).
    pub hints: bool,
    /// The on-screen controls.
    pub controls: OnScreen,
    /// The camera pad shows (off at first: less on the picture).
    pub pad: bool,
    /// The self-timer (seconds, one of `TIMERS`).
    pub timer: u8,
    /// The depth of field was turned on at the viewfinder's first opening
    /// (it used to be off by default, and every profile saved it so).
    pub dof_defaulted: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            display: Display::Full,
            area: AfArea::Avatar,
            peaking: Peaking::White,
            beep: true,
            hints: true,
            controls: OnScreen::Always,
            pad: false,
            timer: 0,
            dof_defaulted: false,
        }
    }
}

/// What the autofocus is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for a click or the shutter.
    Idle,
    /// Looking for the subject (about a third of a second).
    Hunt,
    /// Sharp and held (AF-S).
    Lock,
    /// Too close: nothing changes, no photo.
    Fail,
    /// Following the pointer (AF-C).
    Track,
    /// Set by hand (MF).
    Manual,
}

/// What a frame of the autofocus tells the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Locked,
    Failed,
}

/// The hunt: past the subject, back short of it, then on it (seconds from
/// the start, and the distance at that time as a factor of the target).
const HUNT_PAST: f32 = 0.15;
const HUNT_BACK: f32 = 0.30;
const HUNT_LOCK: f32 = 0.36;
/// The failed hunt lasts a little longer, then blinks this long.
const FAIL_AT: f32 = 0.41;
const FAIL_BLINK: f32 = 1.5;
/// The scale and the distance under the AF point stay after a lock.
const RECENT: f32 = 2.1;
/// How far from the locked distance (a share of it) the subject under the
/// AF point is still the one held.
const HOLD: f32 = 0.05;

/// The autofocus of the viewfinder: in AF-S a click (or the shutter
/// half-pressed) starts a hunt that ends locked on the subject, or failed
/// when it is closer than the lens can focus. Times are seconds of any
/// clock that only goes forward (egui's).
#[derive(Debug, Clone)]
pub struct Autofocus {
    pub phase: Phase,
    /// The distance the lens is at now (m).
    distance: f32,
    from: f32,
    target: f32,
    past: f32,
    back: f32,
    started: f64,
    /// Until then the scale and the distance stay up after a lock.
    recent_until: f64,
    /// The last frame the lens followed its subject.
    followed: f64,
}

impl Default for Autofocus {
    fn default() -> Self {
        Autofocus {
            phase: Phase::Idle,
            distance: 16.0,
            from: 16.0,
            target: 16.0,
            past: 16.0,
            back: 16.0,
            started: 0.0,
            recent_until: 0.0,
            followed: 0.0,
        }
    }
}

/// `t` of the way from `a` to `b`, in dioptres.
fn lerp_dioptre_linear(a: f32, b: f32, t: f32) -> f32 {
    let (ia, ib) = (1.0 / a.max(1e-3), 1.0 / b.max(1e-3));
    1.0 / (ia + (ib - ia) * t.clamp(0.0, 1.0)).max(1.0 / FAR)
}

/// Between two distances, in dioptres: a lens moves evenly in 1/d, so the
/// far end is not crossed in a blink and the near end in an age.
fn lerp_dioptre(a: f32, b: f32, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    let (ia, ib) = (1.0 / a.max(1e-3), 1.0 / b.max(1e-3));
    1.0 / (ia + (ib - ia) * t).max(1.0 / FAR)
}

impl Autofocus {
    /// The distance the lens is at (m).
    pub fn distance(&self) -> f32 {
        self.distance
    }

    /// Start a hunt towards `target` (m; `FAR` for nothing under the
    /// point).
    pub fn hunt(&mut self, target: f32, now: f64) {
        let from = self.distance;
        let target = if target.is_finite() { target.clamp(0.01, FAR) } else { FAR };
        let away = from < target;
        let (past, back) = if target < MIN_FOCUS {
            (MIN_FOCUS, 0.7)
        } else if target >= FAR {
            (FAR, 45.0)
        } else if away {
            ((target * 1.7).min(FAR), target * 0.86)
        } else {
            ((target * 0.6).max(MIN_FOCUS), target * 1.16)
        };
        *self = Autofocus {
            phase: Phase::Hunt,
            distance: from,
            from,
            target,
            past,
            back,
            started: now,
            recent_until: self.recent_until,
            followed: now,
        };
    }

    /// Locked on a subject that moves (or a camera that does): the lens
    /// follows its distance, eased in dioptres, without a new hunt.
    pub fn follow(&mut self, target: f32, now: f64) {
        let dt = ((now - self.followed) as f32).clamp(0.0, 0.1);
        self.followed = now;
        if self.phase != Phase::Lock || !target.is_finite() {
            return;
        }
        let target = target.clamp(MIN_FOCUS, FAR);
        let k = 1.0 - (-dt * 12.0).exp();
        self.distance = lerp_dioptre_linear(self.distance, target, k);
        self.target = target;
    }

    /// Follow `target` without a hunt (AF-C; the lens's own transition
    /// smooths it).
    pub fn track(&mut self, target: f32, now: f64) {
        if target.is_finite() {
            self.distance = target.clamp(MIN_FOCUS, FAR);
        }
        self.phase = Phase::Track;
        self.recent_until = now + f64::from(RECENT);
    }

    /// Set by hand (MF).
    pub fn manual(&mut self, distance: f32) {
        self.distance = distance.clamp(MIN_FOCUS, FAR);
        self.phase = Phase::Manual;
    }

    /// Locked, and `target` (m) is the subject it holds: within a few
    /// percent of its distance, what a small move of the camera or the
    /// subject leaves.
    pub fn holds(&self, target: f32) -> bool {
        self.phase == Phase::Lock && target.is_finite() && (target.clamp(MIN_FOCUS, FAR) - self.distance).abs() <= self.distance * HOLD
    }

    /// The lock kept by a new press: the scale and the distance show again.
    pub fn confirm(&mut self, now: f64) {
        if self.phase == Phase::Lock {
            self.recent_until = now + f64::from(RECENT);
        }
    }

    /// Back to waiting, the lens where it is.
    pub fn idle(&mut self) {
        self.phase = Phase::Idle;
    }

    /// The scale and the distance under the point are still shown.
    pub fn recent(&self, now: f64) -> bool {
        now < self.recent_until
    }

    /// Seconds since the phase began (the hunt, the lock, the failure):
    /// what the AF point's animation follows.
    pub fn phase_time(&self, now: f64) -> f32 {
        let t = (now - self.started) as f32;
        match self.phase {
            Phase::Lock => t - HUNT_LOCK,
            Phase::Fail => t - FAIL_AT,
            _ => t,
        }
    }

    /// One frame at `now`: the lens's distance moves along the hunt, which
    /// ends locked or failed.
    pub fn update(&mut self, now: f64) -> Option<Event> {
        let t = (now - self.started) as f32;
        match self.phase {
            Phase::Hunt if self.target < MIN_FOCUS => {
                if t < HUNT_PAST {
                    self.distance = lerp_dioptre(self.from, self.past, t / HUNT_PAST);
                } else if t < HUNT_BACK {
                    self.distance = lerp_dioptre(self.past, self.back, (t - HUNT_PAST) / (HUNT_BACK - HUNT_PAST));
                } else if t < FAIL_AT {
                    self.distance = lerp_dioptre(self.back, MIN_FOCUS, (t - HUNT_BACK) / (FAIL_AT - HUNT_BACK));
                } else {
                    // the lens goes back where it was
                    self.distance = self.from;
                    self.phase = Phase::Fail;
                    return Some(Event::Failed);
                }
                None
            }
            Phase::Hunt => {
                if t < HUNT_PAST {
                    self.distance = lerp_dioptre(self.from, self.past, t / HUNT_PAST);
                } else if t < HUNT_BACK {
                    self.distance = lerp_dioptre(self.past, self.back, (t - HUNT_PAST) / (HUNT_BACK - HUNT_PAST));
                } else if t < HUNT_LOCK {
                    self.distance = lerp_dioptre(self.back, self.target, (t - HUNT_BACK) / (HUNT_LOCK - HUNT_BACK));
                } else {
                    self.distance = self.target;
                    self.phase = Phase::Lock;
                    self.recent_until = now + f64::from(RECENT);
                    return Some(Event::Locked);
                }
                None
            }
            Phase::Fail if t > FAIL_AT + FAIL_BLINK => {
                self.phase = Phase::Idle;
                None
            }
            _ => None,
        }
    }
}

/// The nearest and farthest distances the lens keeps sharp (a circle
/// under `SHARP_COC` pixels), for `coc_scale` the circle in pixels for
/// (s - z) / z at the focus `focus` (`dof::coc_scale`). The far limit is
/// `FAR` past the hyperfocal distance.
pub fn sharp_range(coc_scale: f32, focus: f32) -> (f32, f32) {
    let k = coc_scale.max(1e-6);
    let near = k * focus / (k + SHARP_COC);
    let far = if k > SHARP_COC { k * focus / (k - SHARP_COC) } else { FAR };
    (near, far.min(FAR))
}

/// The group focus (Shift + click on avatars, AF-S): every face held stays
/// sharp. A camera would stop down until the group fits in its depth of
/// field, and lose the background's blur; here the renderer keeps a zone
/// sharp from the nearest face to the farthest
/// (`aurora_render::DepthOfField::focus_far`) and blurs beyond each end at
/// the aperture chosen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zone {
    /// Where the lens focuses (m): the zone's middle in dioptres, where a
    /// lens splits the depth between the two ends.
    pub middle: f32,
    /// The zone's ends as shares of `middle`: the lens hunts and follows
    /// the middle and the zone moves with it, so the picture pumps through
    /// the hunt as with one subject, then sits on the faces once locked.
    pub near: f32,
    pub far: f32,
}

impl Zone {
    /// The zone's ends (m) with the lens at `distance`.
    pub fn at(&self, distance: f32) -> (f32, f32) {
        (distance * self.near, distance * self.far)
    }
}

/// The zone holding the faces at `distances` (m along the view axis); None
/// under two faces (a single subject is a single plane).
pub fn group_zone(distances: &[f32]) -> Option<Zone> {
    let mut finite = distances.iter().filter(|d| d.is_finite()).map(|d| d.clamp(MIN_FOCUS, FAR));
    let first = finite.next()?;
    let (mut near, mut far, mut n) = (first, first, 1);
    for d in finite {
        near = near.min(d);
        far = far.max(d);
        n += 1;
    }
    if n < 2 {
        return None;
    }
    let middle = 2.0 / (1.0 / near + 1.0 / far);
    Some(Zone {
        middle,
        near: near / middle,
        far: far / middle,
    })
}

/// The nearest and farthest distances kept sharp around a zone from `near`
/// to `far` (`sharp_range` beyond each end), `coc_scale` the circle at the
/// focus `near` (the circle at a focus s scales as 1 / s).
pub fn sharp_zone(coc_scale: f32, near: f32, far: f32) -> (f32, f32) {
    let far = far.max(near);
    let front = sharp_range(coc_scale, near).0;
    let back = sharp_range(coc_scale * near / far.max(1e-3), far).1;
    (front, back)
}

/// What the focus holds: an avatar's face, or a point of the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Subject {
    Avatar(uuid::Uuid),
    Point(glam::Vec3),
}

impl Subject {
    /// The same subject, for a click seen from `eye`: the same avatar, or a
    /// point clicked again on the same spot (a few centimetres, more far
    /// away, where a pixel covers more).
    pub fn same(&self, other: &Subject, eye: glam::Vec3) -> bool {
        match (self, other) {
            (Subject::Avatar(a), Subject::Avatar(b)) => a == b,
            (Subject::Point(a), Subject::Point(b)) => a.distance(*b) < 0.05 + 0.03 * a.distance(eye),
            _ => false,
        }
    }
}

/// `s` into the group, or out of it when it is there already (seen from
/// `eye`); true when it came in.
pub fn toggle_subject(group: &mut Vec<Subject>, s: Subject, eye: glam::Vec3) -> bool {
    match group.iter().position(|g| g.same(&s, eye)) {
        Some(i) => {
            group.remove(i);
            false
        }
        None => {
            group.push(s);
            true
        }
    }
}

/// Where a distance sits on the scale, 0 at `MIN_FOCUS` to 1 at infinity:
/// logarithmic like a lens's, infinity a little apart from the rest.
pub fn scale_position(distance: f32) -> f32 {
    if distance >= INFINITY {
        return 1.0;
    }
    let d = distance.max(MIN_FOCUS);
    (d / MIN_FOCUS).ln() / (INFINITY / MIN_FOCUS).ln() * (530.0 / 560.0)
}

/// The marks of the scale (m), the last one infinity.
pub const SCALE_MARKS: [f32; 10] = [0.3, 0.5, 1.0, 2.0, 3.0, 5.0, 10.0, 20.0, 50.0, FAR];

/// The exposure's stops from Preferences' `base`.
pub fn ev_of(exposure: f32, base: f32) -> f32 {
    (exposure.max(1e-3) / base.max(1e-3)).log2()
}

/// The exposure `ev` stops from `base`, rounded to a third of a stop and
/// kept in the photo light's range.
pub fn exposure_at(ev: f32, base: f32) -> f32 {
    let ev = ((ev * 3.0).round() / 3.0).clamp(-EV_RANGE, EV_RANGE);
    let range = crate::snapshot::light::EXPOSURE;
    (base * ev.exp2()).clamp(*range.start(), *range.end())
}

/// The stop after (`dir` > 0) or before `f` on the dial.
pub fn step_aperture(f: f32, dir: i32) -> f32 {
    let i = STOPS
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - f).abs().total_cmp(&(b.1 - f).abs()))
        .map_or(0, |(i, _)| i);
    STOPS[(i as i32 + dir).clamp(0, STOPS.len() as i32 - 1) as usize]
}

/// The focal length after a click of the dial: 6 % a click, at least one
/// millimetre, within the photo window's range.
pub fn step_focal(mm: f32, dir: i32, range: std::ops::RangeInclusive<f32>) -> f32 {
    let next = (mm * if dir > 0 { 1.06 } else { 1.0 / 1.06 }).round();
    let next = if (next - mm.round()).abs() < 1.0 {
        mm.round() + dir.signum() as f32
    } else {
        next
    };
    next.clamp(*range.start(), *range.end())
}

/// The manual distance after a click of the dial: 10 % a click, infinity
/// past `INFINITY`.
pub fn step_distance(d: f32, dir: i32) -> f32 {
    let d = if d >= INFINITY {
        if dir > 0 { FAR } else { 90.0 }
    } else {
        d * if dir > 0 { 1.1 } else { 1.0 / 1.1 }
    };
    if d >= INFINITY { FAR } else { d.max(MIN_FOCUS) }
}

/// A distance as the viewfinder writes it: one decimal below 10 m, none
/// above, infinity past `INFINITY`.
pub fn distance_text(d: f32) -> String {
    if d >= INFINITY {
        "∞".into()
    } else if d < 10.0 {
        aurora_i18n::number(f64::from(d), 1)
    } else {
        format!("{}", d.round() as i64)
    }
}

/// Points of the histogram's curve drawn by the viewfinder.
pub const CURVE_POINTS: usize = 96;

/// The histogram as a camera draws it: a smooth curve, each count under a
/// square root (the small ones stay visible), blurred over its neighbours
/// and sampled at `CURVE_POINTS` points, 0 to 1 (1 at the highest).
pub fn histogram_curve(bins: &[u32]) -> Vec<f32> {
    if bins.is_empty() {
        return vec![0.0; CURVE_POINTS];
    }
    let raw: Vec<f32> = bins.iter().map(|&b| (b as f32).sqrt()).collect();
    // a binomial kernel over five bins
    const K: [f32; 5] = [1.0, 4.0, 6.0, 4.0, 1.0];
    let n = raw.len() as i32;
    let smooth: Vec<f32> = (0..n)
        .map(|i| {
            let (mut sum, mut w) = (0.0, 0.0);
            for (k, kw) in K.iter().enumerate() {
                let j = i + k as i32 - 2;
                if (0..n).contains(&j) {
                    sum += raw[j as usize] * kw;
                    w += kw;
                }
            }
            sum / w
        })
        .collect();
    let top = smooth.iter().copied().fold(0.0f32, f32::max).max(1e-6);
    // Catmull-Rom between the bins' centres
    let at = |i: i32| smooth[i.clamp(0, n - 1) as usize];
    (0..CURVE_POINTS)
        .map(|k| {
            let x = k as f32 / (CURVE_POINTS - 1) as f32 * (n - 1) as f32;
            let i = x.floor() as i32;
            let t = x - i as f32;
            let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
            let v = 0.5
                * (2.0 * p1 + (p2 - p0) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t * t * t);
            (v / top).clamp(0.0, 1.0)
        })
        .collect()
}

const RATE: u32 = 48_000;

/// The focus beep, made here rather than fetched (the interface sounds are
/// grid assets, none of them a camera's): two short tones at 2.75 kHz, a
/// mono 16-bit WAV at 48 kHz.
pub fn beep_wav() -> Vec<u8> {
    tones_wav(&[(0.0, 0.066), (0.11, 0.066)], 0.2)
}

/// One beep of the same tone: the self-timer's.
pub fn tick_wav() -> Vec<u8> {
    tones_wav(&[(0.0, 0.06)], 0.1)
}

/// Beeps at (start, length) seconds in a sound `length` long.
fn tones_wav(beeps: &[(f32, f32)], length: f32) -> Vec<u8> {
    const TONE: f32 = 2750.0;
    let n = (length * RATE as f32) as usize;
    let samples = (0..n).map(|i| {
        let t = i as f32 / RATE as f32;
        let mut v = 0.0;
        for &(start, len) in beeps {
            let u = t - start;
            if (0.0..len).contains(&u) {
                // 5 ms up, 11 ms down: no click
                let env = (u / 0.005).min(1.0) * ((len - u) / 0.011).min(1.0);
                v += (u * TONE * std::f32::consts::TAU).sin() * env * 0.35;
            }
        }
        v
    });
    wav(samples.collect())
}

/// A lens ring's detent: a small metal click (two damped modes at 2.1 and
/// 4.6 kHz over a few milliseconds of noise), quiet.
pub fn click_wav() -> Vec<u8> {
    let n = (0.04 * RATE as f32) as usize;
    // a fixed seed: the same click every time
    let mut seed = 0x2545_f491_u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    let mut last = 0.0;
    let samples = (0..n).map(|i| {
        let t = i as f32 / RATE as f32;
        let tau = std::f32::consts::TAU;
        let modes = (-t / 0.004).exp() * (tau * 2100.0 * t).sin() + 0.5 * (-t / 0.003).exp() * (tau * 4600.0 * t).sin();
        let white = noise();
        // the noise high-passed (its difference), gone in about a millisecond
        let grain = (white - last) * (-t / 0.0012).exp();
        last = white;
        (modes + 0.3 * grain) * 0.2
    });
    wav(samples.collect())
}

/// Mono 16-bit samples (−1 to 1) as a WAV at 48 kHz.
fn wav(samples: Vec<f32>) -> Vec<u8> {
    let samples: Vec<i16> = samples
        .into_iter()
        .map(|v| (v.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)
        .collect();
    let data = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_beep_is_two_tones_with_silence_between() {
        let wav = beep_wav();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        let pcm: Vec<i16> = wav[44..].as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        let loud = |from: f32, to: f32| {
            let r = (from * 48_000.0) as usize..(to * 48_000.0) as usize;
            pcm[r].iter().map(|v| v.unsigned_abs()).max().unwrap_or(0)
        };
        assert!(loud(0.02, 0.05) > 8000, "first beep");
        assert!(loud(0.075, 0.105) == 0, "gap");
        assert!(loud(0.13, 0.16) > 8000, "second beep");
        assert!(aurora_audio::SoundClip::decode(&wav).is_ok());
        // the self-timer's beep and the ring's click decode too
        assert!(aurora_audio::SoundClip::decode(&tick_wav()).is_ok());
        let click = click_wav();
        assert!(aurora_audio::SoundClip::decode(&click).is_ok());
        let peak = click[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b).unsigned_abs())
            .max()
            .unwrap_or(0);
        assert!(peak > 2000 && peak < 12000, "a quiet click: {peak}");
    }

    fn run(af: &mut Autofocus, from: f64, to: f64) -> Vec<Event> {
        let mut events = Vec::new();
        let mut t = from;
        while t <= to {
            events.extend(af.update(t));
            t += 1.0 / 120.0;
        }
        events
    }

    #[test]
    fn a_hunt_goes_past_the_subject_comes_back_and_locks() {
        let mut af = Autofocus::default();
        af.manual(20.0);
        af.hunt(4.2, 10.0);
        // nearer first: past the subject (closer than it), then back
        // beyond it
        let mut nearest = f32::MAX;
        let mut t = 10.0;
        while t < 10.0 + f64::from(HUNT_BACK) {
            af.update(t);
            nearest = nearest.min(af.distance());
            t += 1.0 / 120.0;
        }
        assert!(nearest < 4.2 * 0.7, "it went past: {nearest}");
        assert!(af.distance() > 4.2, "it came back: {}", af.distance());
        assert_eq!(af.phase, Phase::Hunt);
        assert_eq!(run(&mut af, t, 11.0), vec![Event::Locked]);
        assert_eq!(af.phase, Phase::Lock);
        assert_eq!(af.distance(), 4.2);
        assert!(af.recent(11.5) && !af.recent(13.0));
    }

    #[test]
    fn a_locked_lens_follows_its_subject_smoothly() {
        let mut af = Autofocus::default();
        af.manual(20.0);
        af.hunt(4.0, 0.0);
        run(&mut af, 0.0, 1.0);
        assert_eq!(af.phase, Phase::Lock);
        // the subject walks away to 6 m: the lens follows, without a jump
        af.follow(6.0, 1.0);
        let mut t = 1.0;
        let mut last = af.distance();
        for _ in 0..30 {
            t += 1.0 / 60.0;
            af.follow(6.0, t);
            assert!(af.distance() >= last && af.distance() <= 6.0 + 1e-3);
            last = af.distance();
        }
        assert!((af.distance() - 6.0).abs() < 0.1, "{}", af.distance());
        assert_eq!(af.phase, Phase::Lock);
        // not locked: nothing follows
        af.idle();
        af.follow(2.0, t + 0.1);
        assert!((af.distance() - 6.0).abs() < 0.1);
    }

    #[test]
    fn a_lock_holds_the_same_subject_only() {
        let mut af = Autofocus::default();
        af.manual(20.0);
        // not locked: a press always focuses
        assert!(!af.holds(20.0));
        af.hunt(4.0, 0.0);
        assert!(!af.holds(4.0), "still hunting");
        run(&mut af, 0.0, 1.0);
        // the same subject, a step closer or farther: held
        assert!(af.holds(4.0) && af.holds(3.85) && af.holds(4.15));
        // something else under the point: a new hunt
        assert!(!af.holds(3.5) && !af.holds(6.0) && !af.holds(f32::NAN));
        // the press kept the lock: the distance shows again
        assert!(!af.recent(5.0));
        af.confirm(5.0);
        assert!(af.recent(6.0));
        // the sky stays the sky
        af.hunt(FAR, 10.0);
        run(&mut af, 10.0, 11.0);
        assert!(af.holds(FAR) && af.holds(FAR * 2.0));
    }

    #[test]
    fn too_close_fails_and_leaves_the_lens_where_it_was() {
        let mut af = Autofocus::default();
        af.manual(4.2);
        af.hunt(0.2, 0.0);
        assert_eq!(run(&mut af, 0.0, 1.0), vec![Event::Failed]);
        assert_eq!(af.phase, Phase::Fail);
        assert_eq!(af.distance(), 4.2);
        // the blink ends
        run(&mut af, 1.0, 3.0);
        assert_eq!(af.phase, Phase::Idle);
    }

    #[test]
    fn the_sky_locks_at_infinity() {
        let mut af = Autofocus::default();
        af.manual(4.2);
        af.hunt(f32::INFINITY, 0.0);
        assert_eq!(run(&mut af, 0.0, 1.0), vec![Event::Locked]);
        assert_eq!(af.distance(), FAR);
        assert_eq!(distance_text(af.distance()), "∞");
    }

    #[test]
    fn the_sharp_zone_surrounds_the_focus_and_reaches_infinity_past_the_hyperfocal() {
        let (near, far) = sharp_range(40.0, 4.0);
        assert!(near < 4.0 && far > 4.0);
        // a wider circle (a wider aperture) keeps less sharp
        let (n2, f2) = sharp_range(80.0, 4.0);
        assert!(n2 > near && f2 < far);
        // a lens barely blurring keeps everything to infinity
        assert_eq!(sharp_range(1.0, 4.0).1, FAR);
    }

    #[test]
    fn a_group_zone_spans_its_faces_and_moves_with_the_lens() {
        // one face (or none) is a single plane
        assert_eq!(group_zone(&[]), None);
        assert_eq!(group_zone(&[3.0]), None);
        assert_eq!(group_zone(&[3.0, f32::NAN]), None);
        let z = group_zone(&[4.0, 2.0, 6.0, 3.0]).expect("a zone");
        // the middle in dioptres: 1/3 m, halfway between 1/2 and 1/6
        assert!((z.middle - 3.0).abs() < 1e-5, "{}", z.middle);
        let (near, far) = z.at(z.middle);
        assert!((near - 2.0).abs() < 1e-5 && (far - 6.0).abs() < 1e-5);
        // the lens hunting past the middle carries the zone along
        let (n2, f2) = z.at(z.middle * 2.0);
        assert!((n2 - 4.0).abs() < 1e-5 && (f2 - 12.0).abs() < 1e-5);
        // faces closer than the lens can focus count from its minimum
        let close = group_zone(&[0.1, 2.0]).expect("a zone");
        assert!((close.at(close.middle).0 - MIN_FOCUS).abs() < 1e-5);
    }

    #[test]
    fn the_sharp_zone_reaches_past_the_group_on_both_sides() {
        let k = 40.0;
        let (front, back) = sharp_zone(k, 2.0, 6.0);
        assert!(front < 2.0 && back > 6.0);
        // as one plane at each end
        assert_eq!(front, sharp_range(k, 2.0).0);
        assert!((back - sharp_range(k * 2.0 / 6.0, 6.0).1).abs() < 1e-4);
        // a zone of one plane is the plane's
        assert_eq!(sharp_zone(k, 4.0, 4.0), sharp_range(k, 4.0));
    }

    #[test]
    fn shift_click_adds_a_subject_then_takes_it_out() {
        use glam::Vec3;
        let eye = Vec3::ZERO;
        let (a, b) = (uuid::Uuid::from_u128(1), uuid::Uuid::from_u128(2));
        let mut group = vec![Subject::Avatar(a)];
        assert!(toggle_subject(&mut group, Subject::Avatar(b), eye));
        assert!(toggle_subject(&mut group, Subject::Point(Vec3::new(10.0, 0.0, 0.0)), eye));
        assert_eq!(group.len(), 3);
        // the same avatar again: out
        assert!(!toggle_subject(&mut group, Subject::Avatar(a), eye));
        // a point clicked again a little beside, 10 m away: the same one
        assert!(!toggle_subject(&mut group, Subject::Point(Vec3::new(10.0, 0.2, 0.0)), eye));
        assert_eq!(group, vec![Subject::Avatar(b)]);
        // a point a metre beside is another one
        assert!(toggle_subject(&mut group, Subject::Point(Vec3::new(4.0, 0.0, 0.0)), eye));
        assert!(toggle_subject(&mut group, Subject::Point(Vec3::new(4.0, 1.0, 0.0)), eye));
        assert_eq!(group.len(), 3);
    }

    #[test]
    fn the_scale_is_logarithmic_with_infinity_apart() {
        assert_eq!(scale_position(0.1), 0.0);
        assert_eq!(scale_position(MIN_FOCUS), 0.0);
        assert_eq!(scale_position(FAR), 1.0);
        let (a, b, c) = (scale_position(1.0), scale_position(10.0), scale_position(50.0));
        assert!(a < b && b < c && c < 1.0);
        // 1 to 10 m as wide as 0.3 to 3 m
        assert!(((b - a) - (scale_position(3.0) - scale_position(0.3))).abs() < 1e-4);
    }

    #[test]
    fn exposure_steps_in_thirds_within_two_stops() {
        assert!((exposure_at(1.0, 1.0) - 2.0).abs() < 1e-5);
        assert!((exposure_at(0.34, 1.0) - (1.0f32 / 3.0).exp2()).abs() < 1e-5);
        assert!((exposure_at(9.0, 1.0) - 4.0).abs() < 1e-5);
        assert!((ev_of(0.5, 1.0) + 1.0).abs() < 1e-5);
        // never out of the photo light's range
        assert_eq!(exposure_at(-2.0, 0.25), 0.25);
    }

    #[test]
    fn dials_step_and_stop_at_their_ends() {
        assert_eq!(step_aperture(2.8, 1), 3.5);
        assert_eq!(step_aperture(2.7, -1), 2.2);
        assert_eq!(step_aperture(22.0, 1), 22.0);
        assert_eq!(step_focal(50.0, 1, 10.0..=135.0), 53.0);
        assert_eq!(step_focal(10.0, -1, 10.0..=135.0), 10.0);
        assert_eq!(step_focal(12.0, 1, 10.0..=135.0), 13.0);
        assert_eq!(step_distance(95.0, 1), FAR);
        assert_eq!(step_distance(FAR, -1), 90.0);
        assert_eq!(step_distance(0.3, -1), MIN_FOCUS);
    }

    #[test]
    fn the_histogram_curve_is_smooth_and_normalised() {
        // a scene's spread of tones, two humps
        let bins: Vec<u32> = (0..HISTOGRAM_BINS)
            .map(|i| {
                let x = i as f32;
                (900.0 * (-(x - 20.0).powi(2) / 18.0).exp() + 300.0 * (-(x - 44.0).powi(2) / 30.0).exp()) as u32
            })
            .collect();
        let c = histogram_curve(&bins);
        assert_eq!(c.len(), CURVE_POINTS);
        assert!(c.iter().all(|v| (0.0..=1.0).contains(v)));
        assert!((c.iter().copied().fold(0.0f32, f32::max) - 1.0).abs() < 0.05);
        // no step from one point to the next as large as a bar's
        assert!(c.windows(2).all(|w| (w[0] - w[1]).abs() < 0.2));
        assert_eq!(histogram_curve(&[]), vec![0.0; CURVE_POINTS]);
    }

    #[test]
    fn the_tilt_reaches_a_half_turn_and_stays_fine_near_straight() {
        assert_eq!(roll_of_slider(1.0), ROLL_MAX);
        assert_eq!(roll_of_slider(-1.0), -ROLL_MAX);
        assert_eq!(roll_of_slider(0.0), 0.0);
        // a tenth of the slider is a few degrees, not eighteen
        let small = roll_of_slider(0.1);
        assert!(small > 3.0 && small < 8.0, "{small}");
        // the detents hold
        assert_eq!(roll_of_slider(0.004), 0.0);
        assert_eq!(roll_of_slider(slider_of_roll(91.5)), 90.0);
        assert_eq!(roll_of_slider(slider_of_roll(-89.0)), -90.0);
        // the slider's place gives its value back
        for r in [-180.0, -120.0, -45.0, -6.5, 12.0, 60.0, 135.0, 180.0] {
            assert!((roll_of_slider(slider_of_roll(r)) - r).abs() <= 0.5, "{r}");
        }
        assert_eq!(slider_of_roll(0.0), 0.0);
        // the wheel: a degree a notch, to the ends
        assert_eq!(step_roll(0.0, 1), 1.0);
        assert_eq!(step_roll(-6.5, -1), -8.0);
        assert_eq!(step_roll(180.0, 1), 180.0);
    }

    #[test]
    fn display_levels_cycle_and_sounds_round_trip() {
        assert_eq!(Display::Full.next().next().next(), Display::Full);
        for s in Sounds::ALL {
            let (b, sh) = s.split();
            assert_eq!(Sounds::of(b, sh), s);
        }
    }
}
