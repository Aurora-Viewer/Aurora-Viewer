//! The Aurora Weather window's "Sea" page: which wind raises the waves of
//! enhanced water, or a sea state set here, the presets, and how the sea is
//! shaped (settings in `weather/sea.rs`), in cards as the Weather page's
//! (weather_window.rs), under a profile of the sea drawn now. Aurora's own:
//! the Legacy Viewer's water has no waves to set. Unlike the weather, the
//! sea stays the user's whoever sets the weather.

use super::compass_point;
use crate::theme::Palette;
use crate::ui::weather_window as card;
use crate::ui::widgets;
use crate::weather::sea::{self, SeaSettings, SeaSource, SeaStyle};
use aurora_render::water_sea::Sea;
use egui::{Align, Layout, RichText, Vec2};

fn source_label(source: SeaSource) -> &'static str {
    match source {
        SeaSource::Auto => aurora_i18n::tr!("ui-sea-source-auto"),
        SeaSource::Region => aurora_i18n::tr!("ui-sea-source-region"),
        SeaSource::Weather => aurora_i18n::tr!("ui-sea-source-weather"),
        SeaSource::Mine => aurora_i18n::tr!("ui-sea-source-mine"),
    }
}

fn preset_label(key: &str) -> &'static str {
    match key {
        "calm" => aurora_i18n::tr!("ui-sea-preset-calm"),
        "breeze" => aurora_i18n::tr!("ui-sea-preset-breeze"),
        "swell" => aurora_i18n::tr!("ui-sea-preset-swell"),
        "storm" => aurora_i18n::tr!("ui-sea-preset-storm"),
        _ => aurora_i18n::tr!("ui-sea-preset-stylized"),
    }
}

/// The Beaufort number of a wind of `speed` m/s.
pub(crate) fn beaufort(speed: f32) -> u32 {
    const UPPER: [f32; 12] = [0.5, 1.5, 3.3, 5.5, 7.9, 10.7, 13.8, 17.1, 20.7, 24.4, 28.4, 32.6];
    UPPER.iter().position(|&u| speed < u).unwrap_or(12) as u32
}

/// The page. `name`: the preset name being typed; `now`: the sea enhanced
/// water draws now (its profile on top).
pub(super) fn body(ui: &mut egui::Ui, p: &Palette, s: &mut SeaSettings, name: &mut String, now: &Sea) {
    ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
    now_card(ui, p, s, now);
    ui.add_space(4.0);
    // two columns where the window has room for them, as the weather's
    let gap = ui.spacing().item_spacing.x;
    let room = ui.available_width();
    let two = card::column_count(room, gap, card::COLUMN_MIN_W * crate::ui::fonts::scale(ui.ctx())) >= 2;
    if !two {
        wind_card(ui, p, s);
        presets_card(ui, p, s, name);
        shape_card(ui, p, s);
        foam_card(ui, p, s);
        limits_card(ui, p, s);
        return;
    }
    let ppp = ui.ctx().pixels_per_point();
    let column_w = (((room - gap) / 2.0) * ppp).floor() / ppp;
    let top_left = ui.cursor().min;
    let mut bottom = top_left.y;
    for i in 0..2 {
        let x = top_left.x + i as f32 * (column_w + gap);
        let rect = egui::Rect::from_min_size(egui::pos2(x, top_left.y), Vec2::new(column_w, f32::INFINITY));
        let mut col = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("sea_column", i))
                .max_rect(rect)
                .layout(Layout::top_down(Align::Min)),
        );
        col.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        if i == 0 {
            wind_card(&mut col, p, s);
            col.add_space(4.0);
            presets_card(&mut col, p, s, name);
        } else {
            shape_card(&mut col, p, s);
            col.add_space(4.0);
            foam_card(&mut col, p, s);
            col.add_space(4.0);
            limits_card(&mut col, p, s);
        }
        bottom = bottom.max(col.min_rect().bottom());
    }
    ui.advance_cursor_after_rect(egui::Rect::from_min_max(top_left, egui::pos2(top_left.x + room, bottom)));
}

