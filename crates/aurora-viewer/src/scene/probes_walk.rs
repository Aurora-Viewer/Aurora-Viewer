//! A walk through a forest, to measure what the ambient light does when the
//! probe cells change: the probe manager as it runs (scan, slots, captures at
//! one face a frame), the camera moved along a path at a frame rate. No GPU:
//! what is measured is the weights the shaders would use.
//!
//! The ambient light of a point is an average of the probes that cover it, the
//! default (sky) probe among them, weighted as in `sample_probes_pair`
//! (reflectionProbeF.glsl, sphereWeight). The share of the local probes in
//! that average is what keeps a forest dark: a sudden fall of it is a flash to
//! the bright default probe.

use super::*;
use std::sync::Arc;

fn face() -> crate::scene::FaceDraw {
    crate::scene::FaceDraw {
        te_index: 0,
        record: 0,
        cmd: aurora_render::DrawCmd {
            index_count: 6,
            first_index: 0,
            base_vertex: 0,
            record: 0,
            bounds: [0.0; 4],
        },
        base_slot: 0,
        tex_id: uuid::Uuid::nil(),
        aux_tex: [uuid::Uuid::nil(); 3],
        bump_height_source: uuid::Uuid::nil(),
        te_alpha: 1.0,
        pbr_alpha: None,
        legacy_alpha: None,
        two_sided: false,
        repeats: 1.0,
        glow: false,
        pass: crate::scene::Pass::Opaque,
        glow_pool: false,
        rez_glow: false,
        system: None,
        first_person_hidden: false,
    }
}
/// A number in 0..1 from an index and a stream.
fn hash(i: u32, stream: u32) -> f32 {
    let mut x = i.wrapping_mul(2_654_435_761).wrapping_add(stream.wrapping_mul(40_503));
    x ^= x >> 15;
    x = x.wrapping_mul(2_246_822_519);
    x ^= x >> 13;
    x = x.wrapping_mul(3_266_489_917);
    x ^= x >> 16;
    x as f32 / u32::MAX as f32
}

const OBJECTS: u32 = 6000;
const SPAN: f32 = 600.0;
const EYE_HEIGHT: f32 = 30.0;
const RANGE: f32 = 256.0;
const WALK_SPEED: f32 = 3.2;
const WALK_SECONDS: f32 = 60.0;
const TICK: f32 = 0.1;
/// A fall of the local share in one tick that reads as a flash.
const FLASH: f32 = 0.2;

fn forest() -> (Scene, World, Vec<Vec3>) {
    let world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let mut scene = Scene::new(std::path::PathBuf::new(), world.avatar_lib.clone());
    let points: Vec<Vec3> = (0..OBJECTS)
        .map(|i| Vec3::new((hash(i, 1) - 0.5) * SPAN, (hash(i, 2) - 0.5) * SPAN, 20.0 + hash(i, 3) * 40.0))
        .collect();
    scene.gpu.clear();
    for p in &points {
        scene.gpu.push(ObjGpu {
            faces: vec![face()],
            center: *p,
            radius: 2.0,
            ..Default::default()
        });
    }
    let samples = points.iter().step_by(15).copied().collect();
    (scene, world, samples)
}

/// The share of the local probes in the ambient light at `p`.
fn local_share(list: &[ProbeGpu], p: Vec3) -> f32 {
    let (mut sky, mut local) = (0.0, 0.0);
    for (i, g) in list.iter().enumerate() {
        let c = Vec3::new(g.center_radius[0], g.center_radius[1], g.center_radius[2]);
        let r = g.center_radius[3];
        let d = p.distance(c).max(0.001);
        if i != 0 && d > r {
            continue;
        }
        let r1 = r * 0.5;
        let w = g.params[2] / d * (1.0 - (d - r1).max(0.0) / (r - r1).max(0.001));
        if i == 0 {
            sky += w;
        } else {
            local += w;
        }
    }
    if sky + local > 0.0 { local / (sky + local) } else { 0.0 }
}

#[derive(Clone, Copy)]
enum Path {
    /// In a straight line.
    Straight,
    /// Back and forth, far enough for the anchor to move each way.
    Shuttle,
    /// Back and forth over the point where the anchor would round up, too
    /// little for it to move.
    Waver,
}

impl Path {
    fn eye(self, t: f32) -> Vec3 {
        let x = match self {
            Path::Straight => WALK_SPEED * t,
            Path::Shuttle => {
                // 10 to 22 m, across x = 16
                let phase = (WALK_SPEED * t / 12.0) % 2.0;
                10.0 + 12.0 * if phase < 1.0 { phase } else { 2.0 - phase }
            }
            Path::Waver => {
                // 13 to 19 m, across x = 16
                let phase = (WALK_SPEED * t / 6.0) % 2.0;
                13.0 + 6.0 * if phase < 1.0 { phase } else { 2.0 - phase }
            }
        };
        Vec3::new(x, 3.0, EYE_HEIGHT)
    }

    fn name(self) -> &'static str {
        match self {
            Path::Straight => "straight",
            Path::Shuttle => "shuttle",
            Path::Waver => "waver",
        }
    }
}

