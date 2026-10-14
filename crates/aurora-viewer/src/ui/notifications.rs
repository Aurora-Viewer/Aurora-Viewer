//! Notification center: bell of the navigation bar (unread badge), the
//! list it opens and the toasts shown when something arrives — the
//! Firestorm notification well / toasts, flattened to Aurora cards.
//! Script grids follow LLToastNotifyPanel::updateButtonsLayout
//! (indra/newview/lltoastnotifypanel.cpp, originally LGPL 2.1).

use super::icons::Icons;
use super::presence::Status;
use crate::theme::Palette;
use crate::world::notifications::{Data, Kind, Notification, Notifications, Response};
use egui::{Color32, RichText, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Seconds a toast stays: information / waiting for an answer.
const TOAST_INFO: f64 = 7.0;
const TOAST_OFFER: f64 = 25.0;
/// A friend online / offline (NotificationTipToastLifeTime).
const TOAST_TIP: f64 = 10.0;
const MAX_TOASTS: usize = 3;
const WIDTH: f32 = 320.0;
/// Side of the picture of a friend's or a message's card.
const PIC: f32 = 32.0;
/// Characters of a message shown on its card (the rest is in the conversation).
const MESSAGE_CHARS: usize = 160;

#[derive(Default)]
pub struct NotifUi {
    pub open: bool,
    /// Bottom-left of the bell (where the list opens).
    pub anchor: Option<egui::Pos2>,
    /// (notification id, expiry time)
    toasts: Vec<(u64, f64)>,
    last_seen: u64,
    /// Profile pictures the cards of friends and messages want.
    pub wanted_pics: HashSet<Uuid>,
}

/// What the cards of friends and messages draw with, and when toasts stay
/// away (mouselook, the photo's full frame: nothing over the view).
pub struct ToastEnv<'a> {
    pub pics: &'a HashMap<Uuid, egui::TextureHandle>,
    pub emoji: &'a mut super::emoji::Emoji,
    pub hidden: bool,
    /// Seconds a message toast stays (NotificationToastLifeTime).
    pub message_seconds: f64,
}

/// Show the Conversations window for a clicked card: opened if closed,
/// expanded if minimized (#269), brought to the front. Its tab follows
/// `Social::focus_im`, set by `World::open_from_notification`.
pub fn open_conversations(ctx: &egui::Context, open: &mut bool) {
    super::widgets::open_floater(ctx, "conversations", open);
    ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::new("conversations")));
}

/// How long the toast of `n` stays.
fn toast_seconds(n: &Notification, message_seconds: f64) -> f64 {
    match n.data {
        Data::Presence { .. } => TOAST_TIP,
        Data::Message { .. } => message_seconds,
        _ if n.interactive() => TOAST_OFFER,
        _ => TOAST_INFO,
    }
}

/// Bell button with the unread count; returns true when clicked.
pub fn bell(ui: &mut egui::Ui, p: &Palette, icons: &Icons, unread: usize, open: bool) -> (bool, egui::Rect) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(22.0, 20.0), egui::Sense::click());
    if resp.hovered() || open {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
    }
    let col = if unread > 0 || resp.hovered() || open { p.ink } else { p.muted };
    let icon = if unread > 0 { "bell-ringing-filled" } else { "bell-filled" };
    if let Some(t) = icons.get(icon) {
        let r = egui::Rect::from_center_size(rect.center(), Vec2::splat(16.0));
        ui.painter()
            .image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), col);
    }
    if unread > 0 {
        let text = if unread > 99 { "99+".to_owned() } else { unread.to_string() };
        let galley = ui.painter().layout_no_wrap(text, egui::FontId::proportional(9.0), p.on_accent);
        // round for one digit, a pill beyond
        const H: f32 = 12.0;
        let w = (galley.size().x + 6.0).max(H);
        let badge = egui::Rect::from_min_size(egui::pos2(rect.right() - w + 3.0, rect.top()), Vec2::new(w, H));
        ui.painter().rect_filled(badge, H / 2.0, p.rose);
        super::widgets::paint_ink_centered(ui.painter(), galley, badge.center(), p.on_accent);
    }
    let tip = aurora_i18n::tr!("notifications-unread-count", count = unread);
    (resp.on_hover_text(tip).clicked(), rect)
}

