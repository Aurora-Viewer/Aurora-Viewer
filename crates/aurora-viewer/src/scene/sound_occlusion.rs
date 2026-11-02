//! World sounds muffled by what stands between them and the listener: a
//! sound behind a wall, a sound played by someone shut in a box. Aurora's
//! own (Preferences › Sound, "Muffle sounds behind walls", on): the Legacy
//! Viewer plays every world sound unobstructed, which the switch turned off
//! keeps.
//!
//! About ten times a second, for each world sound playing at a place
//! (`SoundManager::audible`), three rays go from the listener (the camera,
//! as for the panning) to the source and to two points 30 cm to either side
//! of it. What a ray crosses is counted in layers: an object it crosses once
//! or twice (a wall, the box the source sits in) is one layer, three or four
//! times two, and so on; the terrain (a hill) is one more. A ray's occlusion
//! is its layers over two, at most 1; the sound's is the mean of its rays:
//! 0.5 behind one wall, 1 behind two, in between past an edge. The mixer
//! turns it into a low-pass and a gain cut (`aurora_audio`, `WorldVoice`).
//!
//! What blocks: the faces the scene draws (opaque, masked or blended; a face
//! at alpha 0 gets no draw record) of objects that are neither avatars, nor
//! worn, nor rigged, nor trees or grass, nor of the linkset playing the
//! sound.
//!
//! The main thread only gathers: one pass over the pick index's spheres for
//! the objects near the rays (`PickIndex::segment_candidates`), the
//! transform and the shared geometry (`Arc`) of those, and the terrain under
//! the centre rays. The triangles are tested by a background job, the large
//! faces of meshes through their hierarchies (pick_accel.rs); its answer
//! comes back a frame or more later and goes to the mixer
//! (`AudioEngine::world_occlusion`), which glides to it.

use super::pick_accel::MIN_TRIANGLES;
use super::picking::ray_box;
use super::sounds::Audible;
use super::*;
use crate::media::pick::ray_triangle;
use crossbeam_channel::{Receiver, Sender};
use std::time::Duration;

/// Time between two measures.
const PERIOD: Duration = Duration::from_millis(100);
/// Rays per sound, and how far beside the source the side ones end (m).
const RAYS: usize = 3;
const SIDE: f32 = 0.3;
/// Length of each end of a ray where nothing counts (m): the surface the
/// source stands on, the camera grazing a wall.
const END_GAP: f32 = 0.15;
/// Hits closer than this along a ray are one crossing (m).
const SAME_PLACE: f32 = 0.002;
/// Layers at which a ray is fully occluded.
const FULL_LAYERS: u32 = 2;
/// Spacing and margin of the terrain samples under a ray (m), and the most
/// samples taken.
const TERRAIN_STEP: f32 = 2.0;
const TERRAIN_MARGIN: f32 = 0.3;
const TERRAIN_SAMPLES: usize = 128;
/// Sounds measured at most (one bit each in the candidates' masks).
const MAX_SOUNDS: usize = 64;
/// A sound an object starts again within this long starts at the
/// occlusion it last had.
const EMITTER_MEMORY: Duration = Duration::from_secs(10);
/// A new amount goes to the mixer when it moved this much.
const SEND_STEP: f32 = 0.02;
/// The log line of a sound: when its amount moved this much, at most this
/// often.
const LOG_STEP: f32 = 0.05;
const LOG_EVERY: Duration = Duration::from_millis(500);
/// The profile's summary line (AURORA_PROFILE).
const SUMMARY_EVERY: Duration = Duration::from_secs(10);

/// `AURORA_SOUND_OCCLUSION=0|1`: the setting overridden for this launch
/// (A/B measures), never saved.
pub fn forced() -> Option<bool> {
    static FORCED: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| match std::env::var("AURORA_SOUND_OCCLUSION").as_deref() {
        Ok("0") => Some(false),
        Ok("1") => Some(true),
        _ => None,
    })
}

/// A drawn object that stops or reflects sound, in the space of its
/// geometry (the occlusion's blockers, the reverb's reflectors).
pub(super) struct Surface {
    pub(super) geom: Arc<GpuGeom>,
    /// Render space to the space of its geometry.
    pub(super) inv: Mat4,
    /// The faces drawn (texture entry numbers).
    pub(super) faces: u64,
    /// Its large faces may go through their hierarchy: not a prim, whose
    /// triangles a flexible one replaces at each frame.
    pub(super) hierarchy: bool,
}

