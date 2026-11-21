//! What the spinning cat's show looks like at one moment of its music: the
//! cat's pose, the circle under it and the look of the whole view. Pure:
//! the same moment always gives the same picture.
//!
//! The show is made to dazzle: in the drops the view changes colour on the
//! beat and faster, turns to glowing outlines that swell on the kicks, and
//! each phrase of sixteen beats has a look of its own. It flashes, which is
//! why a warning comes before it (`ui/spinning_cat.rs`) and why nothing
//! starts without its answer.
//!
//! With the Reduce motion setting the show is calm instead: the cat and its
//! circle, and one tint per bar by a slow fade, between colours of nearly
//! the same brightness. The tests hold that calm show to WCAG 2.3.1 (no
//! more than three flashes a second; here, none at all).

use std::f32::consts::{PI, TAU};

/// What the music is doing, which the cat and the view follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    /// Before the music: the circle wakes, the cat is not there yet.
    Waking,
    /// Silence, or nearly: the cat looks around.
    Still,
    /// The voice alone: the cat spins fast, beat after beat.
    Voice,
    /// The melody without drums: a slow turn per bar.
    Sway,
    /// The climb to a drop.
    Rise,
    /// The held breath after the climb.
    Hold,
    /// The voice comes back alone, just before a drop.
    Wait,
    /// Everything at once.
    Drop,
    /// The last notes: the cat leaves.
    Leave,
}

/// Where the music is, in the terms the show needs: the cue sheet of the
/// video gives it (`video::moment`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moment {
    pub mood: Mood,
    /// How far through this part of the music, 0..=1; the seconds since
    /// it began and until it ends.
    pub through: f32,
    pub since: f32,
    pub left: f32,
    /// Seconds since the show began.
    pub time: f32,
    /// Beats since the music began, and how far into this one (0..1).
    pub beat: u32,
    pub beat_phase: f32,
    /// Seconds since the last kick; `f32::MAX` when none came lately.
    pub kick_age: f32,
    /// Seconds since the last lone hit of bass in a silence; `f32::MAX`
    /// when there was none.
    pub impact_age: f32,
    /// The cat spins: a run of its singing, or of the beat.
    pub spin: Option<Spin>,
    /// In a drop: its phrase of sixteen beats (0, 1, 2…), the seconds since
    /// the phrase began and until it ends. Each phrase has its own look.
    pub phrase: u32,
    pub phrase_since: f32,
    pub phrase_left: f32,
    /// Seconds since the other cats began to come: the music's last and
    /// fullest part.
    pub peak_since: Option<f32>,
    /// Seconds since the cat arrived (negative before it does).
    pub arrived: f32,
    /// Drops begun so far, this one included: the cat grows with the first
    /// and keeps its size.
    pub drops: u32,
    /// Seconds since the last drop ended; None in a drop and before the
    /// first. Its tint fades over them.
    pub drop_ended: Option<f32>,
}

/// One run of the cat's spin: it turns `turns` whole times over `seconds`,
/// so that it ends facing the way it started, flattened into its loaf
/// between the run's two ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spin {
    /// How far through the run, 0..=1.
    pub through: f32,
    pub seconds: f32,
    pub turns: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cat {
    /// Size, 1 being the model's.
    pub scale: f32,
    /// Its spin (radians) from the way it faces at rest.
    pub yaw: f32,
    /// How flattened into a loaf, 0..=1 (the model's morph target).
    pub squash: f32,
    /// Metres off the ground.
    pub lift: f32,
    /// Glow of its own, 0..=1.
    pub glow: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// None while the cat is not there.
    pub cat: Option<Cat>,
    /// How many smaller cats circle the first; the last one grows with the
    /// fraction.
    pub clones: f32,
    /// The circle on the ground: brightness 0..=1 and turn (radians).
    pub circle: f32,
    pub circle_turn: f32,
    /// The view multiplied by this colour, over this share of it.
    pub tint: [f32; 3],
    pub tint_share: f32,
    /// Red and blue pulled apart from the centre (share of the view).
    pub split: f32,
    /// The picture bulging out on a kick (barrel distortion).
    pub punch: f32,
    /// Share of the view shown as glowing outlines, and their colour.
    pub neon: f32,
    pub neon_color: [f32; 3],
    /// The colours of the view turned around the wheel (degrees).
    pub hue: f32,
    /// The view's brightness read through three colours (shadows, middle,
    /// lights), over this share of it.
    pub map: [[f32; 3]; 3],
    pub map_share: f32,
    /// The view in blocks of this share of its height.
    pub mosaic: f32,
    /// Bands of the view torn sideways by this share of it, in a pattern
    /// that changes with the seed.
    pub glitch: f32,
    pub glitch_seed: f32,
    /// The bright parts of the view glowing: how much, and how far the
    /// glow reaches (share of the view). The outlines swell with it.
    pub bloom: f32,
    pub bloom_radius: f32,
}

