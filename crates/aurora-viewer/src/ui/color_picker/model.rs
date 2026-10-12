//! Color values and saved palette of LLFloaterColorPicker (Firestorm,
//! llfloatercolorpicker.cpp and skins/default/colors.xml, originally LGPL 2.1).

use serde::{Deserialize, Serialize};

pub type Rgb = [f32; 3];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// ApplyColorImmediately: on until chosen otherwise, then persisted.
    pub apply: bool,
    /// ColorPaletteEntry01..32, in sRGB, shared by the construction swatches.
    pub palette: [Rgb; 32],
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            apply: true,
            palette: [
                [0.0, 0.0, 0.0],
                [0.5, 0.5, 0.5],
                [0.5, 0.0, 0.0],
                [0.5, 0.5, 0.0],
                [0.0, 0.5, 0.0],
                [0.0, 0.5, 0.5],
                [0.0, 0.0, 0.5],
                [0.5, 0.0, 0.5],
                [0.5, 0.5, 0.0],
                [0.0, 0.25, 0.25],
                [0.0, 0.5, 1.0],
                [0.0, 0.25, 0.5],
                [0.5, 0.0, 1.0],
                [0.5, 0.25, 0.0],
                [1.0; 3],
                [1.0; 3],
                [1.0; 3],
                [0.75; 3],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 1.0, 1.0],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [1.0, 1.0, 0.5],
                [0.0, 1.0, 0.5],
                [0.5, 1.0, 1.0],
                [0.5, 0.5, 1.0],
                [1.0, 0.0, 0.5],
                [1.0, 0.5, 0.0],
                [1.0; 3],
                [1.0; 3],
            ],
        }
    }
}

impl Preferences {
    pub fn normalize(&mut self) {
        self.palette.iter_mut().for_each(|rgb| *rgb = clamp_rgb(*rgb));
    }

    pub fn store(&mut self, slot: usize, rgb: Rgb) {
        if let Some(color) = self.palette.get_mut(slot) {
            *color = clamp_rgb(rgb);
        }
    }
}

pub fn unit(value: f32) -> f32 {
    if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 }
}

pub fn clamp_rgb(rgb: Rgb) -> Rgb {
    rgb.map(unit)
}

pub fn rgb_to_hsl(rgb: Rgb) -> Rgb {
    let [r, g, b] = clamp_rgb(rgb);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let light = (max + min) * 0.5;
    if delta <= f32::EPSILON {
        return [0.0, 0.0, light];
    }
    let saturation = delta / (1.0 - (2.0 * light - 1.0).abs());
    let hue = if max == r {
        (g - b) / delta
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    [(hue / 6.0).rem_euclid(1.0), unit(saturation), light]
}

pub fn hsl_to_rgb(hsl: Rgb) -> Rgb {
    let [h, s, l] = hsl.map(unit);
    let hue = h * 6.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (hue.rem_euclid(2.0) - 1.0).abs());
    let base = match hue as usize % 6 {
        0 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    };
    base.map(|v| unit(v + l - c * 0.5))
}

pub fn bytes(rgb: Rgb) -> [u8; 3] {
    // Unlike Firestorm's truncated hex output, use the same rounding as the
    // RGB fields so both formats describe exactly the same byte color.
    clamp_rgb(rgb).map(|v| (v * 255.0).round() as u8)
}

pub fn hex(rgb: Rgb) -> String {
    let [r, g, b] = bytes(rgb);
    format!("{r:02X}{g:02X}{b:02X}")
}

pub fn parse_hex(text: &str) -> Option<Rgb> {
    let text = text.trim().strip_prefix('#').unwrap_or(text.trim());
    if text.len() != 6 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some([
        u8::from_str_radix(&text[..2], 16).ok()? as f32 / 255.0,
        u8::from_str_radix(&text[2..4], 16).ok()? as f32 / 255.0,
        u8::from_str_radix(&text[4..], 16).ok()? as f32 / 255.0,
    ])
}

