//! Colour vision deficiencies (Preferences › Accessibility › Colour
//! blindness, #418): the interface palette and the configurable colours
//! adapted so that what tells two things apart by colour stays visible.
//! Firestorm has no such option; this is Aurora's own.
//!
//! - **The palette** (`palette`) keeps the Aurora look: only the colours
//!   that carry a meaning and collide for the deficiency move, to a
//!   colour-blind-safe hue (Okabe & Ito, "Color Universal Design", 2008)
//!   of the same luminance, so that the contrast a skin was designed with
//!   stays. Red–green (protan, deutan): success / online goes blue, like
//!   the colour-blind themes of GitHub. Blue–yellow (tritan): the warning
//!   goes orange, links sky blue, rose reddish purple. Monochrome: every
//!   colour turns grey, the meaningful ones on a ladder of lightness.
//! - **Other colours** (`adapt`: chat, name tags, map markers, complexity)
//!   are chosen by the user or follow a ramp: they are daltonised (Fidaner,
//!   Lin & Ozguven, "Analysis of color blindness", 2005): the part the
//!   deficiency loses, from the simulation of Machado, Oliveira & Fernandes
//!   (2009, severity 1), is moved into the channels still seen.
//! - The status dots also differ by their shape (`ui/presence.rs`), which
//!   needs no colour at all.
//!
//! The choice is process-wide (`set`), read where the palette is built.

use crate::theme::Palette;
use egui::Color32;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, Ordering};

/// A colour vision deficiency, as chosen in the preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Deficiency {
    #[default]
    Off,
    /// Red-weak or red-blind (protanomaly, protanopia).
    Protan,
    /// Green-weak or green-blind (deuteranomaly, deuteranopia), the most
    /// common one.
    Deutan,
    /// Blue–yellow (tritanomaly, tritanopia).
    Tritan,
    /// No colours (achromatopsia).
    Mono,
}

impl Deficiency {
    pub const ALL: [Deficiency; 5] = [
        Deficiency::Off,
        Deficiency::Protan,
        Deficiency::Deutan,
        Deficiency::Tritan,
        Deficiency::Mono,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Deficiency::Off => aurora_i18n::tr!("colorblind-off"),
            Deficiency::Protan => aurora_i18n::tr!("colorblind-protan"),
            Deficiency::Deutan => aurora_i18n::tr!("colorblind-deutan"),
            Deficiency::Tritan => aurora_i18n::tr!("colorblind-tritan"),
            Deficiency::Mono => aurora_i18n::tr!("colorblind-mono"),
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Deficiency::Off => aurora_i18n::tr!("colorblind-off-help"),
            Deficiency::Protan => aurora_i18n::tr!("colorblind-protan-help"),
            Deficiency::Deutan => aurora_i18n::tr!("colorblind-deutan-help"),
            Deficiency::Tritan => aurora_i18n::tr!("colorblind-tritan-help"),
            Deficiency::Mono => aurora_i18n::tr!("colorblind-mono-help"),
        }
    }

    /// `AURORA_DEMO_COLORBLIND` values.
    pub fn parse(s: &str) -> Option<Deficiency> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" => Deficiency::Off,
            "protan" | "protanopia" => Deficiency::Protan,
            "deutan" | "deuteranopia" => Deficiency::Deutan,
            "tritan" | "tritanopia" => Deficiency::Tritan,
            "mono" | "achromatopsia" => Deficiency::Mono,
            _ => return None,
        })
    }

    fn code(self) -> u32 {
        Deficiency::ALL.iter().position(|d| *d == self).unwrap_or(0) as u32
    }
}

/// What the interface adapts to: a deficiency and how strongly (1 = the
/// full correction, lower for an anomaly that keeps some of the colour).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vision {
    pub deficiency: Deficiency,
    pub strength: f32,
}

impl Vision {
    #[cfg(test)]
    pub const OFF: Vision = Vision {
        deficiency: Deficiency::Off,
        strength: 1.0,
    };

    pub fn new(deficiency: Deficiency, strength: f32) -> Vision {
        Vision {
            deficiency,
            strength: if strength.is_finite() { strength.clamp(0.0, 1.0) } else { 1.0 },
        }
    }

    fn active(self) -> bool {
        self.deficiency != Deficiency::Off && self.strength > 0.0
    }
}