/// "Sea now": the sea drawn now seen from the side, moving, and in words.
fn now_card(ui: &mut egui::Ui, p: &Palette, s: &SeaSettings, now: &Sea) {
    card::card(ui, p, "ripple", aurora_i18n::tr!("ui-sea-now"), None, |ui, _| {
        let profile = Profile::of(now, s.shape.choppiness);
        // the height limit, where one is set: the crests stay under half of it
        let limit = (s.max_height < s.limit_top()).then_some(s.max_height);
        draw_profile(ui, p, &profile, limit);
        let speed = now.wind[2];
        let summary = aurora_i18n::tr!(
            "ui-sea-now-summary",
            force = beaufort(speed),
            speed = format!("{speed:.0}"),
            swell = format!("{:.1}", profile.swell_height),
            length = format!("{:.0}", profile.swell_length),
            chop = format!("{:.1}", profile.chop_height)
        );
        ui.label(RichText::new(summary).size(card::ENDS_SIZE).color(p.muted));
    });
}

/// "Wind": which wind raises the waves, or the sea state set here and where
/// its wind comes from.
fn wind_card(ui: &mut egui::Ui, p: &Palette, s: &mut SeaSettings) {
    card::card(ui, p, "windsock-filled", aurora_i18n::tr!("ui-weather-wind"), None, |ui, _| {
        combo_row(
            ui,
            p,
            aurora_i18n::tr!("ui-sea-source"),
            aurora_i18n::tr!("ui-sea-source-tip"),
            |ui, width| {
                egui::ComboBox::from_id_salt("aurora_sea_source")
                    .width(width)
                    .selected_text(source_label(s.source))
                    .show_ui(ui, |ui| {
                        for source in [SeaSource::Auto, SeaSource::Region, SeaSource::Weather, SeaSource::Mine] {
                            ui.selectable_value(&mut s.source, source, source_label(source));
                        }
                    });
            },
        );
        let mine = s.source == SeaSource::Mine;
        let state_top = s.state_top();
        ui.add_enabled_ui(mine, |ui| {
            let speed = s.shape.state * sea::MAX_WIND;
            let state = aurora_i18n::tr!("ui-sea-state-value", speed = format!("{speed:.0}"), force = beaufort(speed)).to_string();
            // the Weather page's wind compass beside the sliders, the same
            // way round: where the wind comes from, a click or a drag on it
            let side = card::compass_side(ui.available_width());
            ui.horizontal_top(|ui| {
                // (its direction in words is the slider's, beside it)
                card::compass(ui, p, &mut s.from, side, aurora_i18n::tr!("ui-sea-from-tip"));
                ui.vertical(|ui| {
                    card::slider_row(
                        ui,
                        p,
                        (aurora_i18n::tr!("ui-sea-state"), aurora_i18n::tr!("ui-sea-state-tip")),
                        &state,
                        None,
                        egui::Slider::new(&mut s.shape.state, 0.0..=state_top),
                    );
                    let from = format!("{} · {:.0}°", compass_point(s.from), s.from);
                    card::slider_row(
                        ui,
                        p,
                        (aurora_i18n::tr!("ui-weather-wind-from"), aurora_i18n::tr!("ui-sea-from-tip")),
                        &from,
                        None,
                        egui::Slider::new(&mut s.from, 0.0..=355.0).step_by(5.0),
                    );
                });
            });
        });
    });
}

