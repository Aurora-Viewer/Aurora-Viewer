//! The Aurora Weather window: the weather drawn for this viewer only
//! (Aurora's own: the Legacy Viewer has none). A summary of the sky shown
//! now, then cards in two columns: what
//! falls and what it leaves on the ground on the left; the wind (with its
//! compass), thunder and the northern lights (with their preview) on the
//! right; the quality and the clean-up and snow-fill actions at the foot.
//!
//! The settings are those of [`WeatherSettings`], unchanged: this module is
//! their window only.

use super::environment::{colour_names, compass_point};
use super::widgets;
use crate::theme::Palette;
use crate::weather::{AuroraColours, DEEP_SNOW_REACH, PUDDLE_DISTANCE, Precipitation, SNOW_DISTANCE, WeatherQuality, WeatherSettings};
use egui::{Align, Layout, RichText, Sense, Stroke, Vec2};

/// The window's width: two columns of cards.
pub const WINDOW_W: f32 = 700.0;
/// The side of the summary's picture of the sky.
const SKY_TILE: f32 = 72.0;
/// The side of the wind's compass.
const COMPASS: f32 = 112.0;
/// The height of the northern lights' preview.
const AURORA_PREVIEW_H: f32 = 70.0;
/// The size of a row's label and of its value.
const LABEL_SIZE: f32 = 12.5;
const VALUE_SIZE: f32 = 12.0;
/// The size of the words under a slider's ends.
const ENDS_SIZE: f32 = 11.0;
/// The compass steps of the wind's direction (degrees), as its slider had.
const DIRECTION_STEP: f32 = 5.0;

/// What the window asks the app to do, besides the settings it edits.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WeatherActions {
    /// Clean up: the snow lying, the wet ground and puddles, the footprints
    /// and the snow on avatars go at once.
    pub clean: bool,
    /// The snow lies everywhere as deep as a long snowfall leaves it.
    pub fill: bool,
}

/// The window's body: the summary, the cards and the foot.
pub fn body(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings) -> WeatherActions {
    let before = w.clone();
    let mut actions = WeatherActions::default();
    ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
    summary(ui, p, w);
    ui.add_space(4.0);
    ui.columns(2, |cols| {
        let [left, right] = cols else { return };
        left.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        right.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        precipitation_card(left, p, w);
        left.add_space(4.0);
        ground_card(left, p, w);
        wind_card(right, p, w);
        right.add_space(4.0);
        thunder_card(right, p, w);
        right.add_space(4.0);
        northern_lights_card(right, p, w);
    });
    ui.add_space(4.0);
    foot(ui, p, w, &mut actions);
    if *w != before {
        w.take_over(&before);
        w.touched = true;
    }
    actions
}

/// What the summary shows: its picture, the picture's colour, its title.
struct SkyNow {
    icon: &'static str,
    color: egui::Color32,
    title: &'static str,
}

/// Rain at least this strong brings thunder (`scene::thunder::MIN_RAIN`).
fn stormy(w: &WeatherSettings) -> bool {
    !w.snowing() && w.thunder && w.rain_or_amount() >= crate::scene::thunder::MIN_RAIN
}

/// The word for how hard it falls, rain or snow, 0 (nothing) to 1.
fn amount_word(snow: bool, amount: f32) -> &'static str {
    let step = if amount <= 0.0 {
        0
    } else if amount < 0.2 {
        1
    } else if amount < 0.45 {
        2
    } else if amount < 0.75 {
        3
    } else {
        4
    };
    match (snow, step) {
        (_, 0) => aurora_i18n::tr!("ui-weather-now-clear"),
        (false, 1) => aurora_i18n::tr!("ui-weather-now-drizzle"),
        (false, 2) => aurora_i18n::tr!("ui-weather-now-light-rain"),
        (false, 3) => aurora_i18n::tr!("ui-weather-now-rain"),
        (false, _) => aurora_i18n::tr!("ui-weather-now-downpour"),
        (true, 1) => aurora_i18n::tr!("ui-weather-now-flurries"),
        (true, 2) => aurora_i18n::tr!("ui-weather-now-light-snow"),
        (true, 3) => aurora_i18n::tr!("ui-weather-now-snow"),
        (true, _) => aurora_i18n::tr!("ui-weather-now-blizzard"),
    }
}

/// The word for how hard the wind blows, 0 (calm) to 1 (a gale).
fn wind_word(v: f32) -> &'static str {
    if v < 0.08 {
        aurora_i18n::tr!("ui-weather-wind-calm")
    } else if v < 0.3 {
        aurora_i18n::tr!("ui-weather-wind-breeze")
    } else if v < 0.55 {
        aurora_i18n::tr!("ui-weather-wind-steady")
    } else if v < 0.8 {
        aurora_i18n::tr!("ui-weather-wind-strong")
    } else {
        aurora_i18n::tr!("ui-weather-wind-gale")
    }
}

/// The wind in a few words: how hard its gusts blow, and from where.
fn wind_summary(w: &WeatherSettings) -> String {
    let from = if w.wind_follow_region {
        aurora_i18n::tr!("ui-weather-wind-region").to_owned()
    } else {
        compass_point(w.wind_from)
    };
    format!("{} · {from}", wind_word(w.wind_max))
}

