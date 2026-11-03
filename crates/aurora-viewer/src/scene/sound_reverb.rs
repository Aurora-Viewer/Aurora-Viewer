//! The reverberation of the surroundings: a small room, a large hall and
//! the open air sound different. Aurora's own (Preferences › Sound,
//! "Reverb from the surroundings", off by default): the Legacy Viewer plays
//! every world sound dry, which the switch turned off keeps.
//!
//! A few times a second, [`RAYS`] rays leave the listener (the camera, as
//! for the panning) in fixed directions spread evenly over the sphere (a
//! Fibonacci lattice), up to [`RANGE`]. Each stops at the nearest drawn
//! surface of an object, at the terrain or at the water. From the hits:
//! the enclosure (the share of the rays at or above the horizon that hit
//! something: a ceiling and walls), the openness (the share of every ray
//! that escaped) and the mean distance of the hits. The estimate glides
//! over about a second ([`SMOOTHING_SECS`]), so that walking through a door
//! changes the reverb gradually; a teleport takes the new place at once.
//!
//! The parameters follow Sabine's law: the mean free path is about
//! [`PATH_PER_MEAN`] times the mean distance, the decay time 0.161 V / (S a)
//! = 0.04 path / a, with walls absorbing [`WALL_ABSORPTION`] and every ray
//! that escaped counting as an open window. The level sent to the reverb
//! rises with the enclosure (the open air: none), the pre-delay is the time
//! sound takes to reach the walls, and larger rooms damp the highs more (the
//! air). The mixer glides to them (`aurora_audio`, `reverb.rs`).
//!
//! Each world sound playing at a place sends more of itself to the reverb
//! the farther it is (the direct sound falls with the distance, the
//! reverberation of a room much less): the square root of its distance,
//! from 1 m, at most [`SEND_MAX`]. Sounds at the listener (HUD sounds) stay
//! dry, as do interface sounds, voice chat, music and media.
//!
//! What reflects: the faces the scene draws of objects that are neither
//! avatars, nor worn, nor rigged, nor trees or grass (the occlusion's rule,
//! `Scene::sound_surface`), at least [`MIN_RADIUS`] across. The main thread
//! only gathers: one pass over the pick index for the objects in range
//! (`PickIndex::sphere_candidates`), the [`MAX_SURFACES`] that look largest
//! from the listener kept, their transforms and shared geometry, and the
//! terrain and water along the rays. A background job finds the nearest hit
//! of each ray (large mesh faces through their hierarchies, `Accel::nearest`).

use super::pick_accel::MIN_TRIANGLES;
use super::picking::ray_box;
use super::sound_occlusion::Surface;
use super::*;
use crate::media::pick::ray_triangle;
use aurora_audio::ReverbParams;
use crossbeam_channel::{Receiver, Sender};
use std::sync::OnceLock;
use std::time::Duration;

