//! The Aurora Weather window: the weather drawn for this viewer only
//! (Aurora's own: the Legacy Viewer has none). A summary of the sky shown
//! now with the main switch on its right; cards in as many columns as the
//! window's width holds, one to five (narrowed, the right ones go under the
//! left ones): what falls and what it leaves on the ground; the wind (with
//! its compass), thunder and the northern lights (with their preview), each
//! switch on its title row greying its card; the quality and the clean-up
//! and snow-fill actions at the foot. The window resizes by its edges: the
//! summary stays on top and the foot at the bottom, the cards scrolling
//! between them.
//!
//! The settings are those of [`WeatherSettings`], unchanged: this module is
//! their window only.

use super::environment::{colour_names, compass_point};
use super::widgets;
use crate::theme::Palette;
use crate::weather::{AuroraColours, DEEP_SNOW_REACH, PUDDLE_DISTANCE, Precipitation, SNOW_DISTANCE, WeatherQuality, WeatherSettings};
use egui::{Align, Layout, RichText, Sense, Stroke, Vec2};

/// The window's size when it first opens: two columns of cards, all of
/// them shown (the floater keeps it within the screen).
pub const WINDOW_SIZE: Vec2 = Vec2::new(700.0, 880.0);
/// The smallest it resizes to: one column of cards, the summary, a card's
/// first rows and the foot.
pub const WINDOW_MIN_SIZE: Vec2 = Vec2::new(360.0, 320.0);
/// The least width of a column of cards at a text scale of 1: one more
/// column fits each time the body widens by this much (and a gap).
const COLUMN_MIN_W: f32 = 300.0;
/// Narrower than this body width at a text scale of 1, the summary
/// compacts: a smaller picture and title.
const COMPACT_SUMMARY_W: f32 = 520.0;
/// The side of the summary's picture of the sky, and in a compact summary.
const SKY_TILE: f32 = 72.0;
const SKY_TILE_COMPACT: f32 = 44.0;
/// The side of the wind's compass, at most and at least.
const COMPASS: f32 = 112.0;
const COMPASS_MIN: f32 = 84.0;
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

/// The window, at `pos`: resized by its edges, its body laid out in its
/// exact rect (`steady_size`) so that nothing in it, rounded to the screen's
/// pixels, measures past the window: at 127 % the columns came out a point
/// wider than it, and a resize took that for the least width the window
/// could have (a window widened into more columns could not be narrowed).
pub fn floater<'a>(pos: egui::Pos2) -> widgets::Floater<'a> {
    let mut floater = widgets::Floater::new("aurora_weather", aurora_i18n::tr!("ui-weather-aurora"), pos, WINDOW_SIZE)
        .help(aurora_i18n::tr!("ui-weather-window-help"))
        .steady_size();
    floater.min_size = WINDOW_MIN_SIZE;
    floater
}

/// The window's body, in the window of [`floater`]: the summary on top and
/// the foot at the bottom (`widgets::above_footer`, the foot's height as it
/// was drawn), the cards scrolling between them in as many columns as its
/// width holds.
pub fn body(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings) -> WeatherActions {
    let before = w.clone();
    let mut actions = WeatherActions::default();
    ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
    summary(ui, p, w);
    ui.add_space(4.0);
    let one_row = foot_in_one_row(ui, w);
    let (mut list, footer) = widgets::above_footer(ui);
    egui::ScrollArea::vertical()
        .id_salt("weather_cards")
        .auto_shrink([false, false])
        .show(&mut list, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            // the main switch off, every card is greyed, its settings kept
            ui.add_enabled_ui(w.enabled, |ui| cards(ui, p, w));
        });
    footer.show(ui, |ui| foot(ui, p, w, &mut actions, one_row));
    if *w != before {
        w.take_over(&before);
        w.touched = true;
    }
    actions
}

/// The cards, in the order they stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Card {
    Precipitation,
    Ground,
    Wind,
    Thunder,
    NorthernLights,
}

/// How many columns of cards a body `width` wide holds, each at least
/// `column_min` wide: 1 to 5.
fn column_count(width: f32, gap: f32, column_min: f32) -> usize {
    (((width + gap) / (column_min + gap)).floor() as usize).clamp(1, 5)
}

