//! When a constellation shows, how far it has drawn itself and how it
//! fades: pure rules, advanced by the frame's time (0 while a photo holds
//! its instant, so every tile of a photo sees the same sky).
//!
//! - A figure is wanted while it is in the field around the view, the sky
//!   is dark enough, the real sky is shown and the overlay is on. It then
//!   draws itself like a pen stroke over the drawing time, starting from
//!   nothing: it never pops in.
//! - Not wanted any more, it fades out over the fade-out time, still
//!   drawing if it had not finished. Wanted again before it is gone, it
//!   fades back in where it stands (no new stroke); once gone, the next
//!   time starts a new stroke.
//! - Its name has an opacity of its own that follows its switch and the
//!   end of the stroke at the same pace, so a switch never blinks it.
//! - Its drawing (from the chosen art set) has a state of its own
//!   ([`Drawing`]): it begins its pen stroke with the figure's lines, or
//!   later if its picture comes later, and fades out on its own when it is
//!   no longer the one to show (the drawing switch, another set chosen),
//!   while the incoming drawing draws itself in: a set never blinks.

/// Where the reveal stops: past the last texels' reveal time, so that the
/// pen tip's glow has faded out (constellations.wgsl `constellation_overlay`).
pub const REVEAL_END: f32 = 1.12;
/// The lines take this share of an engraved figure's stroke (the art's
/// preview); a figure with no picture spends the whole stroke on them.
pub const LINE_SHARE: f32 = 0.45;
/// The name fades in over this part of the stroke.
pub const NAME_FROM: f32 = 0.7;
pub const NAME_TO: f32 = 1.0;
/// Degrees past the field a figure must go before it is let go: a figure on
/// the field's edge does not flicker in and out.
pub const FIELD_HYSTERESIS: f32 = 2.0;
/// The drawing brightness rises over this first share of a stroke (from 1:
/// nothing steps up when the stroke begins) and eases back over this many
/// seconds once its part is drawn (nothing steps down at the end).
pub const GAIN_RISE: f32 = 0.05;
pub const GAIN_SETTLE: f32 = 1.0;
/// The completion pulse: seconds it lasts, and its light at its peak (half
/// way), over the part's own.
pub const PULSE_SECONDS: f32 = 1.0;
pub const PULSE_PEAK: f32 = 0.6;
/// A part's time since it finished stops counting here: past every
/// envelope above.
const DONE_CAP: f32 = 8.0;

/// Seconds since a part finished appearing: `None` while it is not done,
/// counted with the frame's time (a photo's held instant holds it).
fn count_done(done: Option<f32>, finished: bool, dt: f32) -> Option<f32> {
    match (done, finished) {
        (_, false) => None,
        (None, true) => Some(0.0),
        (Some(t), true) => Some((t + dt).min(DONE_CAP)),
    }
}

/// How much brighter a part shows while it draws itself
/// (`draw_brightness`, 1 to 2): `progress` is its own stroke's (0 to 1 and
/// past), `done` the seconds since it finished. 1 at the stroke's start,
/// `brightness` once past `GAIN_RISE`, back to exactly 1 `GAIN_SETTLE`
/// seconds after the end, smoothly at both ends.
pub fn drawing_gain(progress: f32, done: Option<f32>, brightness: f32) -> f32 {
    let rise = smoothstep(0.0, GAIN_RISE, progress);
    let settle = 1.0 - done.map_or(0.0, |t| smoothstep(0.0, GAIN_SETTLE, t));
    1.0 + (brightness.max(1.0) - 1.0) * rise * settle
}

/// The completion pulse's light over the part's own (0 when off or out of
/// it): sin² over `PULSE_SECONDS` after the part finished, so it rises and
/// falls with no step and no kink at either end, `PULSE_PEAK` half way.
pub fn pulse(done: Option<f32>) -> f32 {
    match done {
        Some(t) if t > 0.0 && t < PULSE_SECONDS => {
            let s = (std::f32::consts::PI * t / PULSE_SECONDS).sin();
            PULSE_PEAK * s * s
        }
        _ => 0.0,
    }
}

