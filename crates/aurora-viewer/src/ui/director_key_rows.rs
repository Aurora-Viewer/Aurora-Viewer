//! The selected keyframe's own animation, lens and time of day
//! (`director_keys`), as rows of the Director's keyframe section (its tab,
//! once the window has tabs: Track). An animation is picked from the list of
//! the inventory's or dropped from the inventory; a value that is not set
//! reads "Track's value" (or "Live") with a button to give the keyframe its
//! own.
//!
//! Aurora's own: the Legacy Viewer has no camera path tool. Which
//! animations may be played is the shared rule
//! (`world::inventory::actions::animation_playable`), the one of the
//! inventory's "Play in world".

use super::director::{SLIDER_W, row};
use super::inventory::{drag, tree};
use super::widgets::{self, ActionKind};
use crate::director::{self, Director};
use crate::director_anim;
use crate::director_keys::{AnimRef, DEFAULT_HOUR, KeyExtra};
use crate::theme::Palette;
use crate::world::inventory::Inventory;
use crate::world::inventory::actions as rules;
use egui::{RichText, Vec2};
use uuid::Uuid;

/// The width of an animation's list.
const LIST_W: f32 = 168.0;

/// An animation the list offers: its item and its name.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimChoice {
    pub item: Uuid,
    pub name: String,
}

/// The animations of `inv` that may be played on our avatar, by name: the
/// list the keyframe's row offers.
pub fn anim_choices(inv: &Inventory) -> Vec<AnimChoice> {
    let mut choices: Vec<AnimChoice> = inv
        .items
        .values()
        .filter(|it| it.asset_type == 20)
        .filter_map(|it| rules::animation_playable(inv, it.id))
        .map(|it| AnimChoice {
            item: it.id,
            name: it.name.clone(),
        })
        .collect();
    choices.sort_by_cached_key(|choice| crate::natural_order::key(&choice.name));
    choices
}

/// What the rows need of the app.
pub struct KeyRows<'a> {
    pub inv: &'a Inventory,
    pub anims: &'a [AnimChoice],
}

/// The rows of the selected keyframe, under its others.
pub fn keyframe_rows(ui: &mut egui::Ui, p: &Palette, d: &mut Director, rows: &KeyRows<'_>) {
    let Some(index) = d.selected.filter(|index| *index < d.track.len()) else {
        return;
    };
    animation_rows(ui, p, d, index, rows);
    let extra = d.track.keys()[index].extra;
    let set = extra.aperture.is_some() || extra.focus.is_some() || extra.hour.is_some();
    widgets::collapsing(
        ui,
        p,
        "director_key_lens_time",
        aurora_i18n::tr!("director-key-lens-time"),
        set,
        |ui| {
            lens_time_rows(ui, p, d, index);
        },
    );
}

/// The animation's name as the list shows it, and whether its item is gone.
fn anim_label(anim: Option<AnimRef>, inv: &Inventory) -> (String, bool) {
    match anim {
        None => (aurora_i18n::tr!("director-key-anim-none").to_owned(), false),
        Some(anim) if rules::animation_playable(inv, anim.item).is_none() => {
            (aurora_i18n::tr!("director-key-anim-missing", name = anim.name.as_str()), true)
        }
        Some(anim) => (anim.name.as_str().to_owned(), false),
    }
}

