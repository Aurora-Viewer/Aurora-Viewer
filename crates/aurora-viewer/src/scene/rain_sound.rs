//! Rain prototype: the sound of the rain (Ambient channel). Not a sound at a
//! place: a bed around the listener that leans toward the open sky.
//!
//! Four "speakers" stand in fixed directions around the listener (east,
//! north, west, south), each playing the light and the heavy loops
//! (`assets/rain`) from a different point, so that they sound like four
//! patches of rain rather than one. The rain slider crossfades the loops at
//! equal power (a drizzle's patter to a downpour's hiss) and sets the level.
//! Each speaker is as loud as the sky is open on its side (the reverb's
//! rays, `sound_reverb.rs` `Sky`): in a field all four, in a doorway the
//! door's side. Enclosed, the speakers go through the mixer's occlusion (a
//! dull rumble, as from inside a house), and a roof close above adds the
//! drumming of rain on it. Changes glide over about half a second; the
//! loops decode once, on a thread, the first time it rains.
//!
//! Open slots: a drizzle outdoors, and light rain and a drizzle heard from
//! indoors, are recordings still wanted (assets/rain/README.md, "Wanted").
//! Until one is there, the sound it would play falls back to what plays
//! today (the light loop, muffled indoors); once there, it takes the
//! lightest share of the mix at equal power (`drizzle_share`).

use super::sound_reverb::Sky;
use aurora_audio::{AudioEngine, Channel, SoundClip};
use glam::{Vec2, Vec3};
use std::sync::{Once, OnceLock};

const LIGHT: &[u8] = include_bytes!("../../../../assets/rain/rain-light.ogg");
const HEAVY: &[u8] = include_bytes!("../../../../assets/rain/rain-heavy.ogg");
const ROOF: &[u8] = include_bytes!("../../../../assets/rain/rain-roof.ogg");
// The open slots. To fill one, put its file in assets/rain (levelled as the
// README says) and set it to `Some(include_bytes!("../../../../assets/rain/<file>"))`.
/// `rain-drizzle.ogg`: the lightest rain outdoors, drips and a sparse
/// patter. Falls back to the light loop.
const DRIZZLE: Option<&[u8]> = None;
/// `rain-roof-light.ogg`: light rain on a roof, heard from inside. Falls
/// back to the light loop, muffled.
const ROOF_LIGHT: Option<&[u8]> = None;
/// `rain-roof-drizzle.ogg`: a drizzle heard from inside, drips off the
/// eaves. Falls back to the light rain indoors.
const ROOF_DRIZZLE: Option<&[u8]> = None;

/// What the light and heavy loops lack of the roof's level (dB): they could
/// not be raised that far without clipping (assets/rain/README.md).
const LIGHT_MAKEUP_DB: f32 = 0.0;
const HEAVY_MAKEUP_DB: f32 = 6.2;
/// Overall level of the rain before the Ambient channel's volume.
const LEVEL: f32 = 0.55;
/// The speakers' directions (world: east, north, west, south).
const SPEAKERS: [Vec2; 4] = [Vec2::X, Vec2::Y, Vec2::NEG_X, Vec2::NEG_Y];
/// Voice ids, far from the world sounds' (counted from 1).
const VOICE_BASE: u64 = 0x7261_696E_0000_0000;
/// How muffled the light loop is as a drizzle on a roof (the mixer's
/// occlusion: 0.5 one wall, 1 two).
const LIGHT_ROOF_OCCLUSION: f32 = 0.8;
/// Voices: 4 light, 4 heavy, 2 roof (heavy), 2 roof (light), then the
/// open slots: 4 drizzle, 2 roof (drizzle).
const VOICES: u64 = 18;
const V_DRIZZLE: u64 = 12;
const V_ROOF_DRIZZLE: u64 = 16;
/// Changes glide over about this long (s).
const GLIDE_SECS: f32 = 0.5;