fn sky_now(p: &Palette, w: &WeatherSettings) -> SkyNow {
    let amount = w.rain_or_amount();
    let snow = w.snowing();
    if amount <= 0.0 {
        SkyNow {
            icon: "moon-filled",
            color: p.violet_pale,
            title: aurora_i18n::tr!("ui-weather-now-clear"),
        }
    } else if stormy(w) {
        SkyNow {
            icon: "cloud-storm",
            color: p.amber,
            title: aurora_i18n::tr!("ui-weather-now-storm"),
        }
    } else if snow {
        SkyNow {
            icon: "snowflake",
            color: p.ink,
            title: amount_word(true, amount),
        }
    } else {
        SkyNow {
            icon: "cloud-rain",
            color: p.indigo_light,
            title: amount_word(false, amount),
        }
    }
}

/// The top of the window: the sky shown now (its picture, its name and a
/// few chips).
fn summary(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings) {
    widgets::surface(ui, p, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal_top(|ui| {
            let now = sky_now(p, w);
            let (tile, _) = ui.allocate_exact_size(Vec2::splat(SKY_TILE), Sense::hover());
            ui.painter().rect_filled(tile, ui.visuals().window_corner_radius, p.bar);
            widgets::paint_glyph(ui.painter(), now.icon, tile.center(), SKY_TILE * 0.58, now.color);
            let text_w = ui.available_width().max(120.0);
            ui.allocate_ui_with_layout(Vec2::new(text_w, SKY_TILE), Layout::top_down(Align::Min), |ui| {
                ui.set_width(text_w);
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(RichText::new(aurora_i18n::tr!("ui-weather-now")).size(VALUE_SIZE).color(p.muted));
                ui.add(egui::Label::new(RichText::new(now.title).font(widgets::control_font(ui, 20.0)).strong().color(p.ink)).wrap());
                ui.add_space(2.0);
                chips(ui, p, w);
            });
        });
    });
}

/// The chips under the sky's name: the wind, and what else the weather
/// brings now (lightning, puddles, snow on the ground, the northern lights).
fn chips(ui: &mut egui::Ui, p: &Palette, w: &WeatherSettings) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        chip(ui, p, "wind", p.teal, &wind_summary(w));
        let raining = !w.snowing() && w.rain_or_amount() > 0.0;
        if stormy(w) {
            chip(ui, p, "bolt-filled", p.amber, aurora_i18n::tr!("ui-weather-chip-lightning"));
        }
        if raining && w.puddles {
            chip(ui, p, "droplet-filled", p.indigo_light, aurora_i18n::tr!("ui-weather-puddles"));
        }
        if w.snowing() && w.snow_settles {
            chip(ui, p, "mountain-filled", p.ink, aurora_i18n::tr!("ui-weather-chip-snow-lying"));
        }
        if w.aurora {
            chip(
                ui,
                p,
                "sparkles-filled",
                aurora_top(w.aurora_colours),
                aurora_i18n::tr!("ui-weather-northern-lights"),
            );
        }
    });
}

/// A small pill with an icon and a few words.
fn chip(ui: &mut egui::Ui, p: &Palette, icon: &str, tint: egui::Color32, text: &str) {
    let font = egui::FontId::proportional(VALUE_SIZE);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, p.ink);
    let size = Vec2::new(8.0 + 14.0 + 5.0 + galley.size().x + 8.0, 22.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, ui.visuals().window_corner_radius, p.ink.gamma_multiply(0.07));
    widgets::paint_glyph(ui.painter(), icon, egui::pos2(rect.left() + 15.0, rect.center().y), 14.0, tint);
    widgets::paint_row_text(ui.painter(), galley, rect.left() + 27.0, rect.center().y, p.ink);
}

/// The switch on a card's title row, on its right: what it turns on, its
/// tip, and a word before it (none: the switch alone).
struct Head<'a> {
    on: &'a mut bool,
    tip: &'a str,
    word: Option<&'a str>,
}

/// A card: its icon and title, its switch on the right of the title
/// (`head`), then its rows, given whether that switch is on.
fn card<R>(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    title: &str,
    head: Option<Head<'_>>,
    rows: impl FnOnce(&mut egui::Ui, bool) -> R,
) -> R {
    widgets::surface(ui, p, |ui| {
        ui.set_min_width(ui.available_width());
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        let mut on = true;
        ui.horizontal(|ui| {
            ui.set_min_height(widgets::SWITCH_SIZE.y);
            widgets::glyph(ui, icon, p.violet_light, 16.0);
            ui.label(RichText::new(title).font(widgets::control_font(ui, 13.0)).strong().color(p.ink));
            if let Some(head) = head {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    widgets::switch(ui, p, head.on).on_hover_text(head.tip);
                    if let Some(word) = head.word {
                        ui.label(RichText::new(word).size(VALUE_SIZE).color(p.muted));
                    }
                });
                on = *head.on;
            }
        });
        rows(ui, on)
    })
}

/// A row's label with its tip, cut to `width` so that a long one cannot
/// run under what is on its right.
fn label(ui: &mut egui::Ui, p: &Palette, text: &str, tip: &str, width: f32) {
    ui.allocate_ui_with_layout(Vec2::new(width.max(0.0), 18.0), Layout::left_to_right(Align::Center), |ui| {
        ui.add(egui::Label::new(RichText::new(text).size(LABEL_SIZE).color(p.ink)).truncate())
            .on_hover_text(tip);
    });
}

