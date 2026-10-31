//! Photo quality: while the photo window is open with its "Photo quality"
//! switch on, the graphics settings are raised to the most Aurora renders
//! (draw distance, levels of detail, every avatar in full, shadows,
//! ambient occlusion, reflections at full resolution, anisotropy), the
//! textures in view are loaded at full resolution and the photo itself is
//! drawn at twice its size (`app::snapshot`), the user's own values set
//! aside.
//! Firestorm has no such switch: its Phototools change the real settings
//! for good. Here the user's values are what is saved while the boost is
//! on (`Settings::save`), and they come back when the window closes.
//!
//! When video memory is tight the boost lowers itself to stay under the
//! budget the driver grants (#1374: a busy region after hours in-world,
//! 4,431 MB taken of a budget fallen to 5,280 MB, textures at pressure
//! 6 / 7; MSAA 8x at 3440 x 1369 and the rest of the boost wanted about
//! 1.4 GB more, and the GPU was lost 1.4 s later). See [`plan`].

use crate::graphics_presets::{Preset, Values};
use crate::scene::texture_budget::MAX_PRESSURE;
use crate::settings::Settings;
use std::collections::BTreeSet;

/// The user's values while the boost is on.
#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    values: Values,
    /// The shadows switch, which the level's values do not carry.
    shadows: bool,
    /// What the boost wrote; a setting found otherwise was changed by the
    /// user (Preferences) and is theirs from then on.
    boosted: Values,
    boosted_shadows: bool,
    /// The settings the user changed while the boost is on: theirs, never
    /// moved by the video memory's steps (`Held::retarget`).
    adopted: BTreeSet<&'static str>,
}

/// Every field of `Values`, for `Held::sync`.
macro_rules! adopt {
    ($held:expr, $now:expr, $($f:ident),*) => {
        $(
            if $now.$f != $held.boosted.$f {
                $held.values.$f = $now.$f;
                $held.boosted.$f = $now.$f;
                $held.adopted.insert(stringify!($f));
            }
        )*
    };
}

/// Every field of `Values`, for `Held::retarget`: the step's value where
/// the setting is still the boost's.
macro_rules! steer {
    ($held:expr, $next:expr, $to:expr, $($f:ident),*) => {
        $(
            if !$held.adopted.contains(stringify!($f)) {
                $next.$f = $to.$f;
            }
        )*
    };
}

/// The fields both macros go through.
macro_rules! each_value {
    ($m:ident, $($args:expr),*) => {
        $m!(
            $($args),*,
            draw_distance,
            lod_factor,
            max_avatars,
            max_complexity,
            max_particles,
            shadow_quality,
            projector_shadows,
            ssao,
            ao_method,
            gtao_quality,
            water_reflections,
            mirrors,
            ssr,
            reflection_probes,
            probe_count,
            antialiasing,
            anisotropy,
            sharpen,
            glow,
            effects
        )
    };
}

impl Held {
    /// The user's current values without cloning the entire preferences or
    /// their saved profile library for the FPS hover card's active marker.
    pub(crate) fn user_values(&self, s: &Settings) -> (Values, bool) {
        let mut user = self.clone();
        user.sync(s);
        (user.values, user.shadows)
    }

    /// A setting the user changed while the boost is on (in Preferences)
    /// becomes their value: it is kept when the boost ends, and no longer
    /// raised while it lasts.
    pub fn sync(&mut self, s: &Settings) {
        let now = Values::read(s);
        each_value!(adopt, self, now);
        if s.shadows != self.boosted_shadows {
            self.shadows = s.shadows;
            self.boosted_shadows = s.shadows;
            self.adopted.insert("shadows");
        }
    }

    /// The user's own values (with what they changed meanwhile, once
    /// `sync` has run).
    pub fn user(&self) -> Values {
        self.values
    }