/// An object that may block some of the rays.
struct Blocker {
    surface: Surface,
    /// The sounds whose rays come near it (bits of `Request::sounds`).
    sounds: u64,
}

struct SoundRays {
    voice: u64,
    emitter: Uuid,
    from: Vec3,
    to: [Vec3; RAYS],
    /// The terrain rises above the centre ray.
    terrain: bool,
}

/// What the job needs, gathered by the main thread.
struct Request {
    sounds: Vec<SoundRays>,
    blockers: Vec<Blocker>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Answer {
    voice: u64,
    emitter: Uuid,
    /// 0 clear .. 1.
    amount: f32,
    /// Mean layers of the rays (log).
    layers: f32,
}

/// What a measure cost (profile).
#[derive(Default)]
struct Stats {
    measures: u32,
    sounds: u32,
    blockers: u32,
    main: Duration,
    main_max: Duration,
    job: Duration,
    job_max: Duration,
    since: Option<Instant>,
}

/// The occlusion of the playing sounds, held by the scene.
pub struct SoundOcclusion {
    tx: Sender<(Vec<Answer>, Duration)>,
    rx: Receiver<(Vec<Answer>, Duration)>,
    /// A measure is in the job.
    waiting: bool,
    next: Option<Instant>,
    /// The amount last sent to the mixer, per voice.
    sent: HashMap<u64, f32>,
    /// The last amount of each emitting object, and when.
    by_emitter: HashMap<Uuid, (f32, Instant)>,
    /// The amount last logged per voice, and when.
    logged: HashMap<u64, (f32, Instant)>,
    /// Log lines at info level (AURORA_DEMO_SOUND_OCCLUSION), else debug.
    verbose: bool,
    /// A summary every ten seconds (AURORA_PROFILE).
    profile: bool,
    stats: Stats,
    candidates: Vec<(usize, u64)>,
}

impl Default for SoundOcclusion {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        SoundOcclusion {
            tx,
            rx,
            waiting: false,
            next: None,
            sent: HashMap::new(),
            by_emitter: HashMap::new(),
            logged: HashMap::new(),
            verbose: crate::demo::sound_occlusion::active(),
            profile: std::env::var_os("AURORA_PROFILE").is_some(),
            stats: Stats::default(),
            candidates: Vec::new(),
        }
    }
}

impl SoundOcclusion {
    /// The answers of the job: to the mixer, and to the log.
    fn apply(&mut self, engine: &aurora_audio::AudioEngine, audible: &[Audible], answers: Vec<Answer>, now: Instant) {
        for a in answers {
            // (ended meanwhile)
            if !audible.iter().any(|s| s.voice == a.voice) {
                continue;
            }
            if !a.emitter.is_nil() {
                self.by_emitter.insert(a.emitter, (a.amount, now));
            }
            let sent = self.sent.get(&a.voice).copied();
            let prev = sent.unwrap_or(0.0);
            let reached_end = (a.amount == 0.0 || a.amount == 1.0) && a.amount != prev;
            if (a.amount - prev).abs() >= SEND_STEP || reached_end {
                engine.world_occlusion(a.voice, a.amount);
                self.sent.insert(a.voice, a.amount);
            } else if sent.is_none() {
                self.sent.insert(a.voice, prev);
            }
            let due = self
                .logged
                .get(&a.voice)
                .is_none_or(|(logged, at)| (a.amount - logged).abs() >= LOG_STEP && now.saturating_duration_since(*at) >= LOG_EVERY);
            if due {
                let level = if self.verbose { log::Level::Info } else { log::Level::Debug };
                log::log!(
                    level,
                    "sound occlusion: voice {} {:.2} ({:.1} layers) from {}",
                    a.voice,
                    a.amount,
                    a.layers,
                    a.emitter
                );
                self.logged.insert(a.voice, (a.amount, now));
            }
        }
    }

    fn summary(&mut self, now: Instant) {
        let s = &mut self.stats;
        let since = *s.since.get_or_insert(now);
        if !self.profile || now.duration_since(since) < SUMMARY_EVERY {
            return;
        }
        let n = s.measures.max(1);
        log::info!(
            "sound occlusion summary: {} measures, {:.1} sounds, {:.1} objects, main {:.0}/{:.0} us, job {:.0}/{:.0} us (mean/max)",
            s.measures,
            s.sounds as f32 / n as f32,
            s.blockers as f32 / n as f32,
            s.main.as_secs_f32() * 1e6 / n as f32,
            s.main_max.as_secs_f32() * 1e6,
            s.job.as_secs_f32() * 1e6 / n as f32,
            s.job_max.as_secs_f32() * 1e6,
        );
        *s = Stats {
            since: Some(now),
            ..Default::default()
        };
    }
}

