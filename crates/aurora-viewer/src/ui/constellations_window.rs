//! The Constellations window: the constellations drawn over the real night
//! sky (world/constellations.rs; Aurora's own, the Legacy Viewer has none),
//! built like the Aurora Weather window (weather_window.rs, whose cards and
//! rows it shares). From the top: the main switch, how far the sun must have
//! set, and a line saying why nothing shows when nothing can; the
//! constellation set shown, one at a time from a drop-down (every art set
//! of it used); the figures' opacity and a rescan of the folder; the lines
//! and their width, the names and their size, the figures no longer
//! counted; how they draw and fade and the field around the view; the
//! effects (the drawing brightness, the completion
//! highlight, a colour per constellation, the sparkles at the pen tips and
//! their size, shape and turns, shown while they are on); then "See
//! licences and attribution" (the data's credits, a window of their own)
//! and "Reset to defaults". The sections are cards in a
//! column: a later one (the solar system) is one more card. Every row with
//! a switch has its icon, its label and its switch at the card's right
//! edge, like the cards' own switches.
//!
//! Every control acts at once: the sky follows a slider while it moves,
//! and what a switch or a check mark turns on or off fades in or out.

use super::weather_window::{Head, card, slider_row};
use super::widgets;
use crate::theme::Palette;
use crate::ui::widgets::Floater;
use crate::world::constellations::{
    BRIGHTNESS_RANGE, ConstellationSettings, DARK_RANGE, DRAW_RANGE, DataStatus, FADE_RANGE, FIELD_RANGE, FONT_RANGE, OPACITY_RANGE,
    SPARKLE_SIZE_RANGE, SPARKLE_SPIN_RANGE, SparkleShape, Status, WIDTH_RANGE,
};
use egui::{Align, Layout, RichText, Vec2};

/// The window's id (its place and size are remembered under it).
pub const WINDOW: &str = "constellations";
/// The window's width: one column of cards.
pub const WINDOW_W: f32 = 380.0;
const LABEL_SIZE: f32 = 12.5;
const VALUE_SIZE: f32 = 12.0;

/// The window, when open. `status`: why nothing shows (the overlay's last
/// frame); `credits`: the credits window's open state (its button here);
/// `bodies`: the planets' and the Sun and Moon's switches (their own
/// settings, world/planets.rs). Returns whether a setting changed.
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    open: &mut bool,
    credits: &mut bool,
    s: &mut ConstellationSettings,
    bodies: &mut crate::world::planets::Wanted,
    status: &Status,
) -> bool {
    if !*open {
        return false;
    }
    let screen = ctx.content_rect();
    // at the left of the screen: the weather window opens left of the
    // Environment window, on the right
    let pos = egui::pos2(screen.left() + 60.0, screen.top() + 90.0);
    let before = (s.clone(), *bodies);
    Floater::new(
        WINDOW,
        aurora_i18n::tr!("ui-options-constellations"),
        pos,
        Vec2::new(WINDOW_W, 300.0),
    )
    .auto_height()
    .help(aurora_i18n::tr!("ui-constellations-window-help"))
    .show(ctx, p, open, |ui| {
        if body(ui, p, s, bodies, status, &crate::world::constellations::data_status()) {
            super::widgets::toggle_floater(ctx, CREDITS_WINDOW, credits);
        }
    });
    (s.clone(), *bodies) != before
}