struct Walk {
    /// Falls of [`FLASH`] or more in a tick, per point seen and per minute.
    flashes_per_minute: f32,
    /// The largest fall in a tick.
    worst: f32,
    /// The mean local share.
    share: f32,
    /// Probes waiting for a first capture: the most, and on average.
    waiting_max: usize,
    waiting_mean: f32,
    /// Frames with a probe waiting to be captured again though it was shown
    /// (its bounds moved).
    redrawn: usize,
    /// Seconds to the first capture of every probe that has a slot.
    warm_up: f32,
}

fn walk(scene: &Scene, world: &World, samples: &[Vec3], slots: u32, fps: f32, path: Path) -> Walk {
    let start = Instant::now();
    let step = Duration::from_secs_f32(1.0 / fps);
    let mut m = ProbeManager {
        slots,
        range: RANGE,
        ..Default::default()
    };
    let mut now = start;
    let waiting = |m: &ProbeManager| m.probes.values().filter(|p| p.slot.is_some() && !p.complete).count();
    // standing, until all is captured and has faded in
    let eye = path.eye(0.0);
    let (mut warm_up, mut settled) = (0.0, 0.0);
    while warm_up < 300.0 && settled < 3.0 {
        m.update(scene, world, eye, 0.0, now);
        now += step;
        warm_up += 1.0 / fps;
        let ready = m.default_complete && !m.probes.is_empty() && waiting(&m) == 0;
        settled = if ready { settled + 1.0 / fps } else { 0.0 };
    }
    // walking: the share of each point at each tick
    let mut before: Vec<Option<f32>> = vec![None; samples.len()];
    let (mut t, mut next_tick) = (0.0, 0.0);
    let (mut flashes, mut seen, mut worst, mut total) = (0u64, 0u64, 0.0f32, 0.0f64);
    let (mut waiting_max, mut waiting_sum, mut frames, mut redrawn) = (0, 0usize, 0usize, 0usize);
    while t < WALK_SECONDS {
        let eye = path.eye(t);
        let frame = m.update(scene, world, eye, 0.0, now);
        now += step;
        t += 1.0 / fps;
        let w = waiting(&m);
        waiting_max = waiting_max.max(w);
        waiting_sum += w;
        frames += 1;
        redrawn += usize::from(m.probes.values().any(|p| p.complete && p.last_update.is_none()));
        if t < next_tick {
            continue;
        }
        next_tick += TICK;
        for (p, was) in samples.iter().zip(before.iter_mut()) {
            // what is past the draw distance is not drawn
            if p.distance(eye) > RANGE {
                *was = None;
                continue;
            }
            let share = local_share(&frame.probes, *p);
            if let Some(was) = *was {
                let fall = was - share;
                worst = worst.max(fall);
                flashes += u64::from(fall >= FLASH);
            }
            seen += 1;
            total += f64::from(share);
            *was = Some(share);
        }
    }
    Walk {
        flashes_per_minute: flashes as f32 / (seen as f32 * TICK / 60.0).max(1.0),
        worst,
        share: (total / seen.max(1) as f64) as f32,
        waiting_max,
        waiting_mean: waiting_sum as f32 / frames.max(1) as f32,
        redrawn,
        warm_up,
    }
}

/// `cargo test --workspace walk_report -- --ignored --nocapture`
#[test]
#[ignore = "a report for reading, a minute or more"]
fn walk_report() {
    let (scene, world, samples) = forest();
    println!("slots fps path     flashes/min worst share waiting(max mean) warm_up");
    for slots in [32, 64, 128, 256] {
        for fps in [60.0, 20.0, 12.0] {
            for path in [Path::Straight, Path::Shuttle] {
                let w = walk(&scene, &world, &samples, slots, fps, path);
                println!(
                    "{slots:5} {fps:3.0} {:8} {:11.3} {:5.2} {:5.2} {:7} {:9.1} {:7.1}",
                    path.name(),
                    w.flashes_per_minute,
                    w.worst,
                    w.share,
                    w.waiting_max,
                    w.waiting_mean,
                    w.warm_up,
                );
            }
        }
    }
}

/// Why a probe's weight fell between two ticks.
fn reason(now: Option<&Probe>, then: &Probe, edge_now: f32, edge_then: f32) -> &'static str {
    let Some(p) = now else {
        return "gone from the candidates";
    };
    if p.slot.is_none() {
        return if then.distance <= 0.0 {
            "lost its slot, around the camera"
        } else {
            "lost its slot, outside"
        };
    }
    if !p.complete {
        return "capture dropped";
    }
    if p.retired.is_some() {
        return "retired";
    }
    if edge_fade(p.distance, edge_now) < edge_fade(then.distance, edge_then) - 0.01 {
        return if edge_now < edge_then {
            "edge moved in"
        } else {
            "walked towards the edge"
        };
    }
    "other"
}