impl Scene {
    /// Measure the occlusion of the playing world sounds and hand it to the
    /// mixer (every frame, after `SoundManager::update`; it measures ten
    /// times a second). `enabled`: Preferences › Sound, "Muffle sounds
    /// behind walls"; off, every sound is clear again.
    pub fn update_sound_occlusion(
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
        let occ = &mut self.sound_occlusion;
        let audible = self.sounds.audible();
        while let Ok((answers, job)) = occ.rx.try_recv() {
            occ.waiting = false;
            occ.stats.job += job;
            occ.stats.job_max = occ.stats.job_max.max(job);
            if enabled {
                occ.apply(engine, audible, answers, now);
            }
        }
        if !enabled {
            for (voice, amount) in occ.sent.drain() {
                if amount > 0.0 {
                    engine.world_occlusion(voice, 0.0);
                }
            }
            occ.by_emitter.clear();
            occ.logged.clear();
            return;
        }
        // a sound an object starts again: muffled from its first sample;
        // a sound never measured: measured at once
        for a in audible {
            if occ.sent.contains_key(&a.voice) {
                continue;
            }
            match occ.by_emitter.get(&a.emitter) {
                Some(&(amount, _)) => {
                    if amount > 0.0 {
                        engine.world_occlusion(a.voice, amount);
                    }
                    occ.sent.insert(a.voice, amount);
                }
                None => occ.next = None,
            }
        }
        if occ.waiting || occ.next.is_some_and(|t| now < t) {
            return;
        }
        occ.next = Some(now + PERIOD);
        occ.sent.retain(|v, _| audible.iter().any(|a| a.voice == *v));
        occ.logged.retain(|v, _| audible.iter().any(|a| a.voice == *v));
        occ.by_emitter.retain(|_, (_, at)| now.duration_since(*at) < EMITTER_MEMORY);
        occ.summary(now);
        if audible.is_empty() {
            return;
        }
        let started = Instant::now();
        let audible: Vec<Audible> = audible.iter().take(MAX_SOUNDS).copied().collect();
        let mut candidates = std::mem::take(&mut self.sound_occlusion.candidates);
        let request = self.occlusion_request(world, listener, &audible, now, &mut candidates);
        let occ = &mut self.sound_occlusion;
        occ.candidates = candidates;
        let main = started.elapsed();
        occ.stats.measures += 1;
        occ.stats.sounds += request.sounds.len() as u32;
        occ.stats.blockers += request.blockers.len() as u32;
        occ.stats.main += main;
        occ.stats.main_max = occ.stats.main_max.max(main);
        occ.waiting = true;
        let tx = occ.tx.clone();
        self.jobs.run(move || {
            let started = Instant::now();
            let answers = measure(&request);
            let _ = tx.send((answers, started.elapsed()));
        });
    }

    /// Gather what the job needs for `audible` heard at `listener`.
    fn occlusion_request(
        &self,
        world: &World,
        listener: Vec3,
        audible: &[Audible],
        now: Instant,
        candidates: &mut Vec<(usize, u64)>,
    ) -> Request {
        let sounds: Vec<SoundRays> = audible
            .iter()
            .map(|a| {
                let side = (a.pos - listener).cross(Vec3::Z).normalize_or(Vec3::X) * SIDE;
                SoundRays {
                    voice: a.voice,
                    emitter: a.emitter,
                    from: listener,
                    to: [a.pos, a.pos + side, a.pos - side],
                    terrain: terrain_blocks(listener, a.pos, |p| world.ground_height(p)),
                }
            })
            .collect();
        let segments: Vec<(Vec3, Vec3)> = sounds.iter().map(|s| (s.from, s.to[0])).collect();
        // (the side rays stay within SIDE of the centre one)
        self.pick.segment_candidates(&segments, SIDE + 0.05, candidates);
        let emitters: Vec<Option<usize>> = audible
            .iter()
            .map(|a| world.objects.index_of_uuid(&a.emitter).map(|i| linkset_root(world, i)))
            .collect();
        let mut blockers = Vec::new();
        for &(idx, mut bits) in candidates.iter() {
            let root = linkset_root(world, idx);
            for (k, emitter) in emitters.iter().enumerate() {
                if *emitter == Some(root) {
                    bits &= !(1 << k);
                }
            }
            if bits == 0 {
                continue;
            }
            if let Some(surface) = self.sound_surface(world, idx, now) {
                blockers.push(Blocker { surface, sounds: bits });
            }
        }
        Request { sounds, blockers }
    }