fn kind_style(p: &Palette, kind: Kind) -> (&'static str, Color32) {
    match kind {
        Kind::Info => ("info-circle-filled", p.muted),
        Kind::Alert => ("alert-triangle-filled", p.amber),
        Kind::Teleport => ("plane-departure-filled", p.teal),
        Kind::Friendship => ("heart-handshake", p.violet_light),
        Kind::Inventory => ("gift-filled", p.violet_light),
        Kind::Group => ("users-group", p.indigo_light),
        Kind::Script => ("code", p.teal),
        Kind::Permissions => ("key-filled", p.amber),
        Kind::Url => ("world-filled", p.teal),
        Kind::Presence => ("user-filled", p.success),
        Kind::Message => ("message-filled", p.violet_light),
    }
}

/// The card of a friend online / offline or of the messages of a
/// conversation: their picture, the name, the start of the last message.
/// Clicking the card opens the conversation (`Notification::opens`), its
/// cross hides it.
fn person_card(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    env: &mut ToastEnv<'_>,
    wanted: &mut HashSet<Uuid>,
    n: &Notification,
    toast: bool,
) -> Option<Response> {
    let (who, status, count) = match n.data {
        Data::Presence { who, online } => (who, Some(if online { Status::Online } else { Status::Offline }), 1),
        Data::Message { from, count, .. } => (from, None, count),
        _ => return None,
    };
    let mut close = egui::Rect::NOTHING;
    let surface = if toast { p.panel } else { p.raised.gamma_multiply(0.55) };
    let body = |ui: &mut egui::Ui| {
        ui.set_width(WIDTH - 18.0);
        ui.horizontal_top(|ui| {
            let (pic, _) = ui.allocate_exact_size(Vec2::splat(PIC), egui::Sense::hover());
            wanted.insert(who);
            super::widgets::avatar_disc(ui, p, pic, env.pics.get(&who), &n.title, false);
            if let Some(s) = status {
                super::presence::paint_dot(ui.painter(), p, pic, s, surface);
            }
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let w = (ui.available_width() - 62.0).max(40.0);
                    let title = if count > 1 {
                        format!("{} ({count})", n.title)
                    } else {
                        n.title.clone()
                    };
                    ui.allocate_ui(Vec2::new(w, 18.0), |ui| {
                        ui.add(egui::Label::new(RichText::new(&title).size(12.5).strong().color(p.ink)).truncate());
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (r, resp) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
                        close = r;
                        if resp.hovered() {
                            ui.painter().rect_filled(r, ui.visuals().window_corner_radius, p.raised);
                        }
                        if let Some(t) = icons.get("x") {
                            ui.painter().image(
                                t.id(),
                                egui::Rect::from_center_size(r.center(), Vec2::splat(11.0)),
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                if resp.hovered() { p.ink } else { p.muted },
                            );
                        }
                        ui.label(RichText::new(super::chat::hhmm(n.time)).size(10.5).color(p.muted_dim));
                    });
                });
                if !n.body.is_empty() {
                    let mut text: String = n.body.chars().take(MESSAGE_CHARS).collect();
                    if n.body.chars().count() > MESSAGE_CHARS {
                        text.push('…');
                    }
                    env.emoji.rich_text(ui, &text, 12.0, p.muted, false);
                }
            });
        });
    };
    let frame = if toast {
        super::widgets::overlay(ui, p, egui::Margin::same(8), body).response
    } else {
        egui::Frame::new()
            .fill(surface)
            .corner_radius(ui.visuals().window_corner_radius)
            .inner_margin(egui::Margin::same(8))
            .show(ui, body)
            .response
    };
    // the whole card answers the click, over its text: the cross hides it,
    // the rest opens what it is about
    let cover = ui.interact(frame.rect, egui::Id::new(("notification_card", n.id, toast)), egui::Sense::click());
    let on_close = cover.hover_pos().is_some_and(|pos| close.contains(pos));
    if n.opens() && !on_close && cover.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let tip = if on_close {
        if toast {
            aurora_i18n::tr!("ui-notifications-hide-keep-in-bell")
        } else {
            aurora_i18n::tr!("common-close")
        }
    } else if matches!(n.data, Data::Message { .. }) {
        aurora_i18n::tr!("ui-notifications-open-conversation")
    } else if n.opens() {
        aurora_i18n::tr!("ui-notifications-send-im")
    } else {
        ""
    };
    let cover = if tip.is_empty() { cover } else { cover.on_hover_text(tip) };
    if cover.clicked() {
        return Some(if on_close || !n.opens() { Response::Dismiss } else { Response::Open });
    }
    None
}

