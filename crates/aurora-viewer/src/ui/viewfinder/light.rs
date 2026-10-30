//! The viewfinder's light panel: the photo's light (sky, water, day cycle,
//! sun, exposure) left of the dock, without opening the drawer of every
//! setting (`ui::snapshot::light_rows`).

use super::{Out, Viewfinder, bold, widgets};
use crate::theme::Palette;
use crate::ui::snapshot::{SnapshotUi, View};
use egui::{Align2, Id, Order, Pos2, Rect};

const WIDTH: f32 = 360.0;

impl Viewfinder {
    /// The panel, left of the dock: the Light section of the photo panel.
    pub(super) fn light_panel(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        screen: Rect,
        panel: &mut SnapshotUi,
        view: &View,
        out: &mut Vec<Out>,
    ) {
        egui::Area::new(Id::unique("vf_light"))
            .order(Order::Foreground)
            .fade_in(false)
            .pivot(Align2::RIGHT_CENTER)
            .fixed_pos(Pos2::new(screen.right() - 84.0, screen.center().y))
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::same(12), |ui| {
                    ui.set_width(WIDTH);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(aurora_i18n::tr!("photo-section-light"))
                                .font(bold(ctx, 14.0))
                                .color(p.ink),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-close"), true) {
                                self.light = false;
                            }
                        });
                    });
                    let mut actions = Vec::new();
                    panel.light_only(ui, p, view, &mut actions);
                    out.extend(actions.into_iter().map(Out::Panel));
                });
            });
    }
}
