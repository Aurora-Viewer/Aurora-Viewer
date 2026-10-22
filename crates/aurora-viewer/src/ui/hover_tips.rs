//! The world's hover tips on the screen (`hover_tips.rs` says what they
//! show and when): Aurora's bubble next to the pointer, its card with the
//! details and the actions, its readout in the corner of the work area, or
//! the Legacy Viewer's text with its "i" button (Preferences › Aurora).

use crate::hover_tips::{Tip, TipAction, TipKind};
use crate::settings::HoverTipForm;
use crate::theme::Palette;
use crate::ui::sheet::{icon_at, semibold};
use crate::ui::widgets::{self, ActionKind};
use egui::{Align2, Id, Pos2, Rect, RichText, Sense, Vec2};

/// Where the tip appears from the point the pointer rested on
/// (LLToolTip's offset under the cursor).
const OFFSET: Vec2 = Vec2::new(16.0, 20.0);
/// Gap left when a tip goes to the other side of the pointer.
const FLIP_GAP: f32 = 12.0;
const BUBBLE_MAX_W: f32 = 280.0;
const CARD_W: f32 = 300.0;
/// The card's column of keys.
const KEY_W: f32 = 86.0;

fn area_id(form: &'static str) -> Id {
    Id::unique(("hover_tip", form))
}

/// The tip drawn this frame: where, and the button clicked on it.
pub struct Drawn {
    pub rect: Rect,
    pub action: Option<TipAction>,
}

/// The top left corner of a tip of `size` coming from `anchor`, kept on
/// `screen`: to the right and below the pointer, else on its other side.
pub fn place(anchor: Pos2, size: Vec2, screen: Rect) -> Pos2 {
    let mut pos = anchor + OFFSET;
    if pos.x + size.x > screen.right() {
        pos.x = anchor.x - FLIP_GAP - size.x;
    }
    if pos.y + size.y > screen.bottom() {
        pos.y = anchor.y - FLIP_GAP - size.y;
    }
    Pos2::new(
        pos.x.clamp(screen.left(), (screen.right() - size.x).max(screen.left())),
        pos.y.clamp(screen.top(), (screen.bottom() - size.y).max(screen.top())),
    )
}

/// One tip near the pointer (bubble, card or Legacy text), or the readout.
/// `land` is the parcel under the subject, for the readout's second line.
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    tip: Option<&Tip>,
    land: Option<&Tip>,
    form: HoverTipForm,
    legacy: bool,
    anchor: Pos2,
) -> Option<Drawn> {
    if !legacy && form == HoverTipForm::Readout {
        return readout(ctx, p, tip, land);
    }
    let tip = tip?;
    let (key, interactive): (&'static str, bool) = match (legacy, form) {
        (true, _) => ("legacy", tip.kind != TipKind::Land),
        (false, HoverTipForm::Card) => ("card", true),
        _ => ("bubble", false),
    };
    let id = area_id(key);
    let screen = crate::ui::layout::work_area(ctx);
    // the size of last frame (a new tip is measured on its first frame)
    let size = ctx.memory(|m| m.area_rect(id)).map_or(Vec2::new(CARD_W, 160.0), |r| r.size());
    let mut action = None;
    let shown = egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(place(anchor, size, screen))
        .interactable(interactive)
        .fade_in(false)
        .show(ctx, |ui| match key {
            "legacy" => action = legacy_text(ui, p, tip),
            "card" => action = card(ui, p, tip),
            _ => bubble(ui, p, tip),
        });
    Some(Drawn {
        rect: shown.response.rect,
        action,
    })
}

/// A flag of the subject: its icon and its name on the raised face.
fn chip(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str) {
    const PAD: f32 = 6.0;
    const ICON: f32 = 12.0;
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), egui::FontId::proportional(11.5), p.ink);
    let size = Vec2::new(PAD * 2.0 + ICON + 4.0 + galley.size().x, 20.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
    icon_at(ui, icon, Pos2::new(rect.left() + PAD + ICON / 2.0, rect.center().y), ICON, p.muted);
    let pos = widgets::row_text_origin(ui.painter(), &galley, rect.left() + PAD + ICON + 4.0, rect.center().y);
    ui.painter().galley(pos, galley, p.ink);
}