/// The window's body; true when its credits button was clicked.
pub fn body(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut ConstellationSettings,
    bodies: &mut crate::world::planets::Wanted,
    status: &Status,
    data: &DataStatus,
) -> bool {
    ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
    card(
        ui,
        p,
        "sparkles-filled",
        aurora_i18n::tr!("ui-constellations-show"),
        Some(Head {
            on: &mut s.enabled,
            tip: aurora_i18n::tr!("ui-constellations-show-tip"),
        }),
        |ui, on| {
            // how far the sun must have set: right under the switch it
            // gates, greyed with it
            ui.add_enabled_ui(on, |ui| {
                slider_row(
                    ui,
                    p,
                    (
                        aurora_i18n::tr!("ui-options-constellations-darkness"),
                        aurora_i18n::tr!("ui-options-constellations-darkness-help"),
                    ),
                    &format!("{:.0}°", s.sun_below_degrees),
                    None,
                    egui::Slider::new(&mut s.sun_below_degrees, DARK_RANGE),
                );
            });
            if let Some(line) = status_line(status) {
                ui.add(egui::Label::new(RichText::new(line).size(VALUE_SIZE).color(p.muted)).wrap());
            }
        },
    );
    ui.add_space(4.0);
    // the main switch off, every card is greyed, its settings kept
    ui.add_enabled_ui(s.enabled, |ui| {
        cultures_card(ui, p, s, data);
        ui.add_space(4.0);
        drawings_card(ui, p, s, data);
        ui.add_space(4.0);
        lines_card(ui, p, s, data);
        ui.add_space(4.0);
        view_card(ui, p, s);
        ui.add_space(4.0);
        effects_card(ui, p, s);
    });
    // the solar system: drawn without the constellations, so never greyed
    // by their switch
    ui.add_space(4.0);
    solar_system_card(ui, p, bodies);
    ui.add_space(4.0);
    let mut credits = false;
    ui.horizontal(|ui| {
        credits = widgets::action_button(
            ui,
            p,
            "license",
            aurora_i18n::tr!("ui-constellations-credits"),
            widgets::ActionKind::Flat,
            Vec2::new(0.0, 24.0),
        )
        .on_hover_text(aurora_i18n::tr!("ui-constellations-credits-tip"))
        .clicked();
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::action_button(
                ui,
                p,
                "restore",
                aurora_i18n::tr!("ui-constellations-reset"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, 24.0),
            )
            .on_hover_text(aurora_i18n::tr!("ui-constellations-reset-tip"))
            .clicked()
            {
                *s = ConstellationSettings::default();
                *bodies = crate::world::planets::Wanted::of(&crate::settings::Settings::default());
            }
        });
    });
    credits
}

/// The credits window's id.
pub const CREDITS_WINDOW: &str = "constellation-credits";
/// The most its list takes before it scrolls.
const CREDITS_LIST_H: f32 = 440.0;

/// "Licences and attribution": the sources of the constellation data, as
/// the folder's `credits.json` files name them (world/constellations_credits.rs),
/// one card each, under the place it came from (the general data, each
/// culture, each set, in the catalogue's order). Read when it opens and
/// after a Rescan, off the frame's thread; nothing while it is closed. A
/// link opens only when it is an `https://` address, through the chat's
/// path for the links of a text (its warning for an unknown site).
pub fn show_credits(ctx: &egui::Context, p: &Palette, open: &mut bool) {
    if !*open {
        return;
    }
    let view = crate::world::constellations::credits_view();
    let screen = ctx.content_rect();
    let pos = egui::pos2(screen.left() + 60.0 + WINDOW_W + 12.0, screen.top() + 90.0);
    Floater::new(
        CREDITS_WINDOW,
        aurora_i18n::tr!("ui-constellations-credits-title"),
        pos,
        Vec2::new(WINDOW_W, 300.0),
    )
    .auto_height()
    .help(aurora_i18n::tr!("ui-constellations-credits-help"))
    .show(ctx, p, open, |ui| credits_body(ui, p, &view, demo_credits_scroll()));
}

