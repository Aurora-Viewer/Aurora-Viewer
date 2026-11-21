//! Waves crashing on what stands steep in the sea: the spray a breaking
//! swell throws up a rock face, a cliff or a seawall. The points where it
//! does are looked for near the camera now and then: the steep shores of
//! the ground facing the incoming swell (the bathymetry grid) and the faces
//! of static prims standing in the water that face it (the shelter's boxes,
//! water_shelter.rs). Each frame the swell's phase is read at each point
//! (the same sea as the drawn waves, `Sea::crest_phase`), and as a crest
//! arrives a burst of spray (`WaterEvent` kind 9, drawn in env.wgsl's
//! `vs_water_particle`) and a ring of churned foam (kind 6) are sent. The
//! Legacy Viewer has no such thing: its water has no waves to break.

use super::water_shelter::ShelterBox;
use aurora_render::WaterEvent;
use aurora_render::water_effects::WaterGround;
use aurora_render::water_sea::Sea;
use glam::{Vec2, Vec3};

/// How far from the camera waves crash (m), how many points at most, how
/// far apart, and how often they are looked for again (s).
const RANGE: f32 = 70.0;
const MAX_POINTS: usize = 32;
const SPACING: f32 = 3.0;
const SCAN: f32 = 1.0;
/// A swell lower than this (m, its amplitude) laps; it does not crash.
const MIN_SWELL: f32 = 0.12;
/// Under less room than this above the water (m) nothing is thrown up; no
/// ceiling at all counts as `OPEN_ROOM`.
const MIN_ROOM: f32 = 0.6;
const OPEN_ROOM: f32 = 50.0;

/// The room above the water at `xy` (m): up to the lowest underside of the
/// prims overhead (`OPEN_ROOM`: none).
fn ceiling(boxes: &[ShelterBox], xy: Vec2, level: f32) -> f32 {
    boxes
        .iter()
        .filter_map(|b| b.span(xy, level - OPEN_ROOM))
        .filter(|&(under, _)| under > level + 0.05)
        .map(|(under, _)| under - level)
        .fold(OPEN_ROOM, f32::min)
}

#[derive(Debug, Clone, Copy)]
struct Point {
    /// At the water, just off the face, and the way off it (out to sea).
    at: Vec2,
    away: Vec2,
    /// Where the swell coming in is read (a few metres out, before the
    /// face's own shelter calms it).
    upwave: Vec2,
    /// How far along the face the spray spreads (m), and how high the face
    /// stands above the water (m).
    width: f32,
    height: f32,
    /// How much room there is above the water there (m): up to the lowest
    /// underside overhead (a deck, a floor on stilts), the spray stopping
    /// under it.
    ceiling: f32,
    /// How squarely it faces the swell (0..1), and how much it slopes back
    /// (0 a sheer wall, 1 a 45° rock): the spray runs up a sloping face.
    facing: f32,
    lean: f32,
    /// The swell's phase there last frame, as `crest_share`.
    last: Option<f32>,
}

/// The crash points near the camera and when they were looked for.
#[derive(Debug, Default)]
pub struct Crashes {
    points: Vec<Point>,
    scanned_at: Option<f32>,
}

/// How far through its cycle a swell of phase `theta` is (0..1, rising with
/// time); its crest is at 0.75.
fn crest_share(theta: f32) -> f32 {
    (-theta / std::f32::consts::TAU).rem_euclid(1.0)
}

/// Whether the swell went past its crest between `last` and `now`
/// (`crest_share`), across the turn of the cycle too.
fn crest_passed(last: f32, now: f32) -> bool {
    let moved = (now - last).rem_euclid(1.0);
    moved < 0.5 && (0.75 - last).rem_euclid(1.0) < moved
}

impl Crashes {
    /// Forget everything (the waves turned off, another region).
    pub fn clear(&mut self) {
        self.points.clear();
        self.scanned_at = None;
    }

