//! The Director's shots in its window (`crate::director::shots`): the
//! "Shots" section, which lays a camera move out from the current view in
//! one click (its main parameters shown, the rest under "More options"),
//! and the selected keyframe's rows for the shots: the easing of the way
//! to it, its dolly zoom, and whether it travels with its target.

use super::{Action, Live, SLIDER_W, row};
use crate::camera::track;
use crate::director::Director;
use crate::director::shots::{self, ARC_DEGREES, Ease, KeyShot, Kind, Made, Refused, Shot};
use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind};
use egui::{Id, RichText, Vec2};
use std::ops::RangeInclusive;

/// What the section keeps between frames: the shot chosen, each shot's
/// parameters, where it goes, and what the last one laid.
#[derive(Clone)]
struct Choice {
    kind: Kind,
    shots: [Shot; 7],
    replace: bool,
    travel: bool,
    /// The last shot laid: its keyframes and seconds.
    laid: Option<(usize, f32)>,
}

impl Default for Choice {
    fn default() -> Self {
        Choice {
            kind: Kind::Orbit,
            shots: Kind::ALL.map(Shot::new),
            replace: false,
            travel: true,
            laid: None,
        }
    }
}

impl Choice {
    fn slot(&self) -> usize {
        Kind::ALL.iter().position(|kind| *kind == self.kind).unwrap_or(0)
    }

    /// The chosen shot as it is laid.
    fn shot(&self) -> Shot {
        Shot {
            replace: self.replace,
            travel: self.travel,
            ..self.shots[self.slot()]
        }
    }
}

fn choice_id() -> Id {
    Id::unique("director_shots_choice")
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Orbit => aurora_i18n::tr!("director-shot-orbit"),
        Kind::DollyZoom => aurora_i18n::tr!("director-shot-dolly-zoom"),
        Kind::Crane => aurora_i18n::tr!("director-shot-crane"),
        Kind::Truck => aurora_i18n::tr!("director-shot-truck"),
        Kind::PushIn => aurora_i18n::tr!("director-shot-push-in"),
        Kind::PullOut => aurora_i18n::tr!("director-shot-pull-out"),
        Kind::FlyOver => aurora_i18n::tr!("director-shot-fly-over"),
    }
}

fn kind_tip(kind: Kind) -> &'static str {
    match kind {
        Kind::Orbit => aurora_i18n::tr!("director-shot-orbit-tip"),
        Kind::DollyZoom => aurora_i18n::tr!("director-shot-dolly-zoom-tip"),
        Kind::Crane => aurora_i18n::tr!("director-shot-crane-tip"),
        Kind::Truck => aurora_i18n::tr!("director-shot-truck-tip"),
        Kind::PushIn => aurora_i18n::tr!("director-shot-push-in-tip"),
        Kind::PullOut => aurora_i18n::tr!("director-shot-pull-out-tip"),
        Kind::FlyOver => aurora_i18n::tr!("director-shot-fly-over-tip"),
    }
}

fn ease_label(ease: Ease) -> &'static str {
    match ease {
        Ease::Linear => aurora_i18n::tr!("director-ease-linear"),
        Ease::In => aurora_i18n::tr!("director-ease-in"),
        Ease::Out => aurora_i18n::tr!("director-ease-out"),
        Ease::InOut => aurora_i18n::tr!("director-ease-in-out"),
    }
}

/// A field of metres or seconds, with its tooltip; true when it changed.
fn amount(ui: &mut egui::Ui, value: &mut f32, range: RangeInclusive<f32>, suffix: &str, tip: &str) -> bool {
    let field = egui::DragValue::new(value)
        .speed(0.05)
        .range(range)
        .fixed_decimals(1)
        .custom_parser(aurora_i18n::parse_number)
        .suffix(suffix);
    ui.add(field).on_hover_text(tip).changed()
}