/// A setting with a switch on its right, its icon before its label.
fn switch_row(ui: &mut egui::Ui, p: &Palette, icon: &str, text: &str, tip: &str, on: &mut bool) -> egui::Response {
    ui.horizontal(|ui| {
        widgets::glyph(ui, icon, p.muted, 15.0);
        let gap = ui.spacing().item_spacing.x;
        let width = ui.available_width() - widgets::SWITCH_SIZE.x - gap;
        label(ui, p, text, tip, width);
        widgets::switch(ui, p, on).on_hover_text(tip)
    })
    .inner
}

/// A slider under its label, its value in words on the right, and the
/// words of its two ends under it (`ends`).
fn slider_row(
    ui: &mut egui::Ui,
    p: &Palette,
    (text, tip): (&str, &str),
    value: &str,
    ends: Option<(&str, &str)>,
    slider: egui::Slider<'_>,
) -> egui::Response {
    ui.horizontal(|ui| {
        let galley = ui
            .painter()
            .layout_no_wrap(value.to_owned(), egui::FontId::proportional(VALUE_SIZE), p.violet_pale);
        let gap = ui.spacing().item_spacing.x;
        let width = ui.available_width() - galley.size().x - gap;
        label(ui, p, text, tip, width);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(value).size(VALUE_SIZE).color(p.violet_pale));
        });
    });
    ui.add_space(-3.0);
    let resp = ui
        .scope(|ui| {
            ui.spacing_mut().slider_width = ui.available_width();
            ui.add(slider.show_value(false).trailing_fill(true)).on_hover_text(tip)
        })
        .inner;
    if let Some((low, high)) = ends {
        ui.add_space(-5.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(low).size(ENDS_SIZE).color(p.muted));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(high).size(ENDS_SIZE).color(p.muted));
            });
        });
    }
    resp
}

/// "Precipitation": rain or snow as two tiles, how hard it falls and, in
/// snow, how big the flakes are.
fn precipitation_card(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings) {
    card(ui, p, "cloud-rain", aurora_i18n::tr!("ui-weather-precipitation"), None, |ui, _| {
        // AURORA_SNOW shows as snow until the window is used
        let snowing = w.snowing();
        let (rain_line, snow_line) = (aurora_i18n::tr!("ui-weather-rain-sub"), aurora_i18n::tr!("ui-weather-snow-sub"));
        // both as tall as the taller: a line that wraps in one language
        // keeps the pair even
        let column = (ui.available_width() - ui.spacing().item_spacing.x) * 0.5;
        let height = tile_height(ui, rain_line, column).max(tile_height(ui, snow_line, column));
        ui.columns(2, |tiles| {
            let [rain, snow] = tiles else { return };
            if tile(
                rain,
                p,
                "cloud-rain",
                aurora_i18n::tr!("ui-weather-rain"),
                (rain_line, height),
                !snowing,
            ) {
                w.precipitation = Precipitation::Rain;
            }
            if tile(
                snow,
                p,
                "snowflake",
                aurora_i18n::tr!("ui-weather-snow"),
                (snow_line, height),
                snowing,
            ) {
                w.precipitation = Precipitation::Snow;
            }
        });
        let snowing = w.snowing();
        let mut amount = w.rain_or_amount();
        let ends = if snowing {
            (
                aurora_i18n::tr!("ui-weather-now-flurries"),
                aurora_i18n::tr!("ui-weather-now-blizzard"),
            )
        } else {
            (
                aurora_i18n::tr!("ui-weather-now-drizzle"),
                aurora_i18n::tr!("ui-weather-now-downpour"),
            )
        };
        if slider_row(
            ui,
            p,
            (aurora_i18n::tr!("ui-weather-amount"), aurora_i18n::tr!("ui-weather-amount-tip")),
            amount_word(snowing, amount),
            Some(ends),
            egui::Slider::new(&mut amount, 0.0..=1.0),
        )
        .changed()
        {
            w.amount = amount;
        }
        if snowing {
            let size = if w.snow_size < 0.33 {
                aurora_i18n::tr!("ui-weather-flake-fine")
            } else if w.snow_size < 0.67 {
                aurora_i18n::tr!("ui-weather-flake-medium")
            } else {
                aurora_i18n::tr!("ui-weather-flake-big")
            };
            slider_row(
                ui,
                p,
                (
                    aurora_i18n::tr!("ui-weather-flake-size"),
                    aurora_i18n::tr!("ui-weather-flake-size-tip"),
                ),
                size,
                Some((aurora_i18n::tr!("ui-weather-flake-fine"), aurora_i18n::tr!("ui-weather-flake-big"))),
                egui::Slider::new(&mut w.snow_size, 0.0..=1.0),
            );
        }
    });
}

/// The line under a tile's name, wrapped to a tile `width` wide.
fn tile_line(ui: &egui::Ui, line: &str, width: f32, color: egui::Color32) -> std::sync::Arc<egui::Galley> {
    ui.painter().layout(
        line.to_owned(),
        egui::FontId::proportional(ENDS_SIZE),
        color,
        (width - 20.0).max(1.0),
    )
}

/// How tall a tile `width` wide is with `line` under its name.
fn tile_height(ui: &egui::Ui, line: &str, width: f32) -> f32 {
    10.0 + 24.0 + 4.0 + 16.0 + tile_line(ui, line, width, egui::Color32::PLACEHOLDER).size().y + 10.0
}

