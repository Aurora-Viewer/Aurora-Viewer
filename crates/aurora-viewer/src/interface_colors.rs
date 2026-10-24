//! Interface colours (Preferences › Appearance › Skin): the user picks the
//! accent, the windows' background and the text, and every other colour of
//! the skin's palette follows from them, so that a theme of one's own (a
//! pink interface, say) needs no hand-edited `theme.json`. Aurora's own;
//! the Legacy Viewer has no such option.
//!
//! The other colours keep the relations the skin gives them with their
//! base, so that the skin's own bases give back every colour exactly
//! (`tests`):
//! - the accent family (`violet`: `indigo`, `violet_light`, `violet_pale`,
//!   `indigo_light`) keeps its lightness step, chroma ratio and hue turn
//!   from `violet`, in OKLCH;
//! - the surfaces (`panel`: `bar`, `field`, `raised`, `navy`) and the text
//!   (`ink`: `muted`, `muted_dim`) keep their place on the line from their
//!   base to the other (a button face a little toward the text, a muted
//!   label part way to the panel), in OKLab, and their tint off that line
//!   turned with the base's hue. A light background thus gets darker
//!   button faces and lighter muted text, not white on white.
//!
//! Readability: the colours are moved in lightness, never refused, until
//! text and muted text read 4.5:1 against the panel (WCAG AA), the dim
//! text and the accent 3:1 (non-text); the page says when it happened.
//! `on_accent` is the skin's (white) while it reads at least as well on
//! the new accent as on the skin's own, else the skin's navy.
//!
//! The state colours (danger, success, warn, amber, rose) and the links'
//! teal keep their hue but move in lightness, like the text, to read 4.5:1
//! on a chosen background; so do the configurable colours of the chat
//! (`on_panel`, read by `ui::colors::text`). The high contrast palette
//! (`crate::high_contrast`) ignores these choices: its colours are
//! measured for poor vision.
//!
//! The choice is process-wide (`set`), read where the palette is built
//! (`Theme::palette_for`), before the colour blindness adaptation.

use crate::settings::Rgba;
use crate::theme::Palette;
use egui::Color32;
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, RwLock};

/// The colours chosen in the preferences; `None` keeps the skin's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InterfaceColors {
    pub accent: Option<Rgba>,
    pub surface: Option<Rgba>,
    pub text: Option<Rgba>,
}

impl InterfaceColors {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

static CURRENT: RwLock<InterfaceColors> = RwLock::new(InterfaceColors {
    accent: None,
    surface: None,
    text: None,
});
/// The skin's palette of the last build, for the page's swatches.
static SKIN: Mutex<Option<Palette>> = Mutex::new(None);
/// The chosen panel of the last build (`None`: the skin's), and the chat
/// colours already moved onto it.
static PANEL: Mutex<Option<(Color32, Vec<(Color32, Color32)>)>> = Mutex::new(None);

/// The colours the interface uses from now on (the palette is built again
/// by its users).
pub fn set(colors: InterfaceColors) {
    if let Ok(mut c) = CURRENT.write() {
        *c = colors;
    }
}

pub fn current() -> InterfaceColors {
    CURRENT.read().map(|c| c.clone()).unwrap_or_default()
}

/// The skin's own colours (before these choices), as last built.
pub fn skin() -> Option<Palette> {
    SKIN.lock().ok().and_then(|s| *s)
}

/// `AURORA_DEMO_COLORS=accent:#14B8A6,surface:#3A2433,text:#FCE7F3`
/// (demo mode only, any of the three): this run's choice, never saved.
pub fn demo_override() -> Option<InterfaceColors> {
    std::env::var("AURORA_DEMO_COLORS").ok().map(|raw| parse(&raw))
}

fn parse(raw: &str) -> InterfaceColors {
    let mut out = InterfaceColors::default();
    for part in raw.split(',') {
        let Some((name, value)) = part.split_once(':') else { continue };
        let c = crate::theme::hex(value).to_array();
        let slot = match name.trim() {
            "accent" => &mut out.accent,
            "surface" => &mut out.surface,
            "text" => &mut out.text,
            _ => continue,
        };
        *slot = Some([c[0], c[1], c[2], 255]);
    }
    out
}

/// What the readability rules moved, for the page's note.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Adjusted {
    pub text: bool,
    pub accent: bool,
}

/// The skin's palette with `colors` applied (see the module).
pub fn apply(skin: Palette, colors: &InterfaceColors) -> Palette {
    if let Ok(mut s) = SKIN.lock() {
        *s = Some(skin);
    }
    let p = derive(skin, colors).0;
    if let Ok(mut panel) = PANEL.lock() {
        let wanted = (p.panel != skin.panel).then_some(p.panel);
        if panel.as_ref().map(|(c, _)| *c) != wanted {
            *panel = wanted.map(|c| (c, Vec::new()));
        }
    }
    p
}