/// A value of the orbit that is the camera's own unless set: "Live" with a
/// button that sets it, or the field with a button back to live.
fn live_or_set(ui: &mut egui::Ui, p: &Palette, value: &mut Option<f32>, live: f32, range: RangeInclusive<f32>, tip: &str) {
    let live = live.clamp(*range.start(), *range.end());
    match value {
        None => {
            let text = aurora_i18n::tr!("director-shot-live", value = aurora_i18n::number(f64::from(live), 1));
            ui.label(RichText::new(text).size(12.5).color(p.muted)).on_hover_text(tip);
            if widgets::icon_button(ui, p, "pencil-filled", aurora_i18n::tr!("director-shot-set"), true) {
                *value = Some(live);
            }
        }
        Some(set) => {
            amount(ui, set, range, " m", tip);
            if widgets::icon_button(ui, p, "refresh", aurora_i18n::tr!("director-shot-reset-live"), true) {
                *value = None;
            }
        }
    }
}

/// The section (the window's Camera side): the shot, its main parameters,
/// where it goes, and "Make the shot", which lays its keyframes as one
/// undo step; the rest under "More options". Its defaults give a good
/// shot at once: one turn in 10 s round the lock's target, from where the
/// camera is.
pub(super) fn shots_section(ui: &mut egui::Ui, p: &Palette, d: &mut Director, live: &Live) {
    let mut choice: Choice = ui.ctx().data(|data| data.get_temp(choice_id())).unwrap_or_default();
    let here = &live.shots;
    row(ui, p, aurora_i18n::tr!("director-shot"), |ui| {
        egui::ComboBox::from_id_salt("director_shot_kind")
            .selected_text(kind_label(choice.kind))
            .show_ui(ui, |ui| {
                for kind in Kind::ALL {
                    ui.selectable_value(&mut choice.kind, kind, kind_label(kind))
                        .on_hover_text(kind_tip(kind));
                }
            })
            .response
            .on_hover_text(kind_tip(choice.kind));
    });
    let slot = choice.slot();
    let shot = &mut choice.shots[slot];
    key_parameters(ui, p, shot);
    row(ui, p, aurora_i18n::tr!("director-shot-duration"), |ui| {
        amount(
            ui,
            &mut shot.duration,
            shots::DURATION_RANGE,
            " s",
            aurora_i18n::tr!("director-shot-duration-tip"),
        );
    });
    row(ui, p, aurora_i18n::tr!("director-shot-around"), |ui| {
        let centre = if here.on_target {
            aurora_i18n::tr!("director-shot-around-target")
        } else {
            aurora_i18n::tr!("director-shot-around-focus")
        };
        ui.label(RichText::new(centre).size(12.5).color(p.ink))
            .on_hover_text(aurora_i18n::tr!("director-shot-around-tip"));
    });
    let mut place = usize::from(choice.replace);
    let options = [
        (aurora_i18n::tr!("director-shot-add"), true),
        (aurora_i18n::tr!("director-shot-replace"), true),
    ];
    if widgets::segmented(ui, p, &mut place, &options) {
        choice.replace = place == 1;
    }
    let wanted = choice.shot();
    let made = shots::make(&wanted, here);
    let ready: Result<&Made, &'static str> = match &made {
        None => Err(aurora_i18n::tr!("director-shot-no-room")),
        Some(made) => match shots::room(d, &wanted, made) {
            Ok(_) => Ok(made),
            Err(Refused::Full) => Err(aurora_i18n::tr!("director-shot-full")),
            Err(Refused::TooLong) => Err(aurora_i18n::tr!("director-shot-too-long")),
        },
    };
    let tip = ready.err().unwrap_or(aurora_i18n::tr!("director-shot-make-tip"));
    if widgets::action_button_if(
        ui,
        p,
        "sparkles-filled",
        aurora_i18n::tr!("director-shot-make"),
        ActionKind::Primary,
        Vec2::new(0.0, 26.0),
        ready.is_ok(),
    )
    .on_hover_text(tip)
    .on_disabled_hover_text(tip)
    .clicked()
        && let Ok(made) = ready
    {
        let anchor = here.on_target.then_some(here.centre);
        if shots::lay(d, &wanted, made, anchor).is_ok() {
            choice.laid = Some((made.keys.len(), made.keys.last().map_or(0.0, |key| key.time)));
        }
    }
    if let Some((count, seconds)) = choice.laid {
        let note = aurora_i18n::tr!(
            "director-shot-made",
            count = count,
            seconds = aurora_i18n::number(f64::from(seconds), 1)
        );
        ui.label(RichText::new(note).size(12.0).color(p.muted));
    }
    widgets::collapsing(ui, p, "director_shots_more", aurora_i18n::tr!("director-shot-more"), false, |ui| {
        more_options(ui, p, &mut choice, live);
    });
    ui.ctx().data_mut(|data| data.insert_temp(choice_id(), choice));
}