/// Time between two measures.
const PERIOD: Duration = Duration::from_millis(300);
/// Rays per measure, and how far they look (m).
pub const RAYS: usize = 48;
pub const RANGE: f32 = 40.0;
/// Objects smaller than this (radius, m) do not shape a room.
const MIN_RADIUS: f32 = 0.5;
/// Objects tested at most per measure: those that look largest.
const MAX_SURFACES: usize = 160;
/// Rays this far below the horizon (the sine) or lower are not counted in
/// the enclosure: they meet the floor or the ground.
const HORIZON: f32 = -0.1;
/// Hits nearer than this (m) leave the room's size out when enough others
/// are farther: the wall a camera rests against.
const NEAR: f32 = 0.75;
/// Time constant of the estimate's glide (s).
const SMOOTHING_SECS: f32 = 1.2;
/// A listener this far from the last measure, or measured this long ago,
/// takes the new estimate at once.
const JUMP: f32 = 30.0;
const STALE: Duration = Duration::from_secs(5);
/// Mean free path over the mean distance of the hits (a room seen from
/// inside: 1.2 for a small one, 1.4 for a hall).
const PATH_PER_MEAN: f32 = 1.3;
/// Sabine's 0.161 V / (S a), with V / S = path / 4.
const SABINE: f32 = 0.161 / 4.0;
/// Absorption of the walls (plaster, wood and furniture: a lived-in room).
const WALL_ABSORPTION: f32 = 0.25;
/// Enclosure at which the reverb starts, and at which it is full.
const ENCLOSURE_FROM: f32 = 0.2;
const ENCLOSURE_FULL: f32 = 0.95;
/// The damping (decay of the highs over that of the lows) of a small room
/// and of a hall, and their paths (m).
const DAMPING_SMALL: (f32, f32) = (3.0, 0.75);
const DAMPING_LARGE: (f32, f32) = (25.0, 0.45);
/// Shortest and longest pre-delay (s).
const PREDELAY: (f32, f32) = (0.002, 0.05);
const SOUND_SPEED: f32 = 343.0;
/// Largest distance factor of a sound's send (reached at 4 m).
pub const SEND_MAX: f32 = 2.0;
/// A sound's send goes to the mixer again when it moved this much.
const SEND_STEP: f32 = 0.05;
/// The parameters go to the mixer again when one moved this much
/// (relative), the level this much.
const PARAM_STEP: f32 = 0.02;
const LEVEL_STEP: f32 = 0.01;
/// The log line: when the level moved this much or the decay this much
/// (relative), at most this often.
const LOG_LEVEL_STEP: f32 = 0.05;
const LOG_DECAY_STEP: f32 = 0.08;
const LOG_EVERY: Duration = Duration::from_millis(500);
/// The profile's summary line (AURORA_PROFILE).
const SUMMARY_EVERY: Duration = Duration::from_secs(10);
/// Samples of the terrain along a ray near the horizon, and refinements of
/// a ray going down.
const MARCH_STEPS: usize = 10;
const BISECTIONS: usize = 4;
const DOWN_STEPS: usize = 3;
/// Rays below this (the sine) find the ground from the height under the
/// listener; above, they march; above `MARCH_TOP`, a hill would have to be
/// a cliff.
const STEEP: f32 = -0.25;
const MARCH_TOP: f32 = 0.3;

/// `AURORA_SOUND_REVERB=0|1`: the setting overridden for this launch (A/B
/// measures), never saved.
pub fn forced() -> Option<bool> {
    static FORCED: OnceLock<Option<bool>> = OnceLock::new();
    *FORCED.get_or_init(|| match std::env::var("AURORA_SOUND_REVERB").as_deref() {
        Ok("0") => Some(false),
        Ok("1") => Some(true),
        _ => None,
    })
}

/// The rays' directions: a Fibonacci lattice, evenly spread, the first
/// pointing up.
pub fn directions() -> &'static [Vec3; RAYS] {
    static DIRS: OnceLock<[Vec3; RAYS]> = OnceLock::new();
    DIRS.get_or_init(|| {
        let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
        std::array::from_fn(|i| {
            let z = 1.0 - (2 * i + 1) as f32 / RAYS as f32;
            let r = (1.0 - z * z).max(0.0).sqrt();
            let a = golden * i as f32;
            Vec3::new(r * a.cos(), r * a.sin(), z)
        })
    })
}

/// How much more of a sound `distance` metres away goes to the reverb.
pub fn send_factor(distance: f32) -> f32 {
    if distance.is_finite() {
        distance.max(1.0).sqrt().min(SEND_MAX)
    } else {
        SEND_MAX
    }
}

/// An object that may reflect some of the rays.
struct Reflector {
    surface: Surface,
    /// Its bounding sphere in render space.
    center: Vec3,
    radius: f32,
}

/// What the job needs, gathered by the main thread.
struct Request {
    from: Vec3,
    /// Distance of the terrain or the water along each ray, if in range.
    ground: [Option<f32>; RAYS],
    reflectors: Vec<Reflector>,
}

/// The surroundings, as the rays saw them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    /// Share of the rays at or above the horizon that hit something.
    pub enclosure: f32,
    /// Share of all the rays that escaped.
    pub open: f32,
    /// Mean distance of the hits (m; `RANGE` when none).
    pub mean: f32,
}