/// The cards of each of `n` columns, left to right: what falls and what it
/// leaves together, then the wind and thunder, then the northern lights;
/// narrowed, the right columns go under the left ones.
fn layout(n: usize) -> &'static [&'static [Card]] {
    use Card::*;
    match n {
        0 | 1 => &[&[Precipitation, Ground, Wind, Thunder, NorthernLights]],
        2 => &[&[Precipitation, Ground], &[Wind, Thunder, NorthernLights]],
        3 => &[&[Precipitation, Ground], &[Wind, Thunder], &[NorthernLights]],
        4 => &[&[Precipitation], &[Ground], &[Wind, Thunder], &[NorthernLights]],
        _ => &[&[Precipitation], &[Ground], &[Wind], &[Thunder], &[NorthernLights]],
    }
}

/// The cards in as many columns as the width holds (wider columns at a
/// larger text scale), each laid out in its own rect, its width a whole
/// number of the screen's pixels: the room the columns take is what they
/// are given, whatever a card measures (egui's `columns` rounded their
/// edges past it).
fn cards(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings) {
    let gap = ui.spacing().item_spacing.x;
    let room = ui.available_width();
    let column_min = COLUMN_MIN_W * crate::ui::fonts::scale(ui.ctx());
    let columns = layout(column_count(room, gap, column_min));
    let n = columns.len() as f32;
    let ppp = ui.ctx().pixels_per_point();
    let column_w = (((room - gap * (n - 1.0)) / n) * ppp).floor() / ppp;
    let top_left = ui.cursor().min;
    let mut bottom = top_left.y;
    for (i, cards) in columns.iter().enumerate() {
        let x = top_left.x + i as f32 * (column_w + gap);
        let rect = egui::Rect::from_min_size(egui::pos2(x, top_left.y), Vec2::new(column_w, f32::INFINITY));
        // the column's place in the order, not the frame's count: the same
        // ids whatever the layout
        let mut col = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("weather_column", i))
                .max_rect(rect)
                .layout(Layout::top_down(Align::Min)),
        );
        col.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        for (j, card) in cards.iter().enumerate() {
            if j > 0 {
                col.add_space(4.0);
            }
            show_card(&mut col, p, w, *card);
        }
        bottom = bottom.max(col.min_rect().bottom());
    }
    let used = column_w * n + gap * (n - 1.0);
    ui.advance_cursor_after_rect(egui::Rect::from_min_max(top_left, egui::pos2(top_left.x + used, bottom)));
}