/// The tints of the drops. Their relative luminances are within 0.09 of one
/// another, and none is a saturated red.
pub const TINTS: [[f32; 3]; 5] = [
    [0.75, 0.60, 1.00],
    [1.00, 0.60, 0.15],
    [1.00, 0.50, 0.75],
    [0.20, 0.80, 0.80],
    [0.35, 0.60, 1.00],
];
/// The view under the waking circle.
const DUSK: [f32; 3] = [0.55, 0.45, 0.80];
const NEON: [[f32; 3]; 2] = [[0.20, 0.90, 0.85], [1.00, 0.45, 0.80]];
/// The duotones: shadows, middle, lights.
const MAPS: [[[f32; 3]; 3]; 4] = [
    [[0.05, 0.00, 0.20], [0.90, 0.10, 0.60], [1.00, 0.90, 0.50]],
    [[0.00, 0.05, 0.20], [0.10, 0.60, 0.90], [0.80, 1.00, 0.90]],
    [[0.15, 0.00, 0.10], [1.00, 0.40, 0.10], [1.00, 0.95, 0.70]],
    [[0.00, 0.10, 0.10], [0.20, 0.90, 0.50], [0.90, 1.00, 0.60]],
];
/// Share of the view the drops' tint takes, and the calm show's.
const TINT_SHARE: f32 = 0.6;
const CALM_SHARE: f32 = 0.3;
/// Share of its time a tint takes to replace the one before.
const TINT_FADE: f32 = 0.25;
/// The most cats circling the first.
pub const CLONES: f32 = 12.0;
/// Seconds the cat is in the air when a lone hit of bass makes it jump.
const JUMP: f32 = 0.7;

/// The look of a phrase of a drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    /// A tint per beat, the colours split, the picture bulging.
    Tint,
    /// The colours turning around the wheel, blocks on the kicks.
    Prism,
    /// Two-colour pictures, one per bar.
    Duotone,
    /// Glowing outlines that swell on the kicks.
    Neon,
    /// All of it.
    Finale,
}

/// The phrases of the first drop, and of the second (and any after).
const STYLES: [[Style; 4]; 2] = [
    [Style::Tint, Style::Prism, Style::Duotone, Style::Neon],
    [Style::Neon, Style::Duotone, Style::Prism, Style::Finale],
];

/// 0 to 1 with a soft start and end.
fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// 0 to 1 overshooting a little before it settles: something popping in.
fn pop(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0) - 1.0;
    1.0 + 2.7 * x * x * x + 1.7 * x * x
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// 1 when something just happened, falling to 1/e in `seconds`.
fn fading(age: f32, seconds: f32) -> f32 {
    if age < 20.0 { (-age / seconds).exp() } else { 0.0 }
}

/// The tint of the drops when it changes `rate` times a beat: each takes
/// the place of the one before by a short fade.
fn tint_at(m: &Moment, rate: f32) -> [f32; 3] {
    let at = (m.beat as f32 + m.beat_phase) * rate;
    let index = at as usize;
    let before = TINTS[(index + TINTS.len() - 1) % TINTS.len()];
    mix(before, TINTS[index % TINTS.len()], ease(at.fract() / TINT_FADE))
}

