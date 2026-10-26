//! The viewfinder's filter panel (issue #969): the Legacy Viewer's photo
//! filters as chips, the one pointed at tried on the live view, its
//! strength, and the before / after line over the photo's frame, which
//! leaves the scene as it is on its left (`snapshot::looks`).

use super::{Viewfinder, bold, keep_keys, regular, shadow_text, widgets};
use crate::snapshot::Settings;
use crate::snapshot::looks;
use crate::theme::Palette;
use crate::ui::snapshot::{SnapshotUi, chip};
use egui::{Align2, Id, Order, Pos2, Rect, Sense, Stroke, Vec2};

const WIDTH: f32 = 316.0;

impl Viewfinder {
    /// The panel, left of the dock.
    pub(super) fn looks_panel(&mut self, ctx: &egui::Context, p: &Palette, screen: Rect, s: &mut Settings, panel: &mut SnapshotUi) {
        egui::Area::new(Id::unique("vf_looks"))
            .order(Order::Foreground)
            .fade_in(false)
            .pivot(Align2::RIGHT_CENTER)
            .fixed_pos(Pos2::new(screen.right() - 84.0, screen.center().y))
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::same(12), |ui| {
                    ui.set_width(WIDTH);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(aurora_i18n::tr!("photo-looks"))
                                .font(bold(ctx, 14.0))
                                .color(p.ink),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-close"), true) {
                                self.looks = false;
                            }
                        });
                    });
                    ui.label(
                        egui::RichText::new(aurora_i18n::tr!("photo-look-hint"))
                            .font(regular(11.0))
                            .color(p.muted),
                    );
                    ui.add_space(4.0);
                    // above the bottom bar and under the top one
                    let room = (screen.height() - 360.0).max(120.0);
                    egui::ScrollArea::vertical()
                        .id_salt("vf_looks_list")
                        .max_height(room)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                                let none = chip(ui, p, aurora_i18n::tr!("photo-look-none"), None, s.look.is_none(), true);
                                keep_keys(&none);
                                if none.hovered() {
                                    panel.look_hover = Some(None);
                                }
                                if none.clicked() {
                                    s.look = None;
                                }
                                for (id, name) in looks::list() {
                                    let resp = chip(ui, p, name, None, s.look.as_deref() == Some(id), true);
                                    keep_keys(&resp);
                                    if resp.hovered() {
                                        panel.look_hover = Some(Some(id));
                                    }
                                    if resp.clicked() {
                                        s.look = Some(id.to_owned());
                                    }
                                }
                            });
                        });
                    ui.add_space(8.0);
                    ui.add_enabled_ui(s.look.is_some(), |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(aurora_i18n::tr!("photo-look-strength")).color(p.ink));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.spacing_mut().slider_width = (ui.available_width() - 60.0).max(60.0);
                                let resp = ui.add(egui::Slider::new(&mut s.look_strength, 0..=100).suffix(" %"));
                                keep_keys(&resp);
                            });
                        });
                        let mut compare = panel.look_compare.is_some();
                        let resp = widgets::switch_labeled(ui, p, &mut compare, aurora_i18n::tr!("photo-look-compare"))
                            .on_hover_text(aurora_i18n::tr!("photo-look-compare-tip"));
                        keep_keys(&resp);
                        if resp.changed() {
                            panel.look_compare = compare.then_some(0.5);
                        }
                    });
                });
            });
    }

    /// The before / after line over the photo's `frame`: dragged sideways,
    /// "Before" on its left and "After" on its right.
    pub(super) fn compare_line(&mut self, ctx: &egui::Context, p: &Palette, frame: Rect, panel: &mut SnapshotUi) {
        let Some(at) = panel.look_compare else {
            return;
        };
        let x = frame.left() + at.clamp(0.0, 1.0) * frame.width();
        let grip = Rect::from_x_y_ranges(x - 12.0..=x + 12.0, frame.y_range());
        egui::Area::new(Id::unique("vf_look_compare"))
            .order(Order::Middle)
            .fade_in(false)
            .fixed_pos(grip.min)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(grip.size(), Sense::drag());
                keep_keys(&resp);
                if resp.dragged() {
                    let moved = at + resp.drag_delta().x / frame.width().max(1.0);
                    panel.look_compare = Some(moved.clamp(0.0, 1.0));
                }
                let resp = resp.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
                let painter = ui.painter();
                let strong = resp.hovered() || resp.dragged();
                let line = if strong { p.ink } else { p.ink.gamma_multiply(0.8) };
                painter.line_segment(
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                    Stroke::new(if strong { 2.0 } else { 1.5 }, line),
                );
                // the grip in the middle
                let c = Pos2::new(x, rect.center().y);
                painter.circle_filled(c, 13.0, p.navy.gamma_multiply(0.8));
                painter.circle_stroke(c, 13.0, Stroke::new(1.5, line));
                super::icon(painter, "arrows-horizontal", Rect::from_center_size(c, Vec2::splat(16.0)), p.ink);
            });
        let font = bold(ctx, 12.0);
        let painter = ctx.layer_painter(egui::LayerId::new(Order::Middle, Id::unique("vf_look_compare_labels")));
        let y = frame.top() + 18.0;
        shadow_text(
            &painter,
            p,
            Pos2::new(x - 10.0, y),
            Align2::RIGHT_CENTER,
            aurora_i18n::tr!("photo-look-before"),
            font.clone(),
            p.ink,
        );
        shadow_text(
            &painter,
            p,
            Pos2::new(x + 10.0, y),
            Align2::LEFT_CENTER,
            aurora_i18n::tr!("photo-look-after"),
            font,
            p.ink,
        );
    }
}
