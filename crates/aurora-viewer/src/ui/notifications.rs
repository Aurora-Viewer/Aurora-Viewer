//! Notification center: bell of the navigation bar (unread badge), the
//! list it opens and the toasts shown when something arrives — the
//! Firestorm notification well / toasts, flattened to Aurora cards.
//! Script grids follow LLToastNotifyPanel::updateButtonsLayout
//! (indra/newview/lltoastnotifypanel.cpp, originally LGPL 2.1).

use super::icons::Icons;
use crate::theme::Palette;
use crate::world::notifications::{Data, Kind, Notification, Notifications, Response};
use egui::{Color32, CornerRadius, RichText, Vec2};

/// Seconds a toast stays: information / waiting for an answer.
const TOAST_INFO: f64 = 7.0;
const TOAST_OFFER: f64 = 25.0;
const MAX_TOASTS: usize = 3;
const WIDTH: f32 = 320.0;

#[derive(Default)]
pub struct NotifUi {
    pub open: bool,
    /// Bottom-left of the bell (where the list opens).
    pub anchor: Option<egui::Pos2>,
    /// (notification id, expiry time)
    toasts: Vec<(u64, f64)>,
    last_seen: u64,
}

/// Bell button with the unread count; returns true when clicked.
pub fn bell(ui: &mut egui::Ui, p: &Palette, icons: &Icons, unread: usize, open: bool) -> (bool, egui::Rect) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(22.0, 20.0), egui::Sense::click());
    if resp.hovered() || open {
        ui.painter().rect_filled(rect, 2.0, p.raised);
    }
    let col = if unread > 0 || resp.hovered() || open { p.ink } else { p.muted };
    let icon = if unread > 0 { "bell-ringing" } else { "bell" };
    if let Some(t) = icons.get(icon) {
        let r = egui::Rect::from_center_size(rect.center(), Vec2::splat(15.0));
        ui.painter()
            .image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), col);
    }
    if unread > 0 {
        let text = if unread > 99 { "99+".to_owned() } else { unread.to_string() };
        let galley = ui.painter().layout_no_wrap(text, egui::FontId::proportional(9.0), Color32::WHITE);
        // round for one digit, a pill beyond
        const H: f32 = 12.0;
        let w = (galley.size().x + 6.0).max(H);
        let badge = egui::Rect::from_min_size(egui::pos2(rect.right() - w + 3.0, rect.top()), Vec2::new(w, H));
        ui.painter().rect_filled(badge, H / 2.0, p.rose);
        super::widgets::paint_ink_centered(ui.painter(), galley, badge.center(), Color32::WHITE);
    }
    let tip = match unread {
        0 => "Notifications".to_owned(),
        1 => "1 nouvelle notification".to_owned(),
        n => format!("{n} nouvelles notifications"),
    };
    (resp.on_hover_text(tip).clicked(), rect)
}

fn kind_style(p: &Palette, kind: Kind) -> (&'static str, Color32) {
    match kind {
        Kind::Info => ("info", p.muted),
        Kind::Alert => ("warning", p.amber),
        Kind::Teleport => ("airplane-takeoff", p.teal),
        Kind::Friendship => ("handshake", p.violet_light),
        Kind::Inventory => ("gift", p.violet_light),
        Kind::Group => ("users-three", p.indigo_light),
        Kind::Script => ("scroll", p.teal),
        Kind::Permissions => ("key", p.amber),
        Kind::Url => ("globe", p.teal),
    }
}