    /// Look for the crash points near `eye` again if it is time: the steep
    /// shores of `ground` and the faces of `boxes` turned to the swell
    /// going toward `toward` (unit).
    /// `surface`: where a ray from a point toward a direction meets the real
    /// surface of `boxes[k]` (distance, normal), none if it misses it; none
    /// at all when its shape is not known (its box is taken as it is).
    pub fn scan(&mut self, ground: &WaterGround, boxes: &[ShelterBox], surface: &Surface, eye: Vec2, toward: Vec2, time: f32) {
        if self.scanned_at.is_some_and(|t| (0.0..SCAN).contains(&(time - t))) {
            return;
        }
        self.scanned_at = Some(time);
        let mut found = Vec::new();
        prim_faces(ground, boxes, surface, eye, toward, &mut found);
        steep_shores(ground, eye, toward, &mut found);
        // what stands overhead: no crash under a low deck (the stilts of a
        // house on the water), the spray stopping under a higher one
        found.retain_mut(|p| {
            let Some([_, level]) = ground.sample(p.at) else {
                return false;
            };
            p.ceiling = ceiling(boxes, p.at, level);
            p.height = p.height.min(p.ceiling);
            p.ceiling >= MIN_ROOM
        });
        // nearest first, a few metres apart, carrying over each point's
        // last phase (no burst for a crest seen only by the rescan)
        found.sort_by(|a, b| a.at.distance_squared(eye).total_cmp(&b.at.distance_squared(eye)));
        let mut kept: Vec<Point> = Vec::new();
        for mut p in found {
            if kept.len() >= MAX_POINTS {
                break;
            }
            if kept.iter().any(|k| k.at.distance_squared(p.at) < SPACING * SPACING) {
                continue;
            }
            p.last = self.points.iter().find(|o| o.at.distance_squared(p.at) < 0.25).and_then(|o| o.last);
            kept.push(p);
        }
        self.points = kept;
    }

    /// Follow the swell at each point to `time` and send a burst where a
    /// crest arrives, into `events`, keeping the crashes' events under
    /// `budget` (a new burst is left out rather than live ones evicted:
    /// they flashed out mid-flight).
    pub fn update(&mut self, ground: &WaterGround, sea: &Sea, time: f32, events: &mut Vec<WaterEvent>, budget: usize) {
        let mut live = events.iter().filter(|e| matches!(e.params[1], 9.0 | 10.0)).count();
        for p in &mut self.points {
            let Some([g, w]) = ground.sample(p.at) else {
                p.last = None;
                continue;
            };
            let at = ground.sea_point(p.at, sea, w - g);
            let now = crest_share(sea.crest_phase(p.at, time, &at));
            let last = p.last.replace(now);
            if !last.is_some_and(|last| crest_passed(last, now)) {
                continue;
            }
            // the wave that really reaches the face: read off it, as the
            // sea draws it there (held down by shallows it broke on, by
            // shelter, by islands): no crash on a swell that never gets there
            let swell = ground.sample(p.upwave).map_or(0.0, |[g, w]| {
                let out = ground.sea_point(p.upwave, sea, w - g);
                let (shadow, chop) = out.swell.map_or((1.0, 1.0), |s| (s.shadow, s.chop));
                sea.local(&out, shadow, chop).main
            }) * p.facing;
            if swell < MIN_SWELL || live + 6 > budget {
                continue;
            }
            let before = events.len();
            burst(p, swell, w, time, events);
            live += events.len() - before;
        }
    }
}

/// A number in 0..1 for `n` (a hash: the same burst always draws alike).
fn random(n: u32) -> f32 {
    let mut h = n.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    (h >> 8) as f32 / 16_777_216.0
}