fn chips(ui: &mut egui::Ui, p: &Palette, tip: &Tip, without_actions: bool) {
    let flags: Vec<_> = tip.flags.iter().filter(|f| !(without_actions && f.has_action())).collect();
    if flags.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for f in flags {
            chip(ui, p, f.icon(), f.label());
        }
    });
}

/// A line with a small icon before its text.
fn icon_line(ui: &mut egui::Ui, p: &Palette, icon: &str, text: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        icon_at(ui, icon, rect.center(), 14.0, p.muted);
        text(ui);
    });
}

fn owner_icon(group: bool) -> &'static str {
    if group { "users-group" } else { "user-filled" }
}

/// The small capitals above a title ("LAND"), or the avatar's group title.
fn kicker(ui: &mut egui::Ui, p: &Palette, tip: &Tip) {
    if let Some(k) = &tip.kicker {
        let text = if tip.kind == TipKind::Land { k.to_uppercase() } else { k.clone() };
        ui.add(egui::Label::new(RichText::new(text).size(11.0).color(p.muted)).truncate());
    }
}

fn title(ui: &mut egui::Ui, p: &Palette, tip: &Tip, size: f32) {
    ui.add(egui::Label::new(semibold(ui, &tip.title, size, p.ink)).truncate());
    if let Some(sub) = &tip.sub {
        ui.add(egui::Label::new(RichText::new(sub).size(11.5).color(p.muted)).truncate());
    }
}

/// Aurora's bubble: the name, the owner, the sale and the flags.
fn bubble(ui: &mut egui::Ui, p: &Palette, tip: &Tip) {
    widgets::overlay(ui, p, egui::Margin::symmetric(10, 8), |ui| {
        ui.set_max_width(BUBBLE_MAX_W);
        ui.spacing_mut().item_spacing.y = 4.0;
        kicker(ui, p, tip);
        title(ui, p, tip, 13.0);
        if let Some((name, group)) = &tip.owner {
            icon_line(ui, p, owner_icon(*group), |ui| {
                ui.add(egui::Label::new(RichText::new(name).size(12.5).color(p.ink)).truncate());
            });
        }
        sale_line(ui, p, tip);
        chips(ui, p, tip, false);
    });
}

fn sale_line(ui: &mut egui::Ui, p: &Palette, tip: &Tip) {
    if let Some((price, note)) = &tip.sale {
        icon_line(ui, p, "tag-filled", |ui| {
            ui.label(semibold(ui, price, 12.5, p.ink));
            ui.label(RichText::new(note).size(11.5).color(p.muted));
        });
    }
}

/// Aurora's card: the details, the flags without a button, and the actions;
/// the "i" opens the inspector, the profile or About Land.
fn card(ui: &mut egui::Ui, p: &Palette, tip: &Tip) -> Option<TipAction> {
    let mut action = None;
    widgets::overlay(ui, p, egui::Margin::symmetric(12, 10), |ui| {
        ui.set_width(CARD_W - 24.0);
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(CARD_W - 24.0 - 26.0);
                ui.spacing_mut().item_spacing.y = 1.0;
                kicker(ui, p, tip);
                title(ui, p, tip, 13.5);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if widgets::icon_button(ui, p, "info-circle-filled", tip.inspect.label(), true) {
                    action = Some(tip.inspect);
                }
            });
        });
        if let Some(desc) = &tip.desc {
            ui.add(egui::Label::new(RichText::new(desc).size(12.0).color(p.ink)).wrap());
        }
        egui::Grid::new("hover_tip_rows")
            .num_columns(2)
            .min_col_width(KEY_W)
            .spacing(Vec2::new(8.0, 3.0))
            .show(ui, |ui| {
                for (k, v) in &tip.rows {
                    ui.label(RichText::new(*k).size(12.0).color(p.muted));
                    ui.add(egui::Label::new(RichText::new(v).size(12.0).color(p.ink)).truncate());
                    ui.end_row();
                }
            });
        chips(ui, p, tip, true);
        if !tip.actions.is_empty() {
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for a in &tip.actions {
                    if widgets::action_button(ui, p, a.icon(), a.label(), ActionKind::Flat, Vec2::new(0.0, 26.0)).clicked() {
                        action = Some(*a);
                    }
                }
            });
        }
    });
    action
}