/// One notification card; returns the user's answer.
fn card(ui: &mut egui::Ui, p: &Palette, icons: &Icons, n: &mut Notification, toast: bool) -> Option<Response> {
    let mut out = None;
    let (icon, accent) = kind_style(p, n.kind);
    let fill = if toast || !n.read { p.panel } else { p.bar };
    let script = matches!(n.data, Data::Dialog { .. });
    egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, if toast { accent.gamma_multiply(0.6) } else { p.raised }))
        .corner_radius(CornerRadius::same(3))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(WIDTH - 18.0);
            if script {
                ui.spacing_mut().interact_size.y = 24.0;
            }
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::hover());
                if let Some(t) = icons.get(icon) {
                    ui.painter().image(
                        t.id(),
                        egui::Rect::from_center_size(r.center(), Vec2::splat(16.0)),
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        accent,
                    );
                }
                // keep room for the time and the close button
                let w = (ui.available_width() - 62.0).max(40.0);
                ui.allocate_ui(Vec2::new(w, 18.0), |ui| {
                    ui.add(egui::Label::new(RichText::new(&n.title).size(12.5).strong().color(p.ink)).truncate())
                        .on_hover_text(&n.title);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, 2.0, p.raised);
                    }
                    if let Some(t) = icons.get("x") {
                        ui.painter().image(
                            t.id(),
                            egui::Rect::from_center_size(r.center(), Vec2::splat(11.0)),
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            if resp.hovered() { p.ink } else { p.muted },
                        );
                    }
                    let tip = if script {
                        "Ignorer ce menu"
                    } else if toast {
                        "Masquer (reste dans la cloche)"
                    } else if n.interactive() && !matches!(n.data, Data::Url(_)) {
                        "Fermer (refuse l'offre)"
                    } else {
                        "Fermer"
                    };
                    if resp.on_hover_text(tip).clicked() {
                        out = Some(Response::Dismiss);
                    }
                    ui.label(RichText::new(super::chat::hhmm(n.time)).size(10.5).color(p.muted_dim));
                });
            });
            if !n.body.is_empty() {
                ui.add_space(2.0);
                // selectable: Firestorm shows the body in a read-only
                // LLTextEditor (lltoastnotifypanel.cpp, "text_editor_box")
                if script {
                    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(&n.body).size(12.0).color(p.ink))
                                .wrap()
                                .selectable(true),
                        );
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.add_space(22.0);
                        ui.add(
                            egui::Label::new(RichText::new(&n.body).size(12.0).color(p.muted))
                                .wrap()
                                .selectable(true),
                        );
                    });
                }
            }
            let mut buttons: Vec<(String, Response, bool)> = Vec::new();
            match &n.data {
                Data::None => {}
                Data::Lure { .. } => {
                    buttons.push(("Téléporter".into(), Response::Accept, true));
                    buttons.push(("Refuser".into(), Response::Decline, false));
                }
                Data::Friend { .. } | Data::Group { .. } => {
                    buttons.push(("Accepter".into(), Response::Accept, true));
                    buttons.push(("Refuser".into(), Response::Decline, false));
                }
                Data::Inventory { .. } => {
                    buttons.push(("Garder".into(), Response::Accept, true));
                    buttons.push(("Rejeter".into(), Response::Decline, false));
                }
                Data::Permissions { .. } => {
                    buttons.push(("Autoriser".into(), Response::Accept, true));
                    buttons.push(("Refuser".into(), Response::Decline, false));
                }
                Data::Url(_) => buttons.push(("Ouvrir le lien".into(), Response::OpenUrl, true)),
                Data::Dialog {
                    buttons: b, own_object, ..
                } => {
                    ui.add_space(8.0);
                    let width = (ui.available_width() - 12.0) / 3.0;
                    // Reverse rows, never the labels or their wire indices.
                    for row in script_rows(b.len()) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            for index in row.clone() {
                                if super::widgets::flat_button_sized(ui, p, &b[index], Vec2::new(width, 24.0)).clicked() {
                                    out = Some(Response::Button(index));
                                }
                            }
                            for _ in row.len()..3 {
                                ui.allocate_space(Vec2::new(width, 24.0));
                            }
                        });
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if super::widgets::flat_button_sized(ui, p, "Ignorer", Vec2::new(72.0, 22.0)).clicked() {
                                out = Some(Response::Dismiss);
                            }
                            if !own_object && super::widgets::flat_button_sized(ui, p, "Bloquer", Vec2::new(72.0, 22.0)).clicked() {
                                out = Some(Response::Block);
                            }
                        });
                    });
                }
                Data::TextBox { .. } => {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(22.0);
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut n.text)
                                .hint_text("Réponse…")
                                .desired_width(WIDTH - 110.0),
                        );
                        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if button(ui, p, "Envoyer", true).clicked() || enter {
                            out = Some(Response::SendText);
                        }
                    });
                }
            }
            if !buttons.is_empty() {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(22.0);
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                    for (label, r, primary) in buttons {
                        if button(ui, p, &label, primary).clicked() {
                            out = Some(r);
                        }
                    }
                });
            }
        });
    out
}

fn button(ui: &mut egui::Ui, p: &Palette, label: &str, primary: bool) -> egui::Response {
    let (fill, ink) = if primary { (p.violet, Color32::WHITE) } else { (p.raised, p.ink) };
    ui.add(
        egui::Button::new(RichText::new(label).size(12.0).color(ink))
            .fill(fill)
            .corner_radius(CornerRadius::same(2))
            .min_size(Vec2::new(64.0, 20.0)),
    )
}

