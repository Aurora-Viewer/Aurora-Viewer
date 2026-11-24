//! Where the frame's light changed, for the traced indirect light
//! (rt_shadows.rs `Gi`, shaders/rt_gi.wgsl). That pass keeps half a second
//! of history and, per pixel, the best hit its rays found: a lamp switched
//! off went on lighting the walls around it for that long. Where a lamp
//! came, went, moved or changed its brightness, and everywhere when the
//! sun's or the sky's light jumped, both are let go for a few frames.
//! Measured in a closed room at 240 frames a second (`AURORA_DEMO_LAMP_BLINK`),
//! four frames after its lamp went out: a quarter of the glow the lamp
//! left on the wall behind it, where all of it stayed; a twentieth at
//! twelve frames, where nine tenths stayed at forty.
//!
//! After Majercik et al., "Scaling Probe-Based Real-Time Dynamic Global
//! Illumination for Production" (JCGT 10(2), 2021, §4.3 "Fast Convergence
//! Heuristics"): a large lighting change halves the probes' hysteresis for
//! ten frames, and a change counts from a quarter of the value. The paper
//! shortens every probe's history and notes that only those the change
//! reaches should be; here a change is a ball around its lamp.
//!
//! The lights are compared with what the history holds of them, not with
//! last frame's: `known` follows each light at the history's own rate, so
//! the day going by or a lamp that dims slowly, which the history follows
//! well enough, never count, and a change spread over many frames still
//! does once the history is a quarter behind.

use glam::{Mat4, Vec3};

use crate::types::PointLight;

/// The most changed places a frame carries (rt_gi.wgsl `Params::changed`).
pub(crate) const MAX_CHANGES: usize = 8;
/// Frames a change is held where it fell (the paper's ten).
const HELD_FRAMES: u32 = 10;
/// The most frames the history averages where the light changed: the
/// hysteresis halved.
const CHANGED_HISTORY: f32 = 2.0;
/// A light changed when it moved by this share of the larger of what the
/// history holds and what it is now.
const CHANGE: f32 = 0.25;
/// A lamp is the one known at a place within this share of its reach;
/// further, it is taken as gone from there and come here.
const MOVED: f32 = 0.1;
/// The sun turned when it is this far (radians) from where the history
/// holds it.
const SUN_TURN: f32 = 0.02;
/// A lamp's change matters as far as its light on a face is this share of
/// what the sky's hemisphere gives one: a lamp switched at noon changes a
/// few steps around it, at night all of its reach.
const SIGNIFICANT: f32 = 0.1;
/// A light fainter than this gives the indirect light nothing to follow.
const FAINT: f32 = 1.0e-3;

const LUMINANCE: Vec3 = Vec3::new(0.2126, 0.7152, 0.0722);

/// The sun's and the sky's light of a frame (rt_shadows.rs `TraceLight`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SkyLight {
    /// Towards the sun or moon, unit; None when neither is up.
    pub sun: Option<Vec3>,
    pub sun_color: Vec3,
    pub zenith: Vec3,
    pub horizon: Vec3,
    pub ground: Vec3,
}

impl SkyLight {
    fn sunlight(&self) -> f32 {
        match self.sun {
            Some(_) => self.sun_color.dot(LUMINANCE).max(0.0),
            None => 0.0,
        }
    }

    fn skylight(&self) -> f32 {
        (self.zenith + self.horizon + self.ground).dot(LUMINANCE).max(0.0)
    }

    /// What the hemisphere gives a face turned up, in a lamp's units: the
    /// shading takes the hemisphere whole and a lamp's light over pi
    /// (common.wgsl `shade_pbr`).
    fn on_a_face(&self) -> f32 {
        self.zenith.dot(LUMINANCE).max(0.0) * std::f32::consts::PI
    }
}

/// What the shader reads of the changes: each place's ball (w its radius,
/// 0 for none) and whether the whole frame's light changed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Changed {
    pub places: [[f32; 4]; MAX_CHANGES],
    pub count: u32,
    pub everywhere: bool,
}

impl Changed {
    /// rt_gi.wgsl `Params::changes`.
    pub fn flags(&self) -> [f32; 4] {
        [self.count as f32, if self.everywhere { 1.0 } else { 0.0 }, CHANGED_HISTORY, 0.0]
    }
}

#[derive(Clone, Copy, Debug)]
struct Lamp {
    at: Vec3,
    reach: f32,
    falloff: f32,
    light: f32,
}

#[derive(Clone, Copy, Debug)]
struct Held {
    at: Vec3,
    reach: f32,
    frames: u32,
}