/// The Legacy Viewer's tooltip: its text, and the "i" of its inspector
/// tooltips for objects and avatars (LLInspector "Inspector_I").
fn legacy_text(ui: &mut egui::Ui, p: &Palette, tip: &Tip) -> Option<TipAction> {
    let mut action = None;
    egui::Frame::popup(ui.style()).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(RichText::new(&tip.legacy).size(12.5).color(p.ink)).extend());
            if tip.kind != TipKind::Land && widgets::icon_button(ui, p, "info-circle-filled", tip.inspect.label(), true) {
                action = Some(tip.inspect);
            }
        });
    });
    action
}

/// One line of the readout: an icon, the name, its details, its flags.
fn readout_line(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &Tip, parts: &[String]) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        icon_at(ui, icon, rect.center(), 14.0, p.muted);
        ui.label(semibold(ui, &tip.title, 12.5, p.ink));
        for part in parts {
            ui.label(RichText::new("·").size(12.5).color(p.muted_dim));
            ui.label(RichText::new(part).size(12.5).color(p.ink));
        }
        ui.spacing_mut().item_spacing.x = 4.0;
        for f in &tip.flags {
            chip(ui, p, f.icon(), f.label());
        }
    });
}

/// The details of a readout line: the owner or username, the sale, the
/// group title.
fn readout_parts(tip: &Tip) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(sub) = &tip.sub {
        out.push(sub.clone());
    }
    if tip.kind == TipKind::Avatar
        && let Some(k) = &tip.kicker
    {
        out.push(k.clone());
    }
    if let Some((name, _)) = &tip.owner {
        out.push(name.clone());
    }
    if let Some((price, note)) = &tip.sale {
        out.push(format!("{price} · {note}"));
    }
    out
}

/// Aurora's readout in the bottom left corner of the work area: what is
/// under the pointer, and the parcel it stands on. Nothing near the pointer.
fn readout(ctx: &egui::Context, p: &Palette, tip: Option<&Tip>, land: Option<&Tip>) -> Option<Drawn> {
    let thing = tip.filter(|t| t.kind != TipKind::Land);
    let land = land.or(tip.filter(|t| t.kind == TipKind::Land));
    if thing.is_none() && land.is_none() {
        return None;
    }
    let screen = crate::ui::layout::work_area(ctx);
    let shown = egui::Area::new(area_id("readout"))
        .order(egui::Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(screen.left_bottom())
        .interactable(false)
        .fade_in(false)
        .show(ctx, |ui| {
            widgets::overlay(ui, p, egui::Margin::symmetric(10, 6), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                if let Some(t) = thing {
                    let icon = if t.kind == TipKind::Avatar { "user-filled" } else { "cube" };
                    readout_line(ui, p, icon, t, &readout_parts(t));
                }
                if let Some(l) = land {
                    readout_line(ui, p, "map-pin-filled", l, &readout_parts(l));
                }
            });
        });
    Some(Drawn {
        rect: shown.response.rect,
        action: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect::from_min_max(Pos2::new(8.0, 52.0), Pos2::new(1272.0, 660.0))
    }

    #[test]
    fn a_tip_goes_below_and_right_of_the_pointer_else_on_its_other_side() {
        let size = Vec2::new(300.0, 200.0);
        assert_eq!(place(Pos2::new(400.0, 300.0), size, screen()), Pos2::new(416.0, 320.0));
        // too close to the right edge: on the left of the pointer
        assert_eq!(place(Pos2::new(1100.0, 300.0), size, screen()).x, 1100.0 - 12.0 - 300.0);
        // too low: above it
        assert_eq!(place(Pos2::new(400.0, 600.0), size, screen()).y, 600.0 - 12.0 - 200.0);
        // never off the work area
        let corner = place(Pos2::new(10.0, 55.0), Vec2::new(2000.0, 900.0), screen());
        assert_eq!(corner, Pos2::new(8.0, 52.0));
    }
}