/// A part's light: the drawing brightness, plus the pulse when the
/// highlight is on. Added, not multiplied: the pulse is the same +60 % of
/// the part's own light whatever the brightness, and both are continuous,
/// so their sum has no step either.
pub fn part_gain(progress: f32, done: Option<f32>, brightness: f32, highlight: bool) -> f32 {
    drawing_gain(progress, done, brightness) + if highlight { pulse(done) } else { 0.0 }
}

/// One figure's state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Figure {
    /// Its stroke has begun (and it has not faded out since).
    pub started: bool,
    /// How far the stroke has come, 0 to REVEAL_END.
    pub reveal: f32,
    /// The figure's opacity.
    pub alpha: f32,
    /// A drawing began with it (decided when the stroke begins): the lines'
    /// pace. Its name's opacity.
    pub with_picture: bool,
    pub name_alpha: f32,
    /// In the field (with the hysteresis).
    pub in_field: bool,
    /// Seconds since its lines were all drawn, and since its name was
    /// whole (fully faded in; it counts again after it faded).
    pub lines_done: Option<f32>,
    pub name_done: Option<f32>,
}

/// What a frame asks of a figure.
#[derive(Clone, Copy, Debug)]
pub struct Step {
    pub want: bool,
    /// Whether its stroke may begin, and with a drawing: `None` while its
    /// drawing is still on its way.
    pub ready: Option<bool>,
    /// Its name can show (the names switch, and its picture of the name is
    /// in the atlas).
    pub name_on: bool,
    pub dt: f32,
    pub draw_seconds: f32,
    pub fade_seconds: f32,
}

fn toward(v: f32, target: f32, rate: f32) -> f32 {
    if v < target {
        (v + rate).min(target)
    } else {
        (v - rate).max(target)
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Figure {
    pub fn step(&mut self, s: Step) {
        let dt = s.dt.max(0.0);
        let fade = dt / s.fade_seconds.max(0.1);
        if s.want {
            if !self.started {
                let Some(picture) = s.ready else { return };
                *self = Figure {
                    started: true,
                    with_picture: picture,
                    alpha: 1.0,
                    in_field: self.in_field,
                    ..Figure::default()
                };
            } else {
                self.alpha = toward(self.alpha, 1.0, fade);
            }
        } else if self.started {
            self.alpha = toward(self.alpha, 0.0, fade);
            if self.alpha <= 0.0 {
                *self = Figure {
                    in_field: self.in_field,
                    ..Figure::default()
                };
                return;
            }
        } else {
            return;
        }
        self.reveal = (self.reveal + dt / s.draw_seconds.max(0.1)).min(REVEAL_END);
        let name = if s.name_on {
            smoothstep(NAME_FROM, NAME_TO, self.reveal)
        } else {
            0.0
        };
        self.name_alpha = toward(self.name_alpha, name, fade.max(dt / 0.8));
        self.lines_done = count_done(self.lines_done, self.line_reveal() >= 1.0, dt);
        self.name_done = count_done(self.name_done, self.name_alpha >= 1.0, dt);
    }

    /// The stroke's time the lines have reached (their own 0 to 1).
    pub fn line_reveal(&self) -> f32 {
        if self.with_picture { self.reveal / LINE_SHARE } else { self.reveal }
    }

    /// Anything of it shows.
    pub fn shown(&self) -> bool {
        self.started && self.alpha > 0.0
    }
}

/// One drawing's state: its own pen stroke and opacity.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Drawing {
    pub started: bool,
    /// How far its stroke has come, 0 to REVEAL_END.
    pub reveal: f32,
    pub alpha: f32,
    /// Seconds since its stroke reached its last texels (reveal 1).
    pub done: Option<f32>,
}

impl Drawing {
    /// `want`: it is the one to show (its figure shows, the drawing switch
    /// is on, its set is the chosen one); `ready`: its picture is in the
    /// atlas. Wanted, it begins from nothing once ready and draws itself in
    /// over `draw_seconds`; no longer wanted, it fades out over
    /// `fade_seconds` (still drawing if it had not finished), then is
    /// gone: the next time is a new stroke.
    pub fn step(&mut self, want: bool, ready: bool, dt: f32, draw_seconds: f32, fade_seconds: f32) {
        let dt = dt.max(0.0);
        let fade = dt / fade_seconds.max(0.1);
        if want {
            if !self.started {
                if !ready {
                    return;
                }
                *self = Drawing {
                    started: true,
                    reveal: 0.0,
                    alpha: 1.0,
                    done: None,
                };
            } else {
                self.alpha = toward(self.alpha, 1.0, fade);
            }
        } else if self.started {
            self.alpha = toward(self.alpha, 0.0, fade);
            if self.alpha <= 0.0 {
                *self = Drawing::default();
                return;
            }
        } else {
            return;
        }
        self.reveal = (self.reveal + dt / draw_seconds.max(0.1)).min(REVEAL_END);
        self.done = count_done(self.done, self.reveal >= 1.0, dt);
    }

    pub fn shown(&self) -> bool {
        self.started && self.alpha > 0.0
    }
}

/// The field test with its hysteresis: `cos_angle` the cosine of the angle
/// from the view's direction to the figure's nearest point, `field` the
/// field's half-angle (degrees).
pub fn in_field(was: bool, cos_angle: f32, field: f32) -> bool {
    let limit = if was { field + FIELD_HYSTERESIS } else { field };
    limit >= 180.0 || cos_angle >= limit.to_radians().cos()
}

/// Where a drawing's picture stands: one per drawing of every set. A
/// picture is read and decoded only when its figure first shows (on the
/// overlay's worker, never on the frame's thread), goes into the atlas in
/// bands of rows over a few frames, and draws only once whole: until then
/// its figure shows its lines and name, and its pen stroke begins from
/// nothing once it is ready.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Load {
    /// Never asked, or let go before it reached the atlas.
    #[default]
    NotLoaded,
    /// Asked of the worker at `since` (the overlay's clock, seconds).
    Loading { since: f64 },
    /// Read and decoded (its mip levels), waiting for room in the atlas.
    Decoded { since: f64, levels: Vec<std::sync::Arc<Vec<u8>>> },
    /// Placed in the atlas; level-0 rows before `row` are sent.
    Uploading {
        since: f64,
        levels: Vec<std::sync::Arc<Vec<u8>>>,
        row: u32,
    },
    /// Whole in the atlas.
    Ready,
    /// Was in the atlas; its room went to others. Asked again when needed.
    Evicted,
    /// Could not be read or decoded (said once in the log): its figure
    /// keeps its lines and name.
    Failed,
}