/// Each light as the history holds it, and the places still held.
#[derive(Debug, Default)]
pub(crate) struct LightChanges {
    known: Vec<Lamp>,
    sky: Option<SkyLight>,
    held: Vec<Held>,
    sky_held: u32,
    /// Which of `known` a lamp of the frame was taken for.
    matched: Vec<bool>,
}

/// Whether `now` is a quarter from `then`.
fn changed(then: f32, now: f32) -> bool {
    then.max(now) >= FAINT && (now - then).abs() > CHANGE * then.max(now)
}

/// How far from a lamp of this `reach` and `falloff` a change of `light`
/// in its brightness matters under a sky that gives a face `sky`: where
/// the lamp's light on a face (common.wgsl `ll_light_atten`,
/// 2 ((1 - d) / (1 + falloff))² at `d` of the reach) is still
/// `SIGNIFICANT` of the sky's.
fn mattering(light: f32, reach: f32, falloff: f32, sky: f32) -> f32 {
    if light <= 0.0 || reach <= 0.0 {
        return 0.0;
    }
    let least = SIGNIFICANT * sky / light;
    let d = 1.0 - (1.0 + falloff.max(0.0)) * (least * 0.5).sqrt();
    reach * d.clamp(0.0, 1.0)
}

/// Whether the sun's or the sky's light is a quarter from what the history
/// holds, or the sun elsewhere.
fn sky_changed(then: &SkyLight, now: &SkyLight) -> bool {
    let turned = match (then.sun, now.sun) {
        (Some(a), Some(b)) => a.angle_between(b) > SUN_TURN,
        _ => false,
    };
    turned || changed(then.sunlight(), now.sunlight()) || changed(then.skylight(), now.skylight())
}

/// `then` a frame's share nearer `now`, as the history takes a frame.
fn followed(then: &SkyLight, now: &SkyLight, follow: f32) -> SkyLight {
    SkyLight {
        sun: match (then.sun, now.sun) {
            (Some(a), Some(b)) => Some(a.lerp(b, follow).try_normalize().unwrap_or(b)),
            _ => now.sun,
        },
        sun_color: then.sun_color.lerp(now.sun_color, follow),
        zenith: then.zenith.lerp(now.zenith, follow),
        horizon: then.horizon.lerp(now.horizon, follow),
        ground: then.ground.lerp(now.ground, follow),
    }
}

impl LightChanges {
    /// This frame's changes from its `lights` and its `sky`, for the view
    /// of `clip_from_world` from `eye`. `follow` is a frame's share of the
    /// history (1 over its frames). Without `history` the pass starts
    /// again anyway: everything is taken as it is.
    pub fn update(
        &mut self,
        lights: &[PointLight],
        sky: SkyLight,
        clip_from_world: Mat4,
        eye: Vec3,
        follow: f32,
        history: bool,
    ) -> Changed {
        let follow = follow.clamp(0.0, 1.0);
        if !history {
            self.held.clear();
            self.sky_held = 0;
            self.known.clear();
            self.sky = None;
        }
        let ambient = sky.on_a_face();
        let before = self.known.len();
        self.matched.clear();
        self.matched.resize(before, false);
        for l in lights {
            let now = Lamp {
                at: l.position,
                reach: l.radius,
                falloff: l.falloff,
                light: l.color.dot(LUMINANCE),
            };
            if !(now.at.is_finite() && now.reach.is_finite() && now.light.is_finite()) || now.light < FAINT || now.reach <= 0.0 {
                continue;
            }
            // the nearest lamp known there that no other lamp was taken for
            let mut nearest: Option<(usize, f32)> = None;
            for (i, then) in self.known[..before].iter().enumerate() {
                let apart = then.at.distance(now.at);
                if !self.matched[i] && apart <= MOVED * then.reach.max(now.reach) && nearest.is_none_or(|(_, d)| apart < d) {
                    nearest = Some((i, apart));
                }
            }
            let Some((i, _)) = nearest else {
                if history {
                    self.hold(now.at, mattering(now.light, now.reach, now.falloff, ambient));
                }
                self.known.push(now);
                continue;
            };
            self.matched[i] = true;
            let then = self.known[i];
            if changed(then.reach, now.reach) {
                // all of its light fell elsewhere
                self.hold(
                    now.at,
                    mattering(then.light.max(now.light), then.reach.max(now.reach), now.falloff, ambient),
                );
                self.known[i] = now;
            } else if changed(then.light, now.light) {
                // what it gained or lost
                self.hold(now.at, mattering((now.light - then.light).abs(), now.reach, now.falloff, ambient));
                self.known[i] = now;
            } else {
                self.known[i] = Lamp {
                    at: then.at.lerp(now.at, follow),
                    reach: then.reach + (now.reach - then.reach) * follow,
                    falloff: now.falloff,
                    light: then.light + (now.light - then.light) * follow,
                };
            }
        }
        // a known lamp none was taken for is gone
        for i in 0..before {
            if !self.matched[i] {
                let then = self.known[i];
                self.hold(then.at, mattering(then.light, then.reach, then.falloff, ambient));
            }
        }
        let mut i = 0;
        let matched = &self.matched;
        self.known.retain(|_| {
            let keep = i >= before || matched[i];
            i += 1;
            keep
        });
        self.sky = Some(match self.sky {
            Some(then) if sky_changed(&then, &sky) => {
                self.sky_held = HELD_FRAMES;
                sky
            }
            Some(then) => followed(&then, &sky, follow),
            None => sky,
        });
        let out = self.changed(clip_from_world, eye);
        self.sky_held = self.sky_held.saturating_sub(1);
        self.held.retain_mut(|held| {
            held.frames -= 1;
            held.frames > 0
        });
        out
    }