    /// Write `to` (a step of [`ladder`] of the user's values) in `s`, for
    /// every setting still the boost's: one the user changed meanwhile
    /// stays theirs.
    pub fn retarget(&mut self, s: &mut Settings, to: Values) {
        self.sync(s);
        let mut next = Values::read(s);
        each_value!(steer, self, next, to);
        let shadows = s.shadows;
        next.write(s);
        s.shadows = if self.adopted.contains("shadows") {
            shadows
        } else if s.shadow_quality == self.values.shadow_quality && s.shadow_quality != boosted(self.values).shadow_quality {
            // lowered back to the user's own level: their own switch with it
            self.shadows
        } else {
            s.shadow_quality > 0
        };
        self.boosted = Values::read(s);
        self.boosted_shadows = s.shadows;
    }

    /// Put the user's values back in `s` (the boost's end, or the copy that
    /// is saved).
    pub fn restore(&self, s: &mut Settings) {
        self.values.write(s);
        s.shadows = self.shadows;
    }
}

/// The most each setting goes to (the top of its range in Aurora; Firestorm
/// allows a draw distance of 1,024 m and a LOD factor of 8 by hand, Aurora
/// draws up to 512 m and 4). Every avatar is drawn in full (no impostor up
/// to Aurora's 100, no complexity limit): a photo shows people as they are.
fn photo_maxima() -> Values {
    Values {
        draw_distance: 512.0,
        lod_factor: 4.0,
        max_avatars: 100,
        max_complexity: 0,
        shadow_quality: 4,
        projector_shadows: true,
        ssao: 2,
        water_reflections: 4,
        mirrors: 4,
        reflection_probes: 3,
        probe_count: 64,
        anisotropy: 16,
        ..Preset::Deluxe.values()
    }
}

/// `Settings::antialiasing` for MSAA 8x, the photo's anti-aliasing on top
/// of the supersampling (asked for by the user; the renderer takes the most
/// samples the GPU supports up to 8).
pub const MSAA_8X: u8 = 4;

/// The boosted values: each one at Aurora's most, never lower than the
/// user's (an unlimited complexity stays unlimited), the anti-aliasing at
/// MSAA 8x. The particles, the screen-space reflections, the sharpening, the
/// glow and Aurora's effects (bloom, contact shadows, indirect light) stay
/// the user's own choices.
pub fn boosted(user: Values) -> Values {
    let top = photo_maxima();
    Values {
        antialiasing: MSAA_8X,
        draw_distance: user.draw_distance.max(top.draw_distance),
        lod_factor: user.lod_factor.max(top.lod_factor),
        max_avatars: user.max_avatars.max(top.max_avatars),
        max_complexity: top.max_complexity,
        shadow_quality: user.shadow_quality.max(top.shadow_quality),
        projector_shadows: user.projector_shadows || top.projector_shadows,
        ssao: user.ssao.max(top.ssao),
        water_reflections: user.water_reflections.max(top.water_reflections),
        mirrors: user.mirrors.max(top.mirrors),
        reflection_probes: user.reflection_probes.max(top.reflection_probes),
        probe_count: user.probe_count.max(top.probe_count),
        anisotropy: user.anisotropy.max(top.anisotropy),
        ..user
    }
}

/// The photo is drawn at this many times its size, then reduced (a box
/// filter over whole pixels), while its larger side stays within
/// `snapshot::MAX_SIDE`: an 8K photo is drawn once, a 4K one at 8K.
pub fn supersample(size: (u32, u32)) -> u32 {
    if size.0.max(size.1) * 2 <= super::MAX_SIDE { 2 } else { 1 }
}

/// The longest wait for the textures in view to reach full resolution
/// before the photo is taken anyway.
pub const DETAIL_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// Raise `s`, returning what to restore.
pub fn boost(s: &mut Settings) -> Held {
    let values = Values::read(s);
    let shadows = s.shadows;
    boosted(values).write(s);
    Held {
        values,
        shadows,
        boosted: Values::read(s),
        boosted_shadows: s.shadows,
        adopted: BTreeSet::new(),
    }
}