/// One band of a picture for the atlas.
#[derive(Debug, PartialEq)]
pub struct Band {
    pub rows: [u32; 2],
    pub levels: Vec<std::sync::Arc<Vec<u8>>>,
    /// The last band: the picture is ready, asked `since` (seconds).
    pub done_since: Option<f64>,
}

/// Rows of a band: a multiple of the mips' alignment (4 rows for 3 levels).
pub const BAND_ALIGN: u32 = 1 << (aurora_render::constellations::ATLAS_MIPS - 1);

impl Load {
    /// Wanted with nothing on its way: it is asked of the worker (true).
    pub fn request(&mut self, now: f64) -> bool {
        if matches!(self, Load::NotLoaded | Load::Evicted) {
            *self = Load::Loading { since: now };
            true
        } else {
            false
        }
    }

    /// The worker's answer; an answer it no longer waits for is dropped.
    /// A failure is returned, to be said once.
    pub fn answered(&mut self, result: Result<Vec<Vec<u8>>, String>) -> Option<String> {
        let Load::Loading { since } = *self else { return None };
        match result {
            Ok(levels) => {
                *self = Load::Decoded {
                    since,
                    levels: levels.into_iter().map(std::sync::Arc::new).collect(),
                };
                None
            }
            Err(e) => {
                *self = Load::Failed;
                Some(e)
            }
        }
    }

    /// The request could not reach the worker: failed.
    pub fn unsent(&mut self) {
        *self = Load::Failed;
    }

    /// Its room in the atlas is found: its upload begins.
    pub fn placed(&mut self) {
        if let Load::Decoded { since, levels } = std::mem::take(self) {
            *self = Load::Uploading { since, levels, row: 0 };
        }
    }