    /// Object `idx` as a surface that stops or reflects sound: its drawn
    /// faces, transform and shared geometry. `None` for what never does
    /// (HUDs, avatars, what they wear, rigged meshes, hidden objects, trees
    /// and grass, every face at alpha 0) and while its geometry is not
    /// ready.
    pub(super) fn sound_surface(&self, world: &World, idx: usize, now: Instant) -> Option<Surface> {
        let (g, o) = (self.gpu.get(idx)?, world.objects.get(idx)?);
        if g.hud
            || g.is_avatar
            || g.rigged
            || g.owner_avatar.is_some()
            || g.parcel_hidden
            || g.photo_hidden
            || super::plants::plant_face(o).is_some()
        {
            return None;
        }
        let faces = g
            .faces
            .iter()
            .filter(|f| f.pass != Pass::Hidden && f.te_index < 64)
            .fold(0u64, |m, f| m | (1 << f.te_index));
        if faces == 0 {
            return None;
        }
        let key = g.geom?;
        let geom = self.geom_ready(&key)?;
        let (pos, rot, _) = Self::object_transform(world, idx, now, 0)?;
        let model = self.drawn_model(g, o, world.visual_scale_at(idx, now), Some(key), pos, rot);
        Some(Surface {
            geom,
            inv: model.inverse(),
            faces,
            hierarchy: !matches!(key, GeomKey::Prim { .. }),
        })
    }
}

/// The root of the linkset of object `idx`.
fn linkset_root(world: &World, mut idx: usize) -> usize {
    for _ in 0..64 {
        let Some(o) = world.objects.get(idx) else { break };
        if o.parent_id == 0 {
            break;
        }
        match world.objects.parent_of(o) {
            Some(parent) => idx = parent,
            None => break,
        }
    }
    idx
}

/// The terrain (`ground`: its height under a point) rises above the
/// segment `from`..`to`, away from its ends.
fn terrain_blocks(from: Vec3, to: Vec3, ground: impl Fn(Vec3) -> Option<f32>) -> bool {
    let len = from.distance(to);
    let edge = 1.5;
    if len <= 2.0 * edge {
        return false;
    }
    let steps = (((len - 2.0 * edge) / TERRAIN_STEP).ceil() as usize).clamp(1, TERRAIN_SAMPLES);
    (0..=steps).any(|i| {
        let p = from.lerp(to, (edge + (len - 2.0 * edge) * i as f32 / steps as f32) / len);
        ground(p).is_some_and(|z| z > p.z + TERRAIN_MARGIN)
    })
}

/// The occlusion of every sound of `request` (in the job).
fn measure(request: &Request) -> Vec<Answer> {
    request
        .sounds
        .iter()
        .enumerate()
        .map(|(k, s)| {
            let bit = 1u64 << k;
            let layers: u32 =
                s.to.iter()
                    .map(|to| ray_layers(&request.blockers, bit, s.from, *to, s.terrain))
                    .sum();
            let layers = layers as f32 / RAYS as f32;
            Answer {
                voice: s.voice,
                emitter: s.emitter,
                amount: (layers / FULL_LAYERS as f32).min(1.0),
                layers,
            }
        })
        .collect()
}

/// The layers between `from` and `to`, at most `FULL_LAYERS`.
fn ray_layers(blockers: &[Blocker], bit: u64, from: Vec3, to: Vec3, terrain: bool) -> u32 {
    let mut layers = u32::from(terrain);
    for b in blockers.iter().filter(|b| b.sounds & bit != 0) {
        if layers >= FULL_LAYERS {
            break;
        }
        layers += crossings(&b.surface, from, to).div_ceil(2);
    }
    layers.min(FULL_LAYERS)
}