/// The one to three parameters a shot shows first.
fn key_parameters(ui: &mut egui::Ui, p: &Palette, shot: &mut Shot) {
    let distance = |ui: &mut egui::Ui, shot: &mut Shot, label: &str, tip: &str| {
        row(ui, p, label, |ui| {
            amount(ui, &mut shot.distance, shots::DISTANCE_RANGE, " m", tip);
        });
    };
    let side = |ui: &mut egui::Ui, shot: &mut Shot, labels: [&str; 2]| {
        row(ui, p, aurora_i18n::tr!("director-shot-direction"), |ui| {
            let mut chosen = usize::from(shot.reverse);
            let options = [(labels[0], true), (labels[1], true)];
            if widgets::segmented(ui, p, &mut chosen, &options) {
                shot.reverse = chosen == 1;
            }
        });
    };
    match shot.kind {
        Kind::Orbit => {
            row(ui, p, aurora_i18n::tr!("director-shot-arc"), |ui| {
                let mut shown = shot.arc.to_degrees().clamp(*ARC_DEGREES.start(), *ARC_DEGREES.end());
                ui.spacing_mut().slider_width = SLIDER_W;
                let slider = egui::Slider::new(&mut shown, ARC_DEGREES)
                    .custom_parser(aurora_i18n::parse_number)
                    .fixed_decimals(0)
                    .suffix("°")
                    .trailing_fill(true);
                if ui.add(slider).on_hover_text(aurora_i18n::tr!("director-shot-arc-tip")).changed() {
                    shot.arc = shown.to_radians();
                }
            });
        }
        Kind::DollyZoom => {
            side(
                ui,
                shot,
                [aurora_i18n::tr!("director-shot-closer"), aurora_i18n::tr!("director-shot-farther")],
            );
            distance(
                ui,
                shot,
                aurora_i18n::tr!("director-shot-distance"),
                aurora_i18n::tr!("director-shot-distance-tip"),
            );
        }
        Kind::Crane => distance(
            ui,
            shot,
            aurora_i18n::tr!("director-shot-rise"),
            aurora_i18n::tr!("director-shot-rise-tip"),
        ),
        Kind::Truck => {
            side(
                ui,
                shot,
                [aurora_i18n::tr!("director-shot-right"), aurora_i18n::tr!("director-shot-left")],
            );
            distance(
                ui,
                shot,
                aurora_i18n::tr!("director-shot-distance"),
                aurora_i18n::tr!("director-shot-distance-tip"),
            );
        }
        Kind::PushIn | Kind::PullOut => distance(
            ui,
            shot,
            aurora_i18n::tr!("director-shot-distance"),
            aurora_i18n::tr!("director-shot-distance-tip"),
        ),
        Kind::FlyOver => distance(
            ui,
            shot,
            aurora_i18n::tr!("director-shot-over"),
            aurora_i18n::tr!("director-shot-over-tip"),
        ),
    }
}

