//! The viewfinder's filter panel (issue #969): the looks editor
//! (`ui::looks_editor`) left of the dock, and the before / after line over
//! the photo's frame, which leaves the scene as it is on its left
//! (`snapshot::looks`).

use super::{Out, Viewfinder, bold, keep_keys, shadow_text, widgets};
use crate::snapshot::Settings;
use crate::theme::Palette;
use crate::ui::snapshot::SnapshotUi;
use egui::{Align2, Id, Order, Pos2, Rect, Sense, Stroke, Vec2};

const WIDTH: f32 = 360.0;
/// The cards' room at the least before they scroll: two closed cards.
const HEAD_ROOM: f32 = 88.0;
/// The editor's height at most; the room its title row and margins take
/// over it, and the room kept under it at the screen's bottom.
const MAX_HEIGHT: f32 = 860.0;
const TITLE_ROOM: f32 = 56.0;
const BOTTOM_ROOM: f32 = 24.0 + TITLE_ROOM;

impl Viewfinder {
    /// The panel, left of the dock: the looks editor.
    pub(super) fn looks_panel(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        screen: Rect,
        s: &mut Settings,
        panel: &mut SnapshotUi,
        out: &mut Vec<Out>,
    ) {
        // moved anywhere by its background, its top kept where it was left:
        // it grows and shrinks from its bottom only (centred, it moved at
        // both ends with every change of height)
        let id = Id::unique("vf_looks");
        let width = WIDTH + 24.0;
        let default = Pos2::new(screen.right() - 84.0 - width, (screen.top() + 60.0).max(screen.center().y - 440.0));
        // kept on the screen by its width and its title, whatever its height
        let grip = Vec2::new(width, TITLE_ROOM);
        let pos = match self.looks_drag {
            Some(dragged) => crate::ui::photo_reel::tray_place(None, screen, grip, dragged),
            None => crate::ui::photo_reel::tray_place(s.viewfinder.filters_at, screen, grip, default),
        };
        // the editor reaches down to the screen's bottom at most
        let height = (screen.bottom() - pos.y - BOTTOM_ROOM).clamp(HEAD_ROOM * 4.0, MAX_HEIGHT);
        let area = egui::Area::new(id)
            .order(Order::Foreground)
            .fade_in(false)
            .movable(true)
            .current_pos(pos)
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::same(12), |ui| {
                    ui.set_width(WIDTH);
                    ui.spacing_mut().item_spacing.y = 6.0;
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
                    // the catalogue about two fifths of the room
                    let list_height = (height * 0.42).clamp(150.0, 380.0);
                    let mut actions = Vec::new();
                    let thumbs = &panel.thumbs;
                    let thumb = |id: &str| thumbs.get(id).copied();
                    let opts = crate::ui::looks_editor::Options {
                        list_height,
                        height: Some(height),
                        compare: true,
                        luts: &panel.lut_names,
                        pano: panel.pano,
                        thumb: &thumb,
                    };
                    let mut state = std::mem::take(&mut panel.looks);
                    state.show(ui, p, crate::ui::looks_editor::Stack::photo(s), opts, &mut actions);
                    panel.looks = state;
                    out.extend(actions.into_iter().map(Out::Panel));
                });
            });
        // moved: followed live, kept once let go (the settings are saved
        // when they change, not every frame of the drag)
        let resp = &area.response;
        if resp.dragged() {
            self.looks_drag = Some(pos + resp.drag_delta());
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if resp.drag_stopped()
            && let Some(dragged) = self.looks_drag.take()
        {
            let kept = crate::ui::photo_reel::tray_place(None, screen, grip, dragged);
            s.viewfinder.filters_at = Some(crate::ui::photo_reel::tray_fraction(kept, screen));
        }
    }

    /// The look pointed at in the catalogue, named over the photo while it
    /// is tried there (as Genshin Impact's photo mode and Photos' filters
    /// do): its name and what it does, in a small pill at the top of the
    /// frame; with the stack full and Replace off, why nothing is tried.
    pub(super) fn look_preview(&mut self, ctx: &egui::Context, p: &Palette, frame: Rect, s: &Settings, panel: &SnapshotUi) {
        let Some(pointed) = &panel.looks.hover else {
            return;
        };
        let full = s.looks.len() >= crate::snapshot::looks::MAX_LAYERS && !panel.looks.replace;
        let (title, about) = match pointed {
            None => (aurora_i18n::tr!("photo-look-none").to_owned(), None),
            Some(id) if full => (
                crate::snapshot::looks::label(Some(id)).into_owned(),
                Some(aurora_i18n::tr!("photo-looks-full", max = crate::snapshot::looks::MAX_LAYERS)),
            ),
            Some(id) => (
                crate::snapshot::looks::label(Some(id)).into_owned(),
                crate::snapshot::looks::tip(id).map(str::to_owned),
            ),
        };
        egui::Area::new(Id::unique("vf_look_preview"))
            .order(Order::Foreground)
            .interactable(false)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(Pos2::new(frame.center().x, frame.top() + 16.0))
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::symmetric(14, 7), |ui| {
                    ui.set_max_width(420.0);
                    ui.vertical_centered(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(egui::RichText::new(title).font(bold(ctx, 13.0)).color(p.ink));
                        if let Some(about) = about {
                            ui.add(egui::Label::new(egui::RichText::new(about).size(11.5).color(p.muted)).wrap());
                        }
                    });
                });
            });
    }

    /// The before / after line over the photo's `frame`: dragged sideways,
    /// "Before" on its left and "After" on its right.
    pub(super) fn compare_line(&mut self, ctx: &egui::Context, p: &Palette, frame: Rect, panel: &mut SnapshotUi) {
        let Some(at) = panel.looks.compare else {
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
                    panel.looks.compare = Some(moved.clamp(0.0, 1.0));
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