/// LSL always uses decimal points, independently of the interface locale.
pub fn lsl(rgb: Rgb) -> String {
    let [r, g, b] = clamp_rgb(rgb);
    format!("<{r:.3}, {g:.3}, {b:.3}>")
}

#[derive(Debug, Clone)]
pub struct Draft {
    pub rgb: Rgb,
    pub original: Rgb,
    pub hsl: Rgb,
    pub hex: String,
    pub mode: usize,
}

impl Draft {
    pub fn new(rgb: Rgb) -> Self {
        let rgb = clamp_rgb(rgb);
        Self {
            rgb,
            original: rgb,
            hsl: rgb_to_hsl(rgb),
            hex: hex(rgb),
            mode: 0,
        }
    }

    pub fn set_rgb(&mut self, rgb: Rgb) -> bool {
        let rgb = clamp_rgb(rgb);
        let changed = self.rgb != rgb;
        self.rgb = rgb;
        self.hsl = rgb_to_hsl(rgb);
        self.hex = hex(rgb);
        changed
    }

    pub fn set_hsl(&mut self, hsl: Rgb) -> bool {
        // Preserve hue/saturation at black and white so the lightness slider
        // can return to the chosen hue without a jump to red.
        self.hsl = hsl.map(unit);
        let rgb = hsl_to_rgb(self.hsl);
        let changed = self.rgb != rgb;
        self.rgb = rgb;
        self.hex = hex(rgb);
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_round_trips_primaries_grays_and_a_dense_rgb_grid() {
        for r in 0..=12 {
            for g in 0..=12 {
                for b in 0..=12 {
                    let rgb = [r as f32 / 12.0, g as f32 / 12.0, b as f32 / 12.0];
                    let restored = hsl_to_rgb(rgb_to_hsl(rgb));
                    assert!(
                        rgb.into_iter().zip(restored).all(|(a, b)| (a - b).abs() < 0.000002),
                        "{rgb:?} -> {restored:?}"
                    );
                }
            }
        }
        assert_eq!(hsl_to_rgb([1.0, 1.0, 0.5]), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn lightness_preserves_the_selected_hue_at_the_endpoints() {
        let mut draft = Draft::new([0.0, 1.0, 0.0]);
        draft.set_hsl([1.0 / 3.0, 1.0, 0.0]);
        assert_eq!(draft.rgb, [0.0; 3]);
        draft.set_hsl([draft.hsl[0], draft.hsl[1], 0.5]);
        assert_eq!(draft.rgb, [0.0, 1.0, 0.0]);
        draft.set_hsl([draft.hsl[0], draft.hsl[1], 1.0]);
        assert_eq!(draft.rgb, [1.0; 3]);
    }

    #[test]
    fn rgb_hex_and_lsl_are_consistent_and_invalid_hex_never_changes_color() {
        assert_eq!(hex([0.5, 0.25, 1.0]), "8040FF");
        assert_eq!(parse_hex(" #8040ff "), Some([128.0 / 255.0, 64.0 / 255.0, 1.0]));
        assert_eq!(lsl([0.5, 0.25, 1.0]), "<0.500, 0.250, 1.000>");
        for text in ["", "FFF", "FFFFFG", "🦊123", "１２３４５６", "1234567"] {
            assert_eq!(parse_hex(text), None);
        }
    }

    #[test]
    fn saved_slots_and_apply_mode_survive_serialization_and_bad_values_are_clamped() {
        let mut prefs = Preferences {
            apply: false,
            ..Default::default()
        };
        prefs.store(31, [0.12, 0.34, 0.56]);
        let restored: Preferences = serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert_eq!(restored, prefs);
        prefs.store(32, [0.0; 3]);
        prefs.palette[0] = [f32::NAN, 2.0, -1.0];
        prefs.normalize();
        assert_eq!(prefs.palette[0], [0.0, 1.0, 0.0]);
    }
}
