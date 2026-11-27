//! Enhanced water's spindrift: in a strong wind, spray torn off the crests
//! of the wind sea as they break and blown downwind as a thin mist (from
//! about Beaufort 6, all over the sea in a gale). Each tick, places around
//! the camera are tried; where the spectrum's crests fold over there now
//! (water_fft.rs, the very maps the surface and its whitecaps are drawn
//! from), in open water, a puff of spray leaves the crest (`WaterEvent`
//! kind 12, env.wgsl `water_spindrift_particle`), torn into streaks along
//! the wind. The folds are taken as the surface takes the maps there (its
//! chop: a lake's short fetch or a harbour's shelter keeps it low), so
//! water that is drawn calm throws none.
//!
//! Where crests run into each other and fold hard (the Jacobian's smaller
//! eigenvalue well under 0: two crests meeting), the water is thrown up
//! instead, from a fresh breeze on: a burst of spray that rises and falls
//! back (`params.x` 1, `motion.z` how fast it is thrown up). And wherever a
//! crest breaks white, from where whitecaps start, a little of its lip is
//! thrown forward with it and falls back (`params.x` 2): rare and small in
//! a breeze, frequent in a gale. Both of those go with their crest, the
//! way it folds over (`motion.xy`: `squeeze`, at the speed its foam rides
//! on with it), and the shader keeps them on the water's surface as the
//! waves carry it. The Sea panel's Spray scales all three, and brings the
//! crests' to calmer seas.

use aurora_render::WaterEvent;
use aurora_render::water_effects::WaterGround;
use aurora_render::water_sea::Sea;
use glam::{Vec2, Vec3};

/// The wind (m/s) spindrift starts at, and where it is at its fullest.
const START: f32 = 12.0;
const FULL: f32 = 22.0;
/// How often places are tried (s), how many each time, and how far out
/// (m, nearer ones tried more often).
const PERIOD: f32 = 0.1;
const TRIES: usize = 40;
const NEAR: f32 = 5.0;
const RANGE: f32 = 70.0;
/// How far past the whitecaps' threshold (`cap_start`) a crest must fold
/// (`fold`) to throw spindrift, and crests meeting to throw spray: in the
/// demo's sea the folds' 90th to 99th percentiles run from 0.15 to 0.33
/// between 12 and 18 m/s, the deepest 0.45 (fixed thresholds of 0.45 and
/// 0.8 were never reached).
const FOLD: f32 = 0.12;
/// Puffs alive at once at the fullest.
const LIVE: usize = 28;
/// Crests meeting: the wind they start in, are at their fullest in, the
/// fold they take, and bursts alive at once at the fullest.
const CLASH_START: f32 = 7.0;
const CLASH_FULL: f32 = 16.0;
const CLASH_FOLD: f32 = 0.18;

/// The fold the whitecaps start at for a sea of `roughness` (env.wgsl's
/// `cap_start`).
fn cap_start(roughness: f32) -> f32 {
    0.25 - 0.21 * roughness.clamp(0.0, 1.0).sqrt()
}
const CLASH_LIVE: usize = 16;
/// A breaking crest's lip throwing water forward: the wind it starts in
/// (where whitecaps start), is at its fullest in, the fold past the
/// whitecaps' it takes (just into a breaking crest), and the sprays alive
/// at once at the fullest.
const CREST_START: f32 = 5.0;
const CREST_FULL: f32 = 14.0;
const CREST_FOLD: f32 = 0.05;
const CREST_LIVE: usize = 24;

/// How far the surface folds where its displacement's derivatives are `j`
/// (∂Dx/∂x, ∂Dx/∂y, ∂Dy/∂y): 1 less the smaller eigenvalue of its
/// Jacobian, whose sign is the onset of folding (Tessendorf, "Simulating
/// Ocean Water", 2004). The determinant, the two
/// eigenvalues' product, also grew where two softer crests crossed.
fn fold(j: Vec3) -> f32 {
    let (a, b, c) = (1.0 + j.x, j.y, 1.0 + j.z);
    let low = 0.5 * (a + c) - (0.25 * (a - c) * (a - c) + b * b).sqrt();
    1.0 - low
}

