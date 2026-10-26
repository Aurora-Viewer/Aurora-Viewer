//! The viewfinder's isolation panel (issue #977): what the photo shows
//! (everything, our avatar alone, or the avatars and objects chosen), the
//! list of the chosen with a cross each, the button that chooses in the
//! scene, and the transparent background. While choosing, the pointer over
//! the photo's frame names what is under it, and a click adds or removes
//! it (`app::photo_isolate`). The rule is `snapshot::isolate`.

use super::controls::segmented;
use super::{Frame, Out, Viewfinder, bold, icon, keep_keys, shadow_text, widgets};
use crate::snapshot::isolate::Mode;
use crate::theme::Palette;
use egui::{Align2, Id, Order, Pos2, Rect, Sense, Vec2};

/// What the panel and the choosing ask of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum Isolate {
    Mode(Mode),
    /// Choosing in the scene starts or stops.
    Picking(bool),
    /// A cross of the list.
    Remove(uuid::Uuid),
    Transparent(bool),
    /// A click while choosing, and the pointer over the frame (points).
    Click(Pos2),
    Hover(Pos2),
}

/// A row of the list: what is chosen.
#[derive(Debug, Clone, PartialEq)]
pub struct IsolateRow {
    pub id: uuid::Uuid,
    pub name: String,
    pub avatar: bool,
}

const WIDTH: f32 = 272.0;

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
                    ui.set_width(WIDTH);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(aurora_i18n::tr!("photo-isolate"))
                                .font(bold(ctx, 14.0))
                                .color(p.ink),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-close"), true) {
                                self.isolate = false;
                                out.push(Out::Isolate(Isolate::Picking(false)));
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
                            Mode::Own,
                            aurora_i18n::tr!("photo-isolate-own"),
                            aurora_i18n::tr!("photo-isolate-own-tip"),
                        ),
                        (
                            Mode::Chosen,
                            aurora_i18n::tr!("photo-isolate-chosen"),
                            aurora_i18n::tr!("photo-isolate-chosen-tip"),
                        ),
                    ];
                    let items: Vec<(&str, bool, &str)> = modes.iter().map(|(m, t, tip)| (*t, iso.mode == *m, *tip)).collect();
                    if let Some(i) = segmented(ui, p, &items, Some(ui.available_width())) {
                        out.push(Out::Isolate(Isolate::Mode(modes[i].0)));
                    }
                    if iso.mode == Mode::Chosen {
                        ui.add_space(8.0);
                        self.isolate_list(ui, p, f, out);
                        ui.add_space(6.0);
                        let (glyph, label) = if iso.picking {
                            ("check", aurora_i18n::tr!("photo-isolate-pick-done"))
                        } else {
                            ("pointer-filled", aurora_i18n::tr!("photo-isolate-pick"))
                        };
                        let kind = if iso.picking {
                            widgets::ActionKind::Primary
                        } else {
                            widgets::ActionKind::Flat
                        };
                        let resp = widgets::action_button(ui, p, glyph, label, kind, Vec2::new(ui.available_width(), 30.0));
                        keep_keys(&resp);
                        if resp.clicked() {
                            out.push(Out::Isolate(Isolate::Picking(!iso.picking)));
                        }
                    }
                    if iso.mode != Mode::Off {
                        ui.add_space(8.0);
                        let mut on = iso.transparent;
                        if widgets::switch_labeled(ui, p, &mut on, aurora_i18n::tr!("photo-isolate-transparent")).changed() {
                            out.push(Out::Isolate(Isolate::Transparent(on)));
                        }
                    }
                    let hint = match (iso.mode, iso.picking) {
                        (_, true) => Some(aurora_i18n::tr!("photo-isolate-pick-hint")),
                        (Mode::Off, _) => Some(aurora_i18n::tr!("photo-isolate-hint")),
                        _ if iso.transparent => Some(aurora_i18n::tr!("photo-isolate-png-hint")),
                        _ => None,
                    };
                    if let Some(hint) = hint {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(hint).size(11.5).color(p.muted));
                    }
                });
            });
    }

    /// What is chosen, a row each with its cross.
    fn isolate_list(&mut self, ui: &mut egui::Ui, p: &Palette, f: &Frame, out: &mut Vec<Out>) {
        if f.isolate_rows.is_empty() {
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("photo-isolate-none"))
                    .size(12.0)
                    .color(p.muted),
            );
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("vf_isolate_list")
            .max_height(170.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for row in f.isolate_rows {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                        icon(ui.painter(), if row.avatar { "user" } else { "cube" }, r, p.muted);
                        let cross = 26.0;
                        let room = ui.available_width() - cross;
                        ui.allocate_ui_with_layout(Vec2::new(room, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.set_min_width(room);
                            ui.add(egui::Label::new(egui::RichText::new(&row.name).size(12.5).color(p.ink)).truncate());
                        });
                        if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("photo-isolate-remove"), true) {
                            out.push(Out::Isolate(Isolate::Remove(row.id)));
                        }
                    });
                }
            });
    }

    /// Choosing over the photo's frame: the pointer names what is under it,
    /// a click adds or removes it.
    pub(super) fn isolate_picking(&mut self, ctx: &egui::Context, p: &Palette, frame: Rect, f: &Frame, out: &mut Vec<Out>) {
        if !f.isolation.picking {
            return;
        }
        egui::Area::new(Id::unique("vf_isolate_pick"))
            .order(Order::Middle)
            .fade_in(false)
            .fixed_pos(frame.min)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(frame.size(), Sense::click());
                keep_keys(&resp);
                let Some(at) = resp.hover_pos() else {
                    return;
                };
                ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                out.push(Out::Isolate(Isolate::Hover(at)));
                if resp.clicked() && rect.contains(at) {
                    out.push(Out::Isolate(Isolate::Click(at)));
                }
                if let Some((name, chosen)) = &f.isolate_hover {
                    let text = if *chosen {
                        aurora_i18n::tr!("photo-isolate-hover-remove", name = name.as_str())
                    } else {
                        aurora_i18n::tr!("photo-isolate-hover-add", name = name.as_str())
                    };
                    shadow_text(
                        ui.painter(),
                        p,
                        at + Vec2::new(16.0, 14.0),
                        Align2::LEFT_TOP,
                        &text,
                        bold(ctx, 13.0),
                        if *chosen { p.danger } else { p.ink },
                    );
                }
            });
    }
}
