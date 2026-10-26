//! The viewfinder's isolation panel (issue #977): what the marks do (all
//! drawn, hide them, show only them), the + and − brushes, the count and
//! the "transparent background" switch of "show only"; and the brush
//! itself over the photo's frame, a point of the stroke a frame given to
//! the app (`app::photo_isolate`). The rule is `snapshot::isolate`.

use super::controls::segmented;
use super::{Face, Frame, Out, Viewfinder, bold, dock_button, keep_keys, widgets};
use crate::snapshot::isolate::{Brush, Mode};
use crate::theme::Palette;
use egui::{Align2, Id, Order, Pos2, Rect, Sense, Stroke, Vec2};

/// What the panel and the brush ask of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum Isolate {
    Mode(Mode),
    /// A brush taken up, or put down.
    Brush(Option<Brush>),
    Clear,
    Transparent(bool),
    /// The brush is pressed here (points) this frame.
    Point(Pos2),
}

/// The brush's ring on screen (points).
const RING: f32 = 11.0;

impl Viewfinder {
    /// The panel, left of the dock.
    pub(super) fn isolate_panel(&mut self, ctx: &egui::Context, p: &Palette, screen: Rect, f: &Frame, out: &mut Vec<Out>) {
        let iso = f.isolation;
        egui::Area::new(Id::unique("vf_isolate"))
            .order(Order::Foreground)
            .fade_in(false)
            .pivot(Align2::RIGHT_CENTER)
            .fixed_pos(Pos2::new(screen.right() - 84.0, screen.center().y))
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::same(12), |ui| {
                    ui.set_width(268.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(aurora_i18n::tr!("photo-isolate"))
                                .font(bold(ctx, 14.0))
                                .color(p.ink),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-close"), true) {
                                self.isolate = false;
                                out.push(Out::Isolate(Isolate::Brush(None)));
                            }
                        });
                    });
                    ui.add_space(6.0);
                    let modes = [
                        (
                            Mode::Off,
                            aurora_i18n::tr!("photo-isolate-off"),
                            aurora_i18n::tr!("photo-isolate-off-tip"),
                        ),
                        (
                            Mode::Hide,
                            aurora_i18n::tr!("photo-isolate-hide"),
                            aurora_i18n::tr!("photo-isolate-hide-tip"),
                        ),
                        (
                            Mode::Only,
                            aurora_i18n::tr!("photo-isolate-only"),
                            aurora_i18n::tr!("photo-isolate-only-tip"),
                        ),
                    ];
                    let items: Vec<(&str, bool, &str)> = modes.iter().map(|(m, t, tip)| (*t, iso.mode == *m, *tip)).collect();
                    if let Some(i) = segmented(ui, p, &items, Some(ui.available_width())) {
                        out.push(Out::Isolate(Isolate::Mode(modes[i].0)));
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        for (brush, face, tip) in [
                            (Brush::Add, "+", aurora_i18n::tr!("photo-isolate-add-tip")),
                            (Brush::Remove, "−", aurora_i18n::tr!("photo-isolate-remove-tip")),
                        ] {
                            let on = iso.brush == Some(brush);
                            if dock_button(ui, p, Face::Text(face), tip, on) {
                                out.push(Out::Isolate(Isolate::Brush((!on).then_some(brush))));
                            }
                        }
                        ui.vertical(|ui| {
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(aurora_i18n::tr!("photo-isolate-count", n = iso.count()))
                                    .size(12.5)
                                    .color(p.ink),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("photo-isolate-clear"), iso.count() > 0) {
                                out.push(Out::Isolate(Isolate::Clear));
                            }
                        });
                    });
                    if iso.mode == Mode::Only {
                        ui.add_space(8.0);
                        let mut on = iso.transparent;
                        if widgets::switch_labeled(ui, p, &mut on, aurora_i18n::tr!("photo-isolate-transparent")).changed() {
                            out.push(Out::Isolate(Isolate::Transparent(on)));
                        }
                    }
                    ui.add_space(6.0);
                    let hint = if iso.brush.is_some() {
                        aurora_i18n::tr!("photo-isolate-brush-hint")
                    } else if iso.mode == Mode::Only && iso.transparent {
                        aurora_i18n::tr!("photo-isolate-png-hint")
                    } else {
                        aurora_i18n::tr!("photo-isolate-hint")
                    };
                    ui.label(egui::RichText::new(hint).size(11.5).color(p.muted));
                });
            });
    }

    /// The brush over the photo's frame: pressed, a point a frame; its ring
    /// under the pointer.
    pub(super) fn isolate_brush(&mut self, ctx: &egui::Context, p: &Palette, frame: Rect, f: &Frame, out: &mut Vec<Out>) {
        let Some(brush) = f.isolation.brush else {
            return;
        };
        egui::Area::new(Id::unique("vf_isolate_brush"))
            .order(Order::Middle)
            .fade_in(false)
            .fixed_pos(frame.min)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(frame.size(), Sense::click_and_drag());
                keep_keys(&resp);
                let Some(at) = resp.hover_pos().or_else(|| resp.interact_pointer_pos()) else {
                    return;
                };
                ctx.set_cursor_icon(egui::CursorIcon::None);
                if resp.is_pointer_button_down_on() && rect.contains(at) {
                    out.push(Out::Isolate(Isolate::Point(at)));
                }
                let painter = ui.painter();
                let ring = if brush == Brush::Add { p.ink } else { p.danger };
                painter.circle_stroke(at + Vec2::new(0.0, 1.0), RING, Stroke::new(2.0, p.navy.gamma_multiply(0.7)));
                painter.circle_stroke(at, RING, Stroke::new(1.5, ring));
                let sign = if brush == Brush::Add { "+" } else { "−" };
                painter.text(at, Align2::CENTER_CENTER, sign, bold(ctx, 13.0), ring);
            });
    }
}