impl Estimate {
    /// From the distance each ray travelled (`None`: it escaped).
    pub fn of(hits: &[Option<f32>; RAYS]) -> Estimate {
        let dirs = directions();
        let (mut upper, mut enclosed, mut escaped) = (0u32, 0u32, 0u32);
        // all the hits, and those past the camera's own wall
        let (mut sum, mut n, mut far_sum, mut far_n) = (0.0f32, 0u32, 0.0f32, 0u32);
        for (dir, hit) in dirs.iter().zip(hits) {
            if dir.z >= HORIZON {
                upper += 1;
                enclosed += u32::from(hit.is_some());
            }
            match hit {
                Some(d) => {
                    sum += d;
                    n += 1;
                    if *d >= NEAR {
                        far_sum += d;
                        far_n += 1;
                    }
                }
                None => escaped += 1,
            }
        }
        // a camera against a wall sees half the sphere stop at once: the
        // room's size is in the other hits, when they are enough
        let mean = if far_n as usize >= RAYS / 4 {
            far_sum / far_n as f32
        } else if n > 0 {
            sum / n as f32
        } else {
            RANGE
        };
        Estimate {
            enclosure: enclosed as f32 / upper.max(1) as f32,
            open: escaped as f32 / RAYS as f32,
            mean,
        }
    }

    /// Glide from `self` towards `to` over `dt` seconds (the mean distance
    /// on a log scale: a doorway between a closet and a hall is halfway in
    /// ratio, not in metres).
    fn glide(self, to: Estimate, dt: f32) -> Estimate {
        let k = 1.0 - (-dt.max(0.0) / SMOOTHING_SECS).exp();
        let lerp = |a: f32, b: f32| a + (b - a) * k;
        Estimate {
            enclosure: lerp(self.enclosure, to.enclosure),
            open: lerp(self.open, to.open),
            mean: lerp(self.mean.max(0.1).ln(), to.mean.max(0.1).ln()).exp(),
        }
    }

    /// The reverb's parameters for these surroundings.
    pub fn params(self) -> ReverbParams {
        let path = (PATH_PER_MEAN * self.mean).clamp(ReverbParams::MIN_PATH, ReverbParams::MAX_PATH);
        let absorption = WALL_ABSORPTION + (1.0 - WALL_ABSORPTION) * self.open.clamp(0.0, 1.0);
        let t = ((self.enclosure - ENCLOSURE_FROM) / (ENCLOSURE_FULL - ENCLOSURE_FROM)).clamp(0.0, 1.0);
        let size = ((path.ln() - DAMPING_SMALL.0.ln()) / (DAMPING_LARGE.0.ln() - DAMPING_SMALL.0.ln())).clamp(0.0, 1.0);
        ReverbParams {
            level: t * t * (3.0 - 2.0 * t),
            decay: (SABINE * path / absorption).clamp(ReverbParams::MIN_DECAY, ReverbParams::MAX_DECAY),
            predelay: (self.mean / SOUND_SPEED).clamp(PREDELAY.0, PREDELAY.1),
            damping: DAMPING_SMALL.1 + (DAMPING_LARGE.1 - DAMPING_SMALL.1) * size,
            path,
        }
    }
}

/// What the measures cost (profile).
#[derive(Default)]
struct Stats {
    measures: u32,
    reflectors: u32,
    main: Duration,
    main_max: Duration,
    job: Duration,
    job_max: Duration,
    frames: u32,
    sends: Duration,
    sends_max: Duration,
    since: Option<Instant>,
}

/// The reverb of the surroundings, held by the scene.
pub struct SoundReverb {
    tx: Sender<(Estimate, Duration)>,
    rx: Receiver<(Estimate, Duration)>,
    /// A measure is in the job, sent from there.
    waiting: Option<Vec3>,
    next: Option<Instant>,
    /// The glided estimate, when and where it was last measured.
    smoothed: Option<(Estimate, Instant, Vec3)>,
    /// What the mixer has: off, or these parameters.
    sent: Option<ReverbParams>,
    /// The send factor last given to each voice.
    sends: HashMap<u64, f32>,
    logged: Option<(ReverbParams, Instant)>,
    /// Log lines at info level (AURORA_DEMO_SOUND_REVERB), else debug.
    verbose: bool,
    /// A summary every ten seconds (AURORA_PROFILE).
    profile: bool,
    stats: Stats,
    candidates: Vec<(usize, f32)>,
}

impl Default for SoundReverb {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        SoundReverb {
            tx,
            rx,
            waiting: None,
            next: None,
            smoothed: None,
            sent: None,
            sends: HashMap::new(),
            logged: None,
            verbose: crate::demo::sound_reverb::active(),
            profile: std::env::var_os("AURORA_PROFILE").is_some(),
            stats: Stats::default(),
            candidates: Vec::new(),
        }
    }
}