/// `Settings::antialiasing` for MSAA 4x and 2x.
pub const MSAA_4X: u8 = 3;
pub const MSAA_2X: u8 = 2;

/// MSAA samples of an anti-aliasing setting (1: no multisampled targets).
pub fn samples(antialiasing: u8) -> u32 {
    match antialiasing {
        2 => 2,
        3 => 4,
        4 => 8,
        _ => 1,
    }
}

/// The steps of the boost, from the whole of it (step 0) to the user's own
/// values (the last), each one setting back one more thing: the MSAA
/// first (8x, 4x, 2x, then the user's own anti-aliasing), then the draw
/// distance (384, 256, 128 m, then the user's), then the shadows (level 3,
/// a quarter of level 4's atlas, then the user's), then everything else
/// (reflections, probes, details). Never below the user's own: a step that
/// would change nothing is left out.
pub fn ladder(user: Values) -> Vec<Values> {
    let mut steps = vec![boosted(user)];
    let mut then = |change: &dyn Fn(&mut Values)| {
        let mut v = *steps.last().expect("the ladder starts with the boost");
        change(&mut v);
        if Some(&v) != steps.last() {
            steps.push(v);
        }
    };
    for aa in [MSAA_4X, MSAA_2X] {
        if samples(aa) > samples(user.antialiasing) {
            then(&|v| v.antialiasing = aa);
        }
    }
    then(&|v| v.antialiasing = user.antialiasing);
    for d in [384.0, 256.0, 128.0] {
        if d > user.draw_distance {
            then(&|v| v.draw_distance = d);
        }
    }
    then(&|v| v.draw_distance = user.draw_distance);
    if user.shadow_quality < 3 {
        then(&|v| v.shadow_quality = 3);
    }
    then(&|v| v.shadow_quality = user.shadow_quality);
    then(&|v| *v = user);
    steps
}

const MB: u64 = 1 << 20;

/// Bytes of video memory per MSAA sample of a pixel: the multisampled HDR
/// colour (Rgba16Float, 8), depth (Depth32Float, 4) and G-buffer
/// (Rgba8Unorm, 4, with the screen-space reflections) of
/// `Renderer::make_targets`, with some room; also what a photo's pass
/// counts (`app::snapshot`).
pub const BYTES_PER_SAMPLE: u64 = 18;

/// Bytes per square metre of ground a longer draw distance brings in
/// (geometry, the textures' share the budget lets through), from a grid
/// session in KB: the photo quality's 512 m grew the index arena from
/// 256 to 512 MB and the other buffers by about 50 MB, some 300 MB for
/// the 620,000 m² between 256 and 512 m.
const GROUND_BYTES_PER_M2: f64 = 512.0;

/// Bytes of one reflection probe (six faces of 128 x 128 Rgba16Float with
/// their levels, and its irradiance).
const PROBE_BYTES: u64 = 1_100_000;

/// The window the boost draws in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Screen {
    pub pixels: u64,
    /// The most MSAA samples the GPU supports (the renderer takes no more).
    pub max_samples: u32,
}

/// Estimated video memory the settings `v` hold that grows with the boost.
fn held_bytes(v: &Values, screen: Screen) -> u64 {
    let px = screen.pixels as f64;
    let n = samples(v.antialiasing).min(screen.max_samples.max(1));
    let msaa = if n > 1 {
        u64::from(n) * BYTES_PER_SAMPLE * screen.pixels
    } else {
        0
    };
    // the atlas holds four tiles of the resolution, the merge's scratch a
    // quarter of one (Renderer::make_shadow_atlas, ShadowMerge), Depth32Float
    let res = u64::from(Settings::shadow_params_of(v.shadow_quality).0);
    let shadows = 17 * res * res;
    let ground = std::f64::consts::PI * f64::from(v.draw_distance).powi(2) * GROUND_BYTES_PER_M2;
    // HDR colour and depth at a fraction of the screen
    let reflection = |step: u8| (px * f64::from(Settings::reflection_scale(step)).powi(2) * 12.0) as u64;
    let probes = u64::from(v.probe_count.clamp(8, 64)) * PROBE_BYTES;
    msaa + shadows + ground as u64 + reflection(v.water_reflections) + reflection(v.mirrors) + probes
}