/// How many times the segment crosses drawn surfaces of `b`, its ends left
/// out. Hits at the same place count once: a ray through the edge two
/// triangles share, or through a corner, meets both.
fn crossings(b: &Surface, from: Vec3, to: Vec3) -> u32 {
    let len = from.distance(to);
    if len <= 2.0 * END_GAP {
        return 0;
    }
    let range = (END_GAP / len, 1.0 - END_GAP / len);
    // (the segment's parameter is the same in both spaces: the direction
    // is not normalized)
    let o = b.inv.transform_point3(from);
    let d = b.inv.transform_vector3(to - from);
    if !ray_box(o, d, b.geom.min, b.geom.max).is_some_and(|near| near < range.1) {
        return 0;
    }
    let mut hits = Vec::new();
    for (face, f) in b.geom.pick_faces.iter().enumerate() {
        let Some(f) = f.as_ref().filter(|_| face < 64 && b.faces & (1 << face) != 0) else {
            continue;
        };
        if b.hierarchy && f.indices.len() >= MIN_TRIANGLES * 3 {
            f.accel
                .get_or_build(&f.positions, &f.indices)
                .crossings(&f.positions, &f.indices, o, d, range, &mut hits);
            continue;
        }
        for tri in f.indices.as_chunks::<3>().0 {
            let p = |i: u16| f.positions.get(i as usize).map(|p| Vec3::from_array(*p));
            let (Some(p0), Some(p1), Some(p2)) = (p(tri[0]), p(tri[1]), p(tri[2])) else {
                continue;
            };
            if let Some(h) = ray_triangle(o, d, p0, p1, p2, true)
                && h.t > range.0
                && h.t < range.1
            {
                hits.push(h.t);
            }
        }
    }
    distinct(&mut hits, SAME_PLACE / len)
}