/// The picture of a moment. `calm` (the Reduce motion setting) keeps the
/// cat, its circle and a slow tint per bar, and nothing that flashes or
/// moves the whole view.
pub fn frame(m: &Moment, calm: bool) -> Frame {
    let kick = fading(m.kick_age, 0.12);
    let drop = m.mood == Mood::Drop;
    let impact = (m.impact_age < 6.0).then_some(m.impact_age);
    let peak = m.drops >= 2;

    // ---- the cat
    let (yaw, squash, lift) = match m.spin {
        Some(Spin { through, seconds, turns }) => {
            let edge = (through.min(1.0 - through) * seconds / 0.06).clamp(0.0, 1.0);
            let hop = (PI * (through * seconds / 0.1).fract()).sin().abs();
            (TAU * turns * through, edge, 0.05 * hop * edge)
        }
        // Nothing to spin to: it looks around, breathes, bobs on the beat,
        // and jumps when a lone hit of bass lands.
        None => {
            let settled = ease(m.since / 0.5) * ease(m.left / 0.5);
            let look = 0.5 * (TAU * m.time / 3.2).sin() * settled;
            let breath = (0.07 + 0.07 * (TAU * m.time / 1.6).sin()) * settled;
            let bob = 0.015 * (PI * m.beat_phase).sin() * settled;
            let (jump, landing) = match impact {
                Some(age) if age < JUMP => (0.45 * (PI * age / JUMP).sin(), 0.0),
                Some(age) if age < JUMP + 0.25 => (0.0, 0.6 * (1.0 - (age - JUMP) / 0.25)),
                _ => (0.0, 0.0),
            };
            (look, (breath + landing).min(1.0), bob + jump)
        }
    };
    // Leaving: after the last notes, in a third of a second.
    let leaving = if m.mood == Mood::Leave {
        1.0 - ease((m.since - 1.0) / 0.3)
    } else {
        1.0
    };
    // It grows with the first drop, and keeps that size.
    let grown = if m.drops > 0 { 1.5 } else { 1.0 };
    let scale = match m.mood {
        Mood::Waking => 0.0,
        Mood::Drop => {
            let size = if m.drops <= 1 { 1.0 + 0.5 * ease(m.through * 2.0) } else { grown };
            size + 0.08 * kick
        }
        Mood::Leave => grown * leaving,
        _ => grown * pop(m.arrived / 0.35),
    };
    let cat = (scale > 0.01).then_some(Cat {
        scale,
        yaw,
        squash,
        lift,
        glow: if drop {
            0.25 * kick
        } else {
            0.5 * impact.map_or(0.0, |age| fading(age, 0.3))
        },
    });
    let clones = match (m.peak_since, calm) {
        (Some(since), false) => (1.0 + since / 0.8).min(CLONES),
        (Some(since), true) => (1.0 + since / 1.6).min(4.0),
        (None, _) => 0.0,
    };

    // ---- the circle
    let circle = match m.mood {
        Mood::Waking => (0.15 + 0.85 * m.through * m.through).min(1.0),
        Mood::Drop => 0.5 + 0.5 * kick,
        Mood::Leave => 0.5 * leaving,
        _ => {
            let settled = 0.35 + 0.15 * squash;
            let arriving = (1.0 - settled) * (1.0 - ease(m.arrived / 1.5));
            (settled + arriving + 0.6 * impact.map_or(0.0, |age| fading(age, 0.6))).min(1.0)
        }
    };

    // ---- the view: its tint first, which the calm show keeps
    let style = drop.then(|| STYLES[(m.drops.max(1) as usize - 1).min(STYLES.len() - 1)][m.phrase as usize % 4]);
    // How often the tint changes in a beat: on the beat, on its half in the
    // second drop, on its quarters in the finale.
    let rate = match (calm, style) {
        (true, _) => 0.25,
        (false, Some(Style::Finale)) => 4.0,
        (false, Some(Style::Neon)) => 2.0,
        (false, _) if peak => 2.0,
        (false, _) => 1.0,
    };
    let beat_tint = tint_at(m, rate);
    // The drop's tint lands at once; the calm show's comes over a second.
    let (share, landing) = if calm { (CALM_SHARE, 1.0) } else { (TINT_SHARE, 0.1) };
    let (tint, tint_share) = match (m.mood, m.drop_ended) {
        (Mood::Waking, _) => (DUSK, 0.5 * m.through),
        // The dusk lifts as the cat arrives.
        (_, _) if m.drops == 0 => (DUSK, 0.5 * (1.0 - ease(m.arrived / 0.6))),
        (Mood::Drop, _) => (beat_tint, share * ease(m.since / landing)),
        // After a drop its tint fades.
        (_, Some(ended)) => (beat_tint, share * (1.0 - ease(ended / 0.8))),
        (_, None) => (beat_tint, 0.0),
    };
    let mut frame = Frame {
        cat,
        clones,
        circle,
        circle_turn: 0.3 * m.time,
        tint,
        tint_share,
        split: 0.0,
        punch: 0.0,
        neon: 0.0,
        neon_color: NEON[0],
        hue: 0.0,
        map: MAPS[0],
        map_share: 0.0,
        mosaic: 0.0,
        glitch: 0.0,
        glitch_seed: m.beat as f32,
        bloom: 0.0,
        bloom_radius: 0.0,
    };
    if calm {
        return frame;
    }

    // ---- and everything that moves it
    // A phrase's look comes in and goes out in a third of a second.
    let phrase = ease(m.phrase_since / 0.3) * ease(m.phrase_left / 0.3);
    // The half beats, for what pulses twice as fast as the kick.
    let half = 1.0 - (m.beat_phase * 2.0).fract();
    match (m.mood, style) {
        (Mood::Drop, Some(Style::Tint)) => {
            frame.split = 0.004 + 0.012 * kick + 0.006 * squash;
            frame.punch = 0.06 * kick;
            frame.bloom = 0.35 * kick;
            frame.bloom_radius = 0.02;
        }
        (Mood::Drop, Some(Style::Prism)) => {
            // A whole turn of the wheel per bar: the phrase ends where it
            // began.
            frame.hue = 360.0 * (m.phrase_since / 1.6).fract();
            frame.mosaic = 0.03 * kick * kick;
            frame.split = 0.006 + 0.010 * kick;
            frame.punch = 0.04 * kick;
        }
        (Mood::Drop, Some(Style::Duotone)) => {
            let bar = (m.beat / 4) as usize;
            let into = ease((m.beat % 4) as f32 + m.beat_phase);
            let before = MAPS[(bar + MAPS.len() - 1) % MAPS.len()];
            let now = MAPS[bar % MAPS.len()];
            frame.map = [0, 1, 2].map(|i| mix(before[i], now[i], into));
            frame.map_share = 0.9 * phrase;
            frame.punch = 0.07 * kick;
            frame.split = 0.004 + 0.008 * kick;
            // The beats without a kick tear.
            frame.glitch = if m.kick_age > 0.45 { 0.06 } else { 0.0 };
        }
        (Mood::Drop, Some(Style::Neon)) => {
            // The outlines beat with the half beats and swell on the kicks.
            frame.neon = (0.6 + 0.35 * half) * phrase;
            frame.neon_color = beat_tint;
            frame.bloom = (0.6 + 0.4 * kick) * phrase;
            frame.bloom_radius = 0.008 + 0.04 * kick;
            frame.split = 0.004 + 0.010 * kick;
            frame.punch = 0.04 * kick;
        }
        (Mood::Drop, Some(Style::Finale)) => {
            frame.neon = (0.25 + 0.4 * half) * phrase;
            frame.neon_color = tint_at(m, 2.0);
            frame.bloom = (0.5 + 0.5 * kick) * phrase;
            frame.bloom_radius = 0.01 + 0.04 * kick;
            frame.hue = 360.0 * (m.phrase_since / 1.6).fract();
            frame.split = 0.008 + 0.016 * kick;
            frame.punch = 0.08 * kick;
            frame.mosaic = 0.02 * kick * kick;
        }
        // A lone hit of bass in the silence: the view turns to outlines at
        // once, and they spread as they fade.
        (Mood::Still, _) => {
            if let Some(age) = impact {
                let shock = ease(age / 0.06) * fading(age, 0.7);
                frame.neon = 0.85 * shock;
                frame.neon_color = NEON[1];
                frame.bloom = 0.8 * shock;
                frame.bloom_radius = 0.01 + 0.08 * (1.0 - fading(age, 0.6));
                frame.punch = 0.14 * fading(age, 0.25);
                frame.split = 0.012 * fading(age, 0.4);
            }
        }
        // The voice after a drop tears the picture; before the first one
        // it only splits the colours a little.
        (Mood::Voice, _) if m.drops > 0 => {
            frame.glitch = 0.06;
            frame.split = 0.004 + 0.008 * squash;
        }
        (Mood::Voice, _) => frame.split = 0.004 * squash,
        // The melody: colours drifting, everything bright in a soft glow.
        (Mood::Sway, _) => {
            let on = ease(m.since / 1.0) * ease(m.left / 1.0);
            frame.hue = 40.0 * (TAU * m.since / 6.4).sin() * on;
            frame.bloom = 0.4 * on;
            frame.bloom_radius = 0.025;
        }
        // The climb: the outlines take the view over, faster and faster.
        (Mood::Rise, _) => {
            let up = m.through;
            frame.neon = 0.9 * ease(up);
            frame.neon_color = mix(NEON[0], NEON[1], up);
            frame.bloom = 0.7 * ease(up);
            frame.bloom_radius = 0.008 + 0.04 * up * (1.0 - m.beat_phase);
            frame.split = 0.014 * up;
            frame.glitch = 0.05 * up * up;
            frame.punch = 0.05 * up * (1.0 - m.beat_phase);
        }
        (Mood::Hold, _) => {
            frame.neon = 0.9;
            frame.neon_color = NEON[1];
            frame.bloom = 0.7;
            frame.bloom_radius = 0.01;
        }
        (Mood::Wait, _) => {
            // Back to the picture in the last fifth of a second: the drop
            // lands on it.
            let held = ease(m.left / 0.2);
            frame.neon = 0.9 * held;
            frame.neon_color = mix(NEON[1], NEON[0], ease(m.since / 0.8));
            frame.bloom = 0.7 * held;
            frame.bloom_radius = 0.01 + 0.03 * (1.0 - m.beat_phase);
            frame.split = 0.004 + 0.008 * squash;
        }
        _ => {}
    }
    frame
}