fn animation_rows(ui: &mut egui::Ui, p: &Palette, d: &mut Director, index: usize, rows: &KeyRows<'_>) {
    let extra = d.track.keys()[index].extra;
    let (text, missing) = anim_label(extra.start, rows.inv);
    let mut chosen: Option<Option<AnimRef>> = None;
    let list = row(ui, p, aurora_i18n::tr!("director-key-anim"), |ui| {
        let combo = egui::ComboBox::from_id_salt("director_key_anim")
            .width(LIST_W)
            .selected_text(RichText::new(text).color(if missing { p.warn } else { p.ink }))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(extra.start.is_none(), aurora_i18n::tr!("director-key-anim-none"))
                    .clicked()
                {
                    chosen = Some(None);
                }
                for choice in rows.anims {
                    let on = extra.start.is_some_and(|anim| anim.item == choice.item);
                    if ui.selectable_label(on, &choice.name).clicked() {
                        chosen = Some(Some(AnimRef::new(choice.item, &choice.name)));
                    }
                }
            });
        let tip = if missing {
            aurora_i18n::tr!("director-key-anim-missing-tip")
        } else {
            aurora_i18n::tr!("director-key-anim-tip")
        };
        let rect = combo.response.on_hover_text(tip).rect;
        if extra.start.is_some() && widgets::icon_button(ui, p, "x", aurora_i18n::tr!("director-key-anim-clear"), true) {
            chosen = Some(None);
        }
        rect
    });
    // an animation dragged from the inventory onto the list
    if ui.rect_contains_pointer(list)
        && let Some(ids) = drag::carried_ids(ui.ctx())
    {
        let found = match ids[..] {
            [id] => rules::animation_playable(rows.inv, id).map(|it| AnimRef::new(it.id, &it.name)),
            _ => None,
        };
        tree::drop_outline(ui, p, list, found.is_some());
        let cursor = if found.is_some() {
            egui::CursorIcon::Copy
        } else {
            egui::CursorIcon::NotAllowed
        };
        drag::feedback(ui.ctx(), cursor);
        if ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary)) {
            egui::DragAndDrop::clear_payload(ui.ctx());
            if found.is_some() {
                chosen = Some(found);
            }
        }
    }
    if let Some(anim) = chosen
        && anim != extra.start
    {
        // the stops of the animation it replaces go with it
        director_anim::set_stop(&mut d.track, index, None);
        d.track.set_extra(index, KeyExtra { start: anim, ..extra });
    }
    if extra.start.is_some() {
        until_row(ui, p, d, index);
    }
    let extra = d.track.keys()[index].extra;
    if let Some(stop) = extra.stop {
        row(ui, p, aurora_i18n::tr!("director-key-anim-stops"), |ui| {
            let (text, missing) = anim_label(Some(stop), rows.inv);
            ui.label(RichText::new(text).size(12.5).color(if missing { p.warn } else { p.ink }))
                .on_hover_text(aurora_i18n::tr!("director-key-anim-stops-tip"));
            if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("director-key-anim-stops-clear"), true) {
                d.track.set_extra(index, KeyExtra { stop: None, ..extra });
            }
        });
    }
}

/// Where the keyframe's animation stops: a later keyframe, or the end.
fn until_row(ui: &mut egui::Ui, p: &Palette, d: &mut Director, index: usize) {
    let until = director_anim::stops_at(&d.track, index);
    let label = |at: Option<usize>| match at {
        Some(later) => aurora_i18n::tr!("director-keyframe", number = later + 1),
        None => aurora_i18n::tr!("director-key-anim-end").to_owned(),
    };
    let mut chosen = None;
    row(ui, p, aurora_i18n::tr!("director-key-anim-until"), |ui| {
        egui::ComboBox::from_id_salt("director_key_anim_until")
            .width(LIST_W)
            .selected_text(label(until))
            .show_ui(ui, |ui| {
                for at in std::iter::once(None).chain((index + 1..d.track.len()).map(Some)) {
                    if ui.selectable_label(at == until, label(at)).clicked() {
                        chosen = Some(at);
                    }
                }
            })
            .response
            .on_hover_text(aurora_i18n::tr!("director-key-anim-until-tip"));
    });
    if let Some(at) = chosen
        && at != until
    {
        director_anim::set_stop(&mut d.track, index, at);
    }
}

/// "Track's value" (or "Live") and a button that gives the keyframe its own
/// value; true when it is pressed.
fn unset(ui: &mut egui::Ui, p: &Palette, text: &str) -> bool {
    ui.label(RichText::new(text).size(12.5).color(p.muted));
    widgets::action_button(
        ui,
        p,
        "pencil-filled",
        aurora_i18n::tr!("director-key-set"),
        ActionKind::Flat,
        Vec2::new(0.0, 20.0),
    )
    .on_hover_text(aurora_i18n::tr!("director-key-set-tip"))
    .clicked()
}

/// The button that takes the keyframe's own value away; true when pressed.
fn reset(ui: &mut egui::Ui, p: &Palette, tip: &str) -> bool {
    widgets::icon_button(ui, p, "x", tip, true)
}

