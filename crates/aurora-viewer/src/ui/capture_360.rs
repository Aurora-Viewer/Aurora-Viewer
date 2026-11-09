//! The 360° snapshot window. Port of floater_360capture.xml
//! (LLFloater360Capture, indra/newview/llfloater360capture.cpp, originally
//! LGPL 2.1): on the left the quality level (Preview (fast), Medium, High,
//! Maximum), "Hide all avatars", "Create 360 image" and "Save as...",
//! disabled until an image exists; on the right the panorama, with the
//! quality it was taken at written under it (the Legacy Viewer lays that
//! label over its three.js view, which the user can drag around; Aurora
//! shows the equirectangular image itself, flat). The capture and the
//! save are `app/capture_360.rs`.

use super::widgets::{self, Floater};
use crate::snapshot::capture_360::{Quality, Settings};
use crate::theme::Palette;
use egui::{RichText, Vec2};

/// The window's id (Floater.ToggleOrBringToFront "360capture").
pub const WINDOW: &str = "360capture";

/// Width of the left panel (floater_360capture.xml's ui_panel_left).
const PANEL_W: f32 = 180.0;
const BUTTON_H: f32 = 26.0;

/// What the window shows.
pub struct View<'a> {
    /// A capture, a dialog or a save is in progress: the buttons wait.
    pub busy: bool,
    /// What the capture is doing, while it is.
    pub progress: Option<&'a str>,
    pub preview: Option<&'a egui::TextureHandle>,
    pub can_save: bool,
    /// The status line (an error in the warning colour).
    pub message: Option<(&'a str, bool)>,
    /// The quality the panorama shown was taken at.
    pub quality_shown: Option<Quality>,
}

/// What the window asks the app for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// "Create 360 image".
    Capture,
    /// "Save as...".
    SaveAs,
    /// A setting changed (saved).
    Changed,
}

#[derive(Default)]
pub struct Capture360Ui;

impl Capture360Ui {
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, view: &View, settings: &mut Settings, open: &mut bool) -> Vec<Action> {
        let mut actions = Vec::new();
        if !*open {
            return actions;
        }
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("toolbar-command-360-snapshot"),
            egui::pos2(260.0, 140.0),
            Vec2::new(800.0, 400.0),
        );
        floater.min_size = Vec2::new(400.0, 300.0);
        floater.show(ctx, p, open, |ui| {
            let height = ui.available_height();
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(Vec2::new(PANEL_W, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    Self::panel(ui, p, view, settings, &mut actions);
                });
                ui.add_space(8.0);
                ui.vertical(|ui| Self::panorama(ui, p, view));
            });
        });
        actions
    }

    /// The left panel: the quality level, the avatars, the two buttons.
    fn panel(ui: &mut egui::Ui, p: &Palette, view: &View, settings: &mut Settings, actions: &mut Vec<Action>) {
        ui.add_space(4.0);
        ui.label(
            RichText::new(aurora_i18n::tr!("capture-360-quality-label"))
                .size(12.0)
                .strong()
                .color(p.ink),
        );
        ui.add_space(2.0);
        // (the settings of a capture in progress are the ones it started with)
        ui.add_enabled_ui(!view.busy, |ui| {
            ui.indent("capture_360_quality", |ui| {
                for q in Quality::ALL {
                    let resp = widgets::radio_value(ui, p, &mut settings.quality, q, aurora_i18n::text(q.label_key()))
                        .on_hover_text(aurora_i18n::text(q.tip_key()));
                    if resp.changed() {
                        actions.push(Action::Changed);
                    }
                }
            });
            ui.add_space(6.0);
            if widgets::checkbox(ui, p, &mut settings.hide_avatars, aurora_i18n::tr!("capture-360-hide-avatars")).changed() {
                actions.push(Action::Changed);
            }
        });
        ui.add_space(12.0);
        ui.add_enabled_ui(!view.busy, |ui| {
            if widgets::primary_button_sized(ui, p, aurora_i18n::tr!("capture-360-create"), Vec2::new(PANEL_W, BUTTON_H)).clicked() {
                actions.push(Action::Capture);
            }
        });
        ui.add_space(8.0);
        ui.add_enabled_ui(view.can_save, |ui| {
            if widgets::flat_button_sized(ui, p, aurora_i18n::tr!("photo-save-as"), Vec2::new(PANEL_W, BUTTON_H)).clicked() {
                actions.push(Action::SaveAs);
            }
        });
    }

    /// The right panel: the panorama fitted to the room, 2:1, and the
    /// status line under it.
    fn panorama(ui: &mut egui::Ui, p: &Palette, view: &View) {
        let (mut above, footer) = widgets::above_footer(ui);
        let room = above.available_size();
        let room = Vec2::new(room.x.max(1.0), room.y.max(1.0));
        let (rect, _) = above.allocate_exact_size(room, egui::Sense::hover());
        if let Some(texture) = view.preview {
            let fit = (room.x / 2.0).min(room.y);
            let size = Vec2::new(fit * 2.0, fit);
            let image_rect = egui::Rect::from_center_size(rect.center(), size);
            egui::Image::new((texture.id(), size)).paint_at(&above, image_rect);
        }
        let status = match (view.progress, view.message, view.quality_shown) {
            (Some(progress), _, _) => (progress.to_owned(), false),
            (None, Some((message, error)), _) => (message.to_owned(), error),
            (None, None, Some(q)) => (aurora_i18n::text(q.tip_key()).to_owned(), false),
            (None, None, None) => (String::new(), false),
        };
        let color = if status.1 { p.warn } else { p.muted };
        footer.show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(status.0).size(12.0).color(color)).truncate());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window keeps its size at every scale, its "Save as…" button
    /// inside it, with and without a panorama.
    #[test]
    fn the_window_keeps_its_size() {
        widgets::steady_tests::assert_window_steady(
            WINDOW,
            &[Vec2::new(800.0, 400.0), Vec2::new(400.0, 300.0)],
            aurora_i18n::tr!("photo-save-as"),
            || (Capture360Ui, Settings::default()),
            |(ui_state, settings), ctx, p| {
                let view = View {
                    busy: false,
                    progress: None,
                    preview: None,
                    can_save: true,
                    message: Some(("Saved sl360.jpg", false)),
                    quality_shown: Some(Quality::Medium),
                };
                let actions = ui_state.show(ctx, p, &view, settings, &mut true);
                assert!(actions.is_empty());
            },
        );
    }
}