/// Where a cat circling the first is, from the first cat's place: metres
/// (x, y, z), its size as a share of the first cat's, and the way it turns
/// (1 or -1). `grown` is how much of it is there yet, 0..=1.
pub fn clone_place(index: usize, time: f32, grown: f32) -> ([f32; 3], f32, f32) {
    let way = if index.is_multiple_of(2) { 1.0 } else { -1.0 };
    // The golden angle apart, so that no two share a spoke.
    let angle = index as f32 * 2.399_963 + way * 0.9 * time;
    let radius = 1.3 + 0.35 * ((index * 5) % 4) as f32 / 3.0;
    let height = 0.5 + 0.45 * ((index * 3) % 5) as f32 / 4.0 + 0.1 * (2.0 * time + index as f32).sin();
    ([radius * angle.cos(), radius * angle.sin(), height], 0.45 * ease(grown), way)
}

/// The limits of the calm show, measured on a run of its frames (their
/// time in seconds, 240 a second).
#[cfg(test)]
pub(super) mod check {
    use super::Frame;

    /// Relative luminance of a linear colour.
    pub fn luminance(c: [f32; 3]) -> f32 {
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    }

    /// The brightness of a white view under a frame's look: the tint over
    /// its share, the duotone's lights over its own, the outlines dark.
    pub fn brightness(f: &Frame) -> f32 {
        let tinted = 1.0 - f.tint_share + f.tint_share * luminance(f.tint);
        let mapped = tinted * (1.0 - f.map_share) + f.map_share * luminance(f.map[2]);
        mapped * (1.0 - f.neon) + 0.1 * f.neon
    }