/// `cargo test --workspace walk_why -- --ignored --nocapture`
#[test]
#[ignore = "a report for reading"]
fn walk_why() {
    let (scene, world, samples) = forest();
    for (slots, fps, path) in [
        (64, 60.0, Path::Shuttle),
        (64, 60.0, Path::Straight),
        (128, 20.0, Path::Straight),
        (128, 12.0, Path::Straight),
    ] {
        let step = Duration::from_secs_f32(1.0 / fps);
        let mut m = ProbeManager {
            slots,
            range: RANGE,
            ..Default::default()
        };
        let mut now = Instant::now();
        for _ in 0..(fps * 60.0) as u32 {
            m.update(&scene, &world, path.eye(0.0), 0.0, now);
            now += step;
        }
        let snapshot = |m: &ProbeManager| -> HashMap<Key, Probe> {
            m.probes
                .iter()
                .map(|(k, p)| {
                    (
                        *k,
                        Probe {
                            origin: p.origin,
                            radius: p.radius,
                            kind: p.kind,
                            box_inv: p.box_inv,
                            ambiance: p.ambiance,
                            near: p.near,
                            slot: p.slot,
                            complete: p.complete,
                            last_update: p.last_update,
                            fade: p.fade,
                            distance: p.distance,
                            retired: p.retired,
                        },
                    )
                })
                .collect()
        };
        let weight = |p: &Probe, edge: f32, at: Vec3| {
            if !p.complete || p.slot.is_none() {
                return 0.0;
            }
            let d = at.distance(p.origin).max(0.001);
            if d > p.radius {
                return 0.0;
            }
            let r1 = p.radius * 0.5;
            p.fade * edge_fade(p.distance, edge) / d * (1.0 - (d - r1).max(0.0) / (p.radius - r1).max(0.001))
        };
        let mut before: Vec<Option<f32>> = vec![None; samples.len()];
        let (mut was, mut was_edge) = (snapshot(&m), m.edge.unwrap_or(RANGE));
        let (mut t, mut next_tick) = (0.0, 0.0);
        let mut why: HashMap<&'static str, u32> = HashMap::new();
        let (mut held_edge, mut flashes) = (Vec::new(), 0u32);
        while t < WALK_SECONDS {
            let eye = path.eye(t);
            let frame = m.update(&scene, &world, eye, 0.0, now);
            now += step;
            t += 1.0 / fps;
            if t < next_tick {
                continue;
            }
            next_tick += TICK;
            let edge = m.edge.unwrap_or(RANGE);
            for (s, last) in samples.iter().zip(before.iter_mut()) {
                if s.distance(eye) > RANGE {
                    *last = None;
                    continue;
                }
                let share = local_share(&frame.probes, *s);
                if last.is_some_and(|l| l - share >= FLASH) {
                    flashes += 1;
                    let top = was
                        .iter()
                        .map(|(k, p)| {
                            let after = m.probes.get(k).map_or(0.0, |n| weight(n, edge, *s));
                            (k, p, weight(p, was_edge, *s) - after)
                        })
                        .max_by(|a, b| a.2.total_cmp(&b.2));
                    if let Some((k, p, _)) = top {
                        *why.entry(reason(m.probes.get(k), p, edge, was_edge)).or_default() += 1;
                    }
                    held_edge.push(edge);
                }
                *last = Some(share);
            }
            was = snapshot(&m);
            was_edge = edge;
        }
        let mut list: Vec<_> = why.into_iter().collect();
        list.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        held_edge.sort_by(f32::total_cmp);
        println!(
            "{slots} slots {fps} fps {}: {flashes} flashes, edge at flash median {:.1}, {list:?}",
            path.name(),
            held_edge.get(held_edge.len() / 2).copied().unwrap_or(0.0),
        );
    }
}

/// Walking must not flash, nor lose the forest's dark to the default probe:
/// the limits are a little above what the code gives. 64 slots are the tight
/// case (more probes than slots), 128 hold them all, and 12 fps is the case
/// where captures fall behind.
#[test]
fn walking_through_a_forest_does_not_flash() {
    let (scene, world, samples) = forest();
    for (slots, fps, path, flashes, share) in [
        (64, 60.0, Path::Shuttle, 0.6, 0.1),
        (64, 60.0, Path::Straight, 0.3, 0.2),
        (128, 60.0, Path::Shuttle, 0.1, 0.7),
        (128, 60.0, Path::Straight, 0.1, 0.7),
        (128, 12.0, Path::Straight, 0.4, 0.5),
    ] {
        let w = walk(&scene, &world, &samples, slots, fps, path);
        let case = format!("{slots} slots, {fps} fps, {}", path.name());
        assert!(w.flashes_per_minute <= flashes, "{case}: {} flashes a minute", w.flashes_per_minute);
        assert!(w.share >= share, "{case}: a local share of {}", w.share);
    }
}

/// A camera wavering over the anchor's rounding point changes nothing: no cell
/// changes size, and no bounds move as objects cross the range.
#[test]
fn wavering_over_the_anchor_captures_nothing_again() {
    let (scene, world, samples) = forest();
    let w = walk(&scene, &world, &samples, 128, 60.0, Path::Waver);
    assert_eq!(w.waiting_max, 0, "cells changed");
    assert_eq!(w.redrawn, 0, "bounds moved");
    assert_eq!(w.flashes_per_minute, 0.0);
}