/// The credits window's body: `scroll` holds its list at that offset (the
/// demo's capture of a long list).
pub fn credits_body(ui: &mut egui::Ui, p: &Palette, view: &crate::world::constellations::credits::View, scroll: Option<f32>) {
    use crate::world::constellations::credits::Source;
    ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
    let groups = view.groups.as_deref().map(Vec::as_slice).unwrap_or_default();
    if groups.is_empty() {
        let text = if view.reading || view.groups.is_none() {
            aurora_i18n::tr!("ui-constellations-status-reading")
        } else {
            aurora_i18n::tr!("ui-constellations-credits-empty")
        };
        ui.add(egui::Label::new(RichText::new(text).size(VALUE_SIZE).color(p.muted)).wrap());
        return;
    }
    // scrolls once taller than its room: at most CREDITS_LIST_H, less on a
    // small viewer (the Director's rule, widgets::auto_height_body)
    let room = (super::layout::work_area(ui.ctx()).height() - 230.0).clamp(220.0, CREDITS_LIST_H);
    let mut area = widgets::auto_height_body(room);
    if let Some(offset) = scroll {
        area = area.vertical_scroll_offset(offset);
    }
    area.show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        for group in groups {
            let title = match &group.source {
                Source::General => aurora_i18n::tr!("ui-constellations-credits-general").to_owned(),
                Source::Culture(name) => aurora_i18n::tr!("ui-constellations-credits-culture", name = name.clone()),
                Source::Set(name) => aurora_i18n::tr!("ui-constellations-credits-set", name = name.clone()),
            };
            ui.label(RichText::new(title).size(VALUE_SIZE).strong().color(p.muted));
            for c in &group.credits {
                credit_card(ui, p, c);
            }
            ui.add_space(4.0);
        }
    });
}

/// One source in its card: its holder, what was taken, its licence, its
/// site, when the data was brought in, its note; a line not given is left
/// out.
fn credit_card(ui: &mut egui::Ui, p: &Palette, c: &crate::world::constellations::credits::Credit) {
    widgets::surface(ui, p, |ui| {
        ui.set_min_width(ui.available_width());
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 3.0);
        let line = |ui: &mut egui::Ui, text: &str, color| {
            if !text.is_empty() {
                ui.add(egui::Label::new(RichText::new(text).size(VALUE_SIZE).color(color)).wrap());
            }
        };
        ui.add(egui::Label::new(RichText::new(&c.holder).size(LABEL_SIZE).strong().color(p.ink)).wrap());
        line(ui, &c.work, p.ink);
        line(ui, &c.licence, p.muted);
        match c.link() {
            Some(url) => {
                let resp = ui
                    .add(
                        egui::Label::new(RichText::new(url).size(VALUE_SIZE).color(p.violet_light).underline())
                            .wrap()
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(url);
                if resp.clicked() {
                    super::chat::open_web_link(ui.ctx(), url);
                }
            }
            None => line(ui, &c.url, p.muted),
        }
        if !c.imported.is_empty() {
            line(
                ui,
                &aurora_i18n::tr!("ui-constellations-credits-imported", date = c.imported.clone()),
                p.muted,
            );
        }
        line(ui, &c.note, p.muted);
    });
}

/// The status line under the main switch: why nothing shows, or nothing
/// when something can (or the switch is off).
pub fn status_line(status: &Status) -> Option<String> {
    match status {
        Status::Showing | Status::Off => None,
        Status::Reading => Some(aurora_i18n::tr!("ui-constellations-status-reading").to_owned()),
        Status::NotRealSky => Some(aurora_i18n::tr!("ui-constellations-status-legacy-sky").to_owned()),
        Status::Daylight { below } => Some(aurora_i18n::tr!(
            "ui-constellations-status-daylight",
            degrees = format!("{below:.0}")
        )),
        Status::NoData(dir) => Some(aurora_i18n::tr!(
            "ui-options-constellations-missing",
            folder = dir.display().to_string()
        )),
    }
}

/// The drop-down's width at most: a long culture name is cut.
const CULTURE_LIST_W: f32 = 240.0;