/// "Preset": a ready-made sea or a saved one, and the sea now saved under a
/// name.
fn presets_card(ui: &mut egui::Ui, p: &Palette, s: &mut SeaSettings, name: &mut String) {
    card::card(ui, p, "bookmarks-filled", aurora_i18n::tr!("ui-sea-preset"), None, |ui, _| {
        let current = s.current_preset();
        let shown = match &current {
            Some((key, true)) => preset_label(key).to_string(),
            Some((saved, false)) => saved.clone(),
            None => aurora_i18n::tr!("ui-sea-preset-custom").to_string(),
        };
        let mut pick = None;
        let mut forget = None;
        ui.horizontal(|ui| {
            // (a saved preset shown can be forgotten: its button beside it)
            let saved_shown = matches!(&current, Some((_, false)));
            let gap = ui.spacing().item_spacing.x;
            let width = ui.available_width() - if saved_shown { 24.0 + gap } else { 0.0 };
            egui::ComboBox::from_id_salt("aurora_sea_preset")
                .width(width)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for key in sea::PRESETS {
                        let on = current.as_ref().is_some_and(|(k, built)| *built && k == key);
                        if ui
                            .selectable_label(on, preset_label(key))
                            .on_hover_text(aurora_i18n::tr!("ui-sea-preset-tip"))
                            .clicked()
                        {
                            pick = sea::preset(key);
                        }
                    }
                    for saved in &s.saved {
                        let on = current.as_ref().is_some_and(|(k, built)| !*built && *k == saved.name);
                        if ui.selectable_label(on, saved.name.as_str()).clicked() {
                            pick = Some(saved.shape.clone());
                        }
                    }
                });
            if let Some((saved, false)) = &current
                && widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("ui-sea-forget-tip"), true)
            {
                forget = Some(saved.clone());
            }
        });
        if let Some(shape) = pick {
            s.shape = shape;
        }
        if let Some(gone) = forget {
            s.saved.retain(|p| p.name != gone);
        }
        // the shape now, saved under a name
        ui.horizontal(|ui| {
            let gap = ui.spacing().item_spacing.x;
            let label_w = (ui.available_width() * 0.3).min(90.0);
            card::label(ui, p, aurora_i18n::tr!("ui-sea-save"), aurora_i18n::tr!("ui-sea-save-tip"), label_w);
            let field = ui.add(
                egui::TextEdit::singleline(name)
                    .hint_text(aurora_i18n::tr!("ui-sea-save-hint"))
                    .desired_width(ui.available_width() - 24.0 - gap),
            );
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let ok = !name.trim().is_empty();
            if (widgets::icon_button(ui, p, "device-floppy-filled", aurora_i18n::tr!("ui-sea-save-tip"), ok) || entered)
                && s.save_preset(name)
            {
                name.clear();
            }
        });
    });
}

/// "Shape": the style, the swell's and the chop's heights, the fetch and the
/// crests' sharpness.
fn shape_card(ui: &mut egui::Ui, p: &Palette, s: &mut SeaSettings) {
    card::card(ui, p, "adjustments-filled", aurora_i18n::tr!("ui-sea-shape"), None, |ui, _| {
        let styles = [SeaStyle::Natural, SeaStyle::Ocean, SeaStyle::Stylized];
        let mut i = styles.iter().position(|&st| st == s.shape.style).unwrap_or(0);
        let options = [
            (aurora_i18n::tr!("ui-sea-style-natural"), true),
            (aurora_i18n::tr!("ui-sea-style-ocean"), true),
            (aurora_i18n::tr!("ui-sea-style-stylized"), true),
        ];
        ui.horizontal(|ui| {
            let label_w = (ui.available_width() * 0.3).min(90.0);
            card::label(
                ui,
                p,
                aurora_i18n::tr!("ui-sea-style"),
                aurora_i18n::tr!("ui-sea-style-tip"),
                label_w,
            );
            if widgets::segmented(ui, p, &mut i, &options) {
                s.shape.style = styles[i.min(2)];
            }
        });
        let scale = 0.0..=s.scale_top();
        percent_row(
            ui,
            p,
            (aurora_i18n::tr!("ui-sea-swell"), aurora_i18n::tr!("ui-sea-swell-tip")),
            &mut s.shape.swell,
            scale.clone(),
        );
        percent_row(
            ui,
            p,
            (aurora_i18n::tr!("ui-sea-chop"), aurora_i18n::tr!("ui-sea-chop-tip")),
            &mut s.shape.chop,
            scale,
        );
        let spectrum = s.shape.style != SeaStyle::Stylized;
        ui.add_enabled_ui(spectrum, |ui| {
            let fetch = format!("{:.1} km", s.shape.fetch);
            card::slider_row(
                ui,
                p,
                (aurora_i18n::tr!("ui-sea-fetch"), aurora_i18n::tr!("ui-sea-fetch-tip")),
                &fetch,
                None,
                egui::Slider::new(&mut s.shape.fetch, sea::FETCH_RANGE).logarithmic(true),
            );
            percent_row(
                ui,
                p,
                (aurora_i18n::tr!("ui-sea-sharpness"), aurora_i18n::tr!("ui-sea-sharpness-tip")),
                &mut s.shape.choppiness,
                sea::SHARP_RANGE,
            );
        });
    });
}