impl SoundReverb {
    /// A new measure `e`, taken at `at`: glided into the estimate, to the
    /// mixer and the log.
    fn apply(&mut self, engine: &aurora_audio::AudioEngine, e: Estimate, at: Vec3, now: Instant) {
        let smoothed = match self.smoothed {
            Some((prev, when, pos)) if pos.distance(at) < JUMP && now.saturating_duration_since(when) < STALE => {
                prev.glide(e, now.saturating_duration_since(when).as_secs_f32())
            }
            _ => e,
        };
        self.smoothed = Some((smoothed, now, at));
        let p = smoothed.params();
        let moved = |a: f32, b: f32, step: f32| (a - b).abs() > step * a.abs().max(b.abs()).max(1e-3);
        let changed = self.sent.is_none_or(|s| {
            (s.level - p.level).abs() > LEVEL_STEP
                || (p.level == 0.0) != (s.level == 0.0)
                || moved(s.decay, p.decay, PARAM_STEP)
                || moved(s.predelay, p.predelay, PARAM_STEP)
                || moved(s.damping, p.damping, PARAM_STEP)
                || moved(s.path, p.path, PARAM_STEP)
        });
        if changed {
            engine.set_reverb(Some(p));
            self.sent = Some(p);
        }
        let due = self.logged.is_none_or(|(l, when)| {
            now.saturating_duration_since(when) >= LOG_EVERY
                && ((l.level - p.level).abs() >= LOG_LEVEL_STEP || moved(l.decay, p.decay, LOG_DECAY_STEP))
        });
        if due {
            let level = if self.verbose { log::Level::Info } else { log::Level::Debug };
            log::log!(
                level,
                "sound reverb: enclosure {:.2}, open {:.2}, mean {:.1} m (measured {:.2}, {:.2}, {:.1} m): level {:.2}, decay {:.2} s, pre-delay {:.0} ms, damping {:.2}, path {:.1} m",
                smoothed.enclosure,
                smoothed.open,
                smoothed.mean,
                e.enclosure,
                e.open,
                e.mean,
                p.level,
                p.decay,
                p.predelay * 1000.0,
                p.damping,
                p.path
            );
            self.logged = Some((p, now));
        }
    }

    /// Each sound's send, by its distance (every frame: a new sound gets
    /// its own at once).
    fn update_sends(&mut self, engine: &aurora_audio::AudioEngine, audible: &[super::sounds::Audible], listener: Vec3) {
        let started = Instant::now();
        self.sends.retain(|v, _| audible.iter().any(|a| a.voice == *v));
        for a in audible {
            let f = send_factor(a.pos.distance(listener));
            if self
                .sends
                .get(&a.voice)
                .is_none_or(|s| (s - f).abs() >= SEND_STEP || (f == SEND_MAX) != (*s == SEND_MAX))
            {
                engine.world_reverb_send(a.voice, f);
                self.sends.insert(a.voice, f);
            }
        }
        let took = started.elapsed();
        self.stats.frames += 1;
        self.stats.sends += took;
        self.stats.sends_max = self.stats.sends_max.max(took);
    }

    fn summary(&mut self, now: Instant) {
        let s = &mut self.stats;
        let since = *s.since.get_or_insert(now);
        if !self.profile || now.duration_since(since) < SUMMARY_EVERY {
            return;
        }
        let (n, f) = (s.measures.max(1) as f32, s.frames.max(1) as f32);
        log::info!(
            "sound reverb summary: {} measures, {:.0} objects, main {:.0}/{:.0} us, job {:.0}/{:.0} us, sends {:.1}/{:.0} us a frame (mean/max)",
            s.measures,
            s.reflectors as f32 / n,
            s.main.as_secs_f32() * 1e6 / n,
            s.main_max.as_secs_f32() * 1e6,
            s.job.as_secs_f32() * 1e6 / n,
            s.job_max.as_secs_f32() * 1e6,
            s.sends.as_secs_f32() * 1e6 / f,
            s.sends_max.as_secs_f32() * 1e6,
        );
        *s = Stats {
            since: Some(now),
            ..Default::default()
        };
    }
}