struct Clips {
    light: [SoundClip; 4],
    heavy: [SoundClip; 4],
    roof: [SoundClip; 2],
    /// Light rain on a roof: its own recording, or the light loop again,
    /// muffled (`light_roof_muffled`).
    light_roof: [SoundClip; 2],
    light_roof_muffled: bool,
    /// The open slots' recordings, when there.
    drizzle: Option<[SoundClip; 4]>,
    drizzle_roof: Option<[SoundClip; 2]>,
}

fn load() -> Option<Clips> {
    let decode = |name: &str, bytes: &[u8]| SoundClip::decode(bytes).map_err(|e| log::warn!("rain sound {name}: {e}")).ok();
    let light = decode("light", LIGHT)?;
    let heavy = decode("heavy", HEAVY)?;
    let roof = decode("roof", ROOF)?;
    // an open slot that does not decode is left to its fallback
    let roof_light = ROOF_LIGHT.and_then(|b| decode("roof light", b));
    Some(Clips {
        light: [0.0, 0.25, 0.5, 0.75].map(|f| light.rotated(f)),
        heavy: [0.13, 0.38, 0.63, 0.88].map(|f| heavy.rotated(f)),
        roof: [0.0, 0.5].map(|f| roof.rotated(f)),
        light_roof_muffled: roof_light.is_none(),
        light_roof: [0.4, 0.9].map(|f| roof_light.as_ref().unwrap_or(&light).rotated(f)),
        drizzle: DRIZZLE
            .and_then(|b| decode("drizzle", b))
            .map(|c| [0.06, 0.31, 0.56, 0.81].map(|f| c.rotated(f))),
        drizzle_roof: ROOF_DRIZZLE
            .and_then(|b| decode("roof drizzle", b))
            .map(|c| [0.2, 0.7].map(|f| c.rotated(f))),
    })
}

/// The share of the light end of the mix that goes to the drizzle, at
/// `rain` (0..1, as the loops' mix leans): all of it in the faintest rain,
/// none from about a third up.
fn drizzle_share(rain: f32) -> f32 {
    1.0 - smoothstep(0.08, 0.35, rain)
}

/// `gain` split between the drizzle and the light loop at equal power;
/// all of it to the light loop while the drizzle's slot is empty.
fn split_drizzle(gain: f32, rain: f32, drizzle: bool) -> (f32, f32) {
    if !drizzle {
        return (0.0, gain);
    }
    let s = drizzle_share(rain);
    (gain * s.sqrt(), gain * (1.0 - s).sqrt())
}

/// The loops, once decoded (none until then, or if they cannot be).
fn clips() -> Option<&'static Clips> {
    static CLIPS: OnceLock<Option<Clips>> = OnceLock::new();
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            let _ = CLIPS.set(load());
        });
    });
    CLIPS.get().and_then(Option::as_ref)
}

/// What the rain sound depends on, this frame.
#[derive(Debug, Clone, Copy)]
pub struct RainSoundInput {
    /// How hard it rains (0..1).
    pub rain: f32,
    /// The sky around the listener, when measured.
    pub sky: Option<Sky>,
    /// Where the listener looks (for the panning).
    pub forward: Vec3,
    /// How deep the listener is under the water (m; 0 above it).
    pub underwater: f32,
    /// In the world, not a silent demo.
    pub audible: bool,
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Default)]
pub struct RainSound {
    playing: bool,
    /// The glided sky: openness per speaker, roof, enclosure.
    open: [f32; 4],
    roof: f32,
    roof_distance: f32,
    enclosure: f32,
    exposure: f32,
    surface: f32,
    sent_occlusion: f32,
}

