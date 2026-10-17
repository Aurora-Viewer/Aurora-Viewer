//! "Environment" (LLPanelLandEnvironment / LLPanelEnvironmentInfo:
//! setControlsEnabled, refresh, day length and offset committed on mouse up,
//! "Use region settings" = resetParcel, apparent time, sky
//! altitudes shown read-only for a parcel).

use super::dialogs::Confirm;
use super::{Dialog, LandUi, View, dim, text};
use crate::theme::Palette;
use crate::world::World;
use crate::world::land::{EnvState, EnvSummary, MIN_PARCEL_AREA, powers};
use aurora_net::land::LandCommand;
use egui::{RichText, Vec2};

/// Altitude slider range (min_val / max_val of sld_altitudes).
const ALT_MIN: f32 = 100.0;
const ALT_MAX: f32 = 4000.0;

/// setControlsEnabled: why the panel is unavailable, in Firestorm's order.
pub(super) fn unavailable(v: &View, env: &EnvState) -> Option<&'static str> {
    match (&v.parcel, env) {
        (_, EnvState::Failed(e)) => {
            log::debug!("parcel environment unavailable: {e}");
            Some(aurora_i18n::tr!(
                "ui-land-environment-environment-settings-are-unavailable-in-this-region"
            ))
        }
        (None, _) => Some(aurora_i18n::tr!(
            "ui-land-environment-no-parcel-is-selected-environment-settings-are-disabled"
        )),
        _ if !v.same_region => Some(aurora_i18n::tr!(
            "ui-land-environment-environment-settings-are-unavailable-at-region-boundaries"
        )),
        (Some(pa), _) if !pa.region_allow_env_override => Some(aurora_i18n::tr!(
            "ui-land-environment-the-estate-manager-does-not-allow-parcel-environment-changes"
        )),
        (Some(pa), _) if pa.area < MIN_PARCEL_AREA => Some(aurora_i18n::tr!(
            "ui-land-environment-the-parcel-must-be-at-least-128-square-meters"
        )),
        _ => None,
    }
}

/// The day's length and offset in hours as the sliders show them (the
/// offset between -12 and 12), or the one being dragged.
pub(crate) fn day_hours(summary: Option<&EnvSummary>, drag: Option<(f32, f32)>) -> (f32, f32) {
    match (summary, drag) {
        (_, Some(d)) => d,
        (Some(e), None) if e.day_length > 0 => {
            let mut off = e.day_offset as f32 / 3600.0;
            if off > 12.0 {
                off -= 24.0;
            }
            (e.day_length as f32 / 3600.0, off)
        }
        _ => (4.0, -8.0),
    }
}