/// Estimated bytes the values `v` take beyond the user's own.
pub fn extra_bytes(user: &Values, v: &Values, screen: Screen) -> u64 {
    held_bytes(v, screen).saturating_sub(held_bytes(user, screen))
}

/// Video memory as the driver tells it, and the textures' pressure step
/// (`scene::texture_budget`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Memory {
    pub budget: u64,
    pub usage: u64,
    pub pressure: u8,
}

/// What stays free under the budget: a tenth of it, at least 512 MB, as
/// for the geometry grown ahead (`aurora_render::gpu_alloc`), for the
/// photo's own passes and what the scene loads meanwhile; and up to a
/// tenth more as the textures are pressed: their budget gives the boost's
/// memory back from them, and the more they are already lowered, the less
/// the photo takes from them (at the top step they have nothing left).
pub fn margin(m: Memory) -> u64 {
    let tenth = m.budget / 10;
    tenth.max(512 * MB) + tenth * u64::from(m.pressure.min(MAX_PRESSURE)) / u64::from(MAX_PRESSURE)
}

/// Room a step up needs beyond its own estimate: the readings move by a
/// few hundred MB (textures arriving, the driver's budget), and a step
/// taken back at once would rebuild the render targets twice.
pub fn hysteresis(budget: u64) -> u64 {
    (budget / 20).max(256 * MB)
}

/// The step of [`ladder`] to draw with: the first whose estimated cost
/// (`costs`, by step, beyond the user's own values) fits in the budget
/// with its [`margin`]. `current` is the step in use (its cost is in
/// `usage` already; None: the boost is not on yet). Down at once, as far
/// as needed; up one step at a time, only with [`hysteresis`] of room to
/// spare and when `may_raise`. Without figures from the driver, the whole
/// boost, as before.
pub fn choose(memory: Option<Memory>, costs: &[u64], current: Option<usize>, may_raise: bool) -> usize {
    let Some(m) = memory else {
        return 0;
    };
    let last = costs.len().saturating_sub(1);
    let current = current.map(|c| c.min(last));
    let held = current.map_or(0, |c| costs[c]);
    // what the user's own settings hold, and the room above it
    let base = m.usage.saturating_sub(held);
    let room = m.budget.saturating_sub(margin(m)).saturating_sub(base);
    let fits = |i: usize| costs[i] <= room;
    match current {
        None => (0..=last).find(|&i| fits(i)).unwrap_or(last),
        Some(c) if !fits(c) => (c..=last).find(|&i| fits(i)).unwrap_or(last),
        Some(c) if may_raise && c > 0 && costs[c - 1] + hysteresis(m.budget) <= room => c - 1,
        Some(c) => c,
    }
}

/// The photo quality's step for the video memory of the moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    /// Of [`ladder`]: 0 the whole boost.
    pub step: usize,
    pub steps: usize,
    pub values: Values,
    /// Its estimated bytes beyond the user's own values.
    pub extra: u64,
}

impl Plan {
    /// The boost is held back for the memory.
    pub fn reduced(&self) -> bool {
        self.step > 0
    }
}