/// "Foam and spray".
fn foam_card(ui: &mut egui::Ui, p: &Palette, s: &mut SeaSettings) {
    card::card(ui, p, "droplets-filled", aurora_i18n::tr!("ui-sea-foam-spray"), None, |ui, _| {
        let scale = 0.0..=s.scale_top();
        percent_row(
            ui,
            p,
            (aurora_i18n::tr!("ui-sea-foam"), aurora_i18n::tr!("ui-sea-foam-tip")),
            &mut s.shape.foam,
            scale.clone(),
        );
        percent_row(
            ui,
            p,
            (aurora_i18n::tr!("ui-sea-spray"), aurora_i18n::tr!("ui-sea-spray-tip")),
            &mut s.shape.spray,
            scale,
        );
    });
}

/// "Limits": the highest the waves may stand, and the safety catch for the
/// seas no one would sail (turned off, what went past nature's comes back
/// within it).
fn limits_card(ui: &mut egui::Ui, p: &Palette, s: &mut SeaSettings) {
    card::card(ui, p, "ruler-2", aurora_i18n::tr!("ui-sea-limits"), None, |ui, _| {
        let limit_top = s.limit_top();
        let height = if s.max_height >= limit_top {
            aurora_i18n::tr!("ui-sea-limit-none").to_string()
        } else {
            format!("{:.1} m", s.max_height)
        };
        card::slider_row(
            ui,
            p,
            (aurora_i18n::tr!("ui-sea-limit"), aurora_i18n::tr!("ui-sea-limit-tip")),
            &height,
            None,
            egui::Slider::new(&mut s.max_height, 0.5..=limit_top),
        );
        if card::switch_row(
            ui,
            p,
            "alert-triangle-filled",
            aurora_i18n::tr!("ui-sea-extreme"),
            aurora_i18n::tr!("ui-sea-extreme-tip"),
            &mut s.extreme,
        )
        .changed()
            && !s.extreme
        {
            let top = *sea::SCALE_RANGE.end();
            s.shape.swell = s.shape.swell.min(top);
            s.shape.chop = s.shape.chop.min(top);
            s.shape.foam = s.shape.foam.min(top);
            s.shape.spray = s.shape.spray.min(top);
            s.shape.state = s.shape.state.min(1.0);
            s.max_height = s.max_height.min(sea::NO_LIMIT);
        }
    });
}

/// A slider of a share, its value in percent: `text` its label and tip.
fn percent_row(ui: &mut egui::Ui, p: &Palette, text: (&str, &str), value: &mut f32, range: std::ops::RangeInclusive<f32>) {
    let shown = format!("{:.0}%", *value * 100.0);
    card::slider_row(ui, p, text, &shown, None, egui::Slider::new(value, range));
}

/// A label and, on the rest of the row, what `control` draws `width` wide.
fn combo_row(ui: &mut egui::Ui, p: &Palette, text: &str, tip: &str, control: impl FnOnce(&mut egui::Ui, f32)) {
    ui.horizontal(|ui| {
        let gap = ui.spacing().item_spacing.x;
        let label_w = (ui.available_width() * 0.4).min(130.0);
        card::label(ui, p, text, tip, label_w);
        let width = ui.available_width() - gap;
        control(ui, width);
    });
}