/// "Constellation sets": the one shown, picked from a drop-down of the
/// data's cultures (the environment window's lists: the theme's combo with
/// its rows), its constellations at the right; none stored, the western
/// one. Every art set of it is used, in their order: a figure takes its
/// drawing from the first that has one. The main switch turns everything
/// off: the list has no "none".
fn cultures_card(ui: &mut egui::Ui, p: &Palette, s: &mut ConstellationSettings, data: &DataStatus) {
    use crate::world::constellations::catalogue;
    card(
        ui,
        p,
        "world-filled",
        aurora_i18n::tr!("ui-constellations-cultures"),
        None,
        |ui, _| {
            let ids: Vec<String> = data.cultures.iter().map(|c| c.id.clone()).collect();
            let shown = catalogue::shown_culture(&ids, s.cultures.as_deref());
            let Some(culture) = shown.and_then(|k| data.cultures.get(k)) else {
                return;
            };
            let tip = aurora_i18n::tr!("ui-constellations-culture-tip", count = culture.constellations);
            ui.horizontal(|ui| {
                let count = format!("{}", culture.constellations);
                let galley = ui
                    .painter()
                    .layout_no_wrap(count.clone(), egui::FontId::proportional(VALUE_SIZE), p.muted);
                let width = (ui.available_width() - galley.size().x - ui.spacing().item_spacing.x).clamp(0.0, CULTURE_LIST_W);
                let salt = "constellation-culture-list";
                // the demo's capture of the list open
                // (AURORA_DEMO_CONSTELLATIONS_WINDOW=list)
                if demo_window("list") {
                    let combo = ui.make_persistent_id(egui::IdSalt::new(salt));
                    egui::Popup::open_id(ui.ctx(), combo.with("popup"));
                }
                egui::ComboBox::from_id_salt(salt)
                    .width(width)
                    .selected_text(RichText::new(&culture.name).size(LABEL_SIZE).color(p.ink))
                    .truncate()
                    .show_ui(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                        for (k, c) in data.cultures.iter().enumerate() {
                            let on = shown == Some(k);
                            if widgets::search_list_item(ui, on, RichText::new(&c.name).size(LABEL_SIZE))
                                .on_hover_text(aurora_i18n::tr!("ui-constellations-culture-tip", count = c.constellations))
                                .clicked()
                                && !on
                            {
                                // a pick: this culture alone from now on
                                s.cultures = Some(vec![c.id.clone()]);
                            }
                        }
                    })
                    .response
                    .on_hover_text(&tip);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(count).size(VALUE_SIZE).color(p.muted)).on_hover_text(&tip);
                });
            });
            if culture.left_out > 0 {
                ui.add(
                    egui::Label::new(
                        RichText::new(aurora_i18n::tr!("ui-constellations-culture-left-out", count = culture.left_out))
                            .size(VALUE_SIZE)
                            .color(p.muted),
                    )
                    .wrap(),
                );
            }
            // the names the atlas has no room for, said (never a silent cut)
            if data.names_waiting > 0 {
                ui.add(
                    egui::Label::new(
                        RichText::new(aurora_i18n::tr!("ui-constellations-names-waiting", count = data.names_waiting))
                            .size(VALUE_SIZE)
                            .color(p.muted),
                    )
                    .wrap(),
                );
            }
        },
    );
}

/// "Engraved figures": their opacity, and the rescan of the folder at the
/// foot (every art set of the culture shown is used).
fn drawings_card(ui: &mut egui::Ui, p: &Palette, s: &mut ConstellationSettings, data: &DataStatus) {
    card(
        ui,
        p,
        "photo-filled",
        aurora_i18n::tr!("ui-constellations-figures"),
        None,
        |ui, _| {
            slider_row(
                ui,
                p,
                (
                    aurora_i18n::tr!("ui-constellations-figures-opacity"),
                    aurora_i18n::tr!("ui-constellations-figures-opacity-help"),
                ),
                &format!("{:.2}", s.drawing_opacity),
                None,
                egui::Slider::new(&mut s.drawing_opacity, OPACITY_RANGE),
            );
            // (every art set of the culture shown is used)
            if data.sets == 0 {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-constellations-sets-none"))
                        .size(VALUE_SIZE)
                        .color(p.muted),
                );
            }
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::action_button(
                        ui,
                        p,
                        "refresh",
                        aurora_i18n::tr!("ui-constellations-rescan"),
                        widgets::ActionKind::Flat,
                        Vec2::new(0.0, 22.0),
                    )
                    .on_hover_text(aurora_i18n::tr!("ui-options-constellations-rescan"))
                    .clicked()
                    {
                        crate::world::constellations::rescan();
                        crate::world::constellations::credits::invalidate();
                    }
                    if data.reading {
                        ui.label(
                            RichText::new(aurora_i18n::tr!("ui-constellations-status-reading"))
                                .size(VALUE_SIZE)
                                .color(p.muted),
                        );
                    }
                });
            });
        },
    );
}