impl Scene {
    /// Estimate the surroundings of the listener and hand the reverb's
    /// parameters and each world sound's send to the mixer (every frame,
    /// after `SoundManager::update`; it measures a few times a second).
    /// `enabled`: Preferences › Sound, "Reverb from the surroundings"; off,
    /// the mixer's reverb fades out and stops.
    pub fn update_sound_reverb(
        &mut self,
        world: &World,
        engine: Option<&aurora_audio::AudioEngine>,
        listener: Vec3,
        enabled: bool,
        now: Instant,
    ) {
        let Some(engine) = engine else {
            return;
        };
        let rev = &mut self.sound_reverb;
        while let Ok((estimate, job)) = rev.rx.try_recv() {
            let at = rev.waiting.take().unwrap_or(listener);
            rev.stats.job += job;
            rev.stats.job_max = rev.stats.job_max.max(job);
            if enabled {
                rev.apply(engine, estimate, at, now);
            }
        }
        if !enabled {
            if rev.sent.take().is_some() {
                engine.set_reverb(None);
            }
            rev.sends.clear();
            rev.smoothed = None;
            rev.logged = None;
            return;
        }
        rev.update_sends(engine, self.sounds.audible(), listener);
        if rev.waiting.is_some() || rev.next.is_some_and(|t| now < t) {
            return;
        }
        rev.next = Some(now + PERIOD);
        rev.summary(now);
        let started = Instant::now();
        let mut candidates = std::mem::take(&mut self.sound_reverb.candidates);
        let request = self.reverb_request(world, listener, now, &mut candidates);
        let rev = &mut self.sound_reverb;
        rev.candidates = candidates;
        let main = started.elapsed();
        rev.stats.measures += 1;
        rev.stats.reflectors += request.reflectors.len() as u32;
        rev.stats.main += main;
        rev.stats.main_max = rev.stats.main_max.max(main);
        rev.waiting = Some(listener);
        let tx = rev.tx.clone();
        self.jobs.run(move || {
            let started = Instant::now();
            let estimate = Estimate::of(&measure(&request));
            let _ = tx.send((estimate, started.elapsed()));
        });
    }

    /// Gather what the job needs for a listener at `from`.
    fn reverb_request(&self, world: &World, from: Vec3, now: Instant, candidates: &mut Vec<(usize, f32)>) -> Request {
        self.pick.sphere_candidates(from, RANGE, MIN_RADIUS, candidates);
        if candidates.len() > MAX_SURFACES {
            candidates.select_nth_unstable_by(MAX_SURFACES - 1, |a, b| b.1.total_cmp(&a.1));
            candidates.truncate(MAX_SURFACES);
        }
        let reflectors = candidates
            .iter()
            .filter_map(|&(idx, _)| {
                let g = self.gpu.get(idx)?;
                Some(Reflector {
                    surface: self.sound_surface(world, idx, now)?,
                    center: g.center,
                    radius: g.radius,
                })
            })
            .collect();
        let water = world.main_water_height();
        Request {
            from,
            ground: ground_hits(from, Some(water), |p| world.ground_height(p)),
            reflectors,
        }
    }
}

/// The distance along each ray from `from` to the terrain (`ground`: its
/// height under a point) or the water, if within `RANGE`.
fn ground_hits(from: Vec3, water: Option<f32>, ground: impl Fn(Vec3) -> Option<f32>) -> [Option<f32>; RAYS] {
    let below = ground(from);
    let far = below.is_none_or(|g| from.z - g > RANGE);
    directions().map(|dir| {
        let water_hit = water
            .filter(|_| dir.z.abs() > 1e-3)
            .map(|w| (w - from.z) / dir.z)
            .filter(|t| *t > 0.0 && *t < RANGE);
        let terrain_hit = if far || dir.z > MARCH_TOP {
            None
        } else if dir.z < STEEP {
            // straight to the height under the listener, refined
            let mut t = (from.z - below.unwrap_or(from.z)) / -dir.z;
            for _ in 0..DOWN_STEPS {
                match ground(from + dir * t.min(RANGE)) {
                    Some(g) => t = ((from.z - g) / -dir.z).max(0.0),
                    None => break,
                }
            }
            (t < RANGE).then_some(t)
        } else {
            let step = RANGE / MARCH_STEPS as f32;
            let under = |t: f32| {
                let p = from + dir * t;
                ground(p).is_some_and(|g| g > p.z)
            };
            (1..=MARCH_STEPS).map(|k| k as f32 * step).find(|t| under(*t)).map(|t| {
                // halved down to a quarter of a metre
                let (mut above, mut below) = (t - step, t);
                for _ in 0..BISECTIONS {
                    let mid = (above + below) * 0.5;
                    if under(mid) {
                        below = mid;
                    } else {
                        above = mid;
                    }
                }
                below
            })
        };
        match (water_hit, terrain_hit) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    })
}