/// A tile of a two-way choice: an icon, a name and a line under it, lit
/// with the accent while `on`; `height` is the pair's. True when clicked.
fn tile(ui: &mut egui::Ui, p: &Palette, icon: &str, name: &str, (line, height): (&str, f32), on: bool) -> bool {
    let width = ui.available_width();
    let line_galley = tile_line(ui, line, width, p.muted);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let enabled = ui.is_enabled();
    let radius = ui.visuals().window_corner_radius;
    let face = match (on, resp.hovered() && enabled) {
        (true, _) => p.violet.gamma_multiply(0.22),
        (false, true) => p.raised,
        (false, false) => p.raised.gamma_multiply(0.55),
    };
    let painter = ui.painter();
    painter.rect_filled(rect, radius, face);
    if on {
        painter.rect_stroke(rect, radius, Stroke::new(1.0, p.violet), egui::StrokeKind::Inside);
    }
    let fade = |c: egui::Color32| if enabled { c } else { c.gamma_multiply(0.5) };
    let left = rect.left() + 10.0;
    let icon_color = if on { p.violet_pale } else { p.muted };
    widgets::paint_glyph(painter, icon, egui::pos2(left + 12.0, rect.top() + 22.0), 24.0, fade(icon_color));
    let name_font = egui::FontId::new(13.0, crate::ui::fonts::semibold(ui.ctx()));
    let name_galley = painter.layout_no_wrap(name.to_owned(), name_font, fade(p.ink));
    widgets::paint_row_text(painter, name_galley, left, rect.top() + 46.0, fade(p.ink));
    painter.galley(egui::pos2(left, rect.top() + 56.0), line_galley, fade(p.muted));
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.clicked() && enabled && !on
}

/// "On the ground": in rain the puddles and how far the wet ground goes;
/// in snow the snow lying, the footprints, the snow on avatars, the ice and
/// deep snow.
fn ground_card(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings) {
    card(ui, p, "mountain-filled", aurora_i18n::tr!("ui-weather-ground"), None, |ui, _| {
        if !w.snowing() {
            switch_row(
                ui,
                p,
                "droplet-filled",
                aurora_i18n::tr!("ui-weather-puddles"),
                aurora_i18n::tr!("ui-weather-puddles-tip"),
                &mut w.puddles,
            );
            // the wet ground is drawn in any rain, the puddles or not
            let distance = format!("{:.0} m", w.puddle_distance);
            slider_row(
                ui,
                p,
                (aurora_i18n::tr!("ui-weather-distance"), aurora_i18n::tr!("ui-weather-distance-tip")),
                &distance,
                None,
                egui::Slider::new(&mut w.puddle_distance, PUDDLE_DISTANCE).step_by(1.0),
            );
            return;
        }
        switch_row(
            ui,
            p,
            "mountain-filled",
            aurora_i18n::tr!("ui-weather-snow-settles"),
            aurora_i18n::tr!("ui-weather-snow-settles-tip"),
            &mut w.snow_settles,
        );
        let settles = w.snow_settles;
        ui.add_enabled_ui(settles, |ui| {
            let distance = format!("{:.0} m", w.snow_distance);
            slider_row(
                ui,
                p,
                (
                    aurora_i18n::tr!("ui-weather-snow-distance"),
                    aurora_i18n::tr!("ui-weather-snow-distance-tip"),
                ),
                &distance,
                None,
                egui::Slider::new(&mut w.snow_distance, SNOW_DISTANCE).step_by(1.0),
            );
            switch_row(
                ui,
                p,
                "shoe",
                aurora_i18n::tr!("ui-weather-footprints"),
                aurora_i18n::tr!("ui-weather-footprints-tip"),
                &mut w.snow_footprints,
            );
            let footprints = w.snow_footprints;
            ui.add_enabled_ui(footprints, |ui| {
                let reach = format!("{:.0} m", w.snow_footprints_reach);
                slider_row(
                    ui,
                    p,
                    (
                        aurora_i18n::tr!("ui-weather-footprints-reach"),
                        aurora_i18n::tr!("ui-weather-footprints-reach-tip"),
                    ),
                    &reach,
                    None,
                    egui::Slider::new(&mut w.snow_footprints_reach, 32.0..=64.0).step_by(8.0),
                );
            });
        });
        // snow gathering on avatars, whether or not it settles on the ground
        switch_row(
            ui,
            p,
            "user",
            aurora_i18n::tr!("ui-weather-avatar-snow"),
            aurora_i18n::tr!("ui-weather-avatar-snow-tip"),
            &mut w.snow_on_avatars,
        );
        switch_row(
            ui,
            p,
            "ice-skating",
            aurora_i18n::tr!("ui-weather-ice"),
            aurora_i18n::tr!("ui-weather-ice-tip"),
            &mut w.ice,
        );
        ui.add_enabled_ui(settles, |ui| {
            deep_snow_row(ui, p, w);
            let deep = w.snow_deep;
            ui.add_enabled_ui(deep, |ui| {
                let reach = format!("{:.0} m", w.snow_deep_reach);
                slider_row(
                    ui,
                    p,
                    (
                        aurora_i18n::tr!("ui-weather-deep-snow-reach"),
                        aurora_i18n::tr!("ui-weather-deep-snow-reach-tip"),
                    ),
                    &reach,
                    None,
                    egui::Slider::new(&mut w.snow_deep_reach, DEEP_SNOW_REACH).step_by(1.0),
                );
            });
        });
    });
}

