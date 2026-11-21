//! The Director window's fades and titles (`director_titles.rs`): two rows
//! for the fades from and to black, and the title cards as a short list,
//! each opened in place to be edited. Sections of the Look tab, called from
//! `ui::director::window`.

use super::widgets;
use crate::director_titles::{self, Finish, Place, Title, When};
use crate::theme::Palette;
use egui::{Align2, FontId, Id, RichText, Sense, Vec2};

/// The label column, as the window's other rows.
const LABEL_W: f32 = 112.0;
const SLIDER_W: f32 = 120.0;

fn row<R>(ui: &mut egui::Ui, p: &Palette, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal_wrapped(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(LABEL_W, 20.0), Sense::hover());
        ui.painter()
            .text(rect.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(12.5), p.ink);
        add(ui)
    })
    .inner
}

fn seconds_slider(ui: &mut egui::Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, tip: &str) -> bool {
    ui.spacing_mut().slider_width = SLIDER_W;
    let slider = egui::Slider::new(value, range)
        .custom_parser(aurora_i18n::parse_number)
        .fixed_decimals(1)
        .suffix(" s")
        .trailing_fill(true);
    ui.add(slider).on_hover_text(tip).changed()
}

/// The fades: from black at the start of the track, to black at its end,
/// each its seconds (0: none).
pub fn fades_section(ui: &mut egui::Ui, p: &Palette, finish: &mut Finish) {
    row(ui, p, aurora_i18n::tr!("director-fade-in"), |ui| {
        seconds_slider(
            ui,
            &mut finish.fade_in,
            director_titles::FADE_RANGE,
            aurora_i18n::tr!("director-fade-in-tip"),
        );
    });
    row(ui, p, aurora_i18n::tr!("director-fade-out"), |ui| {
        seconds_slider(
            ui,
            &mut finish.fade_out,
            director_titles::FADE_RANGE,
            aurora_i18n::tr!("director-fade-out-tip"),
        );
    });
}

/// When a title shows, short: "Start, 3 s", "End, 3 s", "0:02–0:05".
fn when_text(title: &Title, duration: f32) -> String {
    let (from, to) = title.span(duration);
    match title.when {
        When::Start => aurora_i18n::tr!(
            "director-title-at-start",
            seconds = aurora_i18n::number_text(&format!("{:.1}", to - from))
        ),
        When::End => aurora_i18n::tr!(
            "director-title-at-end",
            seconds = aurora_i18n::number_text(&format!("{:.1}", to - from))
        ),
        When::Range => aurora_i18n::tr!(
            "director-title-range",
            from = aurora_i18n::number_text(&format!("{from:.1}")),
            to = aurora_i18n::number_text(&format!("{to:.1}"))
        ),
    }
}

/// The titles: one line each (its text, when it shows), opened in place to
/// be edited, and the button that adds one. `duration`: the track's.
pub fn titles_section(ui: &mut egui::Ui, p: &Palette, finish: &mut Finish, duration: f32) {
    let open_id = Id::unique("director_title_open");
    let mut open: Option<usize> = ui.ctx().data(|d| d.get_temp(open_id)).flatten();
    if finish.titles.is_empty() {
        ui.label(RichText::new(aurora_i18n::tr!("director-titles-none")).size(12.0).color(p.muted));
    }
    let mut remove = None;
    for (index, title) in finish.titles.iter_mut().enumerate() {
        ui.push_id(("director_title", index), |ui| {
            ui.horizontal(|ui| {
                let text = title.lines().into_iter().find(|line| !line.is_empty());
                let shown = text.unwrap_or_else(|| aurora_i18n::tr!("director-title-empty").to_owned());
                let room = (ui.available_width() - 150.0).max(60.0);
                let label = ui
                    .add_sized(
                        Vec2::new(room, 20.0),
                        egui::Label::new(RichText::new(shown).size(12.5).color(p.ink))
                            .truncate()
                            .sense(Sense::click()),
                    )
                    .on_hover_text(aurora_i18n::tr!("director-title-edit-tip"));
                if label.clicked() {
                    open = if open == Some(index) { None } else { Some(index) };
                }
                ui.label(RichText::new(when_text(title, duration)).size(12.0).color(p.muted));
                if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("director-title-remove"), true) {
                    remove = Some(index);
                }
            });
            if open == Some(index) {
                title_editor(ui, p, title, duration);
            }
        });
    }
    if let Some(index) = remove {
        finish.titles.remove(index);
        open = None;
    }
    let room = finish.titles.len() < director_titles::MAX_TITLES;
    let add = widgets::action_button_if(
        ui,
        p,
        "circle-plus-filled",
        aurora_i18n::tr!("director-title-add"),
        widgets::ActionKind::Flat,
        Vec2::new(0.0, 26.0),
        room,
    )
    .on_hover_text(aurora_i18n::tr!("director-title-add-tip"))
    .on_disabled_hover_text(aurora_i18n::tr!("director-title-full"));
    if add.clicked() {
        finish.titles.push(Title {
            text: aurora_i18n::tr!("director-title-default").to_owned(),
            ..Default::default()
        });
        open = Some(finish.titles.len() - 1);
    }
    ui.ctx().data_mut(|d| d.insert_temp(open_id, open));
}