/// commitDayLenOffsetChanges.
pub(super) fn commit_day(world: &mut World, length_h: f32, offset_h: f32) {
    let length = (length_h * 3600.0) as i32;
    let mut offset = (offset_h * 3600.0) as i32;
    if offset <= 0 {
        offset += 24 * 3600;
    }
    world.land.environment(|local_id| LandCommand::EnvironmentUpdate {
        local_id,
        day_length: length,
        day_offset: offset,
    });
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World) {
    let parcel = v.parcel.clone();
    let env = world.land.sel.as_ref().map(|s| s.env.clone()).unwrap_or_default();
    if let Some(why) = unavailable(v, &env) {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.add(egui::Label::new(RichText::new(why).size(12.5).color(p.muted)).wrap());
        });
        return;
    }
    let Some(parcel) = parcel else {
        return;
    };
    let summary = match &env {
        EnvState::Ready(e) => Some(e.clone()),
        _ => None,
    };
    // canEdit: LLEnvironment::canAgentUpdateParcelEnvironment and the estate's permission
    let can_edit = v.owns(powers::LAND_ALLOW_ENVIRONMENT) && parcel.region_allow_env_override;
    let can_enable = can_edit && summary.as_ref().is_some_and(|e| e.has_day);
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-land-environment-select-environment"))
                .size(12.5)
                .color(p.ink),
        );
        ui.add_space(2.0);
        let w = Vec2::new(ui.available_width() - 20.0, 22.0);
        if ui
            .add_enabled(
                can_enable,
                egui::Button::new(aurora_i18n::tr!("ui-land-environment-use-region-settings")).min_size(w),
            )
            .clicked()
        {
            s.dialog = Some(Dialog::Confirm(Confirm::ResetEnvironment));
        }
        // onBtnSelect: the settings picker for the whole day
        if ui
            .add_enabled(
                can_edit && summary.is_some(),
                egui::Button::new(aurora_i18n::tr!("ui-land-environment-use-inventory")).min_size(w),
            )
            .clicked()
        {
            s.dialog = Some(Dialog::EnvironmentPicker(super::dialogs::EnvironmentPicker {
                track: None,
                filter: String::new(),
                selected: None,
            }));
        }
        // onBtnEdit: the day cycle editor on the parcel's day
        if ui
            .add_enabled(
                can_edit && summary.is_some(),
                egui::Button::new(aurora_i18n::tr!("common-customize")).min_size(w),
            )
            .clicked()
        {
            s.custom_day = true;
        }
        ui.add_space(10.0);
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-land-environment-day-settings"))
                .size(12.5)
                .color(p.ink),
        );
        let (mut length_h, mut offset_h) = day_hours(summary.as_ref(), s.env_drag);
        dim(ui, p, aurora_i18n::tr!("ui-land-environment-day-length-hours"));
        let r1 = ui.add_enabled(
            can_enable,
            egui::Slider::new(&mut length_h, 4.0..=168.0)
                .custom_parser(aurora_i18n::parse_number)
                .step_by(0.5)
                .fixed_decimals(1),
        );
        dim(ui, p, aurora_i18n::tr!("ui-land-environment-time-offset-hours"));
        let r2 = ui.add_enabled(
            can_enable,
            egui::Slider::new(&mut offset_h, -11.5..=12.0)
                .custom_parser(aurora_i18n::parse_number)
                .step_by(0.5)
                .fixed_decimals(1),
        );
        let dragging = r1.dragged() || r2.dragged() || r1.has_focus() || r2.has_focus();
        if dragging || r1.changed() || r2.changed() {
            s.env_drag = Some((length_h, offset_h));
        }
        if !dragging && s.env_drag.is_some() {
            // onDayLenOffsetMouseUp → commitDayLenOffsetChanges
            s.env_drag = None;
            commit_day(world, length_h, offset_h);
        }
        // the apparent time follows the sliders while they move
        let shown = summary.as_ref().map(|e| EnvSummary {
            day_length: (length_h * 3600.0) as i64,
            day_offset: {
                let o = (offset_h * 3600.0) as i64;
                if o <= 0 { o + 86400 } else { o }
            },
            ..e.clone()
        });
        if let Some((h, m, pct)) = shown.as_ref().and_then(|e| e.apparent_time(v.now)) {
            dim(ui, p, aurora_i18n::tr!("ui-land-environment-apparent-time-of-day"));
            text(ui, p, format!("{h}:{m:02} ({pct}%)"));
        }
        if matches!(env, EnvState::Loading) {
            dim(ui, p, aurora_i18n::tr!("common-loading"));
        }

        let ui = &mut cols[1];
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-land-environment-sky-altitudes"))
                .size(12.5)
                .color(p.ink),
        );
        altitudes(ui, p, summary.as_ref());
    });
}

/// The altitude column: a bar from ground to 4000 m with the starts of
/// skies 2, 3 and 4 (not editable for a parcel), then ground and water.
fn altitudes(ui: &mut egui::Ui, p: &Palette, e: Option<&EnvSummary>) {
    let name = |i: i32| e.map_or_else(|| aurora_i18n::tr!("status-empty").to_owned(), |e| e.track_name(i));
    let h = (ui.available_height() - 60.0).max(160.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), egui::Sense::hover());
    let bar_x = rect.left() + 26.0;
    let painter = ui.painter();
    painter.line_segment(
        [egui::pos2(bar_x, rect.top() + 8.0), egui::pos2(bar_x, rect.bottom() - 8.0)],
        egui::Stroke::new(4.0, p.raised),
    );
    let y_of = |alt: f32| {
        let t = ((alt - ALT_MIN) / (ALT_MAX - ALT_MIN)).clamp(0.0, 1.0);
        rect.bottom() - 8.0 - t * (rect.height() - 16.0)
    };
    // labels pushed apart when skies are close (readjustAltLabels, simplified)
    let mut marks: Vec<(f32, i32, f32)> = (1..4)
        .map(|i| {
            let alt = e.map_or(1000.0 * i as f32, |e| e.altitudes[i]);
            (y_of(alt), i as i32 + 1, alt)
        })
        .collect();
    marks.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut last = f32::INFINITY;
    for (y, sky, alt) in marks {
        painter.rect_filled(
            egui::Rect::from_center_size(egui::pos2(bar_x, y), Vec2::new(14.0, 6.0)),
            super::super::widgets::TRACK_RADIUS,
            p.violet_light,
        );
        let ly = y.min(last - 30.0);
        last = ly;
        painter.text(
            egui::pos2(bar_x + 16.0, ly - 7.0),
            egui::Align2::LEFT_CENTER,
            aurora_i18n::tr!(
                "ui-land-environment-sky-m",
                sky = format!("{}", sky),
                alt = aurora_i18n::number_text(&format!("{:.0}", alt))
            ),
            egui::FontId::proportional(11.5),
            p.ink,
        );
        painter.text(
            egui::pos2(bar_x + 16.0, ly + 7.0),
            egui::Align2::LEFT_CENTER,
            name(sky),
            egui::FontId::proportional(11.5),
            p.muted,
        );
    }
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        dim(ui, p, aurora_i18n::tr!("ui-land-environment-ground"));
        text(ui, p, name(1));
    });
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        dim(ui, p, aurora_i18n::tr!("common-water"));
        text(ui, p, name(0));
    });
}