/// "Deep snow", with the mark of an experimental setting after its label.
fn deep_snow_row(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings) {
    let text = aurora_i18n::tr!("ui-weather-deep-snow");
    let tip = aurora_i18n::tr!("ui-weather-deep-snow-tip");
    ui.horizontal(|ui| {
        widgets::glyph(ui, "snowflake", p.muted, 15.0);
        let gap = ui.spacing().item_spacing.x;
        let width = ui.available_width() - widgets::SWITCH_SIZE.x - 14.0 - gap * 2.0;
        let galley = ui
            .painter()
            .layout_no_wrap(text.to_owned(), egui::FontId::proportional(LABEL_SIZE), p.ink);
        label(ui, p, text, tip, width.min(galley.size().x + 1.0));
        let (mark, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        widgets::experimental_mark(ui, p, mark, ui.scope_id().with("deep_snow_experimental"));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            widgets::switch(ui, p, &mut w.snow_deep).on_hover_text(tip);
        });
    });
}

/// "Wind": its compass (where it comes from: click or drag on it) beside
/// its lulls, its gusts and its direction; on the title's right, whether it
/// blows the region's way.
fn wind_card(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings) {
    let mut follow = w.wind_follow_region;
    let head = Head {
        on: &mut follow,
        tip: aurora_i18n::tr!("ui-weather-wind-follow-region-tip"),
        word: Some(aurora_i18n::tr!("ui-weather-wind-region")),
    };
    card(ui, p, "wind", aurora_i18n::tr!("ui-weather-wind"), Some(head), |ui, follow| {
        w.wind_follow_region = follow;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.add_enabled_ui(!w.wind_follow_region, |ui| compass(ui, p, w));
                let summary = wind_summary(w);
                ui.allocate_ui_with_layout(Vec2::new(COMPASS, 16.0), Layout::top_down(Align::Center), |ui| {
                    ui.label(RichText::new(summary).size(ENDS_SIZE).color(p.muted));
                });
            });
            ui.vertical(|ui| {
                // the wind wanders between its lulls and its gusts;
                // dragging one past the other takes it along
                let lulls = wind_word(w.wind_min);
                if slider_row(
                    ui,
                    p,
                    (aurora_i18n::tr!("ui-weather-lulls"), aurora_i18n::tr!("ui-weather-wind-tip")),
                    lulls,
                    None,
                    egui::Slider::new(&mut w.wind_min, 0.0..=1.0),
                )
                .changed()
                {
                    w.wind_max = w.wind_max.max(w.wind_min);
                }
                let gusts = wind_word(w.wind_max);
                if slider_row(
                    ui,
                    p,
                    (aurora_i18n::tr!("ui-weather-gusts"), aurora_i18n::tr!("ui-weather-gusts-tip")),
                    gusts,
                    None,
                    egui::Slider::new(&mut w.wind_max, 0.0..=1.0),
                )
                .changed()
                {
                    w.wind_min = w.wind_min.min(w.wind_max);
                }
                ui.add_enabled_ui(!w.wind_follow_region, |ui| {
                    let from = format!("{} · {:.0}°", compass_point(w.wind_from), w.wind_from);
                    slider_row(
                        ui,
                        p,
                        (
                            aurora_i18n::tr!("ui-weather-wind-from"),
                            aurora_i18n::tr!("ui-weather-wind-from-tip"),
                        ),
                        &from,
                        None,
                        egui::Slider::new(&mut w.wind_from, 0.0..=355.0).step_by(f64::from(DIRECTION_STEP)),
                    );
                });
            });
        });
    });
}

/// Degrees clockwise from north of `pos` seen from `center`, north up.
fn bearing(center: egui::Pos2, pos: egui::Pos2) -> f32 {
    let d = pos - center;
    d.x.atan2(-d.y).to_degrees().rem_euclid(360.0)
}

/// A direction on the wind's slider: its steps, 0 to 355.
fn snap_direction(degrees: f32) -> f32 {
    ((degrees / DIRECTION_STEP).round() * DIRECTION_STEP).rem_euclid(360.0)
}

/// The point at `degrees` clockwise from north, `radius` from `center`.
fn at(center: egui::Pos2, radius: f32, degrees: f32) -> egui::Pos2 {
    let a = degrees.to_radians();
    center + Vec2::new(a.sin(), -a.cos()) * radius
}