    /// The next band of a picture `height` rows tall, at most `max_rows`
    /// rows (rounded to the bands' alignment); its last band makes it ready.
    pub fn next_band(&mut self, height: u32, max_rows: u32) -> Option<Band> {
        let Load::Uploading { since, levels, row } = self else {
            return None;
        };
        let step = (max_rows / BAND_ALIGN).max(1) * BAND_ALIGN;
        let rows = [*row, row.saturating_add(step).min(height)];
        let levels = levels.clone();
        let since = *since;
        *row = rows[1];
        let done = rows[1] >= height;
        if done {
            *self = Load::Ready;
        }
        Some(Band {
            rows,
            levels,
            done_since: done.then_some(since),
        })
    }

    /// Nobody waits for it before it reached the atlas: its memory goes.
    pub fn unwanted(&mut self) {
        if matches!(self, Load::Decoded { .. }) {
            *self = Load::NotLoaded;
        }
    }

    /// The atlas does not hold it (any more).
    pub fn lost(&mut self) {
        if matches!(self, Load::Ready | Load::Uploading { .. }) {
            *self = Load::Evicted;
        }
    }

    pub fn ready(&self) -> bool {
        matches!(self, Load::Ready)
    }

    /// It has a place in the atlas (whole or going up).
    pub fn in_atlas(&self) -> bool {
        matches!(self, Load::Ready | Load::Uploading { .. })
    }

    pub fn failed(&self) -> bool {
        matches!(self, Load::Failed)
    }
}