/// [`choose`] over the [`ladder`] of the user's values on this screen.
pub fn plan(memory: Option<Memory>, user: Values, screen: Screen, current: Option<usize>, may_raise: bool) -> Plan {
    let steps = ladder(user);
    let costs: Vec<u64> = steps.iter().map(|v| extra_bytes(&user, v, screen)).collect();
    let step = choose(memory, &costs, current, may_raise);
    Plan {
        step,
        steps: steps.len(),
        values: steps[step],
        extra: costs[step],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_boost_raises_without_lowering_and_restores_exactly() {
        let mut s = Settings {
            draw_distance: 64.0,
            lod_factor: 4.0,
            shadow_quality: 0,
            shadows: false,
            antialiasing: 3,
            max_complexity: 80_000,
            ..Settings::default()
        };
        let before = s.clone();
        let held = boost(&mut s);
        assert_eq!((s.draw_distance, s.lod_factor, s.max_avatars), (512.0, 4.0, 100));
        assert_eq!((s.shadow_quality, s.water_reflections, s.mirrors), (4, 4, 4));
        assert!(s.shadows);
        // no complexity limit during the photo, MSAA 8x
        assert_eq!((s.antialiasing, s.max_complexity), (MSAA_8X, 0));
        held.restore(&mut s);
        assert_eq!(Values::read(&s), Values::read(&before));
        assert_eq!(s.shadows, before.shadows);
    }

    #[test]
    fn a_setting_changed_during_the_boost_becomes_the_users() {
        let mut s = Settings {
            draw_distance: 96.0,
            shadow_quality: 1,
            antialiasing: 3,
            ..Settings::default()
        };
        let mut held = boost(&mut s);
        // the user lowers the draw distance and changes the anti-aliasing
        s.draw_distance = 128.0;
        s.antialiasing = 6;
        held.sync(&s);
        // a second sync with nothing new changes nothing
        held.sync(&s);
        // their values stay while the boost lasts, the others stay raised
        assert_eq!((s.draw_distance, s.shadow_quality), (128.0, 4));
        held.restore(&mut s);
        // and are theirs afterwards; the untouched ones come back
        assert_eq!((s.draw_distance, s.antialiasing, s.shadow_quality), (128.0, 6, 1));
    }

    #[test]
    fn photos_up_to_4k_are_drawn_at_twice_their_size() {
        assert_eq!(supersample((3840, 2160)), 2);
        assert_eq!(supersample((1600, 900)), 2);
        assert_eq!(supersample((5120, 2880)), 1);
        assert_eq!(supersample((7680, 4320)), 1);
    }

    const MB: u64 = 1 << 20;

    fn screen(w: u64, h: u64) -> Screen {
        Screen {
            pixels: w * h,
            max_samples: 8,
        }
    }

    fn memory(budget_mb: u64, usage_mb: u64, pressure: u8) -> Option<Memory> {
        Some(Memory {
            budget: budget_mb * MB,
            usage: usage_mb * MB,
            pressure,
        })
    }

    fn user() -> Values {
        Values::read(&Settings::default())
    }

    #[test]
    fn the_ladder_lowers_one_thing_at_a_time_down_to_the_users_own() {
        let user = user();
        let steps = ladder(user);
        assert_eq!(steps[0], boosted(user));
        assert_eq!(*steps.last().expect("steps"), user);
        // MSAA 8x, 4x, 2x, SMAA; 512, 384, 256, 128, 96 m; shadows 4, 3 (the
        // user's); then the rest
        let aa: Vec<u8> = steps.iter().map(|v| v.antialiasing).collect();
        assert_eq!(&aa[..4], &[MSAA_8X, MSAA_4X, MSAA_2X, user.antialiasing]);
        let dd: Vec<f32> = steps[3..].iter().map(|v| v.draw_distance).collect();
        assert_eq!(&dd[..5], &[512.0, 384.0, 256.0, 128.0, 96.0]);
        assert_eq!(steps[8].shadow_quality, 3);
        assert_eq!(steps.len(), 10);
        // never below the user's own
        for v in &steps {
            assert!(v.draw_distance >= user.draw_distance && v.shadow_quality >= user.shadow_quality);
        }
        // a user already at MSAA 4x and 256 m: no step under those
        let mine = Values {
            antialiasing: MSAA_4X,
            draw_distance: 256.0,
            shadow_quality: 1,
            ..user
        };
        let steps = ladder(mine);
        let aa: Vec<u8> = steps.iter().map(|v| v.antialiasing).collect();
        assert!(!aa.contains(&MSAA_2X));
        assert!(steps.iter().all(|v| v.draw_distance >= 256.0 && v.antialiasing >= MSAA_4X));
        let shadows: Vec<u8> = steps.iter().map(|v| v.shadow_quality).collect();
        assert!(shadows.contains(&3) && shadows.ends_with(&[1]));
    }

    #[test]
    fn the_estimates_follow_the_render_targets() {
        let user = user();
        let s = screen(3440, 1369);
        let only_msaa = |aa| extra_bytes(&user, &Values { antialiasing: aa, ..user }, s);
        // 8 samples of 18 bytes over 4.7 Mpx: 678 MB; 4x half, 2x a quarter
        assert_eq!(only_msaa(MSAA_8X), 8 * 18 * 3440 * 1369);
        assert_eq!(only_msaa(MSAA_4X), only_msaa(MSAA_8X) / 2);
        assert_eq!(only_msaa(user.antialiasing), 0);
        // a GPU with 4 samples at most
        let four = Screen { max_samples: 4, ..s };
        assert_eq!(
            extra_bytes(
                &user,
                &Values {
                    antialiasing: MSAA_8X,
                    ..user
                },
                four
            ),
            only_msaa(MSAA_4X)
        );
        // shadows 4 over 3: an atlas four times larger
        let shadows = extra_bytes(&user, &Values { shadow_quality: 4, ..user }, s);
        assert_eq!(shadows, 17 * (4096 * 4096 - 2048 * 2048));
        // 512 m over 96 m: about 400 MB of ground
        let ground = extra_bytes(
            &user,
            &Values {
                draw_distance: 512.0,
                ..user
            },
            s,
        ) / MB;
        assert!((380..430).contains(&ground), "{ground} MB");
        // the whole boost there: about 1.4 GB
        let all = extra_bytes(&user, &boosted(user), s) / MB;
        assert!((1300..1550).contains(&all), "{all} MB");
    }

    #[test]
    fn memory_to_spare_keeps_the_whole_boost() {
        // a 24 GB card in a busy region, or one with no figures from the driver
        let p = plan(memory(23_370, 9_000, 0), user(), screen(2560, 1368), None, false);
        assert_eq!(p.step, 0);
        assert!(!p.reduced());
        assert_eq!(p.values, boosted(user()));
        assert_eq!(plan(None, user(), screen(7680, 4320), None, false).step, 0);
        // under pressure there, the textures cycling with the full resolution
        let p = plan(memory(22_233, 19_500, 1), user(), screen(2560, 1368), Some(0), false);
        assert_eq!(p.step, 0);
    }

    #[test]
    fn the_reports_cases_fall_back_to_the_users_own() {
        let user = user();
        // #1374's report: 4,431 MB of a budget fallen to 5,280, pressure 6
        let p = plan(memory(5_280, 4_431, 6), user, screen(3440, 1369), None, false);
        assert_eq!(p.step, p.steps - 1);
        assert_eq!((p.values, p.extra), (user, 0));
        // #1374: already over the budget
        let p = plan(memory(10_312, 10_415, 7), user, screen(1920, 1080), None, false);
        assert_eq!(p.values, user);
        // the same card earlier, budget still 7,249 MB and the textures
        // fine: part of the boost, the MSAA first
        let p = plan(memory(7_249, 5_800, 0), user, screen(3440, 1369), None, false);
        assert!(p.reduced() && p.step < p.steps - 1, "{p:?}");
        assert!(p.values.antialiasing != MSAA_8X);
        assert!(p.extra <= 7_249 * MB - margin(memory(7_249, 5_800, 0).expect("m")) - 5_800 * MB);
    }

    #[test]
    fn a_step_down_is_at_once_a_step_up_waits_for_room_to_spare() {
        let costs = [1_000 * MB, 700 * MB, 500 * MB, 200 * MB, 0];
        // an 8 GB budget: 819 MB of margin, 410 MB of hysteresis
        let budget = 8_192 * MB;
        let reading = |usage: u64, pressure: u8| Some(Memory { budget, usage, pressure });
        let margin0 = margin(Memory {
            budget,
            usage: 0,
            pressure: 0,
        });
        // the usage that leaves `room` MB above the user's own settings,
        // with `held` bytes of the boost in it
        let usage = |room: u64, held: u64| budget - margin0 - room * MB + held;
        // the boost on at step 0 with its 1,000 MB, the budget shrinks:
        // straight to the step that fits
        assert_eq!(choose(reading(usage(250, costs[0]), 0), &costs, Some(0), true), 3);
        // held at step 3: 500 MB would fit in 600, not with the hysteresis
        assert_eq!(choose(reading(usage(600, costs[3]), 0), &costs, Some(3), true), 3);
        // with room to spare: one step up, and only when allowed
        assert_eq!(choose(reading(usage(1_000, costs[3]), 0), &costs, Some(3), true), 2);
        assert_eq!(choose(reading(usage(1_000, costs[3]), 0), &costs, Some(3), false), 3);
        // the same room read again once that step holds its 500 MB: it stays
        assert_eq!(choose(reading(usage(1_000, costs[2]), 0), &costs, Some(2), true), 2);
        // readings between the two never move it either way
        for room in [500, 600, 700, 800, 900] {
            assert_eq!(choose(reading(usage(room, costs[3]), 0), &costs, Some(3), true), 3, "{room}");
        }
        // the textures pressed hard: more margin, a step down
        assert_eq!(choose(reading(usage(250, costs[3]), 7), &costs, Some(3), true), 4);
        // over the budget: the last step
        assert_eq!(choose(reading(budget + 500 * MB, 0), &costs, None, false), 4);
    }

    #[test]
    fn a_lowered_step_keeps_what_the_user_changed() {
        let mut s = Settings {
            draw_distance: 96.0,
            shadow_quality: 2,
            shadows: true,
            antialiasing: 6,
            ..Settings::default()
        };
        let before = s.clone();
        let mut held = boost(&mut s);
        // the user picks MSAA 4x in Preferences meanwhile
        s.antialiasing = MSAA_4X;
        held.sync(&s);
        // the memory lowers the boost to its last step
        let steps = ladder(held.user());
        held.retarget(&mut s, *steps.last().expect("steps"));
        assert_eq!((s.draw_distance, s.shadow_quality, s.antialiasing), (96.0, 2, MSAA_4X));
        assert!(s.shadows);
        // a sync after the step adopts nothing: the boost's own move
        held.sync(&s);
        // back up to the whole boost: the user's choice still stands
        held.retarget(&mut s, steps[0]);
        assert_eq!((s.draw_distance, s.shadow_quality, s.antialiasing), (512.0, 4, MSAA_4X));
        held.restore(&mut s);
        assert_eq!((s.draw_distance, s.shadow_quality, s.antialiasing), (96.0, 2, MSAA_4X));
        assert_eq!(s.lod_factor, before.lod_factor);
        // a user without shadows gets them back off at their own level
        let mut s = Settings {
            shadow_quality: 2,
            shadows: false,
            ..Settings::default()
        };
        let mut held = boost(&mut s);
        assert!(s.shadows);
        held.retarget(&mut s, held.user());
        assert!(!s.shadows);
    }

    #[test]
    fn a_boosted_profile_saves_the_users_values() {
        let mut s = Settings {
            draw_distance: 96.0,
            ..Settings::default()
        };
        s.photo_held = Some(boost(&mut s));
        assert_eq!(s.draw_distance, 512.0);
        let saved = s.to_saved();
        assert_eq!(saved.draw_distance, 96.0);
        assert_eq!(saved.photo_held, None);
        // and the held values never reach the file
        let json = serde_json::to_string(&s).expect("serialize");
        assert!(!json.contains("photo_held"));
    }
}