/// The values of `hits` farther than `eps` from the one before, sorted.
fn distinct(hits: &mut [f32], eps: f32) -> u32 {
    hits.sort_by(f32::total_cmp);
    let mut count = 0;
    let mut last = f32::NEG_INFINITY;
    for &t in hits.iter() {
        if t - last > eps {
            count += 1;
        }
        last = t;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::super::picking::{PickFace, tests::scene_with_two_objects};
    use super::*;

    /// A sound at `at` played by `emitter`, heard from the origin of the
    /// two objects' row (x = 0): its amount.
    fn amount(world: &World, scene: &Scene, at: Vec3, emitter: Uuid) -> f32 {
        let audible = [Audible {
            voice: 1,
            pos: at,
            emitter,
        }];
        let mut candidates = Vec::new();
        let request = scene.occlusion_request(world, Vec3::new(0.0, 0.0, 50.0), &audible, Instant::now(), &mut candidates);
        measure(&request)[0].amount
    }

    /// The two objects of `scene_with_two_objects` (a square at x = 2.5
    /// and one at x = 6.5, facing the origin) in the pick index.
    fn indexed() -> (World, Scene, usize, usize) {
        let (world, mut scene, front, back) = scene_with_two_objects();
        for idx in [front, back] {
            let o = world.objects.get(idx).unwrap();
            scene.gpu[idx].center = o.position;
            scene.gpu[idx].radius = 1.0;
            scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
        }
        (world, scene, front, back)
    }

    #[test]
    fn walls_between_count_in_layers() {
        let (world, scene, _, _) = indexed();
        let nobody = Uuid::from_u128(0x5);
        // nothing between
        assert_eq!(amount(&world, &scene, Vec3::new(2.0, 0.0, 50.0), nobody), 0.0);
        // one wall, then two
        assert_eq!(amount(&world, &scene, Vec3::new(5.0, 0.0, 50.0), nobody), 0.5);
        assert_eq!(amount(&world, &scene, Vec3::new(9.0, 0.0, 50.0), nobody), 1.0);
        // past the edge of the 1 m square: the side rays get through
        let edge = amount(&world, &scene, Vec3::new(5.0, 0.9, 50.0), nobody);
        assert!(edge > 0.0 && edge < 0.5, "{edge}");
    }

    #[test]
    fn the_emitting_object_and_invisible_faces_do_not_block() {
        let (world, mut scene, front, back) = indexed();
        let back_id = world.objects.get(back).unwrap().full_id;
        // played by the far square itself: only the near one counts
        assert_eq!(amount(&world, &scene, Vec3::new(9.0, 0.0, 50.0), back_id), 0.5);
        // a face at alpha 0 has no draw record: it lets the sound through
        scene.gpu[front].faces.clear();
        assert_eq!(amount(&world, &scene, Vec3::new(9.0, 0.0, 50.0), back_id), 0.0);
        // avatars and what they wear never block
        let (world, mut scene, front, _) = indexed();
        scene.gpu[front].owner_avatar = Some(0);
        assert_eq!(amount(&world, &scene, Vec3::new(5.0, 0.0, 50.0), Uuid::nil()), 0.0);
    }

    #[test]
    fn a_source_shut_in_a_box_is_muffled() {
        let (mut world, mut scene, front, back) = indexed();
        // the far object becomes a closed 2 m cube around (7, 0, 50)
        let quad = |corners: [[f32; 3]; 4]| Some(PickFace::new(corners.to_vec(), vec![[0.0, 0.0]; 4], vec![0, 1, 2, 0, 2, 3]));
        let (l, h) = (-0.5, 0.5);
        let cube = GpuGeom {
            faces: Vec::new(),
            min: Vec3::splat(l),
            max: Vec3::splat(h),
            joint_bounds: Vec::new(),
            pick_faces: vec![
                quad([[l, l, l], [l, h, l], [l, h, h], [l, l, h]]),
                quad([[h, l, l], [h, h, l], [h, h, h], [h, l, h]]),
                quad([[l, l, l], [h, l, l], [h, l, h], [l, l, h]]),
                quad([[l, h, l], [h, h, l], [h, h, h], [l, h, h]]),
                quad([[l, l, l], [h, l, l], [h, h, l], [l, h, l]]),
                quad([[l, l, h], [h, l, h], [h, h, h], [l, h, h]]),
            ],
        };
        let key = GeomKey::Prim { hash: 1, lod: 0 };
        scene.geoms.insert(
            key,
            GeomEntry {
                refs: 1,
                zero_since: 0,
                state: GeomState::Ready(Arc::new(cube)),
            },
        );
        let face = scene.gpu[back].faces[0];
        scene.gpu[back].faces = (0..6).map(|te| FaceDraw { te_index: te, ..face }).collect();
        scene.gpu[back].geom = Some(key);
        world.objects.get_mut(back).unwrap().scale = Vec3::splat(2.0);
        // out of the way of the rays: only the box counts
        world.objects.get_mut(front).unwrap().position.y = 20.0;
        scene.gpu[front].center.y = 20.0;
        scene.pick.set(front, super::super::hover::PickEntry::of(&scene.gpu[front]));
        let inside = Vec3::new(7.0, 0.0, 50.0);
        // its sides seen through: one layer for the rays that cross it twice
        assert_eq!(amount(&world, &scene, Vec3::new(10.0, 0.0, 50.0), Uuid::nil()), 0.5);
        // a sound inside: one crossing, one layer
        assert_eq!(amount(&world, &scene, inside, Uuid::nil()), 0.5);
        // the box's own sound is not muffled by itself
        let box_id = world.objects.get(back).unwrap().full_id;
        assert_eq!(amount(&world, &scene, inside, box_id), 0.0);
    }

    #[test]
    fn terrain_between_is_one_layer() {
        // a hill 10 m high between two points on flat ground at 20 m
        let hill = |p: Vec3| Some(20.0 + 10.0 * (-(p.x - 50.0).powi(2) / 50.0).exp());
        assert!(terrain_blocks(Vec3::new(0.0, 0.0, 22.0), Vec3::new(100.0, 0.0, 22.0), hill));
        assert!(!terrain_blocks(Vec3::new(0.0, 0.0, 40.0), Vec3::new(100.0, 0.0, 40.0), hill));
        // the source and the listener on the ground, the ends left out
        let flat = |_: Vec3| Some(20.0);
        assert!(!terrain_blocks(Vec3::new(0.0, 0.0, 20.05), Vec3::new(30.0, 0.0, 19.9), flat));
        assert!(!terrain_blocks(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), flat));
        // off the regions: nothing known, nothing blocks
        assert!(!terrain_blocks(Vec3::ZERO, Vec3::new(100.0, 0.0, 0.0), |_| None));
        let request = Request {
            sounds: vec![SoundRays {
                voice: 7,
                emitter: Uuid::nil(),
                from: Vec3::ZERO,
                to: [Vec3::X * 50.0; RAYS],
                terrain: true,
            }],
            blockers: Vec::new(),
        };
        assert_eq!(measure(&request)[0].amount, 0.5);
    }
}