/// "hh:mm" for an hour of the day.
fn hour_text(hour: f64) -> String {
    let minutes = ((hour * 60.0).round() as i64).rem_euclid(24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// An hour typed as "hh:mm", or as a number of hours.
fn parse_hour(text: &str) -> Option<f64> {
    match text.trim().split_once(':') {
        Some((h, m)) => {
            let (h, m) = (h.trim().parse::<f64>().ok()?, m.trim().parse::<f64>().ok()?);
            (h.is_finite() && (0.0..60.0).contains(&m)).then_some(h + m / 60.0)
        }
        None => aurora_i18n::parse_number(text),
    }
}

fn lens_time_rows(ui: &mut egui::Ui, p: &Palette, d: &mut Director, index: usize) {
    let mut extra = d.track.keys()[index].extra;
    let back = aurora_i18n::tr!("director-key-unset");
    row(ui, p, aurora_i18n::tr!("director-key-aperture"), |ui| match &mut extra.aperture {
        None => {
            if unset(ui, p, aurora_i18n::tr!("director-key-track-value")) {
                extra.aperture = Some(d.dof.unwrap_or(director::DEFAULT_APERTURE));
            }
        }
        Some(aperture) => {
            ui.spacing_mut().slider_width = SLIDER_W;
            let slider = egui::Slider::new(aperture, director::APERTURE_RANGE)
                .custom_parser(aurora_i18n::parse_number)
                .fixed_decimals(1)
                .prefix("f/")
                .logarithmic(true);
            ui.add(slider).on_hover_text(aurora_i18n::tr!("director-key-aperture-tip"));
            if reset(ui, p, back) {
                extra.aperture = None;
            }
        }
    });
    row(ui, p, aurora_i18n::tr!("director-key-focus"), |ui| match &mut extra.focus {
        None => {
            if unset(ui, p, aurora_i18n::tr!("director-key-track-value")) {
                extra.focus = Some(d.focus);
            }
        }
        Some(focus) => {
            ui.spacing_mut().slider_width = SLIDER_W;
            let slider = egui::Slider::new(focus, director::FOCUS_RANGE)
                .custom_parser(aurora_i18n::parse_number)
                .fixed_decimals(1)
                .suffix(" m")
                .logarithmic(true);
            ui.add(slider).on_hover_text(aurora_i18n::tr!("director-key-focus-tip"));
            if reset(ui, p, back) {
                extra.focus = None;
            }
        }
    });
    row(ui, p, aurora_i18n::tr!("director-key-hour"), |ui| match &mut extra.hour {
        None => {
            if unset(ui, p, aurora_i18n::tr!("director-key-live")) {
                extra.hour = Some(DEFAULT_HOUR);
            }
        }
        Some(hour) => {
            ui.spacing_mut().slider_width = SLIDER_W;
            let slider = egui::Slider::new(hour, 0.0..=24.0)
                .custom_formatter(|value, _| hour_text(value))
                .custom_parser(parse_hour)
                .trailing_fill(true);
            ui.add(slider).on_hover_text(aurora_i18n::tr!("director-key-hour-tip"));
            if reset(ui, p, aurora_i18n::tr!("director-key-unset-live")) {
                extra.hour = None;
            }
        }
    });
    if extra != d.track.keys()[index].extra {
        d.track.set_extra(index, extra);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::inventory::InvItem;

    #[test]
    fn an_hour_reads_and_writes_as_a_clock() {
        assert_eq!(hour_text(0.0), "00:00");
        assert_eq!(hour_text(18.5), "18:30");
        assert_eq!(hour_text(23.999), "00:00");
        assert_eq!(parse_hour("07:45"), Some(7.75));
        assert_eq!(parse_hour(" 7 : 05 "), Some(7.0 + 5.0 / 60.0));
        assert_eq!(parse_hour("7:75"), None);
        assert_eq!(parse_hour("x:10"), None);
        assert_eq!(parse_hour("13"), Some(13.0));
    }

    #[test]
    fn the_list_offers_the_animations_that_play_by_name() {
        let agent = Uuid::from_u128(0xA);
        let mut inv = Inventory {
            owner: agent,
            ..Default::default()
        };
        for (n, name, asset_type, owner) in [
            (1, "Wave 10", 20, agent),
            (2, "Wave 9", 20, agent),
            (3, "A texture", 0, agent),
            (4, "Not mine", 20, Uuid::from_u128(0xB)),
        ] {
            let id = Uuid::from_u128(n);
            inv.items.insert(
                id,
                InvItem {
                    id,
                    name: name.into(),
                    asset_type,
                    asset_id: Uuid::from_u128(n + 100),
                    owner,
                    ..Default::default()
                },
            );
        }
        let names: Vec<String> = anim_choices(&inv).into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["Wave 9", "Wave 10"]);
        // a chosen animation no longer there reads as missing
        let gone = AnimRef::new(Uuid::from_u128(77), "Old dance");
        assert!(anim_label(Some(gone), &inv).1);
        assert!(!anim_label(Some(AnimRef::new(Uuid::from_u128(1), "Wave 10")), &inv).1);
        assert!(!anim_label(None, &inv).1);
    }
}
