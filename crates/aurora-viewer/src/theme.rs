//! Customisable egui theme. Defaults follow the Aurora brand palette
//! (flat, violet/indigo accents on navy). Users can override any color in
//! `theme.json` in the config directory.

use egui::{Color32, CornerRadius, Stroke, Visuals};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub violet: String,
    pub indigo: String,
    pub teal: String,
    pub navy: String,
    pub panel: String,
    pub bar: String,
    pub field: String,
    pub raised: String,
    pub violet_light: String,
    pub violet_pale: String,
    pub ink: String,
    pub muted: String,
    pub muted_dim: String,
    pub amber: String,
    pub rose: String,
    pub danger: String,
    pub success: String,
    pub warn: String,
    pub danger_dim: String,
    pub indigo_light: String,
    pub panel_alpha: u8,
    pub button_radius: u8,
    pub window_radius: u8,
    pub font_scale: f32,
}

impl Default for Theme {
    /// Brand colors from `scripts/aurora/brand.psd1`; window neutrals from the
    /// Aurora Firestorm theme (`themes/aurora/colors.xml`: DkGray, AlchemyBlack1, MouseGray).
    fn default() -> Self {
        Theme {
            violet: "#8B5CF6".into(),
            indigo: "#4F46E5".into(),
            teal: "#5EEAD4".into(),
            navy: "#070B1F".into(),
            panel: "#2A2A2A".into(),
            bar: "#1C1C1C".into(),
            field: "#1C1C1C".into(),
            raised: "#3A3A3A".into(),
            violet_light: "#A78BFA".into(),
            violet_pale: "#C4B5FD".into(),
            ink: "#E8EDFB".into(),
            muted: "#9AA6C6".into(),
            muted_dim: "#6F7BA0".into(),
            amber: "#FCD34D".into(),
            rose: "#F472B6".into(),
            danger: "#F87171".into(),
            success: "#4ADE80".into(),
            warn: "#FB923C".into(),
            danger_dim: "#A85252".into(),
            indigo_light: "#818CF8".into(),
            panel_alpha: 245,
            button_radius: 3,
            window_radius: 2,
            font_scale: 1.0,
        }
    }
}

pub fn hex(s: &str) -> Color32 {
    let t = s.trim().trim_start_matches('#');
    let p = |i: usize| t.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).unwrap_or(255);
    if t.len() >= 8 {
        Color32::from_rgba_unmultiplied(p(0), p(2), p(4), p(6))
    } else {
        Color32::from_rgb(p(0), p(2), p(4))
    }
}

/// Resolved colors for direct use in UI code.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub violet: Color32,
    pub indigo: Color32,
    pub teal: Color32,
    pub navy: Color32,
    pub panel: Color32,
    pub bar: Color32,
    pub field: Color32,
    pub raised: Color32,
    pub violet_light: Color32,
    pub violet_pale: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub muted_dim: Color32,
    pub amber: Color32,
    pub rose: Color32,
    pub danger: Color32,
    pub success: Color32,
    pub warn: Color32,
    #[allow(dead_code)] // part of the skin palette, not used by a widget yet
    pub danger_dim: Color32,
    pub indigo_light: Color32,
    pub panel_alpha: u8,
}

impl Theme {
    pub fn palette(&self) -> Palette {
        Palette {
            violet: hex(&self.violet),
            indigo: hex(&self.indigo),
            teal: hex(&self.teal),
            navy: hex(&self.navy),
            panel: hex(&self.panel),
            bar: hex(&self.bar),
            field: hex(&self.field),
            raised: hex(&self.raised),
            violet_light: hex(&self.violet_light),
            violet_pale: hex(&self.violet_pale),
            ink: hex(&self.ink),
            muted: hex(&self.muted),
            muted_dim: hex(&self.muted_dim),
            amber: hex(&self.amber),
            rose: hex(&self.rose),
            danger: hex(&self.danger),
            success: hex(&self.success),
            warn: hex(&self.warn),
            danger_dim: hex(&self.danger_dim),
            indigo_light: hex(&self.indigo_light),
            panel_alpha: self.panel_alpha,
        }
    }

    pub fn apply(&self, ctx: &egui::Context, extra_font_scale: f32) {
        let p = self.palette();
        let mut v = Visuals::dark();
        let br = CornerRadius::same(self.button_radius);
        let panel = Color32::from_rgba_unmultiplied(p.panel.r(), p.panel.g(), p.panel.b(), p.panel_alpha);
        v.override_text_color = Some(p.ink);
        v.hyperlink_color = p.teal;
        v.faint_bg_color = p.field;
        v.extreme_bg_color = p.field;
        v.code_bg_color = p.field;
        v.warn_fg_color = p.amber;
        v.error_fg_color = p.danger;
        v.window_fill = panel;
        v.panel_fill = panel;
        v.window_stroke = Stroke::new(1.0, p.raised);
        v.window_corner_radius = CornerRadius::same(self.window_radius);
        v.menu_corner_radius = CornerRadius::same(self.window_radius);
        v.window_shadow = egui::Shadow::NONE;
        v.popup_shadow = egui::Shadow::NONE;
        v.selection.bg_fill = p.violet;
        v.selection.stroke = Stroke::new(1.0, p.ink);

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = panel;
        w.noninteractive.weak_bg_fill = panel;
        w.noninteractive.bg_stroke = Stroke::new(1.0, p.raised);
        w.noninteractive.fg_stroke = Stroke::new(1.0, p.muted);
        w.noninteractive.corner_radius = br;

        // also the slider rail / checkbox background
        w.inactive.bg_fill = p.raised;
        w.inactive.weak_bg_fill = p.field;
        w.inactive.bg_stroke = Stroke::new(1.0, p.raised);
        w.inactive.fg_stroke = Stroke::new(1.0, p.ink);
        w.inactive.corner_radius = br;

        w.hovered.bg_fill = p.raised;
        w.hovered.weak_bg_fill = p.raised;
        w.hovered.bg_stroke = Stroke::new(1.0, p.violet_light);
        w.hovered.fg_stroke = Stroke::new(1.0, p.ink);
        w.hovered.corner_radius = br;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = p.indigo;
        w.active.weak_bg_fill = p.indigo;
        w.active.bg_stroke = Stroke::new(1.0, p.violet_light);
        w.active.fg_stroke = Stroke::new(1.0, p.ink);
        w.active.corner_radius = br;
        w.active.expansion = 0.0;

        w.open = w.hovered;
        w.open.bg_fill = p.raised;
        w.open.weak_bg_fill = p.raised;

        let mut style = egui::Style {
            visuals: v,
            ..Default::default()
        };
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 26.0;
        style.spacing.text_edit_width = 280.0;
        // egui makes every label selectable by default; like Firestorm's
        // LLTextBox, UI text (window titles, captions) is not. Text meant to be
        // copied (UUIDs, covenant) opts back in with `Label::selectable(true)`.
        style.interaction.selectable_labels = false;
        if let Some(h) = style.text_styles.get_mut(&egui::TextStyle::Heading) {
            h.size = 15.0;
        }
        for font in style.text_styles.values_mut() {
            font.size *= self.font_scale.clamp(0.6, 2.0) * extra_font_scale.clamp(0.5, 2.0);
        }
        ctx.set_theme(egui::ThemePreference::Dark);
        ctx.all_styles_mut(|s| *s = style.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parsing() {
        assert_eq!(hex("#8B5CF6"), Color32::from_rgb(0x8b, 0x5c, 0xf6));
        assert_eq!(hex("070B1F"), Color32::from_rgb(7, 11, 31));
    }
}