fn show_card(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings, card: Card) {
    match card {
        Card::Precipitation => precipitation_card(ui, p, w),
        // what falls leaves it: greyed with the "Falling" card
        Card::Ground => {
            ui.add_enabled_ui(w.precipitation_on, |ui| ground_card(ui, p, w));
        }
        Card::Wind => wind_card(ui, p, w),
        Card::Thunder => thunder_card(ui, p, w),
        Card::NorthernLights => northern_lights_card(ui, p, w),
    }
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

/// The sky `w` draws (its settings [`WeatherSettings::in_effect`]).
fn sky_now(p: &Palette, w: &WeatherSettings) -> SkyNow {
    let amount = w.rain_or_amount();
    let snow = w.snowing();
    if !w.enabled {
        SkyNow {
            icon: "cloud-off",
            color: p.muted,
            title: aurora_i18n::tr!("ui-weather-now-off"),
        }
    } else if amount <= 0.0 {
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
/// few chips), and on its right the main switch. Narrowed, a smaller
/// picture and title, its chips wrapping under them.
fn summary(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings) {
    widgets::surface(ui, p, |ui| {
        ui.set_min_width(ui.available_width());
        let compact = ui.available_width() < COMPACT_SUMMARY_W * crate::ui::fonts::scale(ui.ctx());
        let (side, title_size) = if compact { (SKY_TILE_COMPACT, 16.0) } else { (SKY_TILE, 20.0) };
        ui.horizontal_top(|ui| {
            // what is drawn, not what the greyed cards still hold
            let drawn = w.in_effect();
            let now = sky_now(p, &drawn);
            let (tile, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
            ui.painter().rect_filled(tile, ui.visuals().window_corner_radius, p.bar);
            widgets::paint_glyph(ui.painter(), now.icon, tile.center(), side * 0.58, now.color);
            let gap = ui.spacing().item_spacing.x;
            let text_w = (ui.available_width() - widgets::SWITCH_SIZE.x - gap).max(80.0);
            ui.allocate_ui_with_layout(Vec2::new(text_w, side), Layout::top_down(Align::Min), |ui| {
                ui.set_width(text_w);
                ui.spacing_mut().item_spacing.y = if compact { 2.0 } else { 3.0 };
                ui.label(RichText::new(aurora_i18n::tr!("ui-weather-now")).size(VALUE_SIZE).color(p.muted));
                ui.add(
                    egui::Label::new(
                        RichText::new(now.title)
                            .font(widgets::control_font(ui, title_size))
                            .strong()
                            .color(p.ink),
                    )
                    .wrap(),
                );
                ui.add_space(if compact { 1.0 } else { 2.0 });
                chips(ui, p, &drawn);
            });
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                widgets::switch(ui, p, &mut w.enabled).on_hover_text(aurora_i18n::tr!("ui-weather-enabled-tip"));
            });
        });
    });
}

/// The chips under the sky's name: the wind, and what else the weather
/// brings now (lightning, puddles, snow on the ground, the northern lights).
/// `w`: what is drawn ([`WeatherSettings::in_effect`]).
fn chips(ui: &mut egui::Ui, p: &Palette, w: &WeatherSettings) {
    if !w.enabled {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        if w.wind_on {
            chip(ui, p, "wind", p.teal, &wind_summary(w));
        }
        let falling = w.rain_or_amount() > 0.0;
        let raining = !w.snowing() && falling;
        if stormy(w) {
            chip(ui, p, "bolt-filled", p.amber, aurora_i18n::tr!("ui-weather-chip-lightning"));
        }
        if raining && w.puddles {
            chip(ui, p, "droplet-filled", p.indigo_light, aurora_i18n::tr!("ui-weather-puddles"));
        }
        if w.snowing() && falling && w.snow_settles {
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

/// The switch on a card's title row, on its right: what it turns on and
/// its tip.
struct Head<'a> {
    on: &'a mut bool,
    tip: &'a str,
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
    let mut on = w.precipitation_on;
    let head = Head {
        on: &mut on,
        tip: aurora_i18n::tr!("ui-weather-precipitation-on-tip"),
    };
    card(
        ui,
        p,
        "cloud-rain",
        aurora_i18n::tr!("ui-weather-precipitation"),
        Some(head),
        |ui, on| {
            w.precipitation_on = on;
            ui.add_enabled_ui(on, |ui| precipitation_rows(ui, p, w));
        },
    );
}

/// The "Falling" card's rows, under its switch.
fn precipitation_rows(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings) {
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

/// "Wind": its switch on the title's right; whether it blows the region's
/// way, then its compass (where it comes from: click or drag on it) beside
/// its lulls, its gusts and its direction.
fn wind_card(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings) {
    let mut on = w.wind_on;
    let head = Head {
        on: &mut on,
        tip: aurora_i18n::tr!("ui-weather-wind-on-tip"),
    };
    card(ui, p, "wind", aurora_i18n::tr!("ui-weather-wind"), Some(head), |ui, on| {
        w.wind_on = on;
        ui.add_enabled_ui(on, |ui| wind_rows(ui, p, w));
    });
}

/// The "Wind" card's rows, under its switch.
fn wind_rows(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings) {
    switch_row(
        ui,
        p,
        "map-pin-filled",
        aurora_i18n::tr!("ui-weather-wind-follow-region"),
        aurora_i18n::tr!("ui-weather-wind-follow-region-tip"),
        &mut w.wind_follow_region,
    );
    // the compass gives up some of its size before the sliders beside it do
    let side = compass_side(ui.available_width());
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.add_enabled_ui(!w.wind_follow_region, |ui| compass(ui, p, w, side));
            let summary = wind_summary(w);
            ui.allocate_ui_with_layout(Vec2::new(side, 16.0), Layout::top_down(Align::Center), |ui| {
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
}

/// The side of the wind's compass in a card row `width` wide: a third of
/// it, between [`COMPASS_MIN`] and [`COMPASS`].
fn compass_side(width: f32) -> f32 {
    (width * 0.36).clamp(COMPASS_MIN, COMPASS)
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
fn compass(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings, side: f32) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(side), Sense::click_and_drag());
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
    let r = side * 0.5 - 1.0;
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
/// heavy rain: greyed while it snows, or while nothing falls.
fn thunder_card(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings) {
    let snowing = w.snowing();
    let mut on = w.thunder;
    ui.add_enabled_ui(!snowing && w.precipitation_on, |ui| {
        let head = Head {
            on: &mut on,
            tip: aurora_i18n::tr!("ui-weather-thunder-tip"),
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

/// How much of the curtain shows at `u` across it (0 and 1 its sides).
/// `sin(PI)` is a hair below zero in `f32`, and its power 1.5 is NaN: a NaN
/// alpha that stopped a development build at the colour's assertion (the
/// merge queue's run of 2026-11-21), so the sine is held at zero first.
fn side_fade(u: f32) -> f32 {
    (u * std::f32::consts::PI).sin().max(0.0).powf(1.5)
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
            let side = side_fade(u);
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

/// The height of the foot's buttons.
const FOOT_BUTTON_H: f32 = 24.0;

/// The quality's options, in their order.
fn quality_options() -> [(&'static str, bool); 3] {
    [
        (aurora_i18n::tr!("ui-weather-quality-low"), true),
        (aurora_i18n::tr!("ui-weather-quality-medium"), true),
        (aurora_i18n::tr!("ui-weather-quality-high"), true),
    ]
}

/// Whether the quality and the foot's buttons fit side by side in `ui`'s
/// width, measured in its language and text scale (a fixed width let a long
/// language's buttons run over the quality).
fn foot_in_one_row(ui: &egui::Ui, w: &WeatherSettings) -> bool {
    let gap = ui.spacing().item_spacing.x;
    let label = ui
        .painter()
        .layout_no_wrap(
            aurora_i18n::tr!("ui-weather-quality").to_owned(),
            egui::FontId::proportional(LABEL_SIZE),
            egui::Color32::PLACEHOLDER,
        )
        .size()
        .x;
    let quality = 15.0 + gap + label + gap + widgets::segmented_size(ui, &quality_options()).x;
    let mut buttons = widgets::action_button_fit(ui, "eraser", aurora_i18n::tr!("ui-weather-clean-up"));
    if w.snowing() {
        buttons += gap + widgets::action_button_fit(ui, "snowflake", aurora_i18n::tr!("ui-weather-snow-fill"));
    }
    // a gap at least between the two groups
    quality + gap * 2.0 + buttons <= ui.available_width()
}

/// The foot of the window: the quality on the left; on the right the
/// clean-up and, in snow, the snow fill, or under the quality when they do
/// not fit beside it (`one_row`). The main switch off greys all but the
/// clean-up: what the weather left can still go at once.
fn foot(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings, actions: &mut WeatherActions, one_row: bool) {
    ui.horizontal(|ui| {
        quality(ui, p, w);
        if one_row {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| foot_buttons(ui, p, w, actions));
        }
    });
    if !one_row {
        // a row of its own, as tall as its buttons
        let row = Vec2::new(ui.available_width(), FOOT_BUTTON_H);
        ui.allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| foot_buttons(ui, p, w, actions));
    }
}

/// The weather's quality, greyed with the main switch.
fn quality(ui: &mut egui::Ui, p: &Palette, w: &mut WeatherSettings) {
    ui.add_enabled_ui(w.enabled, |ui| {
        widgets::glyph(ui, "adjustments-horizontal-filled", p.muted, 15.0);
        ui.label(RichText::new(aurora_i18n::tr!("ui-weather-quality")).size(LABEL_SIZE).color(p.ink))
            .on_hover_text(aurora_i18n::tr!("ui-weather-quality-tip"));
        let mut i = match w.quality {
            WeatherQuality::Low => 0,
            WeatherQuality::Medium => 1,
            WeatherQuality::High => 2,
        };
        if widgets::segmented(ui, p, &mut i, &quality_options()) {
            w.quality = match i {
                0 => WeatherQuality::Low,
                1 => WeatherQuality::Medium,
                _ => WeatherQuality::High,
            };
        }
    });
}

/// The clean-up and, in snow, the snow fill, right to left.
fn foot_buttons(ui: &mut egui::Ui, p: &Palette, w: &WeatherSettings, actions: &mut WeatherActions) {
    let size = Vec2::new(0.0, FOOT_BUTTON_H);
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
            w.enabled && w.precipitation_on && w.snow_settles,
        )
        .on_hover_text(aurora_i18n::tr!("ui-weather-snow-fill-tip"))
        .clicked()
    {
        actions.fill = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curtain_fades_out_at_both_sides_without_a_nan() {
        // the right side's sine is a hair below zero in f32
        for col in 0..=16 {
            let side = side_fade(col as f32 / 16.0);
            assert!(side.is_finite() && (0.0..=1.0).contains(&side), "{col}: {side}");
        }
        assert_eq!(side_fade(1.0), 0.0);
        assert!((side_fade(0.5) - 1.0).abs() < 1e-6);
    }

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
    fn the_columns_follow_the_width_and_every_card_shows_once() {
        assert_eq!(column_count(WINDOW_MIN_SIZE.x - 24.0, 6.0, COLUMN_MIN_W), 1);
        assert_eq!(column_count(WINDOW_SIZE.x - 24.0, 6.0, COLUMN_MIN_W), 2);
        assert_eq!(column_count(960.0, 6.0, COLUMN_MIN_W), 3);
        assert_eq!(column_count(4000.0, 6.0, COLUMN_MIN_W), 5);
        // a larger text: wider columns, fewer of them
        assert_eq!(column_count(960.0, 6.0, COLUMN_MIN_W * 1.5), 2);
        for n in 1..=5 {
            let cards: Vec<Card> = layout(n).iter().flat_map(|c| c.iter().copied()).collect();
            assert_eq!(layout(n).len(), n);
            assert_eq!(
                cards,
                [Card::Precipitation, Card::Ground, Card::Wind, Card::Thunder, Card::NorthernLights],
                "{n}"
            );
        }
    }

    /// The body at `width` and `height`: the width it took.
    fn laid_out(width: f32, height: f32) -> f32 {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let mut w = WeatherSettings {
            touched: true,
            precipitation: Precipitation::Snow,
            aurora: true,
            ..WeatherSettings::default()
        };
        let mut taken = 0.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(2000.0, 1400.0))),
                ..Default::default()
            },
            |ui| {
                let rect = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), Vec2::new(width, height));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                child.set_min_size(rect.size());
                body(&mut child, &p, &mut w);
                taken = child.min_rect().width();
            },
        );
        output.textures_delta.clear();
        taken
    }

    #[test]
    fn the_body_fits_its_width_from_the_narrowest_to_five_columns() {
        for width in [WINDOW_MIN_SIZE.x - 24.0, 420.0, 520.0, 610.0, 676.0, 960.0, 1300.0, 1700.0] {
            for height in [WINDOW_MIN_SIZE.y - 60.0, 900.0] {
                let taken = laid_out(width, height);
                assert!(taken <= width + 0.5, "{width} x {height}: took {taken}");
            }
        }
    }

    #[test]
    fn narrowed_the_compass_shrinks() {
        assert_eq!(compass_side(400.0), COMPASS);
        assert_eq!(compass_side(200.0), COMPASS_MIN);
        let mid = compass_side(280.0);
        assert!(mid > COMPASS_MIN && mid < COMPASS, "{mid}");
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

/// The window resized by its edges, as on a screen at 127 %: it follows the
/// pointer both ways. Its body once came out a few points taller than the
/// window (the foot's height guessed short), and every frame of a resize
/// grew the window to it: dragging the bottom up made it taller, and a
/// corner widened into more columns could not be brought back.
#[cfg(test)]
mod resize_tests {
    use super::*;

    struct Fixture {
        ctx: egui::Context,
        weather: WeatherSettings,
        frame: u32,
    }

    impl Fixture {
        fn show(&mut self, events: Vec<egui::Event>) -> egui::Rect {
            self.frame += 1;
            let p = crate::theme::Theme::default().palette();
            let weather = &mut self.weather;
            let mut out = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(2400.0, 1300.0))),
                    time: Some(f64::from(self.frame) / 60.0),
                    events,
                    ..Default::default()
                },
                |ui| {
                    floater(egui::pos2(100.0, 80.0))
                        .silent()
                        .show(ui.ctx(), &p, &mut true, |ui| body(ui, &p, weather));
                },
            );
            out.textures_delta.clear();
            self.ctx.memory(|m| m.area_rect(egui::Id::unique("aurora_weather"))).expect("shown")
        }

        fn button(&mut self, pos: egui::Pos2, pressed: bool) -> egui::Rect {
            self.show(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }])
        }

        /// Drags from `grip` through `steps` (offsets), checking each
        /// frame's size against `expected`.
        fn drag(&mut self, grip: egui::Pos2, steps: &[Vec2], expected: impl Fn(Vec2) -> Vec2) -> egui::Rect {
            self.show(vec![egui::Event::PointerMoved(grip)]);
            self.button(grip, true);
            for step in steps {
                let rect = self.show(vec![egui::Event::PointerMoved(grip + *step)]);
                let want = expected(*step);
                assert!(
                    (rect.width() - want.x).abs() < 1.0 && (rect.height() - want.y).abs() < 1.0,
                    "dragged by {step:?}: {} x {}, expected {want:?}",
                    rect.width(),
                    rect.height()
                );
            }
            self.button(grip + *steps.last().expect("a step"), false);
            self.show(vec![])
        }
    }

    #[test]
    fn the_window_follows_its_edges_both_ways() {
        for mode in ["rain", "snow"] {
            follows_its_edges(mode);
        }
    }

    /// In the demo's `mode` weather, the viewer's own fonts.
    fn follows_its_edges(mode: &str) {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(1.27);
        crate::theme::Theme::default().apply(&ctx, 1.0);
        crate::ui::fonts::apply(&ctx, &crate::settings::Settings::default().font);
        let mut weather = WeatherSettings::default();
        crate::demo::env::weather_scene(mode, &mut weather);
        let mut f = Fixture { ctx, weather, frame: 0 };
        let mut start = f.show(vec![]);
        for _ in 0..8 {
            start = f.show(vec![]);
        }
        let size = start.size();
        // the bottom dragged up: shorter at every frame
        let ys = [-50.0, -150.0, -300.0];
        let steps: Vec<Vec2> = ys.iter().map(|y| Vec2::new(0.0, *y)).collect();
        let shorter = f.drag(egui::pos2(start.center().x, start.bottom() - 1.0), &steps, |d| size + d);
        // the corner: wider into more columns, then back narrower than at first
        let size = shorter.size();
        let steps = [
            Vec2::new(300.0, 100.0),
            Vec2::new(700.0, 200.0),
            Vec2::new(100.0, 50.0),
            Vec2::new(-200.0, -100.0),
        ];
        let back = f.drag(shorter.max - Vec2::splat(4.0), &steps, |d| size + d);
        // at rest, it keeps its size and the foot stays at the bottom
        let mut rest = back;
        for _ in 0..60 {
            rest = f.show(vec![]);
        }
        assert_eq!(rest.size(), back.size());
        // five columns and back to one, by the right side
        let size = rest.size();
        let steps = [
            Vec2::new(900.0, 0.0),
            Vec2::new(1200.0, 0.0),
            Vec2::new(600.0, 0.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(-(size.x - WINDOW_MIN_SIZE.x), 0.0),
        ];
        let narrow = f.drag(egui::pos2(rest.right() - 1.0, rest.center().y), &steps, |d| size + d);
        // one column, the foot in two rows: it stays put at rest too
        let mut rest = narrow;
        for _ in 0..60 {
            rest = f.show(vec![]);
        }
        assert_eq!(rest.size(), narrow.size(), "{mode}");
        // from there the corner, wider and taller: the foot goes back to
        // one row on the way, and both sides keep following the pointer
        let size = rest.size();
        let steps = [
            Vec2::new(80.0, 40.0),
            Vec2::new(200.0, 120.0),
            Vec2::new(420.0, 200.0),
            Vec2::new(150.0, 60.0),
        ];
        f.drag(rest.max - Vec2::splat(4.0), &steps, |d| size + d);
    }

    /// Snow, rain and the night, the viewer's own fonts and a 127 %
    /// screen: the right side dragged a screen pixel at a time from the
    /// narrowest out to five columns and back, the window as wide as the
    /// pointer asks at every frame (a column a point too wide held it).
    #[test]
    fn every_width_follows_the_pointer_at_127_percent() {
        for mode in ["snow", "rain", "night"] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(1.27);
            crate::theme::Theme::default().apply(&ctx, 1.0);
            crate::ui::fonts::apply(&ctx, &crate::settings::Settings::default().font);
            let mut weather = WeatherSettings::default();
            crate::demo::env::weather_scene(mode, &mut weather);
            let mut f = Fixture { ctx, weather, frame: 0 };
            let mut start = f.show(vec![]);
            for _ in 0..8 {
                start = f.show(vec![]);
            }
            let size = start.size();
            let pixel = 1.0 / 1.27;
            let narrowest = WINDOW_MIN_SIZE.x - size.x;
            let widest = 1800.0 - size.x;
            let mut steps = Vec::new();
            let mut x = 0.0;
            while x > narrowest {
                steps.push(Vec2::new(x, 0.0));
                x -= pixel;
            }
            while x < widest {
                steps.push(Vec2::new(x, 0.0));
                x += pixel;
            }
            while x > narrowest {
                steps.push(Vec2::new(x, 0.0));
                x -= pixel;
            }
            f.drag(egui::pos2(start.right() - 1.0, start.center().y), &steps, |d| size + d);
        }
    }
}