/// The sky is dark enough: the sun (region direction, z up) `below`
/// degrees or more under the horizon.
pub fn dark_enough(sun: glam::Vec3, below: f32) -> bool {
    let sun = sun.normalize_or(glam::Vec3::NEG_Z);
    sun.z <= -below.clamp(0.0, 90.0).to_radians().sin()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(want: bool, ready: Option<bool>, dt: f32) -> Step {
        Step {
            want,
            ready,
            name_on: true,
            dt,
            draw_seconds: 4.0,
            fade_seconds: 3.0,
        }
    }

    fn run(f: &mut Figure, s: Step, seconds: f32) {
        let frames = (seconds * 60.0).round() as u32;
        for _ in 0..frames {
            f.step(Step { dt: 1.0 / 60.0, ..s });
        }
    }

    #[test]
    fn a_figure_draws_from_nothing_and_waits_for_its_picture() {
        let mut f = Figure::default();
        run(&mut f, step(true, None, 0.0), 1.0);
        assert_eq!(f, Figure::default(), "waits, nothing shown");
        f.step(step(true, Some(true), 0.0));
        assert!(f.started && f.alpha == 1.0 && f.reveal == 0.0, "begins at nothing: {f:?}");
        run(&mut f, step(true, Some(true), 0.0), 2.0);
        assert!((f.reveal - 0.5).abs() < 0.01, "half drawn in 2 s of 4: {}", f.reveal);
        assert!((f.line_reveal() - 0.5 / LINE_SHARE).abs() < 0.03);
        assert_eq!(f.name_alpha, 0.0, "the name waits for the end of the stroke");
        run(&mut f, step(true, Some(true), 0.0), 3.0);
        assert_eq!(f.reveal, REVEAL_END, "stops past the end");
        assert!(f.name_alpha > 0.99);
        // a lines-only figure spends the whole stroke on its lines
        let mut l = Figure::default();
        l.step(step(true, Some(false), 0.0));
        run(&mut l, step(true, Some(false), 0.0), 2.0);
        assert!((l.line_reveal() - l.reveal).abs() < 1e-6 && !l.with_picture);
    }

    #[test]
    fn leaving_fades_out_continuously_and_coming_back_fades_in() {
        let mut f = Figure::default();
        f.step(step(true, Some(true), 0.0));
        run(&mut f, step(true, Some(true), 0.0), 5.0);
        let mut last = f.alpha;
        for _ in 0..90 {
            f.step(step(false, Some(true), 1.0 / 60.0));
            assert!(
                f.alpha < last && last - f.alpha < 0.01,
                "a continuous fade: {} -> {}",
                last,
                f.alpha
            );
            last = f.alpha;
        }
        assert!((f.alpha - 0.5).abs() < 0.02, "half gone in 1.5 s of 3: {}", f.alpha);
        // back before it is gone: no new stroke, it fades back in
        let reveal = f.reveal;
        f.step(step(true, Some(true), 1.0 / 60.0));
        assert!(f.alpha > last && f.reveal >= reveal);
        run(&mut f, step(false, Some(true), 0.0), 3.1);
        assert_eq!(f, Figure::default(), "gone: the next time is a new stroke");
        // a photo's held instant changes nothing
        let mut g = Figure::default();
        g.step(step(true, Some(true), 0.0));
        run(&mut g, step(true, Some(true), 0.0), 1.0);
        let held = g;
        g.step(step(false, Some(true), 0.0));
        assert_eq!(g, held);
    }

    #[test]
    fn the_name_switch_fades_the_name_on_its_own() {
        let mut f = Figure::default();
        f.step(step(true, Some(true), 0.0));
        run(&mut f, step(true, Some(true), 0.0), 5.0);
        let off = Step {
            name_on: false,
            ..step(true, Some(true), 0.0)
        };
        f.step(Step { dt: 1.0 / 60.0, ..off });
        assert!(f.name_alpha < 1.0 && f.name_alpha > 0.9);
        run(&mut f, off, 3.1);
        assert_eq!(f.name_alpha, 0.0);
        assert_eq!(f.alpha, 1.0, "the lines stay");
    }

    #[test]
    fn a_new_set_draws_in_while_the_old_one_fades_out() {
        let frame = 1.0 / 60.0;
        let (mut old, mut new) = (Drawing::default(), Drawing::default());
        old.step(true, true, 0.0, 4.0, 3.0);
        for _ in 0..300 {
            old.step(true, true, frame, 4.0, 3.0);
        }
        assert_eq!((old.alpha, old.reveal), (1.0, REVEAL_END));
        // the other set is chosen: its picture is on its way
        new.step(true, false, frame, 4.0, 3.0);
        assert_eq!(new, Drawing::default(), "nothing before its picture");
        let mut last = old.alpha;
        for i in 0..240 {
            old.step(false, true, frame, 4.0, 3.0);
            new.step(true, true, frame, 4.0, 3.0);
            if i == 0 {
                assert!(new.started && new.reveal < 0.01 && new.alpha == 1.0, "drawn from nothing: {new:?}");
            }
            assert!(old.alpha <= last && last - old.alpha < 0.01, "a continuous fade");
            last = old.alpha;
        }
        assert_eq!(old, Drawing::default(), "gone after the fade-out time");
        assert!((new.reveal - 1.0).abs() < 0.01, "drawn in over the drawing time: {}", new.reveal);
        // switched back before it is gone: it fades back in, no new stroke
        let mut d = Drawing::default();
        d.step(true, true, 0.0, 4.0, 3.0);
        for _ in 0..60 {
            d.step(true, true, frame, 4.0, 3.0);
        }
        for _ in 0..30 {
            d.step(false, true, frame, 4.0, 3.0);
        }
        let (a, r) = (d.alpha, d.reveal);
        d.step(true, true, frame, 4.0, 3.0);
        assert!(d.alpha > a && d.reveal > r && d.started);
        // a photo's held instant changes nothing
        let held = d;
        d.step(false, true, 0.0, 4.0, 3.0);
        assert_eq!(d, held);
    }

    #[test]
    fn the_field_and_the_darkness_gate() {
        let cos = |deg: f32| deg.to_radians().cos();
        assert!(in_field(false, cos(49.0), 50.0));
        assert!(!in_field(false, cos(51.0), 50.0));
        assert!(in_field(true, cos(51.0), 50.0), "kept within the hysteresis");
        assert!(!in_field(true, cos(52.5), 50.0));
        assert!(in_field(false, cos(179.0), 180.0));
        let sun = |alt: f32| glam::Vec3::new(alt.to_radians().cos(), 0.0, alt.to_radians().sin());
        assert!(dark_enough(sun(-8.0), 8.0));
        assert!(dark_enough(sun(-20.0), 8.0));
        assert!(!dark_enough(sun(-7.0), 8.0));
        assert!(!dark_enough(sun(30.0), 8.0));
        assert!(dark_enough(sun(-0.5), 0.0));
        assert!(dark_enough(glam::Vec3::ZERO, 8.0), "no sun: night");
        assert!(!dark_enough(sun(-10.0), f32::NAN.max(18.0)));
    }
    #[test]
    fn a_picture_loads_on_demand_and_comes_back_after_eviction() {
        let mut l = Load::default();
        assert_eq!(l, Load::NotLoaded);
        assert!(l.next_band(10, 4).is_none() && !l.ready());
        assert!(l.request(1.0), "asked once");
        assert!(!l.request(1.5), "not twice");
        assert_eq!(l, Load::Loading { since: 1.0 });
        assert_eq!(l.answered(Ok(vec![vec![1; 40], vec![2; 10], vec![3; 4]])), None);
        assert!(matches!(l, Load::Decoded { since: 1.0, .. }));
        // no room yet: nothing goes up
        assert!(l.next_band(10, 4).is_none());
        l.placed();
        assert!(l.in_atlas() && !l.ready(), "placed is not ready");
        // 10 rows in bands of 4: rows 0-4, 4-8, 8-10; ready with the last
        let b = l.next_band(10, 5).expect("first band");
        assert_eq!((b.rows, b.done_since), ([0, 4], None));
        assert_eq!(b.levels.len(), 3);
        assert_eq!(l.next_band(10, 4).map(|b| b.rows), Some([4, 8]));
        let last = l.next_band(10, 4).expect("last band");
        assert_eq!((last.rows, last.done_since), ([8, 10], Some(1.0)));
        assert!(l.ready() && l.next_band(10, 4).is_none());
        // the atlas made room with it: asked again when wanted
        l.lost();
        assert_eq!(l, Load::Evicted);
        assert!(l.request(9.0));
        assert_eq!(l, Load::Loading { since: 9.0 });
        // an answer it no longer waits for is dropped
        let mut idle = Load::Ready;
        assert_eq!(idle.answered(Err("late".into())), None);
        assert!(idle.ready());
    }

    #[test]
    fn a_failed_load_stays_failed_and_is_said_once() {
        let mut l = Load::default();
        assert!(l.request(0.0));
        assert_eq!(l.answered(Err("truncated WebP".into())).as_deref(), Some("truncated WebP"));
        assert!(l.failed());
        // never asked again, never a band, nothing to say again
        assert!(!l.request(5.0));
        assert!(l.next_band(10, 4).is_none());
        assert_eq!(l.answered(Err("again".into())), None);
        // no worker to ask: failed at once
        let mut n = Load::default();
        assert!(n.request(0.0));
        n.unsent();
        assert!(n.failed());
    }

    #[test]
    fn a_picture_nobody_waits_for_lets_its_memory_go() {
        let mut l = Load::default();
        l.request(0.0);
        l.answered(Ok(vec![vec![0; 8]]));
        l.unwanted();
        assert_eq!(l, Load::NotLoaded);
        // unwanted does nothing to one in the atlas
        let mut r = Load::Ready;
        r.unwanted();
        assert!(r.ready());
        // a picture going up that the atlas loses is evicted, not half shown
        let mut u = Load::default();
        u.request(0.0);
        u.answered(Ok(vec![vec![0; 8]]));
        u.placed();
        u.lost();
        assert_eq!(u, Load::Evicted);
    }

    #[test]
    fn the_drawing_brightness_rises_and_settles_without_a_step() {
        // x1: exactly the part's own light, all along
        for p in [0.0, 0.01, 0.5, 1.0, 1.1] {
            for t in [None, Some(0.0), Some(0.5), Some(3.0)] {
                assert_eq!(drawing_gain(p, t, 1.0), 1.0);
            }
        }
        // x2: from 1 at the stroke's start, smoothly
        assert_eq!(drawing_gain(0.0, None, 2.0), 1.0);
        assert!(drawing_gain(0.001, None, 2.0) - 1.0 < 0.01, "no step at the start");
        assert_eq!(drawing_gain(0.5, None, 2.0), 2.0);
        // still x2 the instant it is done, back to exactly 1 after the settle
        assert_eq!(drawing_gain(1.0, Some(0.0), 2.0), 2.0);
        assert!(2.0 - drawing_gain(1.0, Some(0.01), 2.0) < 0.01, "no step at the end");
        assert_eq!(drawing_gain(1.0, Some(GAIN_SETTLE), 2.0), 1.0);
        assert_eq!(drawing_gain(1.0, Some(DONE_CAP), 2.0), 1.0);
        // monotonic on the way down
        let mut last = 2.0;
        for k in 0..=100 {
            let g = drawing_gain(1.0, Some(k as f32 / 100.0 * GAIN_SETTLE), 2.0);
            assert!(g <= last + 1e-6);
            last = g;
        }
    }

    #[test]
    fn the_completion_pulse_is_one_soft_bump() {
        assert_eq!(pulse(None), 0.0);
        assert_eq!(pulse(Some(0.0)), 0.0, "starts from nothing");
        assert!(pulse(Some(0.01)) < 0.01, "no step as it starts");
        assert!((pulse(Some(PULSE_SECONDS * 0.5)) - PULSE_PEAK).abs() < 1e-6, "its peak half way");
        assert!(pulse(Some(PULSE_SECONDS - 0.01)) < 0.01, "no step as it ends");
        assert_eq!(pulse(Some(PULSE_SECONDS)), 0.0, "back to exactly nothing");
        assert_eq!(pulse(Some(5.0)), 0.0);
        // rises then falls, never above its peak
        let samples: Vec<f32> = (0..=100).map(|k| pulse(Some(k as f32 / 100.0 * PULSE_SECONDS))).collect();
        assert!(samples.iter().all(|v| *v >= 0.0 && *v <= PULSE_PEAK + 1e-6));
        assert!(samples[..50].windows(2).all(|w| w[1] >= w[0]) && samples[50..].windows(2).all(|w| w[1] <= w[0]));
        // with the drawing brightness: no step anywhere, back to exactly 1
        let mut last = part_gain(1.0, Some(0.0), 2.0, true);
        for k in 1..=200 {
            let g = part_gain(1.0, Some(k as f32 / 100.0), 2.0, true);
            assert!((g - last).abs() < 0.04, "a step at {k}: {last} -> {g}");
            last = g;
        }
        assert_eq!(last, 1.0);
        assert_eq!(part_gain(1.0, Some(0.5), 1.0, false), 1.0, "off: the part's own light");
    }

    #[test]
    fn each_part_counts_its_own_time_since_it_finished() {
        // a figure with a picture: its lines are done at 0.45 of the stroke,
        // its name once whole
        let mut f = Figure::default();
        f.step(step(true, Some(true), 0.0));
        run(&mut f, step(true, Some(true), 0.0), 1.5);
        assert_eq!(f.lines_done, None);
        run(&mut f, step(true, Some(true), 0.0), 1.0);
        let t = f.lines_done.expect("lines done after 0.45 x 4 s");
        assert!((t - 0.7).abs() < 0.05, "{t}");
        assert_eq!(f.name_done, None, "the name is not whole yet");
        run(&mut f, step(true, Some(true), 0.0), 2.0);
        assert!(f.name_done.is_some());
        // a photo's held instant holds it
        let held = f;
        f.step(step(true, Some(true), 0.0));
        assert_eq!(f, held);
        // the name switched off then on: it is whole again later, and pulses
        let off = Step {
            name_on: false,
            ..step(true, Some(true), 0.0)
        };
        run(&mut f, off, 1.0);
        assert_eq!(f.name_done, None);
        run(&mut f, step(true, Some(true), 0.0), 2.0);
        assert!(f.name_done.is_some());
        // a drawing: done when its stroke reaches 1
        let mut d = Drawing::default();
        d.step(true, true, 0.0, 4.0, 3.0);
        for _ in 0..239 {
            d.step(true, true, 1.0 / 60.0, 4.0, 3.0);
        }
        assert_eq!(d.done, None);
        for _ in 0..30 {
            d.step(true, true, 1.0 / 60.0, 4.0, 3.0);
        }
        assert!(d.done.is_some_and(|t| t > 0.4 && t < 0.55), "{:?}", d.done);
    }

    #[test]
    fn a_late_picture_begins_its_stroke_from_nothing() {
        // the figure's lines began without it; it comes 2 s later
        let mut d = Drawing::default();
        for _ in 0..120 {
            d.step(true, false, 1.0 / 60.0, 4.0, 3.0);
        }
        assert_eq!(d, Drawing::default(), "nothing of it before it is ready");
        d.step(true, true, 1.0 / 60.0, 4.0, 3.0);
        assert!(d.started && d.reveal <= 1.0 / 60.0 / 4.0 + 1e-6, "from the start: {d:?}");
    }
}