/// The wind's compass, north up: a dot on its rim where the wind comes
/// from, an arrow the way it blows. A click or a drag sets where it comes
/// from.
fn compass(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(COMPASS), Sense::click_and_drag());
    let enabled = ui.is_enabled();
    if enabled
        && (resp.dragged() || resp.clicked())
        && let Some(pos) = resp.interact_pointer_pos()
        && pos.distance(rect.center()) > 4.0
    {
        w.wind_from = snap_direction(bearing(rect.center(), pos));
    }
    let resp = resp.on_hover_text(aurora_i18n::tr!("ui-weather-wind-from-tip"));
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    let fade = |c: egui::Color32| if enabled { c } else { c.gamma_multiply(0.45) };
    let painter = ui.painter();
    let c = rect.center();
    let r = COMPASS * 0.5 - 1.0;
    painter.circle_filled(c, r, p.bar);
    painter.circle_stroke(c, r * 0.84, Stroke::new(1.0, fade(p.raised)));
    for i in 0..8 {
        let deg = i as f32 * 45.0;
        let (inner, outer) = if i % 2 == 0 { (r * 0.9, r * 0.98) } else { (r * 0.92, r * 0.97) };
        painter.line_segment([at(c, inner, deg), at(c, outer, deg)], Stroke::new(1.5, fade(p.muted_dim)));
    }
    let font = egui::FontId::proportional(10.0);
    for (deg, north) in [(0.0, true), (90.0, false), (180.0, false), (270.0, false)] {
        let color = if north { p.violet_pale } else { p.muted };
        painter.text(
            at(c, r * 0.66, deg),
            egui::Align2::CENTER_CENTER,
            compass_point(deg),
            font.clone(),
            fade(color),
        );
    }
    // where it comes from, on the rim; the arrow points the way it blows
    painter.circle_filled(at(c, r * 0.84, w.wind_from), 4.0, fade(p.teal));
    let to = w.wind_from + 180.0;
    let tip = at(c, r * 0.42, to);
    let back = r * 0.3;
    let wing_l = at(c, back, to + 145.0);
    let wing_r = at(c, back, to - 145.0);
    let notch = at(c, back * 0.35, to + 180.0);
    let arrow = fade(p.violet_light);
    painter.add(egui::Shape::convex_polygon(vec![tip, wing_r, notch], arrow, Stroke::NONE));
    painter.add(egui::Shape::convex_polygon(vec![tip, notch, wing_l], arrow, Stroke::NONE));
}

/// "Thunder": the switch, then how often lightning strikes. It comes with
/// heavy rain: greyed while it snows.
fn thunder_card(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings) {
    let snowing = w.snowing();
    let mut on = w.thunder;
    ui.add_enabled_ui(!snowing, |ui| {
        let head = Head {
            on: &mut on,
            tip: aurora_i18n::tr!("ui-weather-thunder-tip"),
            word: None,
        };
        card(
            ui,
            p,
            "bolt-filled",
            aurora_i18n::tr!("ui-weather-thunder"),
            Some(head),
            |ui, on| {
                w.thunder = on;
                let note = if snowing {
                    aurora_i18n::tr!("ui-weather-thunder-snow-note")
                } else {
                    aurora_i18n::tr!("ui-weather-thunder-tip")
                };
                ui.add(egui::Label::new(RichText::new(note).size(VALUE_SIZE).color(p.muted)).wrap());
                ui.add_enabled_ui(w.thunder, |ui| {
                    let often = if w.thunder_frequency < 0.25 {
                        aurora_i18n::tr!("ui-weather-strikes-seldom")
                    } else if w.thunder_frequency < 0.7 {
                        aurora_i18n::tr!("ui-weather-strikes-storm")
                    } else {
                        aurora_i18n::tr!("ui-weather-strikes-often")
                    };
                    slider_row(
                        ui,
                        p,
                        (aurora_i18n::tr!("ui-weather-strikes"), aurora_i18n::tr!("ui-weather-strikes-tip")),
                        often,
                        Some((
                            aurora_i18n::tr!("ui-weather-strikes-seldom"),
                            aurora_i18n::tr!("ui-weather-strikes-often"),
                        )),
                        egui::Slider::new(&mut w.thunder_frequency, 0.0..=1.0),
                    );
                });
            },
        );
    });
}

/// The aurora's colour at its foot (green in all three).
fn aurora_foot() -> egui::Color32 {
    egui::Color32::from_rgb(0x34, 0xD3, 0x7A) // theme-guard: content — the aurora's own green
}

/// The aurora's colour toward its top.
fn aurora_top(colours: AuroraColours) -> egui::Color32 {
    match colours {
        AuroraColours::Green => egui::Color32::from_rgb(0x9C, 0xF5, 0xC2), // theme-guard: content — the aurora's pale green top
        AuroraColours::Violet => egui::Color32::from_rgb(0xA7, 0x8B, 0xFA), // theme-guard: content — the aurora's violet top
        AuroraColours::Red => egui::Color32::from_rgb(0xF8, 0x71, 0x71),   // theme-guard: content — the aurora's red top
    }
}