/// A setting with a switch: its icon, its label (cut to fit) and its switch
/// at the card's right edge, like a card's own switch, whatever the
/// label's length (the weather window's row ends its label where its text
/// does, so the switch would follow the text).
fn switch_line(ui: &mut egui::Ui, p: &Palette, icon: &str, text: &str, tip: &str, on: &mut bool) -> egui::Response {
    ui.horizontal(|ui| {
        ui.set_min_height(widgets::SWITCH_SIZE.y);
        widgets::glyph(ui, icon, p.muted, 15.0);
        let width = (ui.available_width() - widgets::SWITCH_SIZE.x - ui.spacing().item_spacing.x).max(0.0);
        ui.allocate_ui_with_layout(Vec2::new(width, 18.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_width(width);
            ui.add(egui::Label::new(RichText::new(text).size(LABEL_SIZE).color(p.ink)).truncate())
                .on_hover_text(tip);
        });
        widgets::switch(ui, p, on).on_hover_text(tip)
    })
    .inner
}

/// "Lines and names": the lines and their width, the names and their
/// size (greyed when the culture shown names none of its figures), and
/// the figures no longer counted.
fn lines_card(ui: &mut egui::Ui, p: &Palette, s: &mut ConstellationSettings, data: &DataStatus) {
    use crate::world::constellations::catalogue;
    let ids: Vec<String> = data.cultures.iter().map(|c| c.id.clone()).collect();
    let named = catalogue::shown_culture(&ids, s.cultures.as_deref())
        .and_then(|k| data.cultures.get(k))
        .is_none_or(|c| c.named);
    card(
        ui,
        p,
        "chart-dots-filled",
        aurora_i18n::tr!("ui-constellations-lines-and-names"),
        None,
        |ui, _| {
            switch_line(
                ui,
                p,
                "vector",
                aurora_i18n::tr!("ui-constellations-lines"),
                aurora_i18n::tr!("ui-constellations-lines-tip"),
                &mut s.lines,
            );
            ui.add_enabled_ui(s.lines, |ui| {
                slider_row(
                    ui,
                    p,
                    (
                        aurora_i18n::tr!("ui-constellations-line-width"),
                        aurora_i18n::tr!("ui-constellations-line-width-tip"),
                    ),
                    &format!("×{:.2}", s.line_width),
                    None,
                    egui::Slider::new(&mut s.line_width, WIDTH_RANGE),
                );
            });
            ui.add_enabled_ui(named, |ui| {
                switch_line(
                    ui,
                    p,
                    "typography",
                    aurora_i18n::tr!("ui-options-constellations-names"),
                    aurora_i18n::tr!("ui-options-constellations-names-help"),
                    &mut s.names,
                );
                // their size, right under their switch, greyed with it
                ui.add_enabled_ui(s.names, |ui| {
                    slider_row(
                        ui,
                        p,
                        (
                            aurora_i18n::tr!("ui-options-constellations-font-size"),
                            aurora_i18n::tr!("ui-options-constellations-font-size-help"),
                        ),
                        &format!("{:.0} pt", s.font_size),
                        None,
                        egui::Slider::new(&mut s.font_size, FONT_RANGE),
                    );
                });
            });
            switch_line(
                ui,
                p,
                "archive-filled",
                aurora_i18n::tr!("ui-options-constellations-obsolete"),
                aurora_i18n::tr!("ui-options-constellations-obsolete-help"),
                &mut s.obsolete,
            );
        },
    );
}