/// A configurable text colour (the chat's: messages, names, links) moved
/// in lightness, its hue kept, until it reads 4.5:1 on the chosen panel;
/// itself with the skin's panel or in high contrast.
pub fn on_panel(c: Color32) -> Color32 {
    if crate::high_contrast::current() {
        return c;
    }
    let Ok(mut panel) = PANEL.lock() else { return c };
    let Some((ground, seen)) = panel.as_mut() else { return c };
    if let Some((_, out)) = seen.iter().find(|(k, _)| *k == c) {
        return *out;
    }
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    let moved = readable(Color32::from_rgb(r, g, b), *ground, TEXT);
    let out = Color32::from_rgba_unmultiplied(moved.r(), moved.g(), moved.b(), a);
    // a handful of colours: the chat's settings and the contact sets'
    if seen.len() > 64 {
        seen.clear();
    }
    seen.push((c, out));
    out
}

/// What the readability rules would move for `colors` over the skin.
pub fn adjusted(colors: &InterfaceColors) -> Adjusted {
    skin().map_or_else(Adjusted::default, |s| derive(s, colors).1)
}

/// Minimum contrasts against the panel (WCAG 2): text, and non-text.
const TEXT: f32 = 4.5;
const UI: f32 = 3.0;

pub fn derive(skin: Palette, colors: &InterfaceColors) -> (Palette, Adjusted) {
    let mut p = skin;
    let mut adjusted = Adjusted::default();
    let rgb = |c: Rgba| Color32::from_rgb(c[0], c[1], c[2]);
    let (surface, text) = (colors.surface.map(rgb), colors.text.map(rgb));
    if surface.is_some() || text.is_some() {
        p.panel = surface.unwrap_or(skin.panel);
        let ink = text.unwrap_or(skin.ink);
        p.ink = readable(ink, p.panel, TEXT);
        adjusted.text = p.ink != ink;
        for (out, of) in [
            (&mut p.bar, skin.bar),
            (&mut p.field, skin.field),
            (&mut p.raised, skin.raised),
            (&mut p.navy, skin.navy),
        ] {
            *out = Along::of(skin.panel, skin.ink, of).on(p.panel, p.ink);
        }
        p.muted = readable(Along::of(skin.ink, skin.panel, skin.muted).on(p.ink, p.panel), p.panel, TEXT);
        p.muted_dim = readable(Along::of(skin.ink, skin.panel, skin.muted_dim).on(p.ink, p.panel), p.panel, UI);
        // the states and the links keep their hue, in the lightness that
        // reads on the new panel: they colour text as well as dots and
        // badges, so the text minimum (the skin's already reach it)
        if p.panel != skin.panel {
            for c in [&mut p.danger, &mut p.success, &mut p.warn, &mut p.amber, &mut p.rose, &mut p.teal] {
                *c = readable(*c, p.panel, TEXT);
            }
        }
    }
    let wanted = colors.accent.map(rgb);
    if wanted.is_some() || surface.is_some() {
        let accent = wanted.unwrap_or(skin.violet);
        p.violet = readable(accent, p.panel, UI);
        adjusted.accent = p.violet != accent;
        if p.violet != skin.violet {
            for (out, of) in [
                (&mut p.indigo, skin.indigo),
                (&mut p.violet_light, skin.violet_light),
                (&mut p.violet_pale, skin.violet_pale),
                (&mut p.indigo_light, skin.indigo_light),
            ] {
                *out = Turn::of(skin.violet, of).on(p.violet);
            }
            p.on_accent = on_accent(&skin, p.violet);
        }
    }
    (p, adjusted)
}

/// Text on the accent: the skin's while it reads at least as well as on
/// the skin's own accent (up to 4.5:1), else the better of it and navy.
fn on_accent(skin: &Palette, accent: Color32) -> Color32 {
    let need = TEXT.min(contrast(skin.on_accent, skin.violet));
    let light = contrast(skin.on_accent, accent);
    if light >= need || light >= contrast(skin.navy, accent) {
        skin.on_accent
    } else {
        skin.navy
    }
}

/// An accent colour's relation to the accent: lightness step, chroma
/// ratio, hue turn (OKLCH).
struct Turn {
    dl: f64,
    chroma: f64,
    hue: f64,
}

impl Turn {
    fn of(base: Color32, c: Color32) -> Self {
        let ([lb, cb, hb], [l, ch, h]) = (lch(lab(base)), lch(lab(c)));
        Turn {
            dl: l - lb,
            chroma: if cb > 1e-9 { ch / cb } else { 1.0 },
            hue: h - hb,
        }
    }

    fn on(&self, base: Color32) -> Color32 {
        let [l, c, h] = lch(lab(base));
        let (c, h) = (c * self.chroma, h + self.hue);
        rgb([l + self.dl, c * h.cos(), c * h.sin()])
    }
}

/// A colour's place between its base and the other pole (the text for a
/// surface, the panel for a text): how far along the line (`t`), and its
/// tint off it in the base's hue frame (along the hue, across it).
struct Along {
    t: f64,
    off: [f64; 3],
    chroma: f64,
}

