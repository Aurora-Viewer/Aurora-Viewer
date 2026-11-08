//! Customisable egui theme. Defaults follow the Aurora brand palette
//! (subtle glass, violet/indigo accents on navy). Users can override any color in
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
    pub on_accent: String,
    pub panel_alpha: u8,
    pub button_radius: u8,
    pub window_radius: u8,
    pub font_scale: f32,
}

impl Default for Theme {
    /// Brand colors from `scripts/aurora/brand.psd1`; window surfaces follow
    /// the shared hierarchy documented in `docs/BRANDING.md`.
    fn default() -> Self {
        Theme {
            violet: "#8B5CF6".into(),
            indigo: "#4F46E5".into(),
            teal: "#5EEAD4".into(),
            navy: "#070B1F".into(),
            panel: "#252830".into(),
            bar: "#1B1E26".into(),
            field: "#181B23".into(),
            raised: "#363B47".into(),
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
            on_accent: "#FFFFFF".into(),
            panel_alpha: 230,
            button_radius: 5,
            window_radius: 3,
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
    pub danger_dim: Color32,
    pub indigo_light: Color32,
    /// Text and icons on a face filled with the accent (primary button,
    /// selected tool, checked box): `ink` belongs to the panels, and a skin
    /// may give them a dark one.
    pub on_accent: Color32,
    pub panel_alpha: u8,
    /// The high contrast palette of Preferences › Accessibility
    /// (`crate::high_contrast`): buttons, fields, windows, islands and
    /// dividers then get a light border (`muted_dim`), which the usual look
    /// leaves out.
    pub high_contrast: bool,
}

impl Palette {
    /// The brand gradient in the world (docs/BRANDING.md: gradients are for
    /// brand elements), from its bright end to its deep one: teal, light
    /// indigo, violet, the colors of the aurora of the login screen. The
    /// rez effect runs along it (scene/rez.rs).
    pub fn aurora_gradient(&self) -> [Color32; 3] {
        [self.teal, self.indigo_light, self.violet]
    }

    /// A northern light, from the green of its foot to the violet of its
    /// top: the Aurora team's marks (`world::staff`) run along it.
    pub fn borealis(&self) -> [Color32; 4] {
        [self.success, self.teal, self.indigo_light, self.violet]
    }

    /// The color `t` along `borealis` (0 the green foot, 1 the violet top).
    pub fn borealis_at(&self, t: f32) -> Color32 {
        let stops = self.borealis();
        let x = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
        let i = (x as usize).min(stops.len() - 2);
        stops[i].lerp_to_gamma(stops[i + 1], x - i as f32)
    }
}

impl Theme {
    /// The colors, those of the high contrast palette instead of the skin's
    /// when it is chosen (`crate::high_contrast`), else the skin's with the
    /// interface colours of the preferences (`crate::interface_colors`),
    /// adapted to the colour blindness chosen in the preferences
    /// (`crate::colorblind`).
    pub fn palette(&self) -> Palette {
        self.palette_for(crate::colorblind::current())
    }

    pub fn palette_for(&self, vision: crate::colorblind::Vision) -> Palette {
        let base = if crate::high_contrast::current() {
            crate::high_contrast::palette()
        } else {
            crate::interface_colors::apply(self.skin_palette(), &crate::interface_colors::current())
        };
        crate::colorblind::palette(base, vision)
    }

    /// The colors as the skin writes them.
    pub fn skin_palette(&self) -> Palette {
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
            on_accent: hex(&self.on_accent),
            panel_alpha: self.panel_alpha,
            high_contrast: false,
        }
    }

    pub fn apply(&self, ctx: &egui::Context, extra_font_scale: f32) {
        let p = self.palette();
        // `v.text_options` stays at epaint's dark-theme defaults on purpose
        // (compared live in demo and chosen by the human on 2026-10-16):
        // coverage → alpha 2c − c² suits light text on the dark glass, where
        // lighter curves (gamma 0.75, linear) wash out the small grey labels
        // and heavier ones (gamma 0.4) clog them; hinting on (off is
        // blurrier); sub-pixel binning on, which keeps the letter spacing
        // even at 11–12 points. The hinting target changes nothing for Inter
        // and Roboto, which ship their own TrueType hinting.
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
        v.window_shadow = egui::Shadow {
            offset: [0, 4],
            blur: 14,
            spread: 0,
            color: p.bar.gamma_multiply(0.35),
        };
        v.popup_shadow = v.window_shadow;
        v.selection.bg_fill = p.violet;
        v.selection.stroke = Stroke::new(1.0, p.on_accent);
        // LLLineEditor::draw (indra/llui/lllineeditor.cpp) and
        // LLTextEditor::drawPreeditMarker (indra/llui/lltexteditor.cpp)
        // underline the composition and distinguish its active segment.
        // egui's Windows default instead selects the whole preedit (#2608).
        v.ime_composition.legacy_visuals = false;
        v.ime_composition.active_underline_stroke = Stroke::new(2.0, p.ink);
        v.ime_composition.inactive_underline_stroke = Stroke::new(1.0, p.muted);

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = panel;
        w.noninteractive.weak_bg_fill = panel;
        w.noninteractive.bg_stroke = Stroke::new(1.0, p.raised);
        w.noninteractive.fg_stroke = Stroke::new(1.0, p.muted);
        w.noninteractive.corner_radius = br;

        // Buttons and combo boxes are plain filled faces (the human's
        // choice): their border has the color of the face, so only text
        // fields and check boxes, drawn on a darker ground, show it.
        // bg_fill is also the slider rail / checkbox background.
        let hot = p.raised.lerp_to_gamma(p.ink, 0.16);
        w.inactive.bg_fill = p.raised;
        w.inactive.weak_bg_fill = p.raised;
        w.inactive.bg_stroke = Stroke::new(1.0, p.raised);
        w.inactive.fg_stroke = Stroke::new(1.0, p.ink);
        w.inactive.corner_radius = br;

        w.hovered.bg_fill = hot;
        w.hovered.weak_bg_fill = hot;
        w.hovered.bg_stroke = Stroke::new(1.0, hot);
        w.hovered.fg_stroke = Stroke::new(1.0, p.ink);
        w.hovered.corner_radius = br;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = p.indigo;
        w.active.weak_bg_fill = p.indigo;
        w.active.bg_stroke = Stroke::new(1.0, p.violet_light);
        w.active.fg_stroke = Stroke::new(1.0, p.ink);
        w.active.corner_radius = br;
        w.active.expansion = 0.0;

        // High contrast (#906): every button, field and window shows a light
        // border (3:1 at least against the black panels), the hovered and
        // pressed ones a white one, and the pressed face is the light accent,
        // so its text takes `on_accent`.
        if p.high_contrast {
            let border = Stroke::new(1.0, p.muted_dim);
            w.noninteractive.bg_stroke = border;
            w.inactive.bg_stroke = border;
            w.hovered.bg_stroke = Stroke::new(1.0, p.ink);
            w.active.bg_stroke = Stroke::new(1.0, p.ink);
            w.active.fg_stroke = Stroke::new(1.0, p.on_accent);
        }

        w.open = w.hovered;
        if p.high_contrast {
            v.window_stroke = Stroke::new(1.0, p.muted_dim);
            v.window_shadow = egui::Shadow::NONE;
            v.popup_shadow = egui::Shadow::NONE;
        }

        let mut style = egui::Style {
            visuals: v,
            number_formatter: egui::style::NumberFormatter::new(|value, decimals| {
                aurora_i18n::number_text(&egui::emath::format_with_decimals_in_range(value, decimals))
            }),
            ..Default::default()
        };
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 26.0;
        style.spacing.text_edit_width = 280.0;
        // never egui's floating bar, drawn over the content's right edge
        style.spacing.scroll = crate::ui::widgets::reserved_scroll_style();
        // egui makes every label selectable by default; like Firestorm's
        // LLTextBox, UI text (window titles, captions) is not. Text meant to be
        // copied (chat and IM messages, notification bodies, profile texts,
        // UUIDs, covenant) opts back in with `Label::selectable(true)`.
        style.interaction.selectable_labels = false;
        if let Some(h) = style.text_styles.get_mut(&egui::TextStyle::Heading) {
            h.size = 15.0;
        }
        crate::ui::fonts::set_scale(ctx, self.font_scale.clamp(0.6, 2.0) * extra_font_scale.clamp(0.5, 2.0));
        ctx.set_theme(egui::ThemePreference::Dark);
        ctx.all_styles_mut(|s| *s = style.clone());
        ctx.replace_widget_theme::<egui::widget_style::TextEditStyle>(FieldSize);
        // every interface context goes through here
        crate::ui::click_away_focus::install(ctx);
    }
}

/// "Reduce animations" (Preferences › Accessibility): egui's own
/// transitions (windows and menus fading in, sections unrolling, switches
/// sliding) end at once; egui's duration otherwise. Checked every frame,
/// since `Theme::apply` (a skin reload) restores egui's style.
pub fn apply_motion(ctx: &egui::Context, reduce: bool) {
    let wanted = if reduce { 0.0 } else { egui::Style::default().animation_time };
    if ctx.global_style().animation_time != wanted {
        ctx.all_styles_mut(|s| s.animation_time = wanted);
    }
}

/// The size of the text fields: egui 0.37 makes them as high as a button
/// (`interact_size`, 26 points here) with a button's padding; they keep egui
/// 0.36's, 4 × 2 points around their text (stroke included) and no minimum
/// height, so no window grows. Their capitals sit in the middle
/// (`widgets::field_text_margin`). The rest is egui's look.
struct FieldSize;

impl egui::theme::StyleProvider<egui::widget_style::TextEditStyle> for FieldSize {
    fn style(&mut self, args: &egui::widget_style::StyleArgs<'_>) -> egui::widget_style::TextEditStyle {
        let mut field: egui::widget_style::TextEditStyle = egui::theme::StyleProvider::style(&mut egui::theme::DefaultStyle, args);
        let frame = &mut field.atom_layout.frame;
        let centre = crate::ui::widgets::field_text_margin(args.ctx, &field.atom_layout.text_style.font_id);
        frame.inner_margin =
            frame.inner_margin - egui::Margin::from(args.style.spacing.button_padding) + egui::Margin::symmetric(4, 2) + centre;
        field.atom_layout.min_size = egui::Vec2::ZERO;
        field
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{
        Event, ImeEvent, Shape,
        text::{CCursor, CCursorRange},
    };

    fn ime_frame(
        ctx: &egui::Context,
        text: &mut String,
        events: Vec<Event>,
        multiline: bool,
    ) -> (egui::text_edit::TextEditOutput, egui::FullOutput) {
        let mut field = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let edit = if multiline {
                    egui::TextEdit::multiline(&mut *text)
                } else {
                    egui::TextEdit::singleline(&mut *text)
                };
                field = Some(edit.id(egui::Id::unique("ime-field")).desired_width(100.0).show(ui));
            },
        );
        output.textures_delta.clear();
        (field.expect("field shown"), output)
    }