impl RainSound {
    pub fn update(&mut self, engine: Option<&AudioEngine>, input: &RainSoundInput, dt: f32) {
        let Some(engine) = engine else {
            return;
        };
        let rain = input.rain.clamp(0.0, 1.0);
        let clips = if input.audible && rain > 0.0 { clips() } else { None };
        let Some(clips) = clips else {
            if self.playing {
                for i in 0..VOICES {
                    engine.stop_world(VOICE_BASE + i);
                }
                self.playing = false;
            }
            return;
        };
        if !self.playing {
            for (i, c) in clips
                .light
                .iter()
                .chain(&clips.heavy)
                .chain(&clips.roof)
                .chain(&clips.light_roof)
                .enumerate()
            {
                engine.play_world(VOICE_BASE + i as u64, Channel::Ambient, c, true, (0.0, 0.0));
            }
            for (i, c) in clips.drizzle.iter().flatten().enumerate() {
                engine.play_world(VOICE_BASE + V_DRIZZLE + i as u64, Channel::Ambient, c, true, (0.0, 0.0));
            }
            for (i, c) in clips.drizzle_roof.iter().flatten().enumerate() {
                engine.play_world(VOICE_BASE + V_ROOF_DRIZZLE + i as u64, Channel::Ambient, c, true, (0.0, 0.0));
            }
            if clips.light_roof_muffled {
                for i in 10..12 {
                    engine.world_occlusion(VOICE_BASE + i, LIGHT_ROOF_OCCLUSION);
                }
            }
            self.playing = true;
            self.sent_occlusion = -1.0;
            self.open = [1.0; 4];
            self.roof = 0.0;
            self.roof_distance = 15.0;
            self.enclosure = 0.0;
            self.exposure = 1.0;
            self.surface = 0.0;
        }

        // the sky glides to the last measure (open all round until one)
        let sky = input.sky.unwrap_or(Sky::OPEN);
        let k = 1.0 - (-dt.max(0.0) / GLIDE_SECS).exp();
        for (o, t) in self.open.iter_mut().zip(sky.open) {
            *o += (t - *o) * k;
        }
        self.roof += (sky.roof - self.roof) * k;
        self.roof_distance += (sky.roof_distance - self.roof_distance) * k;
        self.enclosure += (sky.enclosure - self.enclosure) * k;
        self.exposure += (sky.exposure - self.exposure) * k;
        self.surface += (sky.surface - self.surface) * k;
        // high above what the rain lands on (flying): only a faint, soft
        // hiss from far below; the loops' mix leans to the light one
        let aloft = if input.underwater > 0.0 {
            0.0
        } else {
            smoothstep(6.0, 38.0, self.surface)
        };
        // under the water: the rain on the surface, muffled, fading with
        // the depth; no roof
        let under = input.underwater > 0.0;
        let depth_fade = if under { (-input.underwater / 3.0).exp() * 0.7 } else { 1.0 };

        // the loops' mix: equal power, more of it the harder it rains
        let level = LEVEL * rain.sqrt() * depth_fade * (1.0 - 0.75 * aloft);
        let angle = rain * (1.0 - 0.6 * aloft) * std::f32::consts::FRAC_PI_2;
        let light = angle.cos() * 10f32.powf(LIGHT_MAKEUP_DB / 20.0) * level;
        let lean = rain * (1.0 - 0.6 * aloft);
        let (drizzle, light) = split_drizzle(light, lean, clips.drizzle.is_some());
        let heavy = angle.sin() * 10f32.powf(HEAVY_MAKEUP_DB / 20.0) * level;

        // each speaker panned by its side of the listener, as loud as the
        // sky is open there (a little of it even when closed: the rest is
        // the occlusion's)
        let right = input.forward.cross(Vec3::Z).truncate().normalize_or(Vec2::X);
        for (q, dir) in SPEAKERS.iter().enumerate() {
            let pan = dir.dot(right).clamp(-1.0, 1.0);
            let side = ((1.0 - pan) * 0.5).sqrt();
            let other = ((1.0 + pan) * 0.5).sqrt();
            let open = if under { 1.0 } else { self.open[q] };
            let g = 0.5 * (0.15 + 0.85 * open);
            engine.world_gain(VOICE_BASE + q as u64, (light * g * side, light * g * other));
            engine.world_gain(VOICE_BASE + 4 + q as u64, (heavy * g * side, heavy * g * other));
            if clips.drizzle.is_some() {
                engine.world_gain(VOICE_BASE + V_DRIZZLE + q as u64, (drizzle * g * side, drizzle * g * other));
            }
        }
        // a roof above: its drumming, wide (two copies apart), when it
        // covers much of the sky (an arch overhead is a strip of it), the
        // fainter the higher it is
        let cover = smoothstep(0.35, 0.85, self.roof);
        let near = (3.0 / self.roof_distance.max(0.5)).clamp(0.2, 1.0);
        // (a drizzle: the light loop muffled; harder: the cabin's drumming)
        let drum = if under { 0.0 } else { LEVEL * rain.sqrt() * cover * near };
        // (its own recording of light rain indoors is not muffled: as loud
        // as the light loop through the occlusion would be)
        let patter_gain = if clips.light_roof_muffled { 1.6 } else { 0.8 };
        let (patter, pour) = (drum * angle.cos() * patter_gain, drum * angle.sin() * 0.8);
        let (drip, patter) = split_drizzle(patter, lean, clips.drizzle_roof.is_some());
        if clips.drizzle_roof.is_some() {
            engine.world_gain(VOICE_BASE + V_ROOF_DRIZZLE, (drip * 0.85, drip * 0.5));
            engine.world_gain(VOICE_BASE + V_ROOF_DRIZZLE + 1, (drip * 0.5, drip * 0.85));
        }
        engine.world_gain(VOICE_BASE + 8, (pour * 0.85, pour * 0.5));
        engine.world_gain(VOICE_BASE + 9, (pour * 0.5, pour * 0.85));
        engine.world_gain(VOICE_BASE + 10, (patter * 0.85, patter * 0.5));
        engine.world_gain(VOICE_BASE + 11, (patter * 0.5, patter * 0.85));

        // no falling rain in sight: the open-air speakers muffled as
        // through walls (under an arch or an awning, or in a doorway, rain
        // is in sight and they stay clear; each speaker's own side is
        // already as loud as rain is heard there); under the water, as
        // through two walls
        let occlusion = if under {
            1.0
        } else {
            (1.0 - smoothstep(0.03, 0.3, self.exposure)).max(0.5 * aloft)
        };
        if (occlusion - self.sent_occlusion).abs() > 0.02 {
            let drizzle = if clips.drizzle.is_some() { V_DRIZZLE..V_DRIZZLE + 4 } else { 0..0 };
            for i in (0..8).chain(drizzle) {
                engine.world_occlusion(VOICE_BASE + i, occlusion);
            }
            self.sent_occlusion = occlusion;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_drizzle_slot_leaves_the_light_loop_as_it_was() {
        for rain in [0.0, 0.05, 0.2, 0.6, 1.0] {
            assert_eq!(split_drizzle(0.7, rain, false), (0.0, 0.7));
        }
    }

    #[test]
    fn a_drizzle_takes_the_faintest_rain_at_equal_power() {
        let (d, l) = split_drizzle(1.0, 0.0, true);
        assert!((d - 1.0).abs() < 1e-6 && l.abs() < 1e-6, "all drizzle at the faintest");
        let (d, l) = split_drizzle(1.0, 0.5, true);
        assert!(d.abs() < 1e-6 && (l - 1.0).abs() < 1e-6, "none from a third up");
        for rain in [0.1, 0.15, 0.2, 0.3] {
            let (d, l) = split_drizzle(0.8, rain, true);
            assert!((d * d + l * l - 0.64).abs() < 1e-5, "equal power at {rain}");
        }
        assert!(drizzle_share(0.1) > drizzle_share(0.2));
    }
}
