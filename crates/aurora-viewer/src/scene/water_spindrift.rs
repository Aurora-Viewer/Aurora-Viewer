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
/// How far a crest must fold (1 - the Jacobian's determinant) to throw:
/// well into a whitecap's dense core (env.wgsl's `cap_start` is 0.04 to
/// 0.25 with the roughness).
const FOLD: f32 = 0.45;
/// Puffs alive at once at the fullest.
const LIVE: usize = 28;

#[derive(Default)]
pub struct Spindrift {
    next: f32,
    seed: u64,
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

    /// One frame at `time` (s) around `eye`, under `sea`, over `ground`.
    pub fn update(&mut self, ground: &WaterGround, sea: &Sea, eye: Vec3, time: f32, events: &mut Vec<WaterEvent>) {
        let speed = sea.wind[2];
        let strength = smoothstep(START, FULL, speed);
        if strength <= 0.0 || sea.shore[3] <= 0.0 || ground.ocean.texels.is_empty() || !(self.next..self.next + 5.0).contains(&time) {
            // (and a new clock, or a long pause, starts the ticks afresh)
            if !(self.next..self.next + 5.0).contains(&time) {
                self.next = time;
            }
            return;
        }
        self.next = time + PERIOD;
        let mut live = events.iter().filter(|e| e.params[1] == 12.0).count();
        let budget = (LIVE as f32 * strength).ceil() as usize;
        let toward = Vec2::new(sea.wind[0], sea.wind[1]).normalize_or(Vec2::X);
        for _ in 0..TRIES {
            if live >= budget {
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
            let j = ground.ocean.sample(p).jacobian * chop * sea.look[1];
            let fold = 1.0 - ((1.0 + j.x) * (1.0 + j.z) - j.y * j.y);
            if fold < FOLD {
                continue;
            }
            let z = sea.surface(p, time, 4, level, &at);
            events.push(WaterEvent {
                position: [p.x, p.y, z, time],
                motion: [toward.x * speed, toward.y * speed, 0.0, strength * (fold / 0.6).min(1.0)],
                params: [0.4, 12.0, level, self.random()],
            });
            live += 1;
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
    fn its_places_spread_round_the_camera() {
        let mut s = Spindrift::default();
        let values: Vec<f32> = (0..400).map(|_| s.random()).collect();
        assert!(values.iter().all(|v| (0.0..1.0).contains(v)));
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        assert!((mean - 0.5).abs() < 0.06, "{mean}");
    }
}