/// "Solar system": the planets, their names under them while they are on,
/// then the real Sun and Moon and their hiding (world/planets.rs).
fn solar_system_card(ui: &mut egui::Ui, p: &Palette, b: &mut crate::world::planets::Wanted) {
    card(
        ui,
        p,
        "planet",
        aurora_i18n::tr!("ui-constellations-solar-system"),
        None,
        |ui, _| {
            switch_line(
                ui,
                p,
                "planet",
                aurora_i18n::tr!("ui-options-night-sky-planets"),
                aurora_i18n::tr!("ui-options-night-sky-planets-help"),
                &mut b.planets,
            );
            if b.planets {
                // their names, under their switch
                ui.indent("constellation-planet-options", |ui| {
                    switch_line(
                        ui,
                        p,
                        "typography",
                        aurora_i18n::tr!("ui-options-night-sky-planet-names"),
                        aurora_i18n::tr!("ui-options-night-sky-planet-names-help"),
                        &mut b.names,
                    );
                });
            }
            switch_line(
                ui,
                p,
                "sun-filled",
                aurora_i18n::tr!("ui-options-night-sky-sun-moon-real"),
                aurora_i18n::tr!("ui-options-night-sky-sun-moon-real-help"),
                &mut b.real,
            );
            switch_line(
                ui,
                p,
                "eye-off",
                aurora_i18n::tr!("ui-options-night-sky-sun-moon-hidden"),
                aurora_i18n::tr!("ui-options-night-sky-sun-moon-hidden-help"),
                &mut b.hidden,
            );
        },
    );
    // the demo's look at this card (AURORA_DEMO_CONSTELLATIONS_WINDOW=end)
    if demo_window("end") {
        ui.scroll_to_cursor(Some(egui::Align::Max));
    }
}

/// "Drawing and view": the times and the field (the darkness is under the
/// main switch, the names' size under theirs).
fn view_card(ui: &mut egui::Ui, p: &Palette, s: &mut ConstellationSettings) {
    card(
        ui,
        p,
        "adjustments-horizontal-filled",
        aurora_i18n::tr!("ui-constellations-view"),
        None,
        |ui, _| {
            let tr = |k: &'static str| aurora_i18n::text(k);
            slider_row(
                ui,
                p,
                (
                    tr("ui-options-constellations-draw-time"),
                    tr("ui-options-constellations-draw-time-help"),
                ),
                &format!("{:.1} s", s.draw_seconds),
                None,
                egui::Slider::new(&mut s.draw_seconds, DRAW_RANGE),
            );
            slider_row(
                ui,
                p,
                (
                    tr("ui-options-constellations-fade-time"),
                    tr("ui-options-constellations-fade-time-help"),
                ),
                &format!("{:.1} s", s.fade_seconds),
                None,
                egui::Slider::new(&mut s.fade_seconds, FADE_RANGE),
            );
            slider_row(
                ui,
                p,
                (tr("ui-options-constellations-field"), tr("ui-options-constellations-field-help")),
                &format!("{:.0}°", s.field_degrees),
                None,
                egui::Slider::new(&mut s.field_degrees, FIELD_RANGE),
            );
        },
    );
}