/// "Northern lights": the switch, a preview of the night sky, how bright,
/// where and how high, and their colours.
fn northern_lights_card(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings) {
    let mut on = w.aurora;
    let head = Head {
        on: &mut on,
        tip: aurora_i18n::tr!("ui-weather-northern-lights-tip"),
        word: None,
    };
    card(
        ui,
        p,
        "sparkles-filled",
        aurora_i18n::tr!("ui-weather-northern-lights"),
        Some(head),
        |ui, on| {
            w.aurora = on;
            ui.add_enabled_ui(w.aurora, |ui| {
                aurora_preview(ui, p, w);
                let bright = format!("{:.0} %", w.aurora_intensity * 100.0);
                slider_row(
                    ui,
                    p,
                    (
                        aurora_i18n::tr!("ui-weather-northern-lights-brightness"),
                        aurora_i18n::tr!("ui-weather-northern-lights-brightness-tip"),
                    ),
                    &bright,
                    Some((
                        aurora_i18n::tr!("ui-weather-northern-lights-faint"),
                        aurora_i18n::tr!("ui-weather-northern-lights-display"),
                    )),
                    egui::Slider::new(&mut w.aurora_intensity, 0.0..=1.0),
                );
                ui.columns(2, |cols| {
                    let [left, right] = cols else { return };
                    let heading = format!("{} · {:.0}°", compass_point(w.aurora_heading), w.aurora_heading);
                    slider_row(
                        left,
                        p,
                        (
                            aurora_i18n::tr!("ui-weather-northern-lights-direction"),
                            aurora_i18n::tr!("ui-weather-northern-lights-direction-tip"),
                        ),
                        &heading,
                        None,
                        egui::Slider::new(&mut w.aurora_heading, 0.0..=355.0).step_by(f64::from(DIRECTION_STEP)),
                    );
                    let height = format!("{:.0} %", w.aurora_height * 100.0);
                    slider_row(
                        right,
                        p,
                        (
                            aurora_i18n::tr!("ui-weather-northern-lights-height"),
                            aurora_i18n::tr!("ui-weather-northern-lights-height-tip"),
                        ),
                        &height,
                        None,
                        egui::Slider::new(&mut w.aurora_height, 0.0..=1.0),
                    );
                });
                ui.horizontal(|ui| {
                    let gap = ui.spacing().item_spacing.x;
                    let names = colour_names();
                    let options = [(names[0], true), (names[1], true), (names[2], true)];
                    let width = ui.available_width() - widgets::segmented_size(ui, &options).x - gap;
                    label(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-weather-northern-lights-colours"),
                        aurora_i18n::tr!("ui-weather-northern-lights-colours-tip"),
                        width,
                    );
                    let mut i = usize::from(w.aurora_colours.index());
                    if widgets::segmented(ui, p, &mut i, &options) {
                        w.aurora_colours = AuroraColours::ALL[i.min(2)];
                    }
                });
                ui.horizontal(|ui| {
                    widgets::glyph(ui, "moon-filled", p.violet_pale, 13.0);
                    ui.label(
                        RichText::new(aurora_i18n::tr!("ui-weather-northern-lights-night"))
                            .size(ENDS_SIZE)
                            .color(p.muted),
                    );
                });
            });
        },
    );
}

/// The night sky as a strip of horizon, north in its middle: the curtain
/// of light toward its direction, as high as its foot is seen, as bright as
/// it is set, in its colours.
fn aurora_preview(ui: &mut egui::Ui, p: &Palette, w: &WeatherSettings) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, AURORA_PREVIEW_H), Sense::hover());
    let enabled = ui.is_enabled();
    let fade = |c: egui::Color32| if enabled { c } else { c.gamma_multiply(0.4) };
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let radius = ui.visuals().window_corner_radius;
    painter.rect_filled(rect, radius, p.navy);
    // a few stars, always at the same places
    for (x, y, s) in [
        (0.12, 0.2, 1.2),
        (0.3, 0.42, 0.8),
        (0.47, 0.12, 1.0),
        (0.63, 0.3, 1.3),
        (0.81, 0.16, 0.9),
        (0.9, 0.46, 0.8),
    ] {
        let pos = egui::pos2(rect.left() + rect.width() * x, rect.top() + rect.height() * y);
        painter.circle_filled(pos, s, fade(p.ink.gamma_multiply(0.6)));
    }
    let ground = rect.bottom() - 14.0;
    // north in the middle, the whole round of the horizon across
    let heading = (w.aurora_heading + 180.0).rem_euclid(360.0) - 180.0;
    let center_x = rect.center().x + heading / 360.0 * rect.width();
    let band_w = rect.width() * 0.42;
    let band_h = 34.0;
    let foot = ground - 4.0 - w.aurora_height * (rect.height() - band_h - 22.0);
    let strength = 0.25 + 0.75 * w.aurora_intensity;
    let (low, high) = (aurora_foot(), aurora_top(w.aurora_colours));
    // drawn again one width aside, so that it wraps round the edges
    for shift in [-rect.width(), 0.0, rect.width()] {
        let cx = center_x + shift;
        if cx + band_w * 0.5 < rect.left() || cx - band_w * 0.5 > rect.right() {
            continue;
        }
        painter.add(egui::Shape::mesh(curtain(cx, foot, band_w, band_h, strength, low, high, enabled)));
    }
    painter.rect_filled(
        egui::Rect::from_min_max(egui::pos2(rect.left(), ground), rect.max),
        egui::CornerRadius::ZERO,
        p.bar,
    );
    let font = egui::FontId::proportional(9.5);
    for deg in [0.0f32, 90.0, 180.0, 270.0] {
        let x = rect.center().x + ((deg + 180.0).rem_euclid(360.0) - 180.0) / 360.0 * rect.width();
        painter.text(
            egui::pos2(x, ground + 7.0),
            egui::Align2::CENTER_CENTER,
            compass_point(deg),
            font.clone(),
            fade(p.muted),
        );
    }
    widgets::paint_glyph(
        &painter,
        "moon-filled",
        egui::pos2(rect.right() - 14.0, rect.top() + 13.0),
        13.0,
        fade(p.violet_pale),
    );
}