/// The deficiency in the low byte, the strength in thousandths above.
static CURRENT: AtomicU32 = AtomicU32::new(1000 << 8);

fn encode(v: Vision) -> u32 {
    let v = Vision::new(v.deficiency, v.strength);
    ((v.strength * 1000.0).round() as u32) << 8 | v.deficiency.code()
}

fn decode(raw: u32) -> Vision {
    let deficiency = Deficiency::ALL.get((raw & 0xff) as usize).copied().unwrap_or_default();
    Vision::new(deficiency, (raw >> 8) as f32 / 1000.0)
}

/// The vision the interface adapts to from now on (the palette is built
/// again by its users).
pub fn set(v: Vision) {
    CURRENT.store(encode(v), Ordering::Relaxed);
}

pub fn current() -> Vision {
    decode(CURRENT.load(Ordering::Relaxed))
}

/// `AURORA_DEMO_COLORBLIND=protan|deutan|tritan|mono[:strength]` (demo
/// mode only): the vision of this run, never saved in the settings.
pub fn demo_override() -> Option<Vision> {
    let v = std::env::var("AURORA_DEMO_COLORBLIND").ok()?;
    let (kind, strength) = match v.split_once(':') {
        Some((k, s)) => (k, s.trim().parse::<f32>().ok()?),
        None => (v.as_str(), 1.0),
    };
    Some(Vision::new(Deficiency::parse(kind)?, strength))
}

// ---- colour arithmetic, in linear RGB

fn to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let v = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (v * 255.0).round() as u8
}

type Rgb = [f32; 3];

fn linear(c: Color32) -> Rgb {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    [to_linear(r), to_linear(g), to_linear(b)]
}

fn color(rgb: Rgb, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(to_srgb(rgb[0]), to_srgb(rgb[1]), to_srgb(rgb[2]), alpha)
}

