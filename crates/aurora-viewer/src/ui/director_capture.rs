//! The Director's "Path by hand" section (`director_capture`): "Record my
//! path" keeps the live view while the user flies the camera, Stop turns the
//! take into keyframes laid after the end of the track (the take is the
//! track when it is empty; Ctrl+Z takes it back in one step), Escape or
//! Cancel drops it. The detail slider sets how closely the keyframes follow
//! the take. Its tab, once the window has tabs: Camera.
//!
//! Aurora's own: the Legacy Viewer has no camera path tool.

use super::director::{SLIDER_W, clock, row};
use super::widgets::{self, ActionKind};
use crate::director::Director;
use crate::theme::Palette;
use egui::{RichText, Vec2};

/// What the section asks of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureAction {
    Start,
    /// Stop, and lay the take's keyframes.
    Stop,
    /// Stop, and drop the take.
    Cancel,
}

/// The take as the section shows it.
pub struct CaptureView<'a> {
    /// Seconds and views kept so far, while a path is being taken.
    pub taking: Option<(f32, usize)>,
    /// 0 (the loosest) to 1 (the finest).
    pub detail: &'a mut f32,
    /// RLVa restricts the camera: no take, as no playback.
    pub restricted: bool,
    /// What the last take gave, or why it gave nothing.
    pub note: Option<Result<&'a str, &'a str>>,
    pub out: &'a mut Vec<CaptureAction>,
}

pub fn capture_section(ui: &mut egui::Ui, p: &Palette, d: &Director, view: CaptureView<'_>) {
    let CaptureView {
        taking,
        detail,
        restricted,
        note,
        out,
    } = view;
    let size = Vec2::new(0.0, 26.0);
    match taking {
        None => {
            let why = if restricted {
                Some(aurora_i18n::tr!("director-capture-restricted"))
            } else if d.track.is_full() {
                Some(aurora_i18n::tr!("director-track-full"))
            } else {
                None
            };
            let tip = why.unwrap_or(aurora_i18n::tr!("director-capture-start-tip"));
            if widgets::action_button_if(
                ui,
                p,
                "route",
                aurora_i18n::tr!("director-capture-start"),
                ActionKind::Primary,
                size,
                why.is_none(),
            )
            .on_hover_text(tip)
            .on_disabled_hover_text(tip)
            .clicked()
            {
                out.push(CaptureAction::Start);
            }
        }
        Some((seconds, views)) => {
            let text = aurora_i18n::tr!("director-capture-taking", time = clock(seconds, true), views = views);
            ui.label(RichText::new(text).size(12.5).strong().color(p.danger));
            ui.horizontal_wrapped(|ui| {
                if widgets::action_button(
                    ui,
                    p,
                    "player-stop-filled",
                    aurora_i18n::tr!("director-capture-stop"),
                    ActionKind::Primary,
                    size,
                )
                .on_hover_text(aurora_i18n::tr!("director-capture-stop-tip"))
                .clicked()
                {
                    out.push(CaptureAction::Stop);
                }
                if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size)
                    .on_hover_text(aurora_i18n::tr!("director-capture-cancel-tip"))
                    .clicked()
                {
                    out.push(CaptureAction::Cancel);
                }
            });
        }
    }
    row(ui, p, aurora_i18n::tr!("director-capture-detail"), |ui| {
        ui.spacing_mut().slider_width = SLIDER_W;
        let mut percent = (*detail * 100.0).round();
        let slider = egui::Slider::new(&mut percent, 0.0..=100.0)
            .custom_parser(aurora_i18n::parse_number)
            .fixed_decimals(0)
            .suffix(" %")
            .trailing_fill(true);
        if ui
            .add(slider)
            .on_hover_text(aurora_i18n::tr!("director-capture-detail-tip"))
            .changed()
        {
            *detail = percent / 100.0;
        }
    });
    match note {
        Some(Ok(text)) => {
            ui.label(RichText::new(text).size(12.0).color(p.muted));
        }
        Some(Err(text)) => {
            ui.label(RichText::new(text).size(12.0).color(p.warn));
        }
        None if taking.is_none() => {
            ui.label(RichText::new(aurora_i18n::tr!("director-capture-hint")).size(12.0).color(p.muted));
        }
        None => {}
    }
}