/// The curtain of light: a mesh whose colour goes from `low` at its foot to
/// `high` toward its top, fading out at its top, its foot and its sides.
#[allow(clippy::too_many_arguments)]
fn curtain(
    cx: f32,
    foot: f32,
    width: f32,
    height: f32,
    strength: f32,
    low: egui::Color32,
    high: egui::Color32,
    enabled: bool,
) -> egui::Mesh {
    const COLS: usize = 16;
    const ROWS: usize = 6;
    let mut mesh = egui::Mesh::default();
    for row in 0..=ROWS {
        let v = row as f32 / ROWS as f32;
        // 0 at the foot, 1 at the top: brightest a little above its foot
        let profile = (v * 4.0).min(1.0) * (1.0 - v).powf(1.4);
        let color = low.lerp_to_gamma(high, v.powf(0.8));
        for col in 0..=COLS {
            let u = col as f32 / COLS as f32;
            // a gentle wave along it, the sides fading out
            let side = (u * std::f32::consts::PI).sin().powf(1.5);
            let wave = (u * std::f32::consts::TAU * 1.5).sin() * 3.0 * v;
            let alpha = (profile * side * strength * if enabled { 1.0 } else { 0.4 }).clamp(0.0, 1.0);
            let pos = egui::pos2(cx - width * 0.5 + u * width, foot - v * height + wave);
            mesh.colored_vertex(pos, color.gamma_multiply(alpha));
        }
    }
    let stride = (COLS + 1) as u32;
    for row in 0..ROWS as u32 {
        for col in 0..COLS as u32 {
            let a = row * stride + col;
            mesh.add_triangle(a, a + 1, a + stride);
            mesh.add_triangle(a + 1, a + stride + 1, a + stride);
        }
    }
    mesh
}

/// The foot of the window: the quality on the left; on the right the
/// clean-up and, in snow, the snow fill.
fn foot(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings, actions: &mut WeatherActions) {
    ui.horizontal(|ui| {
        ui.horizontal(|ui| {
            widgets::glyph(ui, "adjustments-horizontal-filled", p.muted, 15.0);
            ui.label(RichText::new(aurora_i18n::tr!("ui-weather-quality")).size(LABEL_SIZE).color(p.ink))
                .on_hover_text(aurora_i18n::tr!("ui-weather-quality-tip"));
            let low = aurora_i18n::tr!("ui-weather-quality-low");
            let medium = aurora_i18n::tr!("ui-weather-quality-medium");
            let high = aurora_i18n::tr!("ui-weather-quality-high");
            let mut i = match w.quality {
                WeatherQuality::Low => 0,
                WeatherQuality::Medium => 1,
                WeatherQuality::High => 2,
            };
            if widgets::segmented(ui, p, &mut i, &[(low, true), (medium, true), (high, true)]) {
                w.quality = match i {
                    0 => WeatherQuality::Low,
                    1 => WeatherQuality::Medium,
                    _ => WeatherQuality::High,
                };
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let size = Vec2::new(0.0, 24.0);
            if widgets::action_button(
                ui,
                p,
                "eraser",
                aurora_i18n::tr!("ui-weather-clean-up"),
                widgets::ActionKind::Flat,
                size,
            )
            .on_hover_text(aurora_i18n::tr!("ui-weather-clean-up-tip"))
            .clicked()
            {
                actions.clean = true;
            }
            if w.snowing()
                && widgets::action_button_if(
                    ui,
                    p,
                    "snowflake",
                    aurora_i18n::tr!("ui-weather-snow-fill"),
                    widgets::ActionKind::Flat,
                    size,
                    w.snow_settles,
                )
                .on_hover_text(aurora_i18n::tr!("ui-weather-snow-fill-tip"))
                .clicked()
            {
                actions.fill = true;
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compass_reads_clockwise_from_north_with_north_up() {
        let c = egui::pos2(50.0, 50.0);
        assert!((bearing(c, egui::pos2(50.0, 10.0)) - 0.0).abs() < 0.01);
        assert!((bearing(c, egui::pos2(90.0, 50.0)) - 90.0).abs() < 0.01);
        assert!((bearing(c, egui::pos2(50.0, 90.0)) - 180.0).abs() < 0.01);
        assert!((bearing(c, egui::pos2(10.0, 50.0)) - 270.0).abs() < 0.01);
        // and back: the point at a bearing reads as that bearing
        for deg in [0.0, 45.0, 135.0, 270.0, 315.0] {
            assert!((bearing(c, at(c, 30.0, deg)) - deg).abs() < 0.01, "{deg}");
        }
    }

    #[test]
    fn a_direction_snaps_to_the_sliders_steps_and_stays_below_a_full_turn() {
        assert_eq!(snap_direction(0.0), 0.0);
        assert_eq!(snap_direction(272.4), 270.0);
        assert_eq!(snap_direction(273.0), 275.0);
        // just short of north is north, never 360
        assert_eq!(snap_direction(358.0), 0.0);
    }

    #[test]
    fn the_sky_is_named_by_what_falls_and_how_hard() {
        assert_eq!(amount_word(false, 0.0), amount_word(true, 0.0));
        assert_ne!(amount_word(false, 0.1), amount_word(false, 0.9));
        assert_ne!(amount_word(false, 0.6), amount_word(true, 0.6));
        let mut w = WeatherSettings {
            touched: true,
            amount: 0.9,
            thunder: true,
            ..WeatherSettings::default()
        };
        assert!(stormy(&w));
        // no thunder in light rain, in snow, or switched off
        w.amount = 0.2;
        assert!(!stormy(&w));
        w.amount = 0.9;
        w.precipitation = Precipitation::Snow;
        assert!(!stormy(&w));
        w.precipitation = Precipitation::Rain;
        w.thunder = false;
        assert!(!stormy(&w));
    }
}