/// The sea seen from the side for its preview: its main swell and the wind's
/// chop as a few waves of the chop's length, each a Gerstner wave (the
/// crests pinched by the sharpness), over `span` m.
#[derive(Debug, Clone, PartialEq)]
struct Profile {
    /// Per wave: wavenumber (rad/m), amplitude (m), angular frequency
    /// (rad/s), how much its crests pinch (Gerstner's Q), phase (rad).
    waves: Vec<[f32; 5]>,
    span: f32,
    /// The swell's height (m, trough to crest) and length (m), and the
    /// chop's significant height (m).
    swell_height: f32,
    swell_length: f32,
    chop_height: f32,
}

/// The chop's waves, relative to its longest, and their shares of its
/// height.
const CHOP_LENGTHS: [f32; 5] = [1.0, 0.62, 0.41, 0.27, 0.18];
const CHOP_SHARES: [f32; 5] = [0.42, 0.25, 0.16, 0.1, 0.07];

impl Profile {
    /// The profile of `sea`, its crests as sharp as `sharpness` (the Sea
    /// panel's Crests, 1 by default).
    fn of(sea: &Sea, sharpness: f32) -> Self {
        let k0 = sea.trains[0][2].max(0.01);
        let a0 = sea.trains[0][3].max(0.0);
        let swell_length = std::f32::consts::TAU / k0;
        // the spectrum's significant height, or the trains' chop
        let chop_height = if sea.shore[3] > 0.0 {
            sea.shore[3]
        } else {
            2.0 * sea.trains[1..].iter().map(|t| t[3]).sum::<f32>()
        }
        .max(0.0);
        // the chop's longest waves: a sea's peak for the wind now (as the
        // trains' `shape`), half of it
        let wind = sea.wind[2].max(0.0);
        let chop_length = ((6.0 + 2.2 * wind.powf(1.5)).min(90.0) * 0.5).max(1.5);
        let span = (swell_length * 1.25).clamp(30.0, 180.0);
        let sharp = sharpness.clamp(0.0, 2.0) * 0.5;
        let mut waves = vec![[k0, a0, (9.81 * k0).sqrt(), 0.0, 0.0]];
        for (i, (&l, &share)) in CHOP_LENGTHS.iter().zip(&CHOP_SHARES).enumerate() {
            let k = std::f32::consts::TAU / (chop_length * l);
            let a = 0.5 * chop_height * share;
            waves.push([k, a, (9.81 * k).sqrt(), 0.0, i as f32 * 1.7 + 0.4]);
        }
        // the crests pinch, never so far that the profile folds over
        // itself: the sum of the waves' Q·k·a stays under 1
        let steep: f32 = waves.iter().map(|w| w[0] * w[1]).sum();
        let q = if steep > 1e-6 { (sharp / steep).min(1.0 / steep) } else { 0.0 };
        for w in &mut waves {
            w[3] = q;
        }
        Self {
            waves,
            span,
            swell_height: 2.0 * a0,
            swell_length,
            chop_height,
        }
    }

    /// The point of the surface that rests at `s` m along the profile at
    /// `time` s: its x (m) and height (m).
    fn at(&self, s: f32, time: f32) -> (f32, f32) {
        let (mut x, mut y) = (s, 0.0);
        for &[k, a, omega, q, phase] in &self.waves {
            let theta = k * s - omega * time + phase;
            x -= q * a * theta.sin();
            y += a * theta.cos();
        }
        (x, y)
    }

    /// The highest the profile can stand (m): all its waves' crests at once.
    fn reach(&self) -> f32 {
        self.waves.iter().map(|w| w[1]).sum()
    }
}