/// The spray of a crest of `swell` m crashing at `p`, on water at `level`.
/// Each burst draws its own: how hard this part of the face is hit, a
/// moment early or late, and now and then a lower swell missing it, so the
/// face does not throw up a row of the same spray at once.
fn burst(p: &Point, swell: f32, level: f32, time: f32, events: &mut Vec<WaterEvent>) {
    let seed = p.at.x.to_bits() ^ p.at.y.to_bits().rotate_left(13) ^ time.to_bits().rotate_left(7);
    let draw = |k: u32| random(seed.wrapping_add(k.wrapping_mul(0x27d4_eb2f)));
    if draw(0) < 0.25 * (1.0 - (swell / 0.6).min(1.0)) {
        return;
    }
    // (a narrow face, a piling, stops little of the water: less is thrown)
    let narrow = (p.width / 2.0).clamp(0.35, 1.0);
    // (a low lip of rock throws little)
    let tall = (p.height / 1.5).clamp(0.3, 1.0);
    let power = (swell * 1.4 * narrow * tall * (0.6 + 0.8 * draw(1))).clamp(0.3, 3.0);
    let delay = draw(2) * 0.35;
    // more of it for a bigger swell, spread along the face (each draws
    // three sheets of water fanned out, and droplets off their rims)
    let packets = 2 + (swell.min(1.5) * 1.5) as usize;
    let along = p.away.perp();
    for i in 0..packets {
        let u = (i as f32 + 0.2 + 0.6 * draw(3 + i as u32)) / packets as f32 * 2.0 - 1.0;
        let at = p.at + along * u * p.width * 0.5;
        events.push(WaterEvent {
            position: at.extend(level).extend(time + delay + i as f32 * 0.04).to_array(),
            // (z: the room above, the spray stopping under it)
            // (xy off the face, longer by how much it slopes back)
            motion: [
                p.away.x * (1.0 + p.lean),
                p.away.y * (1.0 + p.lean),
                p.ceiling,
                power * (0.85 + 0.3 * draw(20 + i as u32)),
            ],
            // (each as wide as its share of the face, and some: they meet;
            // w the seed its curtains are drawn from)
            params: [
                p.width * 0.5 / packets as f32 * 2.0 + 0.3 + swell.min(2.0) * 0.3,
                9.0,
                level,
                draw(40 + i as u32),
            ],
        });
    }
    // the white water it leaves: foam spreading at its foot and streaming
    // back down the face (drawn anywhere), and the churn of the water
    // simulated round the avatar
    events.push(WaterEvent {
        position: p.at.extend(level).extend(time + delay).to_array(),
        // (xy longer by how much the face slopes back: the runoff lies on it)
        motion: [p.away.x * (1.0 + p.lean), p.away.y * (1.0 + p.lean), p.height, power],
        params: [(p.width * 0.6).clamp(1.0, 3.0), 10.0, level, draw(60)],
    });
    events.push(WaterEvent {
        position: p.at.extend(level).extend(time + delay).to_array(),
        motion: [0.0, 0.0, -2.0, (power * 0.6).min(1.6)],
        params: [(p.width * 0.5).clamp(0.8, 2.5), 6.0, level, 0.0],
    });
}

/// Where a ray meets a prim's real surface (`Crashes::scan`).
pub type Surface<'a> = dyn Fn(usize, Vec3, Vec3) -> Option<Option<(f32, Vec3)>> + 'a;

/// The faces of static prims standing in the water that face the swell
/// going toward `toward`: points along each, at the water, just off it. Each
/// is moved onto the prim's real surface at the waterline and takes its
/// slope (`surface`), or left out where the box's face has nothing of the
/// prim at the water (a rock's box stands well off the rock).
fn prim_faces(ground: &WaterGround, boxes: &[ShelterBox], surface: &Surface, eye: Vec2, toward: Vec2, out: &mut Vec<Point>) {
    for (k, b) in boxes.iter().enumerate() {
        let c = b.center.truncate();
        if c.distance(eye) > RANGE + b.half.max_element() {
            continue;
        }
        let Some([_, level]) = ground.sample(c) else {
            continue;
        };
        // it stands through the water, out of it far enough to break on
        let reach = (0..3).map(|k| (b.rotation * Vec3::AXES[k]).z.abs() * b.half[k]).sum::<f32>();
        if b.center.z - reach > level - 0.2 || b.center.z + reach < level + 0.4 {
            continue;
        }
        for (axis, side) in [(0usize, 1usize), (1, 0)] {
            for sign in [-1.0f32, 1.0] {
                let normal = b.rotation * (Vec3::AXES[axis] * sign);
                if normal.z.abs() > 0.5 {
                    continue;
                }
                let away = normal.truncate().normalize_or_zero();
                // turned to the incoming swell (within about 55°)
                let facing = -away.dot(toward);
                if facing < 0.55 {
                    continue;
                }
                let face = b.center.truncate() + normal.truncate() * b.half[axis];
                let tangent = (b.rotation * Vec3::AXES[side]).truncate().normalize_or_zero();
                let length = 2.0 * b.half[side] * (b.rotation * Vec3::AXES[side]).truncate().length();
                let count = ((length / SPACING).floor() as usize).clamp(1, 6);
                for i in 0..count {
                    let u = (i as f32 + 0.5) / count as f32 - 0.5;
                    let mut at = face + tangent * u * length + away * 0.3;
                    let (mut away, mut facing, mut lean) = (away, facing, 0.0);
                    // the real surface at the waterline, from 4 m out at sea
                    let ray = -away;
                    let origin = (at + away * 4.0).extend(level + 0.2);
                    match surface(k, origin, ray.extend(0.0)) {
                        Some(None) => continue,
                        Some(Some((t, normal))) => {
                            // (turned to the sea whatever its triangles' winding)
                            let normal = if normal.truncate().dot(ray) > 0.0 { -normal } else { normal };
                            let flat = normal.truncate();
                            let len = flat.length();
                            if len < 0.2 {
                                continue;
                            }
                            away = flat / len;
                            facing = -away.dot(toward);
                            if facing < 0.55 {
                                continue;
                            }
                            // a rock sloping back leans its spray up the slope
                            lean = (normal.z.max(0.0) / len).min(1.0);
                            at = origin.truncate() + ray * t + away * 0.3;
                        }
                        None => {}
                    }
                    if at.distance(eye) > RANGE {
                        continue;
                    }
                    out.push(Point {
                        at,
                        away,
                        upwave: at + away * 6.0,
                        width: (length / count as f32).min(4.0),
                        height: (b.center.z + reach - level).clamp(0.0, 6.0),
                        ceiling: OPEN_ROOM,
                        facing,
                        lean,
                        last: None,
                    });
                }
            }
        }
    }
}