    /// The most changes of brightness within any one second. WCAG 2.3.1: a
    /// flash is a pair of opposing changes of relative luminance of 10 % or
    /// more, and three flashes a second is the limit; each such change is
    /// counted here, which is stricter.
    pub fn most_brightness_changes_in_a_second(frames: &[(f32, Frame)]) -> usize {
        let mut changes: Vec<f32> = Vec::new();
        let mut from = brightness(&frames[0].1);
        let mut way = 0.0f32;
        for (time, f) in frames {
            let now = brightness(f);
            if (now - from).abs() >= 0.1 {
                if (now - from).signum() != way {
                    changes.push(*time);
                    way = (now - from).signum();
                }
                from = now;
            } else if (now - from) * way > 0.0 {
                from = now;
            }
        }
        (0..changes.len())
            .map(|i| changes[i..].iter().take_while(|t| **t - changes[i] <= 1.0).count())
            .max()
            .unwrap_or(0)
    }

    /// The most a channel of the tint moves from one frame to the next.
    pub fn largest_tint_step(frames: &[(f32, Frame)]) -> f32 {
        frames
            .windows(2)
            .map(|pair| {
                let (a, b) = (&pair[0].1, &pair[1].1);
                (0..3)
                    .map(|i| (a.tint[i] * a.tint_share - b.tint[i] * b.tint_share).abs())
                    .fold(0.0, f32::max)
            })
            .fold(0.0, f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::super::video;
    use super::check::luminance;
    use super::*;

    fn frames(calm: bool) -> Vec<(f32, Frame)> {
        // 240 pictures a second: finer than any screen.
        (0..)
            .map(|i| i as f64 / 240.0)
            .map_while(|t| video::moment(t).map(|m| (t as f32, frame(&m, calm))))
            .collect()
    }

    /// A moment before the music, the circle `through` its waking.
    fn waking(through: f32) -> Moment {
        Moment {
            mood: Mood::Waking,
            through,
            since: through * 2.5,
            left: f32::MAX,
            time: through * 2.5,
            beat: 0,
            beat_phase: 0.0,
            kick_age: f32::MAX,
            impact_age: f32::MAX,
            spin: None,
            phrase: 0,
            phrase_since: 0.0,
            phrase_left: f32::MAX,
            peak_since: None,
            arrived: -1.0,
            drops: 0,
            drop_ended: None,
        }
    }

    #[test]
    fn the_tints_are_of_one_brightness_and_none_is_a_saturated_red() {
        let all: Vec<f32> = TINTS.iter().map(|c| luminance(*c)).collect();
        let (low, high) = all.iter().fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
        assert!(high - low < 0.1, "{all:?}");
        // WCAG's saturated red: R / (R + G + B) of 0.8 or more.
        let colours = TINTS.iter().chain(NEON.iter()).chain(MAPS.iter().flatten());
        assert!(colours.clone().all(|c| c[0] / (c[0] + c[1] + c[2]) < 0.8));
    }

    #[test]
    fn the_calm_show_never_flashes_and_keeps_the_view_still() {
        let frames = frames(true);
        // A flash is two opposing changes of brightness of a tenth; the calm
        // show never has two such changes within a second: no flash at all.
        assert!(check::most_brightness_changes_in_a_second(&frames) <= 1);
        assert!(check::largest_tint_step(&frames) < 0.01, "A tint per bar, by a slow fade");
        for (_, f) in &frames {
            let moving = [f.split, f.punch, f.neon, f.hue, f.map_share, f.mosaic, f.glitch, f.bloom];
            assert!(moving.iter().all(|v| *v == 0.0), "{f:?}");
            assert!(f.tint_share <= 0.5 && f.clones <= 4.0);
        }
        assert!(
            frames.iter().any(|(_, f)| f.cat.is_some_and(|c| c.yaw > 1.0)),
            "The cat still spins"
        );
    }

    #[test]
    fn each_phrase_of_a_drop_has_its_own_look() {
        // The middle of a beat with a kick, 3 s into each phrase.
        let look = |drop: u32, phrase: u32| {
            let first = if drop == 1 { 8 } else { 192 };
            frame(
                &video::moment(video::BEAT * f64::from(first + 16 * phrase + 8) + 0.24 + 0.05).unwrap(),
                false,
            )
        };
        let tint = look(1, 0);
        assert!(tint.neon == 0.0 && tint.hue == 0.0 && tint.map_share == 0.0 && tint.split > 0.01);
        let prism = look(1, 1);
        assert!(prism.hue > 0.0 && prism.mosaic > 0.0 && prism.neon == 0.0);
        let duotone = look(1, 2);
        assert!(duotone.map_share > 0.8 && duotone.hue == 0.0);
        let neon = look(1, 3);
        assert!(
            neon.neon > 0.5 && neon.bloom > 0.5 && neon.bloom_radius > 0.02,
            "The outlines swell on the kick"
        );
        // The second drop opens on the outlines and ends on everything.
        assert!(look(2, 0).neon > 0.5);
        let finale = look(2, 3);
        assert!(finale.neon > 0.2 && finale.hue > 0.0 && finale.split > 0.015 && finale.punch > 0.04);
        // Every look carries the drop's tint.
        assert!([tint, prism, duotone, neon, finale].iter().all(|f| f.tint_share > 0.5));
    }

    #[test]
    fn the_tint_changes_on_the_beat_then_faster() {
        let changes = |from: f64, to: f64| {
            let mut count = 0;
            let mut last = frame(&video::moment(from).unwrap(), false).tint;
            let mut t = from;
            while t < to {
                let tint = frame(&video::moment(t).unwrap(), false).tint;
                // Settled on a new colour of the list.
                if tint != last && TINTS.iter().any(|c| (0..3).all(|i| (c[i] - tint[i]).abs() < 1e-4)) {
                    count += 1;
                    last = tint;
                }
                t += 1.0 / 240.0;
            }
            count
        };
        // Four seconds of the first drop: a tint per beat.
        assert_eq!(changes(4.0, 8.0), 10);
        // Of the second drop's last phrase: four per beat.
        assert_eq!(changes(97.0, 101.0), 40);
    }

    #[test]
    fn the_cat_arrives_spins_whole_turns_and_leaves() {
        let frames = frames(false);
        // It arrives with the music and pops in: there from the first
        // instants to the last second.
        assert!(frames[0].1.cat.is_none());
        assert!(
            frames
                .iter()
                .all(|(t, f)| !(0.02..video::LENGTH as f32 - 0.8).contains(t) || f.cat.is_some())
        );
        let spun = frames.iter().filter_map(|(_, f)| f.cat).map(|c| c.yaw).fold(0.0, f32::max);
        assert!(spun > TAU, "{spun}");
        // Its spin ends on whole turns and its looking around starts from
        // there: it never snaps round. A beat's run turns it by two turns
        // in 0.4 s, 0.13 rad a frame at most.
        let turns: Vec<f32> = frames.iter().filter_map(|(_, f)| f.cat).map(|c| c.yaw).collect();
        let snap = turns
            .windows(2)
            .map(|w| {
                let d = (w[1] - w[0]).rem_euclid(TAU);
                d.min(TAU - d)
            })
            .fold(0.0, f32::max);
        assert!(snap < 0.2, "{snap}");
        assert!(
            frames
                .last()
                .is_some_and(|(_, f)| f.cat.is_none() && f.circle < 0.01 && f.tint_share < 0.01)
        );
    }

    #[test]
    fn the_cat_is_never_idle_in_the_silences() {
        // The long silence between the drops, 46 to 57 s.
        let quiet: Vec<Cat> = (0..)
            .map(|i| 46.0 + f64::from(i) / 60.0)
            .take_while(|t| *t < 57.0)
            .filter_map(|t| frame(&video::moment(t).unwrap(), false).cat)
            .collect();
        let range = |of: fn(&Cat) -> f32| {
            let (low, high) = quiet.iter().map(of).fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(v), h.max(v)));
            high - low
        };
        assert!(range(|c| c.yaw) > 0.8, "It looks around");
        assert!(range(|c| c.squash) > 0.1, "It breathes");
        assert!(range(|c| c.lift) > 0.4, "It jumps on the hit of bass");
    }