/// One title's fields: its text, when, where, size, color and fade.
fn title_editor(ui: &mut egui::Ui, p: &Palette, title: &mut Title, duration: f32) {
    ui.add(
        egui::TextEdit::multiline(&mut title.text)
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .char_limit(director_titles::MAX_TEXT)
            .hint_text(aurora_i18n::tr!("director-title-text-hint")),
    )
    .on_hover_text(aurora_i18n::tr!("director-title-text-tip"));
    row(ui, p, aurora_i18n::tr!("director-title-when"), |ui| {
        let whens = [When::Start, When::End, When::Range];
        let mut at = whens.iter().position(|w| *w == title.when).unwrap_or(0);
        let names = [
            (aurora_i18n::tr!("director-title-when-start"), true),
            (aurora_i18n::tr!("director-title-when-end"), true),
            (aurora_i18n::tr!("director-title-when-range"), true),
        ];
        if widgets::segmented(ui, p, &mut at, &names) {
            title.when = whens[at];
            if title.when == When::Range {
                // the range starts where it showed
                let (from, to) = title.span(duration);
                (title.from, title.to) = (from, to.max(from + 0.1));
            }
        }
    });
    let end = duration.max(0.1);
    match title.when {
        When::Start | When::End => {
            row(ui, p, aurora_i18n::tr!("director-title-length"), |ui| {
                seconds_slider(
                    ui,
                    &mut title.length,
                    0.1..=end.max(1.0),
                    aurora_i18n::tr!("director-title-length-tip"),
                );
            });
        }
        When::Range => {
            row(ui, p, aurora_i18n::tr!("director-title-from"), |ui| {
                seconds_slider(ui, &mut title.from, 0.0..=end, aurora_i18n::tr!("director-title-from-tip"));
            });
            row(ui, p, aurora_i18n::tr!("director-title-to"), |ui| {
                seconds_slider(ui, &mut title.to, 0.0..=end, aurora_i18n::tr!("director-title-to-tip"));
            });
        }
    }
    row(ui, p, aurora_i18n::tr!("director-title-place"), |ui| {
        let places = [Place::Centre, Place::LowerThird];
        let mut at = places.iter().position(|w| *w == title.place).unwrap_or(0);
        let names = [
            (aurora_i18n::tr!("director-title-centre"), true),
            (aurora_i18n::tr!("director-title-lower-third"), true),
        ];
        if widgets::segmented(ui, p, &mut at, &names) {
            title.place = places[at];
        }
    });
    row(ui, p, aurora_i18n::tr!("director-title-size"), |ui| {
        ui.spacing_mut().slider_width = SLIDER_W;
        let (low, high) = (*director_titles::SIZE_RANGE.start(), *director_titles::SIZE_RANGE.end());
        // rounded first: egui's slider would call its own rounding a change
        let mut percent = (title.size() * 100.0).round();
        let slider = egui::Slider::new(&mut percent, low * 100.0..=high * 100.0)
            .custom_parser(aurora_i18n::parse_number)
            .fixed_decimals(0)
            .suffix(" %")
            .trailing_fill(true);
        if ui.add(slider).on_hover_text(aurora_i18n::tr!("director-title-size-tip")).changed() {
            title.size = percent / 100.0;
        }
    });
    row(ui, p, aurora_i18n::tr!("director-title-color"), |ui| {
        let [r, g, b] = title.color;
        let mut color = [r, g, b, 255];
        let swatch = widgets::color_button(ui, p, "director_title_color", &mut color, false);
        if swatch.on_hover_text(aurora_i18n::tr!("director-title-color-tip")).changed() {
            title.color = [color[0], color[1], color[2]];
        }
    });
    row(ui, p, aurora_i18n::tr!("director-title-fade"), |ui| {
        seconds_slider(
            ui,
            &mut title.fade,
            director_titles::FADE_RANGE,
            aurora_i18n::tr!("director-title-fade-tip"),
        );
    });
}