/// The profile drawn across the card, moving with the sea's time: the water
/// under it filled, the still water's level dashed, and the height
/// `limit` (m) where one is set.
fn draw_profile(ui: &mut egui::Ui, p: &Palette, profile: &Profile, limit: Option<f32>) {
    const HEIGHT: f32 = 64.0;
    const STEPS: usize = 160;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, HEIGHT), egui::Sense::hover());
    response.on_hover_text(aurora_i18n::tr!("ui-sea-now-tip"));
    let painter = ui.painter_at(rect);
    let time = ui.input(|i| i.time) as f32;
    let middle = rect.center().y;
    // as high as it gets fills most of the card's height (a glassy sea:
    // under a quarter of a metre still shows as ripples, not a ruler line)
    let scale = HEIGHT * 0.42 / profile.reach().max(0.25);
    let points: Vec<egui::Pos2> = (0..=STEPS)
        .map(|i| {
            let (x, y) = profile.at(i as f32 / STEPS as f32 * profile.span, time);
            egui::pos2(rect.left() + x / profile.span * width, middle - y * scale)
        })
        .collect();
    let mut mesh = egui::Mesh::default();
    let fill = p.indigo.gamma_multiply(0.45);
    for (i, pt) in points.iter().enumerate() {
        mesh.colored_vertex(*pt, fill);
        mesh.colored_vertex(egui::pos2(pt.x, rect.bottom()), fill);
        if i > 0 {
            let j = (2 * i) as u32;
            mesh.add_triangle(j - 2, j - 1, j);
            mesh.add_triangle(j - 1, j + 1, j);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.add(egui::Shape::dashed_line(
        &[egui::pos2(rect.left(), middle), egui::pos2(rect.right(), middle)],
        egui::Stroke::new(1.0, p.muted_dim),
        4.0,
        4.0,
    ));
    if let Some(limit) = limit {
        let y = middle - 0.5 * limit * scale;
        if y > rect.top() {
            painter.add(egui::Shape::dashed_line(
                &[egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                egui::Stroke::new(1.0, p.amber),
                6.0,
                3.0,
            ));
        }
    }
    painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, p.violet_light)));
    // moving: a few dozen frames a second while the page shows
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beaufort_numbers_follow_the_scale() {
        assert_eq!(beaufort(0.0), 0);
        assert_eq!(beaufort(5.0), 3);
        assert_eq!(beaufort(14.0), 7);
        assert_eq!(beaufort(24.0), 9);
        assert_eq!(beaufort(40.0), 12);
    }

    /// The preview shows the sea drawn now: its swell's length and height,
    /// its chop's height; its crests pinch with the sharpness but the
    /// profile never folds over itself, however sharp.
    #[test]
    fn the_profile_is_the_sea_now_and_never_folds() {
        let mut sea = Sea::default();
        sea.trains[0][2] = std::f32::consts::TAU / 80.0;
        sea.trains[0][3] = 1.2;
        sea.shore[3] = 1.5;
        sea.wind[2] = 14.0;
        for sharpness in [0.0, 1.0, 2.0] {
            let profile = Profile::of(&sea, sharpness);
            assert!((profile.swell_length - 80.0).abs() < 1e-3);
            assert!((profile.swell_height - 2.4).abs() < 1e-5);
            assert!((profile.chop_height - 1.5).abs() < 1e-5);
            assert!(profile.span >= 80.0, "a whole swell shows: {}", profile.span);
            for time in [0.0, 3.7, 11.2] {
                let mut last = f32::NEG_INFINITY;
                for i in 0..=400 {
                    let (x, _) = profile.at(i as f32 / 400.0 * profile.span, time);
                    assert!(x > last, "folded at {i} (sharpness {sharpness}, time {time})");
                    last = x;
                }
            }
        }
        // sharper crests: a crest narrower than its trough
        let flat = Profile::of(&sea, 0.0);
        let sharp = Profile::of(&sea, 2.0);
        assert!(sharp.waves[1][3] > 0.0 && flat.waves[1][3] == 0.0);
    }
}