/// Relative luminance (WCAG, Rec. 709).
fn luminance(c: Rgb) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn mul(m: &[[f32; 3]; 3], c: Rgb) -> Rgb {
    [0, 1, 2].map(|i| (m[i][0] * c[0] + m[i][1] * c[1] + m[i][2] * c[2]).clamp(0.0, 1.0))
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// Machado, Oliveira & Fernandes 2009, severity 1.0, in linear RGB.
const PROTAN: [[f32; 3]; 3] = [
    [0.152_286, 1.052_583, -0.204_868],
    [0.114_503, 0.786_281, 0.099_216],
    [-0.003_882, -0.048_116, 1.051_998],
];
const DEUTAN: [[f32; 3]; 3] = [
    [0.367_322, 0.860_646, -0.227_968],
    [0.280_085, 0.672_501, 0.047_413],
    [-0.011_820, 0.042_940, 0.968_881],
];
const TRITAN: [[f32; 3]; 3] = [
    [1.255_528, -0.076_749, -0.178_779],
    [-0.078_411, 0.930_809, 0.147_602],
    [0.004_733, 0.691_367, 0.303_900],
];

/// What a person with the full deficiency sees of a colour.
pub fn simulate(c: Color32, d: Deficiency) -> Color32 {
    let lin = linear(c);
    let seen = match d {
        Deficiency::Off => lin,
        Deficiency::Protan => mul(&PROTAN, lin),
        Deficiency::Deutan => mul(&DEUTAN, lin),
        Deficiency::Tritan => mul(&TRITAN, lin),
        Deficiency::Mono => [luminance(lin); 3],
    };
    color(seen, c.a())
}

/// A colour that is not the palette's (chosen by the user, or a ramp),
/// daltonised for the vision.
pub fn adapt(c: Color32, v: Vision) -> Color32 {
    if !v.active() {
        return c;
    }
    let lin = linear(c);
    let target = match v.deficiency {
        Deficiency::Off => return c,
        Deficiency::Mono => [luminance(lin); 3],
        d => {
            let seen = linear(simulate(c, d));
            let e = [0, 1, 2].map(|i| lin[i] - seen[i]);
            // the lost difference, moved where it is still seen
            let shift = if d == Deficiency::Tritan {
                [e[0] + 0.7 * e[2], e[1] + 0.7 * e[2], 0.0]
            } else {
                [0.0, 0.7 * e[0] + e[1], 0.7 * e[0] + e[2]]
            };
            [0, 1, 2].map(|i| (lin[i] + shift[i]).clamp(0.0, 1.0))
        }
    };
    color(mix(lin, target, v.strength), c.a())
}

/// The hue of `target` at the luminance of `like` (a skin's contrast
/// kept); a channel past white mixes towards white at that luminance.
fn same_luminance(target: Color32, like: Rgb) -> Rgb {
    let y = luminance(like);
    let t = linear(target);
    let ty = luminance(t);
    if ty <= 0.0 {
        return [y; 3];
    }
    let mut c = t.map(|v| v * y / ty);
    for _ in 0..8 {
        if c.iter().all(|v| *v <= 1.0) {
            break;
        }
        c = c.map(|v| v.min(1.0));
        let cy = luminance(c);
        let k = ((y - cy) / (1.0 - cy).max(1e-6)).clamp(0.0, 1.0);
        c = c.map(|v| v + (1.0 - v) * k);
    }
    c
}

/// Okabe–Ito colours.
const BLUE: Color32 = Color32::from_rgb(0x00, 0x72, 0xB2);
const ORANGE: Color32 = Color32::from_rgb(0xE6, 0x9F, 0x00);
const SKY_BLUE: Color32 = Color32::from_rgb(0x56, 0xB4, 0xE9);
const REDDISH_PURPLE: Color32 = Color32::from_rgb(0xCC, 0x79, 0xA7);

/// CIE L* to relative luminance.
fn lightness_to_luminance(l: f32) -> f32 {
    let f = (l + 16.0) / 116.0;
    if f > 6.0 / 29.0 { f * f * f } else { (l / 903.3).max(0.0) }
}

/// The palette for a vision (see the module's comment).
pub fn palette(mut p: Palette, v: Vision) -> Palette {
    if !v.active() {
        return p;
    }
    let move_to = |c: &mut Color32, target: Rgb| {
        *c = color(mix(linear(*c), target, v.strength), c.a());
    };
    let recolor = |c: &mut Color32, hue: Color32| {
        let target = same_luminance(hue, linear(*c));
        *c = color(mix(linear(*c), target, v.strength), c.a());
    };
    match v.deficiency {
        Deficiency::Off => {}
        Deficiency::Protan | Deficiency::Deutan => recolor(&mut p.success, BLUE),
        Deficiency::Tritan => {
            recolor(&mut p.warn, ORANGE);
            recolor(&mut p.teal, SKY_BLUE);
            recolor(&mut p.rose, REDDISH_PURPLE);
        }
        Deficiency::Mono => {
            // the meaningful colours on a ladder of lightness, away from the
            // surfaces (lighter over dark surfaces, darker over light ones)
            // (in high contrast, a ladder whose lowest rung still reads 7:1
            // on the black panels: crate::high_contrast)
            let dark = luminance(linear(p.panel)) < 0.5;
            let rungs = match (p.high_contrast, dark) {
                (true, _) => [75.0, 66.0, 83.0, 91.0, 98.0],
                (false, true) => [48.0, 61.0, 72.0, 83.0, 94.0],
                (false, false) => [62.0, 50.0, 40.0, 30.0, 20.0],
            };
            let ladder: [(&mut Color32, f32); 5] = [
                (&mut p.muted_dim, rungs[0]),
                (&mut p.danger, rungs[1]),
                (&mut p.warn, rungs[2]),
                (&mut p.success, rungs[3]),
                (&mut p.amber, rungs[4]),
            ];
            for (c, l) in ladder {
                move_to(c, [lightness_to_luminance(l); 3]);
            }
            for c in [
                &mut p.violet,
                &mut p.indigo,
                &mut p.teal,
                &mut p.navy,
                &mut p.panel,
                &mut p.bar,
                &mut p.field,
                &mut p.raised,
                &mut p.violet_light,
                &mut p.violet_pale,
                &mut p.ink,
                &mut p.muted,
                &mut p.rose,
                &mut p.danger_dim,
                &mut p.indigo_light,
                &mut p.on_accent,
            ] {
                let grey = [luminance(linear(*c)); 3];
                move_to(c, grey);
            }
        }
    }
    p
}

#[cfg(test)]
mod tests;