/// List under the bell and toasts; returns the answers to apply.
pub fn show(ctx: &egui::Context, p: &Palette, icons: &Icons, list: &mut Notifications, nui: &mut NotifUi) -> Vec<(u64, Response)> {
    let mut answers = Vec::new();
    let now = ctx.input(|i| i.time);

    // new arrivals become toasts (unless the list is open)
    for n in list.list.iter().filter(|n| n.id > nui.last_seen) {
        if !nui.open {
            let ttl = if n.interactive() { TOAST_OFFER } else { TOAST_INFO };
            nui.toasts.push((n.id, now + ttl));
        }
    }
    if let Some(last) = list.list.iter().map(|n| n.id).max() {
        nui.last_seen = nui.last_seen.max(last);
    }
    nui.toasts.retain(|(id, until)| {
        list.list
            .iter()
            .any(|n| n.id == *id && (*until > now || matches!(n.data, Data::Dialog { .. })))
    });
    if nui.toasts.len() > MAX_TOASTS {
        let extra = nui.toasts.len() - MAX_TOASTS;
        nui.toasts.drain(..extra);
    }

    let anchor = nui.anchor.unwrap_or(egui::pos2(8.0, 60.0));
    if nui.open {
        nui.toasts.clear();
        list.mark_all_read();
        let area = egui::Area::new(egui::Id::new("notification_list"))
            .fade_in(false)
            .order(egui::Order::Foreground)
            .fixed_pos(anchor + Vec2::new(0.0, 2.0))
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(p.bar)
                    .stroke(egui::Stroke::new(1.0, p.raised))
                    .corner_radius(CornerRadius::same(3))
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.set_width(WIDTH);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Notifications").size(13.0).strong().color(p.ink));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let any_info = list.list.iter().any(|n| !n.interactive());
                                if ui
                                    .add_enabled(
                                        any_info,
                                        egui::Button::new(RichText::new("Tout effacer").size(11.5).color(p.muted)).frame(false),
                                    )
                                    .on_hover_text("Efface les notifications sans réponse en attente")
                                    .clicked()
                                {
                                    list.clear_info();
                                }
                            });
                        });
                        ui.add_space(4.0);
                        if list.list.is_empty() {
                            ui.add_space(14.0);
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new("Aucune notification").size(12.0).color(p.muted_dim));
                            });
                            ui.add_space(14.0);
                            return;
                        }
                        egui::ScrollArea::vertical()
                            .max_height(ctx.content_rect().height() * 0.7)
                            .min_scrolled_height(ctx.content_rect().height() * 0.5)
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 4.0;
                                // newest first
                                for n in list.list.iter_mut().rev() {
                                    if let Some(r) = card(ui, p, icons, n, false) {
                                        answers.push((n.id, r));
                                    }
                                }
                            });
                    });
            });
        // click outside closes the list (not on the bell: it toggles itself)
        let clicked_out = ctx.input(|i| i.pointer.any_pressed())
            && ctx.pointer_interact_pos().is_some_and(|pos| {
                !area.response.rect.contains(pos)
                    && !egui::Rect::from_min_size(anchor - Vec2::new(0.0, 22.0), Vec2::new(24.0, 22.0)).contains(pos)
            });
        if clicked_out {
            nui.open = false;
        }
    } else if !nui.toasts.is_empty() {
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("notification_toasts"))
            .fade_in(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-10.0, anchor.y.min(screen.height()) + 6.0))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                let ids: Vec<u64> = nui.toasts.iter().map(|(id, _)| *id).collect();
                for id in ids.into_iter().rev() {
                    if let Some(n) = list.list.iter_mut().find(|n| n.id == id) {
                        match card(ui, p, icons, n, true) {
                            // closing a toast only hides it: the offer waits in the list
                            Some(Response::Dismiss) if !matches!(n.data, Data::Dialog { .. }) => nui.toasts.retain(|(t, _)| *t != id),
                            Some(r) => answers.push((id, r)),
                            None => {}
                        }
                    }
                }
            });
        // keep toasts expiring without input
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
    answers
}

fn script_rows(count: usize) -> impl Iterator<Item = std::ops::Range<usize>> {
    (0..count.div_ceil(3)).rev().map(move |row| row * 3..(row * 3 + 3).min(count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_rows_keep_original_indices_in_bottom_up_groups_of_three() {
        assert_eq!(script_rows(12).collect::<Vec<_>>(), vec![9..12, 6..9, 3..6, 0..3]);
        assert_eq!(script_rows(4).collect::<Vec<_>>(), vec![3..4, 0..3]);
        assert_eq!(script_rows(2).collect::<Vec<_>>(), vec![0..2]);
        assert_eq!(script_rows(0).count(), 0);
    }
}