    fn underlines(output: &egui::FullOutput, stroke: Stroke) -> Vec<[egui::Pos2; 2]> {
        fn collect(shape: &Shape, stroke: Stroke, lines: &mut Vec<[egui::Pos2; 2]>) {
            match shape {
                Shape::LineSegment { points, stroke: painted } if *painted == stroke && points[0].y == points[1].y => lines.push(*points),
                Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, stroke, lines)),
                _ => {}
            }
        }
        let mut lines = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, stroke, &mut lines);
        }
        lines
    }

    fn has_selection(field: &egui::text_edit::TextEditOutput, color: Color32) -> bool {
        field.galley.rows.iter().any(|row| {
            row.visuals
                .mesh
                .vertices
                .iter()
                .any(|v| v.color == color && v.uv == egui::epaint::WHITE_UV)
        })
    }

    /// Exercise TextEdit's painted output, not just the theme flag: the active
    /// Japanese segment moves, while preedit never receives selection quads.
    #[test]
    fn ime_composition_underlines_the_active_segment_without_a_selection_background() {
        let light = Theme {
            ink: "#151515".into(),
            muted: "#555555".into(),
            field: "#FAFAFA".into(),
            ..Theme::default()
        };
        for theme in [Theme::default(), light] {
            for multiline in [false, true] {
                let ctx = egui::Context::default();
                theme.apply(&ctx, 1.0);
                let visuals = ctx.global_style().visuals.clone();
                let mut text = String::new();
                let (field, _) = ime_frame(&ctx, &mut text, vec![], multiline);
                ctx.memory_mut(|m| m.request_focus(field.response.id));
                let preedit = "かなを変換して文章を入力します";
                let event = |range| {
                    Event::Ime(ImeEvent::Preedit {
                        text: preedit.into(),
                        active_range_chars: Some(range),
                    })
                };
                ctx.all_styles_mut(|s| s.visuals.ime_composition.legacy_visuals = true);
                let (old, _) = ime_frame(&ctx, &mut text, vec![event(0..2)], multiline);
                assert!(
                    has_selection(&old, visuals.selection.bg_fill),
                    "the previous rendering reproduces #2608"
                );
                theme.apply(&ctx, 1.0);
                let (field, output) = ime_frame(&ctx, &mut text, vec![event(0..2)], multiline);
                assert_eq!(text, preedit);
                assert!(!has_selection(&field, visuals.selection.bg_fill));
                let first = underlines(&output, visuals.ime_composition.active_underline_stroke);
                assert_eq!(first.len(), 1);
                assert!(!underlines(&output, visuals.ime_composition.inactive_underline_stroke).is_empty());
                let (field, output) = ime_frame(&ctx, &mut text, vec![event(3..5)], multiline);
                assert!(!has_selection(&field, visuals.selection.bg_fill));
                let next = underlines(&output, visuals.ime_composition.active_underline_stroke);
                assert_eq!(next.len(), 1);
                assert!(next[0][0].x > first[0][0].x, "the active segment must move with the IME range");
                if multiline {
                    assert!(field.galley.rows.len() > 1, "exercise wrapped composition");
                }
                theme.apply(&ctx, 1.0);
                let (_, output) = ime_frame(&ctx, &mut text, vec![Event::Ime(ImeEvent::Commit("漢字を変換".into()))], multiline);
                assert_eq!(text, "漢字を変換");
                assert!(underlines(&output, visuals.ime_composition.active_underline_stroke).is_empty());
                assert!(underlines(&output, visuals.ime_composition.inactive_underline_stroke).is_empty());
            }
        }
    }

    #[test]
    fn ime_composition_keeps_insertion_cancellation_and_ordinary_selection() {
        for multiline in [false, true] {
            let ctx = egui::Context::default();
            Theme::default().apply(&ctx, 1.0);
            let visuals = ctx.global_style().visuals.clone();
            let mut text = "before after".to_owned();
            let (mut field, _) = ime_frame(&ctx, &mut text, vec![], multiline);
            field.state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(7))));
            field.state.store(&ctx, field.response.id);
            ctx.memory_mut(|m| m.request_focus(field.response.id));
            // No active range (some IMEs), then a caret inside Korean preedit.
            for range in [None, Some(1..1)] {
                let (field, output) = ime_frame(
                    &ctx,
                    &mut text,
                    vec![Event::Ime(ImeEvent::Preedit {
                        text: "한글".into(),
                        active_range_chars: range,
                    })],
                    multiline,
                );
                assert_eq!(text, "before 한글after");
                assert!(!has_selection(&field, visuals.selection.bg_fill));
                assert!(!underlines(&output, visuals.ime_composition.inactive_underline_stroke).is_empty());
            }
            ime_frame(
                &ctx,
                &mut text,
                vec![Event::Ime(ImeEvent::Preedit {
                    text: String::new(),
                    active_range_chars: None,
                })],
                multiline,
            );
            assert_eq!(text, "before after", "cancellation removes only provisional text");
            let (mut field, _) = ime_frame(&ctx, &mut text, vec![], multiline);
            field
                .state
                .cursor
                .set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(6))));
            field.state.store(&ctx, field.response.id);
            let (field, _) = ime_frame(&ctx, &mut text, vec![], multiline);
            assert!(
                has_selection(&field, visuals.selection.bg_fill),
                "ordinary selection must still have its accent background"
            );
        }
    }

    /// "Reduce animations" ends egui's transitions at once, and gives
    /// them back their duration when it is turned off, a skin reload in
    /// between included.
    #[test]
    fn reduced_motion_ends_egui_transitions() {
        let ctx = egui::Context::default();
        let normal = egui::Style::default().animation_time;
        assert!(normal > 0.0);
        apply_motion(&ctx, true);
        assert_eq!(ctx.global_style().animation_time, 0.0);
        Theme::default().apply(&ctx, 1.0);
        apply_motion(&ctx, true);
        assert_eq!(ctx.global_style().animation_time, 0.0, "again after a skin reload");
        apply_motion(&ctx, false);
        assert_eq!(ctx.global_style().animation_time, normal);
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(hex("#8B5CF6"), Color32::from_rgb(0x8b, 0x5c, 0xf6));
        assert_eq!(hex("070B1F"), Color32::from_rgb(7, 11, 31));
    }

    /// A text field is as high as its text and 2 points above and below
    /// (egui 0.36), not a button's 26 points (egui 0.37): no window grows.
    #[test]
    fn text_fields_keep_their_size() {
        let ctx = egui::Context::default();
        Theme::default().apply(&ctx, 1.0);
        let mut heights = (0.0, 0.0, 0.0);
        for _ in 0..2 {
            let (mut a, mut b) = (String::new(), String::new());
            let mut out = ctx.run_ui(Default::default(), |ui| {
                let row = ui.fonts_mut(|f| f.row_height(&egui::TextStyle::Body.resolve(ui.style())));
                let framed = ui.add(egui::TextEdit::singleline(&mut a)).rect.height();
                let bare = ui.add(egui::TextEdit::singleline(&mut b).frame(egui::Frame::NONE)).rect.height();
                heights = (row, framed, bare);
            });
            out.textures_delta.clear();
        }
        let (row, framed, bare) = heights;
        assert!((framed - (row + 4.0)).abs() < 0.5, "framed field {framed} for a {row} row");
        assert!((bare - row).abs() < 0.5, "bare field {bare} for a {row} row");
    }
}