/// The way the surface is squeezed where its displacement's derivatives
/// are `j` (as `fold`): the eigenvector of its Jacobian's smaller
/// eigenvalue, across a crest the way it folds over (the spray direction
/// map of Blender's ocean modifier), turned to run with the waves,
/// `toward`; `toward` itself where it is squeezed alike every way.
fn squeeze(j: Vec3, toward: Vec2) -> Vec2 {
    let (a, b, c) = (1.0 + j.x, j.y, 1.0 + j.z);
    let low = 0.5 * (a + c) - (0.25 * (a - c) * (a - c) + b * b).sqrt();
    // (two forms of the same vector, one of them nought along an axis: the
    // longer is the sound one; Blender's divided by b, nought there)
    let (u, v) = (Vec2::new(b, low - a), Vec2::new(low - c, b));
    let e = if u.length_squared() >= v.length_squared() { u } else { v };
    if e.length_squared() < 1e-8 {
        return toward;
    }
    let e = e.normalize();
    if e.dot(toward) < 0.0 { -e } else { e }
}

#[derive(Default)]
pub struct Spindrift {
    next: f32,
    seed: u64,
}

/// The kinds' budgets (`wanted`, alive at once) brought within `room` in
/// all, each in proportion.
fn share(wanted: [f32; 3], room: usize) -> [usize; 3] {
    let total: f32 = wanted.iter().sum();
    let scale = if total > room as f32 { room as f32 / total } else { 1.0 };
    wanted.map(|b| (b * scale).floor().max(if b > 0.0 { 1.0 } else { 0.0 }) as usize)
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Spindrift {
    fn random(&mut self) -> f32 {
        self.seed = self.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut x = self.seed;
        x ^= x >> 30;
        x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
        x ^= x >> 31;
        (x >> 40) as f32 / (1u64 << 24) as f32
    }

    /// One frame at `time` (s) around `eye`, under `sea`, over `ground`;
    /// `spray` the Sea panel's (1: as a real sea throws: more, and from
    /// calmer seas, the higher; 0: none); `room` how many may be alive at
    /// once in all (scene/water_effects.rs `spray_room`).
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, ground: &WaterGround, sea: &Sea, eye: Vec3, time: f32, spray: f32, room: usize, events: &mut Vec<WaterEvent>) {
        let speed = sea.wind[2];
        let spray = if spray.is_finite() { spray.max(0.0) } else { 1.0 };
        // (past 1, the winds each starts at come down, 3 m/s a step, and
        // the folds they need ease)
        let lower = 3.0 * (spray - 1.0).clamp(0.0, 2.0);
        let ease = 0.04 * (spray - 1.0).clamp(-1.0, 2.0);
        // (spindrift is the wind's own: from a gale's start whatever the
        // slider, which brings only the crests' to calmer seas)
        let strength = smoothstep(START, FULL, speed);
        let clash = smoothstep(CLASH_START - lower, CLASH_FULL - lower, speed);
        let crest = smoothstep(CREST_START - lower, CREST_FULL - lower, speed);
        if spray <= 0.0
            || crest.max(clash) <= 0.0
            || sea.shore[3] <= 0.0
            || ground.ocean.texels.is_empty()
            || !(self.next..self.next + 5.0).contains(&time)
        {
            // (and a new clock, or a long pause, starts the ticks afresh)
            if !(self.next..self.next + 5.0).contains(&time) {
                self.next = time;
            }
            return;
        }
        self.next = time + PERIOD;
        let kind = |e: &WaterEvent, k: f32| e.params[1] == 12.0 && (e.params[0] - k).abs() < 0.3;
        let mut live = [0.4, 1.0, 2.0].map(|k| events.iter().filter(|e| kind(e, k)).count());
        let wanted = [LIVE as f32 * strength, CLASH_LIVE as f32 * clash, CREST_LIVE as f32 * crest].map(|b| b * spray);
        let budget = share(wanted, room);
        let toward = Vec2::new(sea.wind[0], sea.wind[1]).normalize_or(Vec2::X);
        let cap = cap_start(sea.frame[3]);
        for _ in 0..TRIES {
            if (0..3).all(|k| live[k] >= budget[k]) {
                break;
            }
            let (a, r) = (self.random() * std::f32::consts::TAU, self.random());
            let p = eye.truncate() + Vec2::from_angle(a) * (NEAR + (RANGE - NEAR) * r * r);
            let Some([bed, level]) = ground.sample(p) else {
                continue;
            };
            if level - bed < 2.0 {
                continue;
            }
            let at = ground.sea_point(p, sea, level - bed);
            if at.shelter < 0.6 {
                continue;
            }
            // (the chop the surface takes the maps at here: water_sea.rs
            // `open`)
            let (shadow, reach) = at.swell.as_ref().map_or((1.0, 1.0), |s| (s.shadow, s.chop));
            let chop = sea.local(&at, shadow, reach).chop;
            let ocean = ground.ocean.sample(p);
            let j = ocean.jacobian * chop * sea.look[1];
            let fold = fold(j) + ease;
            let past = fold - cap;
            // (the crests' spray goes the way the crest folds over, at the
            // speed its foam rides on with it)
            let ride = squeeze(j, toward) * aurora_render::water_fft::ride_speed(1, speed);
            let (k, motion) = if past >= CLASH_FOLD && live[1] < budget[1] {
                // crests meeting: the water thrown up
                // (up to some 0.6 m: thrown higher, it flew about the sea)
                let up = (1.4 + 1.6 * ((past - CLASH_FOLD) / 0.15).min(1.0)) * (0.8 + 0.4 * self.random());
                (1, [ride.x * 0.5, ride.y * 0.5, up, clash])
            } else if past >= FOLD && live[0] < budget[0] && strength > 0.0 {
                // spindrift blown off the crest
                (0, [toward.x * speed, toward.y * speed, 0.0, strength * (past / 0.3).min(1.0)])
            } else if past >= CREST_FOLD && live[2] < budget[2] {
                // a breaking crest's lip throwing its water forward
                // (a hop of a few centimetres to some 0.4 m)
                let up = (1.0 + 1.4 * ((past - CREST_FOLD) / 0.15).min(1.0)) * (0.8 + 0.4 * self.random());
                (2, [ride.x, ride.y, up, crest])
            } else {
                continue;
            };
            // (where the crest is drawn: the maps' point moved as the waves
            // move it, the surface's height there)
            let shown = p + ocean.offset.truncate() * chop * sea.look[1];
            let z = sea.surface(shown, time, 4, level, &at);
            events.push(WaterEvent {
                position: [shown.x, shown.y, z, time],
                motion,
                params: [[0.4, 1.0, 2.0][k], 12.0, level, self.random()],
            });
            live[k] += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_starts_with_a_strong_wind_and_is_full_in_a_gale() {
        assert_eq!(smoothstep(START, FULL, 8.0), 0.0);
        assert!(smoothstep(START, FULL, 15.0) > 0.1);
        assert_eq!(smoothstep(START, FULL, 25.0), 1.0);
    }

    #[test]
    fn the_fold_is_the_smaller_eigenvalue_s_not_the_determinant_s() {
        // one crest folding: both the same
        assert!((fold(Vec3::new(-0.6, 0.0, 0.0)) - 0.6).abs() < 1e-5);
        // two softer crests crossing: the determinant's 1 - 0.49 = 0.51,
        // but neither folds that far
        assert!((fold(Vec3::new(-0.3, 0.0, -0.3)) - 0.3).abs() < 1e-5);
        // turned: the same fold
        let (c, s) = (0.6f32, 0.8f32);
        let turned = Vec3::new(-0.6 * c * c, -0.6 * c * s, -0.6 * s * s);
        assert!((fold(turned) - 0.6).abs() < 1e-4, "{}", fold(turned));
    }

    #[test]
    fn the_spray_goes_across_the_crest_the_way_the_waves_run() {
        let toward = Vec2::new(0.6, 0.8);
        // a crest along y, squeezed along x: across it, downwind
        let e = squeeze(Vec3::new(-0.8, 0.0, 0.0), toward);
        assert!((e - Vec2::X).length() < 1e-5, "{e}");
        // squeezed along y
        let e = squeeze(Vec3::new(0.0, 0.0, -0.5), toward);
        assert!((e - Vec2::Y).length() < 1e-5, "{e}");
        // a crest turned: across it still, turned with it
        let (c, s) = (0.8f32, 0.6f32);
        let e = squeeze(Vec3::new(-0.6 * c * c, -0.6 * c * s, -0.6 * s * s), toward);
        assert!((e - Vec2::new(c, s)).length() < 1e-4, "{e}");
        // squeezed alike every way: with the wind
        assert_eq!(squeeze(Vec3::new(-0.3, 0.0, -0.3), toward), toward);
    }

    #[test]
    fn the_spray_stays_within_its_room() {
        assert_eq!(share([28.0, 16.0, 24.0], 200), [28, 16, 24]);
        let b = share([56.0, 32.0, 48.0], 64);
        assert!(b.iter().sum::<usize>() <= 64 + 3, "{b:?}");
        assert!(b[0] > b[1] && b[2] > b[1], "{b:?}");
        assert_eq!(share([0.0, 3.0, 0.0], 0), [0, 1, 0]);
    }

    #[test]
    fn its_places_spread_round_the_camera() {
        let mut s = Spindrift::default();
        let values: Vec<f32> = (0..400).map(|_| s.random()).collect();
        assert!(values.iter().all(|v| (0.0..1.0).contains(v)));
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        assert!((mean - 0.5).abs() < 0.06, "{mean}");
    }
}