/// What refines a shot: the orbit's radius, height, climb and direction,
/// and whether the keyframes travel with the lock's target.
fn more_options(ui: &mut egui::Ui, p: &Palette, choice: &mut Choice, live: &Live) {
    let here = &live.shots;
    if choice.kind == Kind::Orbit {
        let rel = here.view.position - here.centre;
        let slot = choice.slot();
        let orbit = &mut choice.shots[slot];
        row(ui, p, aurora_i18n::tr!("director-shot-radius"), |ui| {
            let tip = aurora_i18n::tr!("director-shot-radius-tip");
            live_or_set(ui, p, &mut orbit.radius, rel.truncate().length(), shots::RADIUS_RANGE, tip);
        });
        row(ui, p, aurora_i18n::tr!("director-shot-height"), |ui| {
            let tip = aurora_i18n::tr!("director-shot-height-tip");
            live_or_set(ui, p, &mut orbit.height, rel.z, shots::HEIGHT_RANGE, tip);
        });
        row(ui, p, aurora_i18n::tr!("director-shot-climb"), |ui| {
            amount(
                ui,
                &mut orbit.rise,
                shots::RISE_RANGE,
                " m",
                aurora_i18n::tr!("director-shot-climb-tip"),
            );
        });
        row(ui, p, aurora_i18n::tr!("director-shot-direction"), |ui| {
            let mut chosen = usize::from(orbit.clockwise);
            let options = [
                (aurora_i18n::tr!("director-shot-ccw"), true),
                (aurora_i18n::tr!("director-shot-cw"), true),
            ];
            if widgets::segmented(ui, p, &mut chosen, &options) {
                orbit.clockwise = chosen == 1;
            }
        });
    }
    let why = if here.on_target {
        aurora_i18n::tr!("director-shot-travel-tip")
    } else {
        aurora_i18n::tr!("director-shot-travel-off")
    };
    ui.add_enabled_ui(here.on_target, |ui| {
        widgets::checkbox(ui, p, &mut choice.travel, aurora_i18n::tr!("director-shot-travel"))
            .on_hover_text(why)
            .on_disabled_hover_text(why);
    });
}

/// The selected keyframe's rows for the shots (the window's Track side):
/// the easing of the way to it and its dolly zoom (not on the first, which
/// no way leads to), and whether it travels with its target.
pub(super) fn keyframe_rows(ui: &mut egui::Ui, p: &Palette, d: &mut Director, index: usize, actions: &mut Vec<Action>) {
    let Some(key) = d.track.keys().get(index).copied() else {
        return;
    };
    if index > 0 {
        row(ui, p, aurora_i18n::tr!("director-key-ease"), |ui| {
            let mut ease = key.shot.ease;
            egui::ComboBox::from_id_salt("director_key_ease")
                .selected_text(ease_label(ease))
                .show_ui(ui, |ui| {
                    for choice in Ease::ALL {
                        ui.selectable_value(&mut ease, choice, ease_label(choice));
                    }
                })
                .response
                .on_hover_text(aurora_i18n::tr!("director-key-ease-tip"));
            if ease != key.shot.ease {
                d.track.set_shot(index, KeyShot { ease, ..key.shot });
            }
        });
        let mut dolly = key.shot.dolly_zoom;
        let moves = d.track.span_length(index - 1) >= track::MIN_TRAVEL;
        let tip = if moves {
            aurora_i18n::tr!("director-key-dolly-zoom-tip")
        } else {
            aurora_i18n::tr!("director-span-speed-still")
        };
        ui.add_enabled_ui(moves || dolly, |ui| {
            if widgets::checkbox(ui, p, &mut dolly, aurora_i18n::tr!("director-key-dolly-zoom"))
                .on_hover_text(tip)
                .on_disabled_hover_text(tip)
                .changed()
            {
                d.track.set_shot(
                    index,
                    KeyShot {
                        dolly_zoom: dolly,
                        ..key.shot
                    },
                );
            }
        });
    }
    let mut travel = key.shot.anchor.is_some();
    let has_target = d.key_aim(index).is_some();
    let tip = if has_target {
        aurora_i18n::tr!("director-key-travel-tip")
    } else {
        aurora_i18n::tr!("director-key-travel-none")
    };
    ui.add_enabled_ui(has_target || travel, |ui| {
        if widgets::checkbox(ui, p, &mut travel, aurora_i18n::tr!("director-key-travel"))
            .on_hover_text(tip)
            .on_disabled_hover_text(tip)
            .changed()
        {
            actions.push(Action::KeyTravel(index, travel));
        }
    });
}