/// The steep shores of the ground (rising at least as steeply as it runs)
/// facing the swell going toward `toward`, where the water meets them.
fn steep_shores(ground: &WaterGround, eye: Vec2, toward: Vec2, out: &mut Vec<Point>) {
    let n = 64usize;
    if ground.heights.len() != n * n || !ground.extent.is_finite() || ground.extent <= 0.0 {
        return;
    }
    let cell = ground.extent / n as f32;
    let origin = Vec2::from_array(ground.origin);
    let at = |x: usize, y: usize| ground.heights[y * n + x];
    for y in 1..n - 1 {
        for x in 1..n - 1 {
            let [g, w] = at(x, y);
            // where the water meets the face: from a little above it to a
            // couple of metres under it
            if !(-0.5..2.0).contains(&(w - g)) {
                continue;
            }
            let xy = origin + (Vec2::new(x as f32, y as f32) + Vec2::splat(0.5)) * cell;
            if xy.distance(eye) > RANGE {
                continue;
            }
            let rise = Vec2::new(at(x + 1, y)[0] - at(x - 1, y)[0], at(x, y + 1)[0] - at(x, y - 1)[0]) / (2.0 * cell);
            let steep = rise.length();
            if steep < 1.0 {
                continue;
            }
            // the face turned to the incoming swell (within about 55°),
            // rising well out of the water: a bank that comes up a little
            // and flattens out takes the waves as a beach
            let away = -rise / steep;
            let facing = -away.dot(toward);
            if facing < 0.55 {
                continue;
            }
            let up = xy - away * (2.0 * cell);
            let Some([top, _]) = ground.sample(up).filter(|[g, w]| *g >= w + 0.8) else {
                continue;
            };
            out.push(Point {
                at: xy + away * 0.3,
                away,
                upwave: xy + away * 6.0,
                width: cell.min(4.0),
                height: (top - w).clamp(0.0, 4.0),
                ceiling: OPEN_ROOM,
                facing,
                lean: (1.0 / steep).min(1.0),
                last: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bursts_differ_from_one_crest_and_one_place_to_the_next() {
        let point = |x: f32| Point {
            at: Vec2::new(x, 10.0),
            away: -Vec2::X,
            upwave: Vec2::new(x - 6.0, 10.0),
            width: 3.0,
            height: 1.5,
            ceiling: OPEN_ROOM,
            facing: 1.0,
            lean: 0.0,
            last: None,
        };
        let spray = |x: f32, time: f32| {
            let mut events = Vec::new();
            burst(&point(x), 1.0, 20.0, time, &mut events);
            events
        };
        let (a, b, c) = (spray(5.0, 10.0), spray(8.0, 10.0), spray(5.0, 17.6));
        assert!(!a.is_empty() && !b.is_empty() && !c.is_empty(), "a metre of swell always crashes");
        let looks = |e: &[WaterEvent]| (e[0].position[3], e[0].motion[3], e[0].params[3]);
        assert_ne!(looks(&a), looks(&b));
        assert_ne!(looks(&a), looks(&c));
        // the same burst always draws alike
        assert_eq!(looks(&a), looks(&spray(5.0, 10.0)));
        // the spray never comes before the crest
        assert!(a.iter().all(|e| e.position[3] >= 10.0));
    }

    #[test]
    fn the_stilts_under_a_low_floor_throw_nothing_and_a_high_deck_caps_the_spray() {
        let ground = WaterGround {
            origin: [0.0, 0.0],
            extent: 64.0,
            heights: std::sync::Arc::from(vec![[10.0f32, 20.0]; 64 * 64]),
            ..Default::default()
        };
        let stilt = ShelterBox {
            center: Vec3::new(32.0, 32.0, 18.5),
            rotation: glam::Quat::IDENTITY,
            half: Vec3::new(1.2, 1.2, 2.5),
            mesh: false,
        };
        let floor = |underside: f32| ShelterBox {
            center: Vec3::new(32.0, 32.0, underside + 0.125),
            rotation: glam::Quat::IDENTITY,
            half: Vec3::new(4.0, 4.0, 0.125),
            mesh: false,
        };
        let crashes = |boxes: &[ShelterBox]| {
            let mut c = Crashes::default();
            c.scan(&ground, boxes, &|_, _, _| None, Vec2::new(20.0, 32.0), Vec2::X, 0.0);
            c.points
        };
        // in the open, the stilt's seaward face crashes
        assert!(!crashes(&[stilt]).is_empty());
        // under a floor 40 cm above the water: not at all
        assert!(crashes(&[stilt, floor(20.4)]).is_empty());
        // under a deck 2 m up: the spray stops under it
        let under = crashes(&[stilt, floor(22.0)]);
        assert!(
            !under.is_empty() && under.iter().all(|p| (p.ceiling - 2.0).abs() < 0.01),
            "{under:?}"
        );
    }

    #[test]
    fn a_wall_behind_shallows_the_swell_has_broken_on_throws_nothing() {
        // a gale's swell, toward +x, a wall at x = 32 facing it
        let mut driver = aurora_render::water_sea::SeaDriver::default();
        driver.advance(0.0, Vec2::new(18.0, 0.0), Vec2::new(32.0, 32.0));
        let ground = |depth: f32| WaterGround {
            origin: [0.0, 0.0],
            extent: 64.0,
            heights: std::sync::Arc::from(vec![[20.0 - depth, 20.0f32]; 64 * 64]),
            ..Default::default()
        };
        let wall = ShelterBox {
            center: Vec3::new(32.0, 32.0, 20.0),
            rotation: glam::Quat::IDENTITY,
            half: Vec3::new(0.5, 6.0, 3.0),
            mesh: false,
        };
        let mut bursts = |depth: f32| {
            let g = ground(depth);
            let mut c = Crashes::default();
            c.scan(&g, &[wall], &|_, _, _| None, Vec2::new(20.0, 32.0), Vec2::X, 0.0);
            let mut events = Vec::new();
            for i in 0..1200 {
                let t = i as f32 / 60.0;
                let sea = driver.advance(t, Vec2::new(18.0, 0.0), Vec2::new(32.0, 32.0));
                c.update(&g, &sea, t, &mut events, 1000);
            }
            events.iter().filter(|e| e.params[1] == 9.0).count()
        };
        assert!(bursts(8.0) > 0, "deep water: the crests crash");
        assert_eq!(bursts(0.25), 0, "the swell broke on the shallows before");
    }

    #[test]
    fn a_rock_s_spray_starts_at_its_real_surface_and_takes_its_slope() {
        let ground = WaterGround {
            origin: [0.0, 0.0],
            extent: 64.0,
            heights: std::sync::Arc::from(vec![[10.0f32, 20.0]; 64 * 64]),
            ..Default::default()
        };
        // a rock whose box faces the swell (toward +x) at x = 28, but whose
        // stone meets the water 3 m further in, sloping back at 45°
        let rock = ShelterBox {
            center: Vec3::new(32.0, 32.0, 20.0),
            rotation: glam::Quat::IDENTITY,
            half: Vec3::new(4.0, 2.0, 3.0),
            mesh: false,
        };
        let stone = |_: usize, origin: Vec3, dir: Vec3| {
            let face_x = 31.0;
            Some(((dir.x > 0.0) && origin.x < face_x).then(|| (face_x - origin.x, Vec3::new(-1.0, 0.0, 1.0).normalize())))
        };
        let mut found = Vec::new();
        prim_faces(&ground, &[rock], &stone, Vec2::new(10.0, 32.0), Vec2::X, &mut found);
        assert!(!found.is_empty());
        for p in &found {
            assert!((p.at.x - 30.7).abs() < 0.05, "on the stone, not its box: {:?}", p.at);
            assert!((p.lean - 1.0).abs() < 0.05, "it slopes back 45°: {}", p.lean);
        }
        // nothing of it at the waterline: no spray off its box
        let mut none = Vec::new();
        prim_faces(&ground, &[rock], &|_, _, _| Some(None), Vec2::new(10.0, 32.0), Vec2::X, &mut none);
        assert!(none.is_empty());
    }

    #[test]
    fn a_crest_is_seen_passing_once_across_the_turn_of_the_cycle() {
        assert!(crest_passed(0.70, 0.78));
        assert!(!crest_passed(0.78, 0.86));
        assert!(!crest_passed(0.10, 0.20));
        // a jump back (a new clock) is no crest
        assert!(!crest_passed(0.80, 0.70));
        // the phase falling with time is the cycle going on
        let theta = |s: f32| -s * std::f32::consts::TAU;
        assert!((crest_share(theta(0.3)) - 0.3).abs() < 1e-5);
    }

    #[test]
    fn a_wall_facing_the_swell_crashes_and_one_turned_away_does_not() {
        let ground = WaterGround {
            origin: [0.0, 0.0],
            extent: 64.0,
            heights: std::sync::Arc::from(vec![[10.0f32, 20.0]; 64 * 64]),
            ..Default::default()
        };
        let wall = |yaw: f32| ShelterBox {
            center: Vec3::new(32.0, 32.0, 20.0),
            rotation: glam::Quat::from_rotation_z(yaw),
            half: Vec3::new(0.5, 6.0, 3.0),
            mesh: false,
        };
        // the swell going east: the wall's west face takes it
        let mut found = Vec::new();
        prim_faces(&ground, &[wall(0.0)], &|_, _, _| None, Vec2::new(10.0, 32.0), Vec2::X, &mut found);
        assert!(
            !found.is_empty() && found.iter().all(|p| p.away.x < -0.9 && p.at.x < 31.5),
            "{found:?}"
        );
        // a wall laid along the swell's way shows it only its narrow end
        let mut along = Vec::new();
        prim_faces(
            &ground,
            &[wall(std::f32::consts::FRAC_PI_2)],
            &|_, _, _| None,
            Vec2::new(10.0, 32.0),
            Vec2::X,
            &mut along,
        );
        assert!(along.len() == 1 && along[0].width <= 1.0 && along[0].at.x < 26.0, "{along:?}");
        // a deck high above the water breaks nothing
        let mut deck = Vec::new();
        let high = ShelterBox {
            center: Vec3::new(32.0, 32.0, 24.0),
            ..wall(0.0)
        };
        prim_faces(&ground, &[high], &|_, _, _| None, Vec2::new(10.0, 32.0), Vec2::X, &mut deck);
        assert!(deck.is_empty());
    }

    #[test]
    fn a_cliff_facing_the_swell_crashes_and_a_gentle_beach_does_not() {
        // the ground rising eastward out of the water: steeply at x = 40,
        // gently from x = 10
        let heights = |steep: bool| -> Vec<[f32; 2]> {
            (0..64 * 64)
                .map(|i| {
                    let x = (i % 64) as f32 + 0.5;
                    let g = if steep {
                        15.0 + (x - 40.0).max(0.0) * 3.0
                    } else {
                        15.0 + (x - 10.0).max(0.0) * 0.1
                    };
                    [g.min(40.0), 20.0]
                })
                .collect()
        };
        let ground = |steep| WaterGround {
            origin: [0.0, 0.0],
            extent: 64.0,
            heights: std::sync::Arc::from(heights(steep)),
            ..Default::default()
        };
        let mut found = Vec::new();
        steep_shores(&ground(true), Vec2::new(30.0, 32.0), Vec2::X, &mut found);
        assert!(!found.is_empty() && found.iter().all(|p| p.away.x < -0.9), "{}", found.len());
        // the swell going west, away from it: none
        let mut lee = Vec::new();
        steep_shores(&ground(true), Vec2::new(30.0, 32.0), -Vec2::X, &mut lee);
        assert!(lee.is_empty());
        let mut beach = Vec::new();
        steep_shores(&ground(false), Vec2::new(30.0, 32.0), Vec2::X, &mut beach);
        assert!(beach.is_empty());
    }
}