/// The distance each ray of `request` travels before it meets something
/// (in the job).
fn measure(request: &Request) -> [Option<f32>; RAYS] {
    let dirs = directions();
    std::array::from_fn(|k| {
        let dir = dirs[k];
        let mut best = request.ground[k].unwrap_or(RANGE);
        for r in &request.reflectors {
            // the bounding sphere first
            let to = r.center - request.from;
            let along = to.dot(dir);
            let off = to.length_squared() - along * along;
            if off > r.radius * r.radius || along + r.radius < 0.0 || along - r.radius > best {
                continue;
            }
            if let Some(t) = nearest(&r.surface, request.from, dir * best) {
                best *= t;
            }
        }
        (best < RANGE).then_some(best)
    })
}

/// Where the segment `from`..`from + span` first meets a drawn surface of
/// `s`, as a share of `span` (0..1). Front faces only, as they are drawn: a
/// camera inside a wall prim sees through it to the room.
fn nearest(s: &Surface, from: Vec3, span: Vec3) -> Option<f32> {
    // (the segment's parameter is the same in both spaces)
    let o = s.inv.transform_point3(from);
    let d = s.inv.transform_vector3(span);
    if !ray_box(o, d, s.geom.min, s.geom.max).is_some_and(|near| near < 1.0) {
        return None;
    }
    let mut best = 1.0f32;
    for (face, f) in s.geom.pick_faces.iter().enumerate() {
        let Some(f) = f.as_ref().filter(|_| face < 64 && s.faces & (1 << face) != 0) else {
            continue;
        };
        if s.hierarchy && f.indices.len() >= MIN_TRIANGLES * 3 {
            best = f
                .accel
                .get_or_build(&f.positions, &f.indices)
                .nearest(&f.positions, &f.indices, o, d, false, best);
            continue;
        }
        for tri in f.indices.as_chunks::<3>().0 {
            let p = |i: u16| f.positions.get(i as usize).map(|p| Vec3::from_array(*p));
            let (Some(p0), Some(p1), Some(p2)) = (p(tri[0]), p(tri[1]), p(tri[2])) else {
                continue;
            };
            if let Some(h) = ray_triangle(o, d, p0, p1, p2, false)
                && h.t > 0.0
                && h.t < best
            {
                best = h.t;
            }
        }
    }
    (best < 1.0).then_some(best)
}

#[cfg(test)]
mod tests {
    use super::super::picking::PickFace;
    use super::*;

    /// A closed box `size` across, centred on the origin, seen from inside
    /// (its six faces turned inwards: sides, then floor and ceiling).
    fn room_box(size: Vec3) -> Reflector {
        box_facing(size, [0, 1, 2, 0, 2, 3])
    }

    /// The box with each quad's triangles in this order (reversed: a solid
    /// prim, its faces turned outwards).
    fn box_facing(size: Vec3, order: [u16; 6]) -> Reflector {
        let quad = |c: [[f32; 3]; 4]| Some(PickFace::new(c.to_vec(), vec![[0.0, 0.0]; 4], order.to_vec()));
        let (l, h) = (-0.5, 0.5);
        let geom = GpuGeom {
            faces: Vec::new(),
            min: Vec3::splat(l),
            max: Vec3::splat(h),
            joint_bounds: Vec::new(),
            pick_faces: vec![
                quad([[l, l, l], [l, h, l], [l, h, h], [l, l, h]]),
                quad([[h, l, l], [h, l, h], [h, h, h], [h, h, l]]),
                quad([[l, l, l], [l, l, h], [h, l, h], [h, l, l]]),
                quad([[l, h, l], [h, h, l], [h, h, h], [l, h, h]]),
                quad([[l, l, l], [h, l, l], [h, h, l], [l, h, l]]),
                quad([[l, l, h], [l, h, h], [h, h, h], [h, l, h]]),
            ],
        };
        Reflector {
            surface: Surface {
                geom: Arc::new(geom),
                inv: Mat4::from_scale(size).inverse(),
                faces: 0b11_1111,
                hierarchy: false,
            },
            center: Vec3::ZERO,
            radius: size.length() * 0.5,
        }
    }