    /// Holds the ball of `reach` about `at`; one still held that covers it
    /// is held again instead.
    fn hold(&mut self, at: Vec3, reach: f32) {
        if !reach.is_finite() || reach <= 0.0 {
            return;
        }
        match self.held.iter_mut().find(|held| held.at.distance(at) + reach <= held.reach) {
            Some(held) => held.frames = HELD_FRAMES,
            None => self.held.push(Held {
                at,
                reach,
                frames: HELD_FRAMES,
            }),
        }
    }

    /// The held places the shader takes: with more than it holds, those
    /// that reach the view first, then the nearest. The others keep their
    /// history, as every place did without this.
    fn changed(&self, clip_from_world: Mat4, eye: Vec3) -> Changed {
        let mut out = Changed {
            everywhere: self.sky_held > 0,
            ..Changed::default()
        };
        if out.everywhere {
            return out;
        }
        let mut order: Vec<usize> = (0..self.held.len()).collect();
        if order.len() > MAX_CHANGES {
            let planes = crate::gpu_cull::view_planes(clip_from_world);
            let rank = |i: &usize| {
                let held = &self.held[*i];
                let seen = planes.iter().all(|p| p.truncate().dot(held.at) + p.w >= -held.reach);
                (!seen, (held.at.distance(eye) - held.reach).max(0.0))
            };
            order.sort_by(|a, b| {
                let (a, b) = (rank(a), rank(b));
                a.0.cmp(&b.0).then(a.1.total_cmp(&b.1))
            });
        }
        for (slot, i) in out.places.iter_mut().zip(order) {
            let held = &self.held[i];
            *slot = held.at.extend(held.reach).to_array();
            out.count += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lamp(at: Vec3, light: f32) -> PointLight {
        PointLight {
            position: at,
            radius: 10.0,
            color: Vec3::splat(light),
            falloff: 0.75,
            projector: None,
            source_radius: 0.0,
            source_id: 0,
        }
    }

    fn night() -> SkyLight {
        SkyLight {
            sun: Some(Vec3::Z),
            sun_color: Vec3::splat(0.05),
            zenith: Vec3::ZERO,
            horizon: Vec3::ZERO,
            ground: Vec3::ZERO,
        }
    }

    fn noon() -> SkyLight {
        SkyLight {
            sun: Some(Vec3::Z),
            sun_color: Vec3::splat(4.0),
            zenith: Vec3::splat(0.5),
            horizon: Vec3::splat(0.4),
            ground: Vec3::splat(0.2),
        }
    }

    /// A view from the origin along -Y, as wide as it is high.
    fn view() -> Mat4 {
        glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.0, 0.1)
            * glam::camera::rh::view::look_at_mat4(Vec3::ZERO, Vec3::NEG_Y, Vec3::Z)
    }

    const FOLLOW: f32 = 1.0 / 32.0;

    fn step(c: &mut LightChanges, lights: &[PointLight], sky: SkyLight) -> Changed {
        c.update(lights, sky, view(), Vec3::ZERO, FOLLOW, true)
    }

    /// The pass's first frame, which has no history: what it finds is not
    /// a change.
    fn started(lights: &[PointLight], sky: SkyLight) -> LightChanges {
        let mut c = LightChanges::default();
        assert_eq!(c.update(lights, sky, view(), Vec3::ZERO, FOLLOW, false), Changed::default());
        c
    }

    #[test]
    fn steady_lights_change_nothing() {
        let a = [lamp(Vec3::new(0.0, -5.0, 2.0), 1.0), lamp(Vec3::new(8.0, -5.0, 2.0), 0.5)];
        let mut c = started(&a, night());
        for _ in 0..40 {
            assert_eq!(step(&mut c, &a, night()), Changed::default());
        }
        // the list in another order (the nearest first, as the eye moves)
        let b = [a[1], a[0]];
        assert_eq!(step(&mut c, &b, night()), Changed::default());
    }

    #[test]
    fn a_lamp_switched_is_held_ten_frames_where_it_shines() {
        let at = Vec3::new(0.0, -5.0, 2.0);
        let on = [lamp(at, 1.0)];
        let mut c = started(&on, night());
        step(&mut c, &on, night());
        let off = step(&mut c, &[], night());
        assert_eq!(off.count, 1);
        assert!(!off.everywhere);
        assert_eq!(off.places[0], [at.x, at.y, at.z, 10.0]);
        for _ in 1..HELD_FRAMES {
            assert_eq!(step(&mut c, &[], night()).count, 1);
        }
        assert_eq!(step(&mut c, &[], night()), Changed::default());
        // and when it comes back
        assert_eq!(step(&mut c, &on, night()).count, 1);
        // off again while still held: the same place, held anew
        step(&mut c, &on, night());
        assert_eq!(step(&mut c, &[], night()).count, 1);
        for _ in 1..HELD_FRAMES {
            assert_eq!(step(&mut c, &[], night()).count, 1);
        }
        assert_eq!(step(&mut c, &[], night()), Changed::default());
    }

    #[test]
    fn a_lamp_that_brightens_by_a_quarter_changed() {
        let at = Vec3::new(0.0, -5.0, 2.0);
        let mut c = started(&[lamp(at, 1.0)], night());
        assert_eq!(step(&mut c, &[lamp(at, 1.2)], night()).count, 0);
        assert_eq!(step(&mut c, &[lamp(at, 2.0)], night()).count, 1);
    }

    #[test]
    fn a_lamp_that_dims_slowly_is_followed() {
        let at = Vec3::new(0.0, -5.0, 2.0);
        let mut light = 1.0;
        let mut c = started(&[lamp(at, light)], night());
        // a fifth of its light a second at 60 frames a second, down to a tenth
        while light > 0.1 {
            light *= 1.0 - 0.2 / 60.0;
            assert_eq!(step(&mut c, &[lamp(at, light)], night()).count, 0);
        }
        // and one that falls as far in a few frames is not
        let mut light = 1.0;
        let mut c = started(&[lamp(at, light)], night());
        let mut held = 0;
        for _ in 0..8 {
            light *= 0.9;
            held += step(&mut c, &[lamp(at, light)], night()).count;
        }
        assert!(held > 0);
    }

    #[test]
    fn a_lamp_moved_changed_where_it_was_and_where_it_is() {
        let at = Vec3::new(0.0, -5.0, 2.0);
        let mut c = started(&[lamp(at, 1.0)], night());
        // half a metre of ten: the same lamp
        assert_eq!(step(&mut c, &[lamp(at + Vec3::X * 0.5, 1.0)], night()).count, 0);
        let there = at + Vec3::X * 6.0;
        let moved = step(&mut c, &[lamp(there, 1.0)], night());
        assert_eq!(moved.count, 2);
        let mut places = [moved.places[0], moved.places[1]];
        places.sort_by(|a, b| a[0].total_cmp(&b[0]));
        assert!((places[0][0] - 0.5 * FOLLOW).abs() < 1e-4);
        assert_eq!(places[1], [there.x, there.y, there.z, 10.0]);
    }

    #[test]
    fn a_lamp_carried_along_is_held_as_it_goes() {
        let mut at = Vec3::new(0.0, -5.0, 2.0);
        let mut c = started(&[lamp(at, 1.0)], night());
        let (mut held, mut most) = (0, 0);
        // a walk: 3.2 m a second at 60 frames a second
        for _ in 0..120 {
            at.x += 3.2 / 60.0;
            let changed = step(&mut c, &[lamp(at, 1.0)], night());
            held += changed.count.min(1);
            most = most.max(changed.count);
        }
        // each time the history is a metre behind, a tenth of its reach
        assert!((20..=60).contains(&held), "{held}");
        assert!(most <= 4, "{most}");
        // and let go once it stands
        for _ in 0..HELD_FRAMES {
            step(&mut c, &[lamp(at, 1.0)], night());
        }
        assert_eq!(step(&mut c, &[lamp(at, 1.0)], night()), Changed::default());
    }

    #[test]
    fn the_day_going_by_changes_nothing_and_a_new_hour_everything() {
        let mut sky = noon();
        let mut c = started(&[], sky);
        // a four hour day at 60 frames a second, for ten seconds
        let turn = std::f32::consts::TAU / (4.0 * 3600.0 * 60.0);
        for i in 0..600 {
            sky.sun = Some(Vec3::new((turn * i as f32).sin(), 0.0, (turn * i as f32).cos()));
            assert_eq!(step(&mut c, &[], sky), Changed::default());
        }
        // the sun moved by hand
        sky.sun = Some(Vec3::new(0.5, 0.0, 0.866).normalize());
        let jumped = step(&mut c, &[], sky);
        assert!(jumped.everywhere);
        for _ in 1..HELD_FRAMES {
            assert!(step(&mut c, &[], sky).everywhere);
        }
        assert_eq!(step(&mut c, &[], sky), Changed::default());
        // the sky's light, and the sun set
        assert!(step(&mut c, &[], night()).everywhere);
        for _ in 0..HELD_FRAMES {
            step(&mut c, &[], night());
        }
        let dark = SkyLight { sun: None, ..night() };
        assert!(step(&mut c, &[], dark).everywhere);
    }

    #[test]
    fn a_turn_spread_over_frames_counts_once_the_history_is_behind() {
        let mut sky = noon();
        let mut c = started(&[], sky);
        let mut everywhere = false;
        // 0.01 rad a frame: none of the steps is SUN_TURN
        for i in 1..=20 {
            let a = 0.01 * i as f32;
            sky.sun = Some(Vec3::new(a.sin(), 0.0, a.cos()));
            everywhere |= step(&mut c, &[], sky).everywhere;
        }
        assert!(everywhere);
    }

    #[test]
    fn a_change_reaches_less_under_a_bright_sky() {
        assert_eq!(mattering(1.0, 10.0, 0.75, 0.0), 10.0);
        let by_day = mattering(1.0, 10.0, 0.75, noon().on_a_face());
        assert!(by_day > 1.0 && by_day < 6.0, "{by_day}");
        // a faint lamp at noon changes nothing the indirect light shows
        assert_eq!(mattering(0.01, 10.0, 0.75, noon().on_a_face()), 0.0);
        let at = Vec3::new(0.0, -5.0, 2.0);
        let mut c = started(&[lamp(at, 0.01)], noon());
        assert_eq!(step(&mut c, &[], noon()), Changed::default());
    }

    #[test]
    fn with_more_places_than_slots_those_in_view_come_first() {
        // twelve lamps behind the eye, nearer than the six ahead of it
        let mut lights: Vec<PointLight> = (0..12).map(|i| lamp(Vec3::new(i as f32 * 30.0 - 165.0, 20.0, 0.0), 1.0)).collect();
        lights.extend((0..6).map(|i| lamp(Vec3::new(i as f32 * 10.0 - 25.0, -60.0, 0.0), 1.0)));
        let mut c = started(&lights, night());
        let all = step(&mut c, &[], night());
        assert_eq!(all.count as usize, MAX_CHANGES);
        let ahead = all.places.iter().filter(|p| p[1] < 0.0).count();
        assert_eq!(ahead, 6);
    }

    #[test]
    fn without_history_nothing_is_held() {
        let at = Vec3::new(0.0, -5.0, 2.0);
        let mut c = started(&[lamp(at, 1.0)], noon());
        // the targets were made again as the lamp went and the night came
        assert_eq!(c.update(&[], night(), view(), Vec3::ZERO, FOLLOW, false), Changed::default());
        assert_eq!(step(&mut c, &[], night()), Changed::default());
    }

    #[test]
    fn a_light_that_is_no_number_is_left_out() {
        let mut c = started(&[], night());
        let mut bad = lamp(Vec3::new(f32::NAN, 0.0, 0.0), 1.0);
        assert_eq!(step(&mut c, &[bad], night()), Changed::default());
        bad.position = Vec3::ZERO;
        bad.color = Vec3::splat(f32::INFINITY);
        assert_eq!(step(&mut c, &[bad], night()), Changed::default());
        assert_eq!(step(&mut c, &[], night()), Changed::default());
    }
}