    #[test]
    fn a_hit_of_bass_in_the_silence_sends_a_wave_of_outlines() {
        // The hit on beat 128, 51.44 s in.
        let at = |after: f64| frame(&video::moment(51.44 + after).unwrap(), false);
        assert_eq!(at(-0.1).neon, 0.0);
        let (struck, later) = (at(0.1), at(1.0));
        assert!(struck.neon > 0.6 && struck.punch > 0.05);
        assert!(later.neon < struck.neon && later.neon > 0.0);
        assert!(later.bloom_radius > struck.bloom_radius, "The outlines spread as they fade");
        assert!(at(5.0).neon < 0.01);
    }

    #[test]
    fn the_circle_wakes_before_the_cat_is_there() {
        let (dark, bright) = (frame(&waking(0.0), false), frame(&waking(1.0), false));
        assert!(dark.cat.is_none() && bright.cat.is_none());
        assert!(dark.circle < 0.2 && bright.circle > 0.99);
        assert!(dark.tint_share == 0.0 && bright.tint_share == 0.5);
        // The dusk it leaves is the one the cat's arrival lifts: no jump.
        let arrived = frame(&video::moment(0.0).unwrap(), false);
        assert_eq!((arrived.tint, arrived.tint_share), (bright.tint, bright.tint_share));
    }

    #[test]
    fn the_other_cats_come_one_by_one_with_the_second_drop() {
        let frames = frames(false);
        let most = frames.iter().map(|(_, f)| f.clones).fold(0.0, f32::max);
        assert_eq!(most, CLONES);
        let first = frames.iter().find(|(_, f)| f.clones > 0.0).map(|(t, _)| *t);
        assert!(first.is_some_and(|t| t > 60.0), "{first:?}");
        let (place, size, _) = clone_place(3, 10.0, 1.0);
        assert!(place[2] > 0.3 && (place[0].hypot(place[1]) - 1.3).abs() < 0.4 && size > 0.4);
        assert_eq!(clone_place(3, 10.0, 0.0).1, 0.0);
    }
}