    /// Flat ground `height` below the listener, at the origin.
    fn on_ground(height: f32) -> [Option<f32>; RAYS] {
        ground_hits(Vec3::ZERO, None, |_| Some(-height))
    }

    fn estimate(ground: [Option<f32>; RAYS], reflectors: Vec<Reflector>) -> Estimate {
        Estimate::of(&measure(&Request {
            from: Vec3::ZERO,
            ground,
            reflectors,
        }))
    }

    #[test]
    fn the_rays_cover_the_sphere_evenly() {
        let dirs = directions();
        assert!(dirs.iter().all(|d| (d.length() - 1.0).abs() < 1e-5));
        let mean = dirs.iter().copied().sum::<Vec3>() / RAYS as f32;
        assert!(mean.length() < 0.05, "balanced: {mean}");
        let upper = dirs.iter().filter(|d| d.z >= HORIZON).count();
        assert!((24..=30).contains(&upper), "{upper} at or above the horizon");
    }

    #[test]
    fn the_open_air_is_almost_dry() {
        // a meadow, the camera 1.7 m above the grass
        let e = estimate(on_ground(1.7), Vec::new());
        assert!(e.enclosure < 0.1, "{e:?}");
        assert!((0.4..0.65).contains(&e.open), "{e:?}");
        assert_eq!(e.params().level, 0.0);
        // high in the sky, nothing at all
        let e = estimate([None; RAYS], Vec::new());
        assert_eq!((e.enclosure, e.open, e.mean), (0.0, 1.0, RANGE));
        assert_eq!(e.params().level, 0.0);
    }

    #[test]
    fn a_small_room_rings_briefly_and_a_hall_long() {
        // 5 x 5 x 3 m, the camera 1.7 m above its floor
        let small = estimate(on_ground(1.7), vec![room_box(Vec3::new(5.0, 5.0, 3.4))]);
        // 30 x 22 x 12 m
        let hall = estimate(on_ground(2.0), vec![room_box(Vec3::new(30.0, 22.0, 12.0))]);
        assert_eq!((small.enclosure, small.open), (1.0, 0.0), "{small:?}");
        assert_eq!((hall.enclosure, hall.open), (1.0, 0.0), "{hall:?}");
        let (s, h) = (small.params(), hall.params());
        assert_eq!((s.level, h.level), (1.0, 1.0));
        assert!((0.3..0.7).contains(&s.decay), "small room {s:?}");
        assert!((1.5..3.0).contains(&h.decay), "hall {h:?}");
        assert!(s.predelay < h.predelay && s.path < h.path && s.damping > h.damping);
        // a courtyard: walls, no roof
        let mut court = room_box(Vec3::new(14.0, 14.0, 8.0));
        court.surface.faces = 0b1111;
        let c = estimate(on_ground(1.7), vec![court]);
        let p = c.params();
        assert!(
            c.enclosure > 0.2 && c.enclosure < 0.9 && p.level > 0.0 && p.level < 1.0,
            "{c:?} {p:?}"
        );
        assert!(p.decay < s.decay * 2.0, "open above: {p:?}");
    }

    #[test]
    fn a_camera_against_a_wall_still_sees_the_room() {
        let centre = estimate(on_ground(1.7), vec![room_box(Vec3::new(6.0, 6.0, 3.4))]);
        // 10 cm from the back wall
        let mut against = room_box(Vec3::new(6.0, 6.0, 3.4));
        against.center = Vec3::new(2.9, 0.0, 0.0);
        against.surface.inv = Mat4::from_scale_rotation_translation(Vec3::new(6.0, 6.0, 3.4), Quat::IDENTITY, against.center).inverse();
        let wall = estimate(on_ground(1.7), vec![against]);
        assert_eq!(wall.enclosure, 1.0);
        assert!((wall.mean / centre.mean - 1.0).abs() < 0.5, "{wall:?} {centre:?}");
        let (a, b) = (wall.params(), centre.params());
        assert!((a.decay / b.decay - 1.0).abs() < 0.5, "{a:?} {b:?}");
    }