/// "Effects": how a figure lights up as it draws itself.
fn effects_card(ui: &mut egui::Ui, p: &Palette, s: &mut ConstellationSettings) {
    card(ui, p, "wand", aurora_i18n::tr!("ui-constellations-effects"), None, |ui, _| {
        slider_row(
            ui,
            p,
            (
                aurora_i18n::tr!("ui-constellations-brightness"),
                aurora_i18n::tr!("ui-constellations-brightness-tip"),
            ),
            &format!("×{:.2}", s.draw_brightness),
            None,
            egui::Slider::new(&mut s.draw_brightness, BRIGHTNESS_RANGE),
        );
        switch_line(
            ui,
            p,
            "bulb-filled",
            aurora_i18n::tr!("ui-constellations-highlight"),
            aurora_i18n::tr!("ui-constellations-highlight-tip"),
            &mut s.highlight,
        );
        switch_line(
            ui,
            p,
            "palette-filled",
            aurora_i18n::tr!("ui-constellations-colours"),
            aurora_i18n::tr!("ui-constellations-colours-tip"),
            &mut s.colour_variation,
        );
        switch_line(
            ui,
            p,
            "star-filled",
            aurora_i18n::tr!("ui-constellations-sparkles"),
            aurora_i18n::tr!("ui-constellations-sparkles-tip"),
            &mut s.sparkles,
        );
        if s.sparkles {
            // its size, shape and turns, under its switch
            ui.indent("constellation-sparkle-options", |ui| {
                slider_row(
                    ui,
                    p,
                    (
                        aurora_i18n::tr!("ui-constellations-sparkle-size"),
                        aurora_i18n::tr!("ui-constellations-sparkle-size-tip"),
                    ),
                    &format!("×{:.2}", s.sparkle_size),
                    None,
                    egui::Slider::new(&mut s.sparkle_size, SPARKLE_SIZE_RANGE),
                );
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-constellations-sparkle-shape"))
                        .size(LABEL_SIZE)
                        .color(p.ink),
                )
                .on_hover_text(aurora_i18n::tr!("ui-constellations-sparkle-shape-tip"));
                for shape in SparkleShape::ALL {
                    let text = match shape {
                        SparkleShape::CircleCross => aurora_i18n::tr!("ui-constellations-sparkle-circle-cross"),
                        SparkleShape::Cross => aurora_i18n::tr!("ui-constellations-sparkle-cross"),
                        SparkleShape::Star => aurora_i18n::tr!("ui-constellations-sparkle-star"),
                    };
                    widgets::radio_value(
                        ui,
                        p,
                        &mut s.sparkle_shape,
                        shape,
                        RichText::new(text).size(LABEL_SIZE).color(p.ink),
                    );
                }
                slider_row(
                    ui,
                    p,
                    (
                        aurora_i18n::tr!("ui-constellations-sparkle-spin"),
                        aurora_i18n::tr!("ui-constellations-sparkle-spin-tip"),
                    ),
                    &format!("{:.2}", s.sparkle_spin),
                    None,
                    egui::Slider::new(&mut s.sparkle_spin, SPARKLE_SPIN_RANGE),
                );
            });
        }
    });
}

/// `AURORA_DEMO_CONSTELLATIONS_WINDOW=1|end|list` (the demo only): the
/// window opens at the start (beside the weather's, for a comparison by
/// eye); `end` scrolled to its end, the Solar system card; `list` with its
/// constellation sets' drop-down open. `mode` "1": any of them.
pub fn demo_window(mode: &str) -> bool {
    static S: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let v = S.get_or_init(|| {
        let on = std::env::var_os("AURORA_DEMO").is_some();
        let v = std::env::var("AURORA_DEMO_CONSTELLATIONS_WINDOW").unwrap_or_default();
        if on { v.trim().to_owned() } else { String::new() }
    });
    if mode == "1" { !v.is_empty() && v != "0" } else { v == mode }
}

/// `AURORA_DEMO_CONSTELLATION_CREDITS=<pixels>` (the demo only): the
/// credits window opens at the start, its list scrolled that far down (0:
/// its top).
pub fn demo_credits_scroll() -> Option<f32> {
    static S: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *S.get_or_init(|| {
        std::env::var_os("AURORA_DEMO")?;
        std::env::var("AURORA_DEMO_CONSTELLATION_CREDITS")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .map(|v| v.max(0.0))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_line_names_each_reason() {
        assert_eq!(status_line(&Status::Showing), None);
        assert_eq!(status_line(&Status::Off), None);
        let lines = [
            status_line(&Status::Reading),
            status_line(&Status::NotRealSky),
            status_line(&Status::Daylight { below: 8.0 }),
            status_line(&Status::NoData("/x/constellations".into())),
        ];
        for l in &lines {
            assert!(l.as_ref().is_some_and(|l| !l.is_empty()));
        }
        assert!(lines[2].as_deref().is_some_and(|l| l.contains('8')), "{lines:?}");
        assert!(lines[3].as_deref().is_some_and(|l| l.contains("/x/constellations")), "{lines:?}");
        // four different lines
        let mut unique: Vec<&String> = lines.iter().flatten().collect();
        unique.dedup();
        assert_eq!(unique.len(), 4);
    }
}