impl Along {
    fn of(base: Color32, pole: Color32, c: Color32) -> Self {
        let (b, q, x) = (lab(base), lab(pole), lab(c));
        let axis = sub(q, b);
        let len = dot(axis, axis);
        let t = if len > 1e-12 { dot(sub(x, b), axis) / len } else { 0.0 };
        let r = sub(sub(x, b), scale(axis, t));
        let (chroma, [u, v]) = hue_frame(b);
        Along {
            t,
            off: [r[0], r[1] * u + r[2] * v, r[2] * u - r[1] * v],
            chroma,
        }
    }

    fn on(&self, base: Color32, pole: Color32) -> Color32 {
        let (b, q) = (lab(base), lab(pole));
        let (chroma, [u, v]) = hue_frame(b);
        // a grey base gets no tint; a more colourful one no more than the
        // skin's
        let k = if self.chroma > 1e-9 { (chroma / self.chroma).min(1.0) } else { 0.0 };
        let [dl, along, across] = self.off;
        let tint = [dl, (along * u - across * v) * k, (along * v + across * u) * k];
        rgb(add(add(b, scale(sub(q, b), self.t)), tint))
    }
}

/// `c` moved in lightness, as little as it takes, until it reads `min`:1
/// against `ground`: away from the ground on its own side, or the other
/// side if that one cannot reach it.
fn readable(c: Color32, ground: Color32, min: f32) -> Color32 {
    if contrast(c, ground) >= min {
        return c;
    }
    let [l, a, b] = lab(c);
    let lighter = luminance(c) >= luminance(ground);
    let reach = |end: f64| {
        let at = |x: f64| rgb([l + (end - l) * x, a, b]);
        if contrast(at(1.0), ground) < min {
            return None;
        }
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..32 {
            let mid = (lo + hi) / 2.0;
            if contrast(at(mid), ground) >= min { hi = mid } else { lo = mid }
        }
        Some(at(hi))
    };
    let (first, second) = if lighter { (1.0, 0.0) } else { (0.0, 1.0) };
    reach(first).or_else(|| reach(second)).unwrap_or_else(|| {
        // neither side reaches it (a mid grey ground and a high minimum):
        // the side that reads best
        let (w, k) = (rgb([1.0, 0.0, 0.0]), rgb([0.0, 0.0, 0.0]));
        if contrast(w, ground) >= contrast(k, ground) { w } else { k }
    })
}

/// Contrast ratio (WCAG 2) between two opaque colours, 1 to 21.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    ((la.max(lb) + 0.05) / (la.min(lb) + 0.05)) as f32
}

fn linear(v: u8) -> f64 {
    let v = f64::from(v) / 255.0;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn encode(v: f64) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round() as u8
}

fn luminance(c: Color32) -> f64 {
    0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
}

/// OKLab (Björn Ottosson, 2020) of an opaque sRGB colour.
fn lab(c: Color32) -> [f64; 3] {
    let (r, g, b) = (linear(c.r()), linear(c.g()), linear(c.b()));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    [
        0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    ]
}

fn linear_rgb([l, a, b]: [f64; 3]) -> [f64; 3] {
    let l_ = (l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m_ = (l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s_ = (l - 0.089_484_177_5 * a - 1.291_485_548_0 * b).powi(3);
    [
        4.076_741_662_1 * l_ - 3.307_711_591_3 * m_ + 0.230_969_929_2 * s_,
        -1.268_438_004_6 * l_ + 2.609_757_401_1 * m_ - 0.341_319_396_5 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_614_7 * m_ + 1.707_614_701_0 * s_,
    ]
}

/// The sRGB colour of an OKLab one, its chroma reduced (lightness and hue
/// kept) until it fits in sRGB.
fn rgb([l, a, b]: [f64; 3]) -> Color32 {
    let l = l.clamp(0.0, 1.0);
    let fits = |k: f64| linear_rgb([l, a * k, b * k]).iter().all(|v| (-1e-7..=1.0 + 1e-7).contains(v));
    let k = if fits(1.0) {
        1.0
    } else {
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..32 {
            let mid = (lo + hi) / 2.0;
            if fits(mid) { lo = mid } else { hi = mid }
        }
        lo
    };
    let [r, g, bl] = linear_rgb([l, a * k, b * k]);
    Color32::from_rgb(encode(r), encode(g), encode(bl))
}

fn lch([l, a, b]: [f64; 3]) -> [f64; 3] {
    [l, a.hypot(b), b.atan2(a)]
}

/// A colour's chroma and the unit vector of its hue (+a for a grey).
fn hue_frame([_, a, b]: [f64; 3]) -> (f64, [f64; 2]) {
    let c = a.hypot(b);
    if c > 1e-9 { (c, [a / c, b / c]) } else { (0.0, [1.0, 0.0]) }
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], k: f64) -> [f64; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[cfg(test)]
mod tests;