    #[test]
    fn back_faces_let_the_rays_through() {
        // a solid wall prim (its faces turned outwards) with the camera
        // inside it: the rays leave through its back faces
        let solid = box_facing(Vec3::new(0.3, 4.0, 3.0), [0, 2, 1, 0, 3, 2]);
        let e = estimate([None; RAYS], vec![solid]);
        assert_eq!(e.open, 1.0, "{e:?}");
    }

    #[test]
    fn the_nearest_surface_stops_a_ray() {
        // a room inside a hall: the room's walls are what the rays meet
        let small = estimate(on_ground(1.7), vec![room_box(Vec3::new(5.0, 5.0, 3.4))]);
        let both = estimate(
            on_ground(1.7),
            vec![room_box(Vec3::new(30.0, 22.0, 12.0)), room_box(Vec3::new(5.0, 5.0, 3.4))],
        );
        assert!((both.mean - small.mean).abs() < 1e-4, "{both:?} {small:?}");
        // an invisible face lets the rays through
        let mut glassless = room_box(Vec3::new(5.0, 5.0, 3.4));
        glassless.surface.faces = 0b01_1111;
        let e = estimate(on_ground(1.7), vec![glassless]);
        assert!(e.enclosure < 1.0 && e.open > 0.0, "{e:?}");
    }

    #[test]
    fn ground_and_water_are_found_along_the_rays() {
        let dirs = directions();
        let flat = on_ground(2.0);
        for (d, hit) in dirs.iter().zip(flat) {
            match hit {
                Some(t) => assert!(d.z < 0.0 && (t * -d.z - 2.0).abs() < 0.05, "{d} {t}"),
                None => assert!(d.z > -2.0 / RANGE - 0.05, "{d} should meet the ground"),
            }
        }
        // a hill rising 20 m away
        let hill = ground_hits(Vec3::ZERO, None, |p| Some(if p.x > 20.0 { 10.0 } else { -2.0 }));
        let level = dirs
            .iter()
            .position(|d| d.z.abs() < 0.15 && d.x > 0.9)
            .expect("a ray east near the horizon");
        assert!(hill[level].is_some_and(|t| t > 18.0 && t < 26.0), "{:?}", hill[level]);
        // water 1 m below, the terrain deep under it
        let sea = ground_hits(Vec3::ZERO, Some(-1.0), |_| Some(-30.0));
        let down = dirs.iter().position(|d| d.z < -0.9).unwrap();
        assert!(sea[down].is_some_and(|t| (t * -dirs[down].z - 1.0).abs() < 1e-3));
        // high above the terrain: nothing below in range
        assert!(
            ground_hits(Vec3::new(0.0, 0.0, 500.0), None, |_| Some(20.0))
                .iter()
                .all(Option::is_none)
        );
    }

    #[test]
    fn the_estimate_glides_through_a_door() {
        let room = Estimate {
            enclosure: 1.0,
            open: 0.0,
            mean: 2.0,
        };
        let outside = Estimate {
            enclosure: 0.0,
            open: 0.5,
            mean: 8.0,
        };
        let mut e = room;
        let mut levels = Vec::new();
        for _ in 0..20 {
            e = e.glide(outside, 0.3);
            levels.push(e.params().level);
        }
        assert!(levels.windows(2).all(|w| w[1] <= w[0]), "falls steadily: {levels:?}");
        assert!(levels[0] > 0.7, "not at once: {levels:?}");
        assert!(*levels.last().unwrap() < 0.01, "there in a few seconds: {levels:?}");
        // halfway in ratio for the distance
        let half = room.glide(outside, SMOOTHING_SECS * std::f32::consts::LN_2);
        assert!((half.mean - 4.0).abs() < 0.05, "{half:?}");
    }

    #[test]
    fn farther_sounds_send_more() {
        assert_eq!(send_factor(0.2), 1.0);
        assert_eq!(send_factor(1.0), 1.0);
        assert!((send_factor(2.25) - 1.5).abs() < 1e-6);
        assert_eq!(send_factor(4.0), SEND_MAX);
        assert_eq!(send_factor(100.0), SEND_MAX);
        assert_eq!(send_factor(f32::NAN), SEND_MAX);
    }
}