/// One notification card; returns the user's answer.
fn card(ui: &mut egui::Ui, p: &Palette, icons: &Icons, n: &mut Notification, toast: bool) -> Option<Response> {
    let mut out = None;
    let (icon, accent) = kind_style(p, n.kind);
    // in the list, an unread card is lighter than a read one
    let fill = if n.read {
        p.field.gamma_multiply(0.8)
    } else {
        p.raised.gamma_multiply(0.55)
    };
    let script = matches!(n.data, Data::Dialog { .. });
    let body = |ui: &mut egui::Ui| {
        {
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
                        ui.painter().rect_filled(r, ui.visuals().window_corner_radius, p.raised);
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
                        aurora_i18n::tr!("ui-notifications-ignore-this-menu")
                    } else if toast {
                        aurora_i18n::tr!("ui-notifications-hide-keep-in-bell")
                    } else if n.close_declines() {
                        aurora_i18n::tr!("ui-notifications-close-decline-offer")
                    } else {
                        aurora_i18n::tr!("common-close")
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
                // drawn by `person_card`
                Data::None | Data::Presence { .. } | Data::Message { .. } => {}
                Data::Lure { change_rating, .. } => {
                    let accept = if *change_rating {
                        aurora_i18n::tr!("ui-chat-invite-change-and-continue")
                    } else {
                        aurora_i18n::tr!("common-teleport-3")
                    };
                    buttons.push((accept.into(), Response::Accept, true));
                    buttons.push((aurora_i18n::tr!("ui-notifications-decline").into(), Response::Decline, false));
                }
                // TeleportRequest: Yes offers a teleport, No sends nothing
                Data::TeleportRequest { .. } => {
                    buttons.push((aurora_i18n::tr!("common-yes").into(), Response::Accept, true));
                    buttons.push((aurora_i18n::tr!("common-no").into(), Response::Decline, false));
                }
                Data::Friend { .. } | Data::Group { .. } => {
                    buttons.push((aurora_i18n::tr!("ui-notifications-accept").into(), Response::Accept, true));
                    buttons.push((aurora_i18n::tr!("ui-notifications-decline").into(), Response::Decline, false));
                }
                Data::Inventory { .. } => {
                    buttons.push((aurora_i18n::tr!("ui-notifications-keep").into(), Response::Accept, true));
                    buttons.push((aurora_i18n::tr!("ui-notifications-reject").into(), Response::Decline, false));
                }
                // ScriptQuestionExperience: Yes, No, Block object (Mute),
                // Block experience; the experience's name opens its profile
                Data::Permissions {
                    experience: Some(prompt), ..
                } => {
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.add_space(22.0);
                        let link = ui
                            .add(
                                egui::Label::new(
                                    RichText::new(aurora_i18n::tr!("ui-notifications-experience-profile"))
                                        .size(12.0)
                                        .underline()
                                        .color(p.teal),
                                )
                                .sense(egui::Sense::click()),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(&prompt.name);
                        if link.clicked() {
                            out = Some(Response::OpenExperience(prompt.id));
                        }
                    });
                    buttons.push((aurora_i18n::tr!("common-yes").into(), Response::Accept, true));
                    buttons.push((aurora_i18n::tr!("common-no").into(), Response::Decline, false));
                    buttons.push((aurora_i18n::tr!("ui-notifications-block-object").into(), Response::Block, false));
                    buttons.push((
                        aurora_i18n::tr!("ui-notifications-block-experience").into(),
                        Response::BlockExperience,
                        false,
                    ));
                }
                Data::Permissions { .. } => {
                    buttons.push((aurora_i18n::tr!("ui-notifications-allow").into(), Response::Accept, true));
                    buttons.push((aurora_i18n::tr!("ui-notifications-decline").into(), Response::Decline, false));
                }
                Data::Url(_) => buttons.push((aurora_i18n::tr!("ui-notifications-open-link").into(), Response::OpenUrl, true)),
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
                                if button_sized(ui, p, &b[index], false, Some(Vec2::new(width, 24.0))).clicked() {
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
                            if button_sized(ui, p, aurora_i18n::tr!("common-ignore"), false, Some(Vec2::new(72.0, 22.0))).clicked() {
                                out = Some(Response::Dismiss);
                            }
                            if !own_object
                                && button_sized(ui, p, aurora_i18n::tr!("common-block"), false, Some(Vec2::new(72.0, 22.0))).clicked()
                            {
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
                                .hint_text(aurora_i18n::tr!("ui-notifications-reply"))
                                .desired_width(WIDTH - 110.0),
                        );
                        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if button(ui, p, aurora_i18n::tr!("common-send"), true).clicked() || enter {
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
        }
    };
    if toast {
        // a toast floats over the world: the windows' glass
        super::widgets::overlay(ui, p, egui::Margin::same(8), body);
    } else {
        egui::Frame::new()
            .fill(fill)
            .corner_radius(ui.visuals().window_corner_radius)
            .inner_margin(egui::Margin::same(8))
            .show(ui, body);
    }
    out
}

fn button(ui: &mut egui::Ui, p: &Palette, label: &str, primary: bool) -> egui::Response {
    button_sized(ui, p, label, primary, None)
}

/// Button of a card: a plain filled face for every kind of notification, the
/// accent for the expected answer. A sized button cuts a long script label
/// and shows it whole when hovered.
fn button_sized(ui: &mut egui::Ui, p: &Palette, label: &str, primary: bool, size: Option<Vec2>) -> egui::Response {
    let button = egui::Button::new(RichText::new(label).size(12.0).color(if primary { p.on_accent } else { p.ink }))
        .fill(if primary { p.violet } else { p.raised })
        .corner_radius(ui.visuals().widgets.inactive.corner_radius);
    match size {
        Some(size) => ui.add_sized(size, button.truncate()).on_hover_text(label),
        None => ui.add(button.min_size(Vec2::new(72.0, 22.0))),
    }
}

/// The toasts that new arrivals start and those that end. A toast-only
/// notice (a friend online) leaves the list with its toast.
fn update_toasts(list: &mut Notifications, nui: &mut NotifUi, now: f64, message_seconds: f64, hidden: bool) {
    // new arrivals become toasts (unless the list is open or the view must
    // stay clear, or they were already seen where they arrived: a card in
    // the conversation in sight)
    for n in list.list.iter().filter(|n| n.id > nui.last_seen) {
        if !nui.open && !hidden && !n.read {
            nui.toasts.push((n.id, now + toast_seconds(n, message_seconds)));
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
    let toasts = &nui.toasts;
    list.list.retain(|n| !n.transient || toasts.iter().any(|(id, _)| *id == n.id));
}

/// List under the bell and toasts; returns the answers to apply.
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    icons: &Icons,
    list: &mut Notifications,
    nui: &mut NotifUi,
    env: &mut ToastEnv<'_>,
) -> Vec<(u64, Response)> {
    let mut answers = Vec::new();
    let now = ctx.input(|i| i.time);
    update_toasts(list, nui, now, env.message_seconds, env.hidden);

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
                super::widgets::overlay(ui, p, egui::Margin::same(6), |ui| {
                    {
                        ui.set_width(WIDTH);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(aurora_i18n::tr!("ui-notifications-notifications"))
                                    .size(13.0)
                                    .strong()
                                    .color(p.ink),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let any_info = list.list.iter().any(|n| !n.interactive());
                                if ui
                                    .add_enabled(
                                        any_info,
                                        egui::Button::new(
                                            RichText::new(aurora_i18n::tr!("ui-notifications-clear-all"))
                                                .size(11.5)
                                                .color(p.muted),
                                        )
                                        .frame(false),
                                    )
                                    .on_hover_text(aurora_i18n::tr!(
                                        "ui-notifications-clear-notifications-that-are-not-awaiting-a-response"
                                    ))
                                    .clicked()
                                {
                                    list.clear_info();
                                }
                            });
                        });
                        ui.add_space(4.0);
                        if list.listed().next().is_none() {
                            ui.add_space(14.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new(aurora_i18n::tr!("ui-notifications-no-notifications"))
                                        .size(12.0)
                                        .color(p.muted_dim),
                                );
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
                                for n in list.list.iter_mut().rev().filter(|n| !n.transient) {
                                    let r = if matches!(n.data, Data::Message { .. }) {
                                        person_card(ui, p, icons, env, &mut nui.wanted_pics, n, false)
                                    } else {
                                        card(ui, p, icons, n, false)
                                    };
                                    if let Some(r) = r {
                                        answers.push((n.id, r));
                                    }
                                }
                            });
                    }
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
    } else if !nui.toasts.is_empty() && !env.hidden {
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
                        let r = if matches!(n.data, Data::Presence { .. } | Data::Message { .. }) {
                            person_card(ui, p, icons, env, &mut nui.wanted_pics, n, true)
                        } else {
                            card(ui, p, icons, n, true)
                        };
                        match r {
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

    const JANE: Uuid = Uuid::from_u128(2);

    /// A friend's notice lives as long as its toast; a message's entry
    /// stays under the bell once its toast is gone.
    #[test]
    fn a_friend_toast_leaves_with_its_toast_and_a_message_stays_listed() {
        let mut list = Notifications::default();
        let mut nui = NotifUi::default();
        let friend = list.push_presence(JANE, "Jane Doe is online.", true);
        let message = list.push_message(JANE, JANE, "Jane Doe", "hello");
        update_toasts(&mut list, &mut nui, 0.0, 30.0, false);
        assert_eq!(nui.toasts.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![friend, message]);
        // the friend's toast ends after TOAST_TIP, the message's after 30 s
        update_toasts(&mut list, &mut nui, TOAST_TIP + 1.0, 30.0, false);
        assert!(!list.list.iter().any(|n| n.id == friend));
        assert!(list.list.iter().any(|n| n.id == message));
        update_toasts(&mut list, &mut nui, 31.0, 30.0, false);
        assert!(nui.toasts.is_empty());
        assert_eq!(list.listed().map(|n| n.id).collect::<Vec<_>>(), vec![message]);
        assert_eq!(list.unread(), 1);
    }

    /// In mouselook or the photo's full frame no toast starts: a friend's
    /// notice is dropped, a message waits under the bell.
    #[test]
    fn hidden_toasts_drop_friends_and_keep_messages() {
        let mut list = Notifications::default();
        let mut nui = NotifUi::default();
        list.push_presence(JANE, "Jane Doe is online.", true);
        let message = list.push_message(JANE, JANE, "Jane Doe", "hello");
        update_toasts(&mut list, &mut nui, 0.0, 30.0, true);
        assert!(nui.toasts.is_empty());
        assert_eq!(list.list.iter().map(|n| n.id).collect::<Vec<_>>(), vec![message]);
    }

    /// Run the Conversations floater for a few frames.
    fn frames(ctx: &egui::Context, open: &mut bool, n: usize) {
        let p = crate::theme::Theme::default().palette();
        for i in 0..n {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                    time: Some(i as f64 / 60.0),
                    ..Default::default()
                },
                |ui| {
                    super::super::widgets::Floater::new("conversations", "Conversations", egui::pos2(300.0, 200.0), egui::vec2(440.0, 300.0))
                        .silent()
                        .show(ui.ctx(), &p, open, |ui| {
                            ui.label("lines");
                        });
                },
            );
            out.textures_delta.clear();
        }
    }

    /// A clicked card shows the Conversations window from closed and from
    /// minimized (#269): open, expanded.
    #[test]
    fn a_clicked_card_opens_and_expands_the_conversations() {
        // closed
        let ctx = egui::Context::default();
        let mut open = false;
        frames(&ctx, &mut open, 2);
        open_conversations(&ctx, &mut open);
        frames(&ctx, &mut open, 4);
        assert!(open && !super::super::widgets::floater_minimized(&ctx, "conversations"));
        // minimized
        let ctx = egui::Context::default();
        let mut open = true;
        frames(&ctx, &mut open, 4);
        super::super::widgets::minimize_floater(&ctx, "conversations");
        frames(&ctx, &mut open, 4);
        assert!(super::super::widgets::floater_minimized(&ctx, "conversations"));
        open_conversations(&ctx, &mut open);
        frames(&ctx, &mut open, 4);
        assert!(open && !super::super::widgets::floater_minimized(&ctx, "conversations"));
    }
}
