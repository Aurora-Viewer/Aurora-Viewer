//! Notification center: bell of the navigation bar (unread badge), the
//! list it opens and the toasts shown when something arrives — the
//! Firestorm notification well / toasts, flattened to Aurora cards.
//! Script grids follow LLToastNotifyPanel::updateButtonsLayout
//! (indra/newview/lltoastnotifypanel.cpp, originally LGPL 2.1).

use super::icons::Icons;
use super::presence::Status;
use crate::theme::Palette;
use crate::world::World;
use crate::world::notifications::{Data, Kind, Notification, Notifications, Response};
use egui::{Color32, RichText, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub(crate) mod text;

/// Seconds a toast stays: information / waiting for an answer.
const TOAST_INFO: f64 = 7.0;
const TOAST_OFFER: f64 = 25.0;
const MAX_TOASTS: usize = 3;
const WIDTH: f32 = 320.0;
/// Side of the picture of a friend's or a message's card.
const PIC: f32 = 32.0;
/// Characters of a message shown on its card (the rest is in the conversation).
const MESSAGE_CHARS: usize = 160;
/// Between the rows inside a toast's card. The gap between two cards is
/// ToastGap (`ToastEnv::toast_gap`), and does not reach inside them.
pub(crate) const CARD_ROW_SPACING: f32 = 6.0;

#[derive(Default)]
pub struct NotifUi {
    pub privacy: crate::world::rlva::privacy::TextFilter,
    pub open: bool,
    /// Bottom-left of the bell (where the list opens).
    pub anchor: Option<egui::Pos2>,
    /// (notification id, expiry time)
    toasts: Vec<(u64, f64)>,
    /// The scripted windows whose toast the user hid with its cross: they
    /// wait in the list, not brought back (`update_toasts`).
    hidden: HashSet<u64>,
    last_seen: u64,
    /// Profile pictures the cards of friends and messages want.
    pub wanted_pics: HashSet<Uuid>,
    /// Textures the cards want: the insignias of the group notices' groups.
    pub wanted_images: HashSet<Uuid>,
    /// The bell's rectangle (Aurora's center unrolls under it).
    pub bell: Option<egui::Rect>,
    /// With the islands, from the system island's left edge to the bell
    /// island's right edge, as drawn this frame: Aurora's center (its list,
    /// its cards, the friends' toasts at the top right) stands exactly under
    /// it. `None` with Firestorm's bars.
    pub column: Option<egui::Rect>,
    /// Aurora's center (`ui::notification_center`).
    pub center: super::notification_center::CenterUi,
}

/// What the cards of friends and messages draw with, and when toasts stay
/// away (mouselook, the photo's full frame: nothing over the view).
pub struct ToastEnv<'a> {
    pub pics: &'a HashMap<Uuid, egui::TextureHandle>,
    /// Interface textures by id (the group notices' insignias).
    pub images: &'a HashMap<Uuid, egui::TextureHandle>,
    pub emoji: &'a mut super::emoji::Emoji,
    pub hidden: bool,
    /// Seconds a message toast stays (NotificationToastLifeTime).
    pub message_seconds: f64,
    /// Seconds a friend's online / offline toast stays
    /// (NotificationTipToastLifeTime): these toasts only, Aurora's center
    /// keeps its own timing (`notification_center::PRESENCE`).
    pub tip_seconds: f64,
    /// Points between two stacked toasts (ToastGap, `settings::toast_gap`),
    /// in both styles: the Legacy toasts, the cards under Aurora's bell
    /// and its friends' toasts.
    pub toast_gap: f32,
    /// The region restart counting down: its card in Aurora's center shows
    /// the live count (`ui::region_restart::bell_card`).
    pub restart: Option<super::region_restart::BellInfo>,
    /// Where Aurora's center shows the friends' online / offline toasts
    /// (`notification_center::presence`), and whether Firestorm's
    /// full-width bars are shown (their room has no margin of its own).
    pub presence_at: crate::settings::PresencePosition,
    pub classic_bars: bool,
}

/// Show the Conversations window for a clicked card: opened if closed,
/// expanded if minimized (#269), brought to the front. Its tab follows
/// `Social::focus_im`, set by `World::open_from_notification`.
pub fn open_conversations(ctx: &egui::Context, open: &mut bool) {
    super::widgets::open_floater(ctx, "conversations", open);
    ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("conversations")));
}

/// How long the toast of `n` stays.
fn toast_seconds(n: &Notification, message_seconds: f64, tip_seconds: f64) -> f64 {
    match n.data {
        // LLNotificationTipHandler::processNotification
        Data::Presence { .. } => tip_seconds,
        Data::Message { .. } => message_seconds,
        // ShowMyComplexityChanges (the notice's expiry)
        Data::Complexity { seconds } => f64::from(seconds),
        _ if n.interactive() => TOAST_OFFER,
        _ => TOAST_INFO,
    }
}

/// Bell button with the unread count, in an island of the Aurora bars
/// (`island`) or in the Legacy Viewer's top bar; in do not disturb the
/// paused bell of Aurora's center (`bell-z`, grey). Returns its response: a
/// click opens the notifications, a right click turns do not disturb on or
/// off (#491).
pub fn bell(ui: &mut egui::Ui, p: &Palette, icons: &Icons, unread: usize, open: bool, dnd: bool, island: bool) -> egui::Response {
    let tip = bell_tip(aurora_i18n::tr!("notifications-unread-count", count = unread), dnd);
    if island {
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(super::bars::islands::SLOT), egui::Sense::click());
        let hot = resp.hovered() || open;
        if hot {
            ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
        }
        let icon = if dnd {
            "bell-z-filled"
        } else if unread > 0 {
            "bell-ringing-filled"
        } else {
            "bell-filled"
        };
        if let Some(t) = icons.get(icon) {
            let r = egui::Rect::from_center_size(rect.center(), Vec2::splat(16.0));
            ui.painter().image(
                t.id(),
                r,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                if dnd { p.muted } else { p.ink },
            );
        }
        if unread > 0 {
            super::bars::islands::count(ui, p, rect, unread, 1.0, dnd);
        }
        return resp.on_hover_text(tip);
    }
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(22.0, 20.0), egui::Sense::click());
    if resp.hovered() || open {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
    }
    let col = if !dnd && (unread > 0 || resp.hovered() || open) {
        p.ink
    } else {
        p.muted
    };
    let icon = if dnd {
        "bell-z-filled"
    } else if unread > 0 {
        "bell-ringing-filled"
    } else {
        "bell-filled"
    };
    if let Some(t) = icons.get(icon) {
        let r = egui::Rect::from_center_size(rect.center(), Vec2::splat(16.0));
        ui.painter()
            .image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), col);
    }
    if unread > 0 {
        // the interface's one counter, 3 px past the button's right edge
        let anchor = rect.right_top() + Vec2::new(3.0, 0.0);
        super::widgets::unread_count(ui.painter(), p, anchor, unread, super::widgets::CountSize::Bar);
    }
    resp.on_hover_text(tip)
}

/// The bell's tooltip: `main`, then what its right click does (#491).
pub fn bell_tip(main: String, dnd: bool) -> String {
    let hint = if dnd {
        aurora_i18n::tr!("ui-notifications-bell-dnd-off-hint")
    } else {
        aurora_i18n::tr!("ui-notifications-bell-dnd-on-hint")
    };
    format!("{main}\n{hint}")
}

/// The online status a right click on the bell asks for (#491): do not
/// disturb turned over, the rest as it is, applied like a choice of the
/// status selector (`BarAction::SetStatus`) so that both and the server's
/// animation stay in step.
pub fn bell_dnd_toggled(status: crate::world::status::StatusModes) -> crate::world::status::StatusModes {
    crate::world::status::StatusModes {
        dnd: !status.dnd,
        ..status
    }
}

pub(super) fn kind_style(p: &Palette, kind: Kind) -> (&'static str, Color32) {
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
        Kind::Message => ("message-circle-filled", p.violet_light),
        Kind::Call => ("phone-filled", p.success),
    }
}

/// The icon and colour of a history entry's kind.
pub(super) fn look_style(p: &Palette, look: crate::world::notification_history::Look) -> (&'static str, Color32) {
    kind_style(p, Kind::of_look(look))
}

/// The start of a message on its card: its links by their labels first,
/// then cut, so never inside an address (#947).
fn message_excerpt(world: &World, body: &str) -> String {
    let body = super::chat::resolved_text_in(world, Kind::Message.text(), body);
    let mut text: String = body.chars().take(MESSAGE_CHARS).collect();
    if body.chars().count() > MESSAGE_CHARS {
        text.push('…');
    }
    text
}

/// The width of a card's time ("20:41", or "8:41 PM" in 12 hours), so its
/// title is cut before it.
pub(super) fn time_width(ui: &egui::Ui, time: &str) -> f32 {
    ui.fonts_mut(|f| {
        f.layout_no_wrap(time.to_owned(), egui::FontId::proportional(10.5), Color32::PLACEHOLDER)
            .size()
            .x
    })
}

/// The card of a friend online / offline or of the messages of a
/// conversation: their picture, the name, the start of the last message.
/// Clicking the card opens the conversation (`Notification::opens`), its
/// cross hides it.
pub(super) fn person_card(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    world: &World,
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
    let mut name_hover = None;
    let surface = if toast { p.panel } else { p.raised.gamma_multiply(0.55) };
    // a toast floats at its own width, a card fills its list
    let width = if toast { WIDTH - 18.0 } else { ui.available_width() - 16.0 };
    let body = |ui: &mut egui::Ui| {
        ui.set_width(width);
        ui.horizontal_top(|ui| {
            let (pic, _) = ui.allocate_exact_size(Vec2::splat(PIC), egui::Sense::hover());
            wanted.insert(who);
            super::widgets::avatar_disc(ui, p, pic, env.pics.get(&who), &n.title, false);
            if let Some(s) = status {
                // cut in the card's face as it shows over the panel (#744)
                super::presence::paint_dot(ui.painter(), p, pic, s, p.panel.blend(surface));
            }
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let time = super::chat::hhmm(n.time);
                    let w = (ui.available_width() - 34.0 - time_width(ui, &time)).max(40.0);
                    let title = if count > 1 {
                        format!("{} ({count})", n.title)
                    } else {
                        n.title.clone()
                    };
                    ui.allocate_ui(Vec2::new(w, 18.0), |ui| {
                        name_hover = text::show(ui, p, &title, &n.name_links, 12.5, p.ink, true, false, n.kind.names());
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
                        ui.label(RichText::new(time).size(10.5).color(p.muted_dim));
                    });
                });
                let shown = world.notification_body(n);
                if !shown.is_empty() {
                    let text = message_excerpt(world, &shown);
                    name_hover = name_hover.or(text::person_body(ui, p, env.emoji, &text, &n.name_links, n.kind.names()));
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
    let cover = ui.interact(
        frame.rect,
        egui::Id::unique(("notification_card", n.id, toast)),
        egui::Sense::click(),
    );
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
        if !on_close && let Some(target) = name_hover {
            text::open(ui.ctx(), n.kind.names(), target);
            return None;
        }
        return Some(if on_close || !n.opens() {
            Response::Dismiss
        } else {
            Response::Open
        });
    }
    None
}

/// One notification card; returns the user's answer. `aurora`: the card of
/// Aurora's center (its kind in a coloured disc beside the text, the width
/// of its panel). A group notice shows its group's insignia instead of the
/// kind (LLToastGroupNotifyPanel's group_icon), from `images` once loaded
/// (asked for in `wanted`), and its attachment to save.
pub(super) fn card(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    world: &World,
    images: &HashMap<Uuid, egui::TextureHandle>,
    wanted: &mut HashSet<Uuid>,
    n: &mut Notification,
    toast: bool,
    aurora: bool,
) -> Option<Response> {
    let mut out = None;
    // a new version: the updates' icon (the menu's "Check for updates")
    let update = matches!(n.data, Data::Update { .. });
    let (icon, accent) = if update {
        ("download-filled", p.violet)
    } else {
        kind_style(p, n.kind)
    };
    let insignia = match &n.data {
        Data::GroupNotice { group, .. } => super::chat::group_insignia(world, group),
        _ => None,
    };
    if let Some(i) = insignia {
        wanted.insert(i);
    }
    let picture = insignia.and_then(|i| images.get(&i));
    // in the list, an unread card is lighter than a read one
    let fill = if aurora {
        p.field
    } else if n.read {
        p.field.gamma_multiply(0.8)
    } else {
        p.raised.gamma_multiply(0.55)
    };
    let script = matches!(n.data, Data::Dialog { .. });
    // narrower in the list beside its scroll bar's strip, when it shows
    let width = if aurora {
        ui.available_width() - 16.0
    } else {
        (WIDTH - 18.0).min(ui.available_width() - 16.0)
    };
    // the text and the answers line up after the kind's icon
    let indent = if aurora { 0.0 } else { 22.0 };
    let mut content = |ui: &mut egui::Ui| {
        {
            if script {
                ui.spacing_mut().interact_size.y = 24.0;
            }
            ui.horizontal(|ui| {
                if !aurora {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::hover());
                    if let Some(t) = picture {
                        super::widgets::square_picture(ui, r, t);
                    } else if let Some(t) = icons.get(icon) {
                        ui.painter().image(
                            t.id(),
                            egui::Rect::from_center_size(r.center(), Vec2::splat(16.0)),
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            accent,
                        );
                    }
                }
                // the rating of the region the entry is about
                if n.maturity != 0 {
                    super::widgets::maturity_badge_of(ui, p, n.maturity, 14.0);
                }
                // keep room for the time and the close button
                let time = super::chat::hhmm(n.time);
                let w = (ui.available_width() - 34.0 - time_width(ui, &time)).max(40.0);
                ui.allocate_ui(Vec2::new(w, 18.0), |ui| {
                    if update && aurora {
                        let title = super::notification_center::aurora::title(ui, p, &n.title, 12.5);
                        ui.add(egui::Label::new(title).wrap());
                    } else {
                        text::show(ui, p, &n.title, &n.name_links, 12.5, p.ink, true, true, n.kind.names());
                    }
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
                    ui.label(RichText::new(time).size(10.5).color(p.muted_dim));
                });
            });
            let shown = world.notification_body(n);
            if !shown.is_empty() {
                ui.add_space(2.0);
                // selectable: Firestorm shows the body in a read-only
                // LLTextEditor (lltoastnotifypanel.cpp, "text_editor_box")
                if script {
                    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        super::chat::text_with_links(ui, p, world, n.kind.text(), &shown, &n.name_links, 12.0, p.ink);
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.add_space(indent);
                        super::chat::text_with_links(ui, p, world, n.kind.text(), &shown, &n.name_links, 12.0, p.muted);
                    });
                }
            }
            let mut buttons: Vec<(String, Response, bool)> = Vec::new();
            match &n.data {
                // drawn by `person_card`; the restart's live card by
                // `region_restart::bell_card`, its text alone here
                Data::None
                | Data::Presence { .. }
                | Data::Message { .. }
                | Data::RegionRestart
                | Data::RegionMessage { .. }
                | Data::Complexity { .. } => {}
                // the attachment as in the group's Notices tab, saved from
                // here (LLToastGroupNotifyPanel::onClickAttachment)
                Data::GroupNotice { attachment: Some(a), .. } => {
                    ui.add_space(6.0);
                    // the card's width: the frame's content is laid out in
                    // a column of that width (in the indent's row it would
                    // follow the row and grow with its content)
                    let inner = ui.available_width() - indent - 16.0;
                    ui.horizontal(|ui| {
                        ui.add_space(indent);
                        egui::Frame::new()
                            .fill(p.bar)
                            .corner_radius(ui.visuals().widgets.inactive.corner_radius)
                            .inner_margin(egui::Margin::symmetric(8, 5))
                            .show(ui, |ui| {
                                ui.vertical(|ui| {
                                    ui.set_width(inner);
                                    let (icon, label) = if a.saved {
                                        ("check", aurora_i18n::tr!("ui-group-notices-saved"))
                                    } else {
                                        ("download-filled", aurora_i18n::tr!("ui-group-notices-save"))
                                    };
                                    // beside the name while it leaves the name
                                    // room, else under it (a long translation)
                                    let button =
                                        super::widgets::action_button_keyed_content(ui, icon, label, None) + super::widgets::KEYED_PAD;
                                    let inline = inner - 22.0 - button >= 110.0;
                                    let mut save = |ui: &mut egui::Ui| {
                                        let save = super::widgets::action_button_if(
                                            ui,
                                            p,
                                            icon,
                                            label,
                                            super::widgets::ActionKind::Flat,
                                            Vec2::new(0.0, 24.0),
                                            !a.saved,
                                        );
                                        if save.clicked() {
                                            out = Some(Response::Accept);
                                        }
                                    };
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 7.0;
                                        super::widgets::glyph(ui, super::group_profile::attachment_icon(a.asset_type), p.amber, 15.0);
                                        let w = if inline { inner - 22.0 - button - 8.0 } else { inner - 22.0 };
                                        ui.allocate_ui(Vec2::new(w, 28.0), |ui| {
                                            ui.vertical(|ui| {
                                                ui.spacing_mut().item_spacing.y = 0.0;
                                                ui.add(
                                                    egui::Label::new(RichText::new(&a.name).size(12.0).strong().color(p.ink)).truncate(),
                                                )
                                                .on_hover_text(&a.name);
                                                ui.label(
                                                    RichText::new(super::group_profile::asset_label(a.asset_type))
                                                        .size(11.0)
                                                        .color(p.muted),
                                                );
                                            });
                                        });
                                        if inline {
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| save(ui));
                                        }
                                    });
                                    if !inline {
                                        ui.horizontal(|ui| {
                                            ui.add_space(22.0);
                                            save(ui);
                                        });
                                    }
                                });
                            });
                    });
                }
                Data::GroupNotice { .. } => {}
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
                // LLIncomingCallDialog: Accept, Reject, and Start IM for a
                // resident's call
                Data::Call { invite } => {
                    buttons.push((aurora_i18n::tr!("ui-notifications-accept").into(), Response::Accept, true));
                    buttons.push((aurora_i18n::tr!("ui-notifications-decline").into(), Response::Decline, false));
                    if invite.p2p {
                        buttons.push((
                            aurora_i18n::tr!("ui-notifications-call-start-im").into(),
                            Response::Button(0),
                            false,
                        ));
                    }
                }
                Data::RlvaQuery { .. } => {
                    buttons.push((aurora_i18n::tr!("rlva-im-share-once").into(), Response::Accept, true));
                    buttons.push((aurora_i18n::tr!("rlva-im-share-always").into(), Response::Button(1), false));
                    buttons.push((aurora_i18n::tr!("ui-notifications-decline").into(), Response::Decline, false));
                }
                // JoinGroupCanAfford: Join (the cost was named) or Cancel
                Data::Group { confirming: true, .. } => {
                    buttons.push((aurora_i18n::tr!("ui-group-profile-join").into(), Response::Accept, true));
                    buttons.push((aurora_i18n::tr!("common-cancel").into(), Response::Decline, false));
                }
                // an invitation with a fee names it on its button
                Data::Group { fee, .. } if *fee > 0 => {
                    buttons.push((
                        aurora_i18n::tr!("ui-group-profile-join-fee", fee = aurora_i18n::integer(i64::from(*fee))),
                        Response::Accept,
                        true,
                    ));
                    buttons.push((aurora_i18n::tr!("ui-notifications-decline").into(), Response::Decline, false));
                }
                Data::Friend { .. } | Data::Group { .. } | Data::CallingCard { .. } => {
                    buttons.push((aurora_i18n::tr!("ui-notifications-accept").into(), Response::Accept, true));
                    buttons.push((aurora_i18n::tr!("ui-notifications-decline").into(), Response::Decline, false));
                }
                Data::Inventory {
                    from,
                    task: true,
                    task_offer,
                    ..
                } => {
                    buttons.push((aurora_i18n::tr!("ui-notifications-accept").into(), Response::Accept, true));
                    buttons.push((aurora_i18n::tr!("ui-notifications-discard").into(), Response::Decline, false));
                    if !from.is_nil() && task_offer.as_ref().is_some_and(|offer| !offer.own_object) {
                        buttons.push((aurora_i18n::tr!("ui-notifications-mute-owner").into(), Response::Block, false));
                    }
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
                        ui.add_space(indent);
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
                // ScriptQuestionCaution: Deny is the default, Allow access
                // a plain choice beside it
                Data::Permissions { caution: Some(_), .. } => {
                    buttons.push((aurora_i18n::tr!("ui-notifications-deny").into(), Response::Decline, true));
                    buttons.push((aurora_i18n::tr!("ui-notifications-allow-access").into(), Response::Accept, false));
                }
                Data::Permissions { .. } => {
                    buttons.push((aurora_i18n::tr!("ui-notifications-allow").into(), Response::Accept, true));
                    buttons.push((aurora_i18n::tr!("ui-notifications-decline").into(), Response::Decline, false));
                }
                Data::Url(_) => buttons.push((aurora_i18n::tr!("ui-notifications-open-link").into(), Response::OpenUrl, true)),
                Data::Delivery { .. } => {
                    buttons.push((aurora_i18n::tr!("ui-notifications-show-in-inventory").into(), Response::Open, true));
                }
                // Update downloads it in the update window; once downloaded,
                // Install opens that window, which says what installing does
                Data::Update { ready, .. } => {
                    let primary = if *ready {
                        aurora_i18n::tr!("update-notice-install")
                    } else {
                        aurora_i18n::tr!("update-install")
                    };
                    buttons.push((primary.into(), Response::Accept, true));
                    buttons.push((aurora_i18n::tr!("update-later").into(), Response::Decline, false));
                }
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
                    // LLToastScriptTextbox (panel_notify_textbox.xml): a
                    // multi-line editor three lines tall that scrolls beyond
                    // them. Enter starts a new line; only the button sends.
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(indent);
                        text_box_field(ui, n.id, &mut n.text);
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(indent);
                        if button(ui, p, aurora_i18n::tr!("common-send"), true).clicked() {
                            out = Some(Response::SendText);
                        }
                    });
                }
            }
            if !buttons.is_empty() {
                // Indent the whole wrap column: a leading `add_space` inside
                // `horizontal_wrapped` only shifts the first row, so a long
                // Mute Owner label in French started left of the card text
                // (#2932). Nesting the wrap after the indent keeps every row
                // aligned with the body.
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(indent);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                        for (label, r, primary) in buttons {
                            if button(ui, p, &label, primary).clicked() {
                                out = Some(r);
                            }
                        }
                    });
                });
            }
        }
    };
    let body = |ui: &mut egui::Ui| {
        ui.set_width(width);
        if aurora {
            ui.horizontal_top(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(30.0), egui::Sense::hover());
                if let Some(t) = picture {
                    super::widgets::square_picture(ui, r, t);
                } else {
                    ui.painter().circle_filled(r.center(), 15.0, accent);
                    if let Some(t) = icons.get(icon) {
                        egui::Image::new(t)
                            .tint(p.on_accent)
                            .paint_at(ui, egui::Rect::from_center_size(r.center(), Vec2::splat(15.0)));
                    }
                }
                ui.vertical(|ui| content(ui));
            });
        } else {
            content(ui);
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

/// Close Window takes the newest visible LLScriptFloater (llDialog, llTextBox,
/// llLoadURL and object inventory offers). Aurora also stacks permission
/// questions here, so their shortcut follows their cross without answering.
/// Cards kept in the list receive Close All only.
pub(super) fn close_script_prompt(ctx: &egui::Context, n: &Notification, window: bool) -> bool {
    if !n.stays_until_answered() {
        return false;
    }
    let id = egui::Id::NULL.with(("script_prompt", n.id));
    // Apply the shortcut to prompts already shown when the key was pressed.
    if super::close_key::take(ctx, id) {
        return true;
    }
    if window {
        super::close_key::shown_prompt(ctx, id, n.id);
    } else {
        super::close_key::shown_extra(ctx, id);
    }
    false
}

/// Lines of an llTextBox reply shown before it scrolls: the 50 pixels of the
/// "message" editor of panel_notify_textbox.xml.
const TEXT_BOX_ROWS: usize = 3;

/// The reply field of an llTextBox prompt, as wide as the card: as high as
/// [`TEXT_BOX_ROWS`] lines, then the text scrolls inside the field, its
/// scroll bar within the frame, like LLTextEditor. The frame is the theme's
/// own text field face (`theme::FieldSize`, hover and focus included), drawn
/// around the scroll area so that it never scrolls away with the text. The
/// text is kept within what the Legacy Viewer's editor accepts
/// (`clamp_text_box_reply`). Returns the frame's rectangle and the editor.
///
/// The field's place is set first, from the card's width, and the field is
/// drawn in a child that does not size the card: the scroll area's content
/// can run a fraction of a point past its width, which grew the card on
/// some frames, and a toast anchored to the right then shimmered sideways.
fn text_box_field(ui: &mut egui::Ui, id: u64, text: &mut String) -> (egui::Rect, egui::Response) {
    let edit_id = egui::Id::unique(("script_text_box", id));
    let style: egui::widget_style::TextEditStyle = ui.widget_style(edit_id, &egui::class::Classes::default());
    let frame = style.atom_layout.frame;
    // a line as TextEdit lays it out: the font's row plus the theme's spacing
    let row = ui.fonts_mut(|f| f.row_height(&egui::TextStyle::Body.resolve(ui.style()))) + ui.spacing().extra_text_line_spacing;
    let rows = row * TEXT_BOX_ROWS as f32;
    let size = Vec2::new(ui.available_width(), rows + frame.total_margin().sum().y);
    let (place, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    // a paste reaches the editor cleaned as LLTextEditor::pasteHelper does it,
    // before egui inserts it, so the cursor lands after the cleaned text
    let focused = ui.memory(|m| m.has_focus(edit_id));
    ui.input_mut(|i| {
        for event in &mut i.events {
            match event {
                egui::Event::Paste(pasted) if focused => crate::world::notifications::clean_text_box_paste(pasted),
                egui::Event::MiddleClickPaste { pos, text } if place.contains(*pos) => {
                    crate::world::notifications::clean_text_box_paste(text)
                }
                _ => {}
            }
        }
    });
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(place)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let shown = frame.show(&mut child, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical()
            .id_salt(edit_id)
            .min_scrolled_height(rows)
            .max_height(rows)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(text)
                        .id(edit_id)
                        .frame(egui::Frame::NONE)
                        .hint_text(aurora_i18n::tr!("ui-notifications-reply"))
                        .desired_rows(TEXT_BOX_ROWS)
                        .desired_width(ui.available_width()),
                )
            })
            .inner
    });
    crate::world::notifications::clamp_text_box_reply(text);
    (place, shown.inner)
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
/// notice (a friend online) leaves the list with its toast. In do not
/// disturb (`dnd`) only the script menus toast, the others wait in the list
/// (LLScreenChannel::addToast; #491). A scripted object's window
/// (`Notification::stays_until_answered`) has no time: its toast stays
/// until answered, whatever comes after it, and is only kept in while the
/// list is open, the view must stay clear or do not disturb holds it.
fn update_toasts(list: &mut Notifications, nui: &mut NotifUi, now: f64, message_seconds: f64, tip_seconds: f64, hidden: bool, dnd: bool) {
    // new arrivals become toasts (unless the list is open or the view must
    // stay clear, or they were already seen where they arrived: a card in
    // the conversation in sight)
    for n in list.list.iter().filter(|n| n.id > nui.last_seen && !n.stays_until_answered()) {
        if !nui.open && !hidden && !n.read && !n.quiet && (!dnd || super::notification_center::dnd_shows(n)) {
            nui.toasts.push((n.id, now + toast_seconds(n, message_seconds, tip_seconds)));
        }
    }
    if let Some(last) = list.list.iter().map(|n| n.id).max() {
        nui.last_seen = nui.last_seen.max(last);
    }
    // a scripted object's window, from its arrival until answered or hidden
    // by its cross (seen in the list or not, it is still waiting)
    nui.hidden.retain(|id| list.list.iter().any(|n| n.id == *id));
    let mut back = false;
    for n in list
        .list
        .iter()
        .filter(|n| n.stays_until_answered() && !n.quiet && !nui.hidden.contains(&n.id))
    {
        if !nui.toasts.iter().any(|(id, _)| *id == n.id) {
            nui.toasts.push((n.id, now));
            back = true;
        }
    }
    if back {
        nui.toasts.sort_by_key(|(id, _)| *id);
    }
    nui.toasts.retain(|(id, until)| {
        list.list
            .iter()
            .any(|n| n.id == *id && (n.stays_until_answered() || (*until > now && (!dnd || super::notification_center::dnd_shows(n)))))
    });
    // the oldest information gives way; a scripted window waits its turn
    // (`shown_toasts`)
    let pinned = |id: u64| list.list.iter().any(|n| n.id == id && n.stays_until_answered());
    while nui.toasts.iter().filter(|(id, _)| !pinned(*id)).count() > MAX_TOASTS {
        if let Some(i) = nui.toasts.iter().position(|(id, _)| !pinned(*id)) {
            nui.toasts.remove(i);
        }
    }
    let toasts = &nui.toasts;
    list.list.retain(|n| !n.transient || toasts.iter().any(|(id, _)| *id == n.id));
}

/// The toasts drawn (oldest first) and how many wait behind them: the
/// scripted windows first (the newest three), the newest others in the
/// room left, as Aurora's center does (`notification_center::visible`). In
/// do not disturb only the script menus come out; the rest waits there.
fn shown_toasts(list: &Notifications, nui: &NotifUi, dnd: bool) -> (Vec<u64>, usize) {
    let drawn: Vec<(u64, bool)> = nui
        .toasts
        .iter()
        .filter_map(|(id, _)| list.list.iter().find(|n| n.id == *id))
        .filter(|n| !dnd || super::notification_center::dnd_shows(n))
        .map(|n| (n.id, n.stays_until_answered()))
        .collect();
    let mut keep: Vec<u64> = drawn.iter().rev().filter(|t| t.1).take(MAX_TOASTS).map(|t| t.0).collect();
    let room = MAX_TOASTS - keep.len();
    keep.extend(drawn.iter().rev().filter(|t| !t.1).take(room).map(|t| t.0));
    keep.sort_unstable();
    let behind = drawn.len() - keep.len();
    (keep, behind)
}

/// List under the bell and toasts; returns the answers to apply.
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    icons: &Icons,
    world: &World,
    list: &mut Notifications,
    hist: &mut crate::world::notification_history::History,
    nui: &mut NotifUi,
    env: &mut ToastEnv<'_>,
    dnd: bool,
) -> (Vec<(u64, Response)>, Vec<super::notification_center::Ask>) {
    let mut answers = Vec::new();
    let mut asks = Vec::new();
    let now = ctx.input(|i| i.time);
    // Firestorm's toasts keep their own place: the islands' column is for
    // Aurora's center only (not kept for a later frame of it)
    nui.column = None;
    update_toasts(list, nui, now, env.message_seconds, env.tip_seconds, env.hidden, dnd);

    let anchor = nui.anchor.unwrap_or(egui::pos2(8.0, 60.0));
    if nui.open {
        nui.toasts.clear();
        list.mark_all_read();
        let area = egui::Area::new(egui::Id::unique("notification_list"))
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
                        // the history, as in Aurora's center
                        if super::notification_center::tabs(ui, p, &mut nui.center.tab) {
                            ui.add_space(4.0);
                            super::notification_center::history_list(
                                ui,
                                p,
                                icons,
                                world,
                                hist,
                                &mut nui.center.query,
                                ctx.content_rect().height() * 0.7,
                                &mut asks,
                                &nui.privacy,
                            );
                            return;
                        }
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
                                    if close_script_prompt(ctx, n, false) {
                                        answers.push((n.id, Response::Dismiss));
                                        continue;
                                    }
                                    let r = if matches!(n.data, Data::Message { .. }) {
                                        person_card(ui, p, icons, world, env, &mut nui.wanted_pics, n, false)
                                    } else {
                                        card(ui, p, icons, world, env.images, &mut nui.wanted_images, n, false, false)
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
        egui::Area::new(egui::Id::unique("notification_toasts"))
            .fade_in(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-10.0, anchor.y.min(screen.height()) + 6.0))
            .show(ctx, |ui| {
                // ToastGap between the toasts of the channel
                // (LLScreenChannel::redrawToasts)
                ui.spacing_mut().item_spacing.y = env.toast_gap;
                let (ids, behind) = shown_toasts(list, nui, dnd);
                for id in ids.into_iter().rev() {
                    if let Some(n) = list.list.iter_mut().find(|n| n.id == id) {
                        let r = if close_script_prompt(ctx, n, true) {
                            Some(Response::Dismiss)
                        } else {
                            // the card keeps its own row spacing; the stack's
                            // spacing stands only between the cards
                            ui.scope(|ui| {
                                ui.spacing_mut().item_spacing.y = CARD_ROW_SPACING;
                                if matches!(n.data, Data::Presence { .. } | Data::Message { .. }) {
                                    person_card(ui, p, icons, world, env, &mut nui.wanted_pics, n, true)
                                } else {
                                    card(ui, p, icons, world, env.images, &mut nui.wanted_images, n, true, false)
                                }
                            })
                            .inner
                        };
                        match r {
                            // closing a toast only hides it: the offer waits in
                            // the list (a scripted window too, until answered)
                            Some(Response::Dismiss) if !matches!(n.data, Data::Dialog { .. }) => {
                                nui.toasts.retain(|(t, _)| *t != id);
                                nui.hidden.insert(id);
                            }
                            Some(r) => answers.push((id, r)),
                            None => {}
                        }
                    }
                }
                if behind > 0 {
                    let more = ui
                        .add(
                            egui::Label::new(
                                RichText::new(aurora_i18n::tr!("ui-notifications-more", count = behind))
                                    .size(12.0)
                                    .strong()
                                    .color(p.violet_light),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if more.clicked() {
                        nui.open = true;
                    }
                }
            });
        // keep toasts expiring without input
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
    (answers, asks)
}

fn script_rows(count: usize) -> impl Iterator<Item = std::ops::Range<usize>> {
    (0..count.div_ceil(3)).rev().map(move |row| row * 3..(row * 3 + 3).min(count))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NotificationTipToastLifeTime's default (settings.xml).
    const TOAST_TIP: f64 = 10.0;

    /// Close Window reaches the Legacy Viewer's script floaters (a dialog,
    /// a text box, a URL, an object's offer) and Aurora's stacked permission
    /// question; a resident's offer stays out.
    #[test]
    fn script_windows_include_permission_questions_over_the_legacy_viewer() {
        let note = |data| Notification {
            id: 1,
            time: std::time::SystemTime::UNIX_EPOCH,
            kind: Kind::Script,
            title: String::new(),
            body: String::new(),
            name_links: Vec::new(),
            read: false,
            data,
            text: String::new(),
            transient: false,
            quiet: false,
            maturity: 0,
        };
        let sender = ([127, 0, 0, 1], 1).into();
        let offer = |task, object| Data::Inventory {
            from: Uuid::nil(),
            tx: Uuid::nil(),
            asset_type: 6,
            object,
            task,
            task_offer: None,
        };
        for data in [
            Data::Dialog {
                sender,
                object: Uuid::nil(),
                object_name: String::new(),
                own_object: false,
                channel: 0,
                buttons: Vec::new(),
            },
            Data::TextBox {
                sender,
                object: Uuid::nil(),
                channel: 0,
            },
            Data::Url("https://example.invalid".into()),
            offer(true, Uuid::nil()),
            Data::Permissions {
                task: Uuid::nil(),
                item: Uuid::nil(),
                questions: 0,
                region: Default::default(),
                experience: None,
                caution: None,
            },
        ] {
            let visible = note(data);
            let new_prompt = Notification {
                id: 2,
                ..note(Data::Url("https://example.invalid".into()))
            };
            let mut frames = crate::ui::widgets::elide_tests::Frames::new();
            frames.run(Vec::new(), |ui| {
                super::super::close_key::begin(ui.ctx());
                assert!(!close_script_prompt(ui.ctx(), &visible, true));
            });
            let close_all = egui::Event::Key {
                key: egui::Key::W,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            };
            let mut dismissed = false;
            frames.run(vec![close_all], |ui| {
                super::super::close_key::begin(ui.ctx());
                dismissed |= close_script_prompt(ui.ctx(), &visible, true);
                assert!(!close_script_prompt(ui.ctx(), &new_prompt, true));
            });
            assert!(dismissed);
        }
        assert!(!note(offer(false, Uuid::from_u128(7))).stays_until_answered());
    }

    #[test]
    fn closing_a_script_permission_or_object_offer_uses_its_visible_cross() {
        use crate::world::notifications::im;
        use aurora_net::NetCommand;

        let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let theme = crate::theme::Theme::default();
        let p = theme.palette();
        let icons = Icons::default();
        let pics = HashMap::new();
        let images = HashMap::new();
        let mut emoji = crate::ui::emoji::Emoji::load();
        for legacy in [false, true] {
            for (index, permission) in [(1, true), (3, false)] {
                let mut frames = crate::ui::widgets::elide_tests::Frames::new();
                theme.apply(&frames.ctx, 1.0);
                let mut list = Notifications::default();
                let id = super::super::notification_center::tests::scripted_windows(&mut list)[index];
                list.list.retain(|n| n.id == id);
                let mut nui = NotifUi {
                    bell: Some(egui::Rect::from_min_size(egui::pos2(550.0, 12.0), egui::vec2(32.0, 32.0))),
                    ..Default::default()
                };
                let mut hist = Default::default();
                let mut frame = |list: &mut Notifications, nui: &mut NotifUi, events| {
                    let mut result = Vec::new();
                    frames.run(events, |ui| {
                        super::super::close_key::begin(ui.ctx());
                        let mut env = ToastEnv {
                            pics: &pics,
                            images: &images,
                            emoji: &mut emoji,
                            hidden: false,
                            message_seconds: 30.0,
                            tip_seconds: TOAST_TIP,
                            restart: None,
                            presence_at: Default::default(),
                            classic_bars: true,
                            toast_gap: crate::settings::TOAST_GAP,
                        };
                        let draw = if legacy { show } else { super::super::notification_center::show };
                        result.extend(draw(ui.ctx(), &p, &icons, &world, list, &mut hist, nui, &mut env, false).0);
                    });
                    result
                };
                let key = |pressed, modifiers| egui::Event::Key {
                    key: egui::Key::W,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                };
                for _ in 0..4 {
                    assert!(frame(&mut list, &mut nui, vec![]).is_empty());
                }
                let command = egui::Modifiers::COMMAND;
                let mut answer = frame(&mut list, &mut nui, vec![key(true, command)]);
                if legacy {
                    assert!(answer.is_empty(), "hiding sends nothing");
                    assert!(nui.hidden.contains(&id));
                    assert!(list.listed().any(|n| n.id == id));
                    assert!(!shown_toasts(&list, &nui, false).0.contains(&id));
                    nui.open = true;
                    assert!(frame(&mut list, &mut nui, vec![key(false, command)]).is_empty());
                    assert!(frame(&mut list, &mut nui, vec![]).is_empty());
                    answer = frame(&mut list, &mut nui, vec![key(true, command | egui::Modifiers::SHIFT)]);
                }
                assert!(matches!(answer.as_slice(), [(got, Response::Dismiss)] if *got == id));
                let (out, url) = list.respond(id, Response::Dismiss, |_| Uuid::nil(), Uuid::nil());
                assert!(url.is_none() && !list.listed().any(|n| n.id == id));
                if permission {
                    assert!(out.is_empty(), "a permission question is closed silently");
                } else {
                    assert!(matches!(
                        out.as_slice(),
                        [NetCommand::SendImDialog {
                            dialog: im::TASK_INVENTORY_DECLINED,
                            ..
                        }]
                    ));
                }
            }
        }
    }

    #[test]
    fn bell_right_click_turns_do_not_disturb_over_and_keeps_the_rest() {
        let away = crate::world::status::StatusModes {
            away: true,
            autorespond: true,
            reject_teleports: true,
            ..Default::default()
        };
        let on = bell_dnd_toggled(away);
        assert!(on.dnd);
        assert_eq!(crate::world::status::StatusModes { dnd: false, ..on }, away);
        assert_eq!(bell_dnd_toggled(on), away);
    }

    /// #2832: the llTextBox field holds several lines, shows three, then
    /// scrolls instead of growing the card; Enter starts a new line.
    #[test]
    fn script_text_box_shows_three_lines_then_scrolls() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let height = |text: &str| {
            let mut text = text.to_owned();
            let mut h = 0.0;
            for _ in 0..2 {
                let mut out = ctx.run_ui(Default::default(), |ui| {
                    ui.set_width(300.0);
                    // the frame, which holds still while the text scrolls
                    h = text_box_field(ui, 7, &mut text).0.height();
                });
                out.textures_delta.clear();
            }
            h
        };
        let empty = height("");
        let three = height("Line 1\nLine 2\nLine 3");
        let six = height("1\n2\n3\n4\n5\n6");
        assert!((empty - three).abs() < 0.5, "three lines fit an empty field: {empty} / {three}");
        assert!((six - three).abs() < 0.5, "more lines scroll: {six} / {three}");

        let mut text = String::from("Line 1");
        let typed = |text: &mut String, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    text_box_field(ui, 7, text).1.request_focus();
                },
            );
            out.textures_delta.clear();
        };
        typed(&mut text, Vec::new());
        typed(&mut text, Vec::new());
        typed(
            &mut text,
            vec![
                egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::Text("Line 2".into()),
            ],
        );
        assert_eq!(text, "Line 1\nLine 2");
        // a paste is cleaned before it is inserted: a tab as four spaces
        typed(&mut text, vec![egui::Event::Paste("\tTab\r\nEnd".into())]);
        assert_eq!(text, "Line 1\nLine 2    Tab\nEnd");
    }

    /// #2832: the llTextBox toast holds still from one frame to the next:
    /// its field, scrolling past three lines, ran a fraction of a point
    /// past the card on some frames, and the toast, anchored to the right,
    /// shimmered sideways. Toasts are the Legacy card (`aurora: false`); the
    /// Aurora card takes the width of the notification center's list.
    #[test]
    fn script_text_box_toast_keeps_its_place() {
        let aurora = false;
        for lines in ["a", "1\n2\n3\n4\n5", "1\n2\n3\n4\n5\n6\n7\n8\n9"] {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            let p = theme.palette();
            let icons = Icons::default();
            let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let mut list = Notifications::default();
            list.push(
                Kind::Script,
                "Shop sign",
                "Write your name for the guest book",
                Data::TextBox {
                    sender: ([127, 0, 0, 1], 13000).into(),
                    object: Uuid::from_u128(3),
                    channel: -42,
                },
            );
            list.list[0].text = lines.into();
            let mut rects = Vec::new();
            for frame in 0..8 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                        time: Some(frame as f64 * 0.016),
                        ..Default::default()
                    },
                    |ui| {
                        let area = egui::Area::new(egui::Id::unique("toast_test"))
                            .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-10.0, 60.0))
                            .show(ui.ctx(), |ui| {
                                card(
                                    ui,
                                    &p,
                                    &icons,
                                    &world,
                                    &HashMap::new(),
                                    &mut HashSet::new(),
                                    &mut list.list[0],
                                    true,
                                    aurora,
                                );
                            });
                        rects.push(area.response.rect);
                    },
                );
                output.textures_delta.clear();
            }
            let settled = &rects[3..];
            assert!(
                settled
                    .windows(2)
                    .all(|w| (w[0].min - w[1].min).length() < 0.01 && (w[0].size() - w[1].size()).length() < 0.01),
                "{lines:?}: {rects:?}"
            );
        }
    }

    /// ToastGap: the Legacy toasts' stack grows by the gap chosen between
    /// each two of its toasts, and by nothing else.
    #[test]
    fn the_legacy_toasts_stand_the_toast_gap_apart() {
        let height = |gap: f32| {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            let p = theme.palette();
            let icons = Icons::default();
            let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let mut emoji = crate::ui::emoji::Emoji::load();
            let mut list = Notifications::default();
            for i in 0..3 {
                list.push(Kind::Info, format!("Tip {i}"), "A short line", Data::None);
            }
            let (mut hist, mut nui) = (Default::default(), NotifUi::default());
            let pics = HashMap::new();
            let images = HashMap::new();
            for frame in 0..6 {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                        time: Some(frame as f64 * 0.016),
                        ..Default::default()
                    },
                    |ui| {
                        let mut env = ToastEnv {
                            pics: &pics,
                            images: &images,
                            emoji: &mut emoji,
                            hidden: false,
                            message_seconds: 30.0,
                            tip_seconds: TOAST_TIP,
                            restart: None,
                            presence_at: Default::default(),
                            classic_bars: true,
                            toast_gap: gap,
                        };
                        show(ui.ctx(), &p, &icons, &world, &mut list, &mut hist, &mut nui, &mut env, false);
                    },
                );
                out.textures_delta.clear();
            }
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("notification_toasts"));
            ctx.memory(|m| m.area_rect(layer.id)).expect("the toasts are shown").height()
        };
        let (none, legacy, widest) = (
            height(0.0),
            height(crate::settings::TOAST_GAP),
            height(crate::settings::TOAST_GAP_MAX),
        );
        assert!((legacy - none - 2.0 * crate::settings::TOAST_GAP).abs() < 0.5, "{none} / {legacy}");
        assert!(
            (widest - none - 2.0 * crate::settings::TOAST_GAP_MAX).abs() < 0.5,
            "{none} / {widest}"
        );
    }

    #[test]
    fn script_rows_keep_original_indices_in_bottom_up_groups_of_three() {
        assert_eq!(script_rows(12).collect::<Vec<_>>(), vec![9..12, 6..9, 3..6, 0..3]);
        assert_eq!(script_rows(4).collect::<Vec<_>>(), vec![3..4, 0..3]);
        assert_eq!(script_rows(2).collect::<Vec<_>>(), vec![0..2]);
        assert_eq!(script_rows(0).count(), 0);
    }

    #[test]
    fn script_dialog_pointer_clicks_return_the_original_button_in_both_styles() {
        for aurora in [false, true] {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            let p = theme.palette();
            let icons = Icons::default();
            let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let labels: Vec<_> = (0..12).map(|i| format!("Choice {i:02}")).collect();
            let mut list = Notifications::default();
            list.push(
                Kind::Script,
                "HUD",
                "Choose a button",
                Data::Dialog {
                    sender: ([127, 0, 0, 1], 13000).into(),
                    object: Uuid::from_u128(3),
                    object_name: "HUD".into(),
                    own_object: false,
                    channel: -42,
                    buttons: labels.clone(),
                },
            );
            let mut time = 0.0;
            let mut frame = |events| {
                time += 0.1;
                let mut answer = None;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        ui.set_width(390.0);
                        answer = card(
                            ui,
                            &p,
                            &icons,
                            &world,
                            &HashMap::new(),
                            &mut HashSet::new(),
                            &mut list.list[0],
                            !aurora,
                            aurora,
                        );
                    },
                );
                output.textures_delta.clear();
                (output, answer)
            };
            for _ in 0..3 {
                frame(vec![]);
            }
            for (index, label) in labels.iter().enumerate() {
                let (output, _) = frame(vec![]);
                let pos = output
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos + t.galley.size() * 0.5),
                        _ => None,
                    })
                    .expect("button visible");
                frame(vec![egui::Event::PointerMoved(pos)]);
                let click = |pressed| {
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }]
                };
                assert!(frame(click(true)).1.is_none());
                let (_, answer) = frame(click(false));
                assert!(
                    matches!(answer, Some(Response::Button(i)) if i == index),
                    "aurora={aurora}, button={index}: {answer:?}"
                );
            }
        }
    }

    const JANE: Uuid = Uuid::from_u128(2);

    /// A group notice's attachment: its name and its save button inside the
    /// card in both styles, the click saving it (then "Saved", greyed).
    #[test]
    fn a_notice_attachment_is_saved_from_its_card_in_both_styles() {
        for aurora in [false, true] {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            let p = theme.palette();
            let icons = Icons::default();
            let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let mut list = Notifications::default();
            list.push(
                Kind::Group,
                "Northern Wolves : Workshop",
                "Sent by Jane Doe\nBring the kit.",
                Data::GroupNotice {
                    group: Uuid::from_u128(6),
                    attachment: Some(crate::world::notifications::NoticeAttachment {
                        from: JANE,
                        tx: Uuid::from_u128(3),
                        name: "Texture kit".into(),
                        asset_type: 6,
                        saved: false,
                    }),
                },
            );
            let mut time = 0.0;
            let mut frame = |events, list: &mut Notifications| {
                time += 0.1;
                let mut answer = None;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        ui.set_width(320.0);
                        answer = card(
                            ui,
                            &p,
                            &icons,
                            &world,
                            &HashMap::new(),
                            &mut HashSet::new(),
                            &mut list.list[0],
                            true,
                            aurora,
                        );
                    },
                );
                output.textures_delta.clear();
                (output, answer)
            };
            for _ in 0..3 {
                frame(vec![], &mut list);
            }
            let text_rect = |output: &egui::FullOutput, text: &str| {
                output.shapes.iter().find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == text => Some(egui::Rect::from_min_size(t.pos, t.galley.size())),
                    _ => None,
                })
            };
            let (output, _) = frame(vec![], &mut list);
            let body = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text().contains("Bring the kit.") => {
                        Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
                    }
                    _ => None,
                })
                .expect("body shown");
            assert!(text_rect(&output, "Texture kit").is_some(), "aurora={aurora}: attachment named");
            let save = text_rect(&output, aurora_i18n::tr!("ui-group-notices-save")).expect("save button");
            // the toast's own width: nothing past its time and close button
            let right = 320.0 + 16.0;
            assert!(
                save.right() <= right,
                "aurora={aurora}: button inside the card ({} > {right})",
                save.right()
            );
            assert!(save.top() > body.bottom(), "under the text");
            frame(vec![egui::Event::PointerMoved(save.center())], &mut list);
            for pressed in [true, false] {
                let (_, answer) = frame(
                    vec![egui::Event::PointerButton {
                        pos: save.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    &mut list,
                );
                if !pressed {
                    assert!(matches!(answer, Some(Response::Accept)), "aurora={aurora}: {answer:?}");
                }
            }
            let (out, _) = list.respond(list.list[0].id, Response::Accept, |_| Uuid::from_u128(7), Uuid::nil());
            assert_eq!(out.len(), 1);
            let (output, _) = frame(vec![], &mut list);
            assert!(
                text_rect(&output, aurora_i18n::tr!("ui-group-notices-saved")).is_some(),
                "aurora={aurora}: saved"
            );
        }
    }

    /// In the Legacy toast style, French Mute Owner wraps under Accept /
    /// Discard and must keep the same left edge as those buttons (#2932).
    /// The interface language belongs to the whole process: switched here,
    /// it turned the texts of the tests running beside this one French
    /// halfway through. So this test runs again, alone, in a test process
    /// of its own, and checks that run.
    #[test]
    fn legacy_object_offer_wrapped_buttons_keep_the_text_indent_in_french() {
        const ALONE: &str = "AURORA_TEST_FRENCH_OFFER_ALONE";
        if std::env::var_os(ALONE).is_none() {
            // the name libtest gives this test: its path without the crate
            let path = concat!(
                module_path!(),
                "::legacy_object_offer_wrapped_buttons_keep_the_text_indent_in_french"
            );
            let name = path.split_once("::").map_or(path, |(_, rest)| rest);
            let run = std::process::Command::new(std::env::current_exe().expect("the test binary"))
                .args([name, "--exact", "--test-threads=1"])
                .env(ALONE, "1")
                .output()
                .expect("the test binary starts");
            let stdout = String::from_utf8_lossy(&run.stdout);
            let stderr = String::from_utf8_lossy(&run.stderr);
            assert!(run.status.success(), "the French run failed:\n{stdout}\n{stderr}");
            assert!(stdout.contains("1 passed"), "the French run ran this test:\n{stdout}");
            return;
        }
        aurora_i18n::select("fr", Some("fr"));
        let mute = aurora_i18n::tr!("ui-notifications-mute-owner");
        let accept = aurora_i18n::tr!("ui-notifications-accept");
        let discard = aurora_i18n::tr!("ui-notifications-discard");
        assert_eq!(mute, "Ignorer le propriétaire");

        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let icons = Icons::default();
        let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let mut list = Notifications::default();
        let body = "Un objet nommé Distributeur appartenant à Jane Doe vous a donné cet objet :\nNote de bienvenue";
        list.push(
            Kind::Inventory,
            "Distributeur",
            body,
            Data::Inventory {
                from: JANE,
                tx: Uuid::from_u128(3),
                asset_type: 7,
                object: Uuid::nil(),
                task: true,
                task_offer: Some(crate::world::inventory_offers::TaskOffer {
                    object_name: "Distributeur".into(),
                    item_name: "Note de bienvenue".into(),
                    from_group: false,
                    own_object: false,
                }),
            },
        );
        list.link_name(list.last_id(), "Jane Doe", crate::world::notifications::NameTarget::Avatar(JANE));

        let mut time = 0.0;
        let mut frame = || {
            time += 0.1;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(320.0);
                    card(
                        ui,
                        &p,
                        &icons,
                        &world,
                        &HashMap::new(),
                        &mut HashSet::new(),
                        &mut list.list[0],
                        true,
                        false,
                    );
                },
            );
            output.textures_delta.clear();
            output
        };
        for _ in 0..3 {
            frame();
        }
        let output = frame();
        let text_rect = |wanted: &str| {
            output.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == wanted => Some(egui::Rect::from_min_size(t.pos, t.galley.size())),
                _ => None,
            })
        };
        let accept_rect = text_rect(accept).expect("Accept visible");
        let discard_rect = text_rect(discard).expect("Discard visible");
        let mute_rect = text_rect(mute).expect("Mute Owner visible");
        assert!(
            (accept_rect.min.y - discard_rect.min.y).abs() < 1.0,
            "Accept and Discard stay on the first row: {accept_rect:?} {discard_rect:?}"
        );
        assert!(
            mute_rect.min.y > accept_rect.max.y + 1.0,
            "Mute Owner wraps under the first row: {mute_rect:?} above {accept_rect:?}"
        );
        // Label glyphs sit a couple of pixels inside each button face, so
        // allow a small inset difference while still catching the old bug
        // that dropped the whole 22 px Legacy indent on the wrapped row.
        assert!(
            (mute_rect.min.x - accept_rect.min.x).abs() < 4.0,
            "wrapped Mute Owner keeps Accept's left edge: mute={mute_rect:?} accept={accept_rect:?}"
        );
        assert!(
            mute_rect.min.x > accept_rect.min.x - 8.0,
            "wrapped Mute Owner must not lose the Legacy indent: mute={mute_rect:?} accept={accept_rect:?}"
        );
    }

    #[test]
    fn object_offer_buttons_are_visible_and_clickable_in_both_styles() {
        for aurora in [false, true] {
            for own_object in [false, true] {
                let ctx = egui::Context::default();
                let theme = crate::theme::Theme::default();
                theme.apply(&ctx, 1.0);
                let p = theme.palette();
                let icons = Icons::default();
                let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
                let mut list = Notifications::default();
                let body = "An object named Gift dispenser owned by Jane Doe has given you this object:\nWelcome note";
                list.push(
                    Kind::Inventory,
                    "Gift dispenser",
                    body,
                    Data::Inventory {
                        from: JANE,
                        tx: Uuid::from_u128(3),
                        asset_type: 7,
                        object: Uuid::nil(),
                        task: true,
                        task_offer: Some(crate::world::inventory_offers::TaskOffer {
                            object_name: "Gift dispenser".into(),
                            item_name: "Welcome note".into(),
                            from_group: false,
                            own_object,
                        }),
                    },
                );
                let notification = list.last_id();
                list.link_name(notification, "Jane Doe", crate::world::notifications::NameTarget::Avatar(JANE));
                let mut time = 0.0;
                let mut frame = |events| {
                    time += 0.1;
                    let mut answer = None;
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                            time: Some(time),
                            events,
                            ..Default::default()
                        },
                        |ui| {
                            ui.set_width(320.0);
                            answer = card(
                                ui,
                                &p,
                                &icons,
                                &world,
                                &HashMap::new(),
                                &mut HashSet::new(),
                                &mut list.list[0],
                                true,
                                aurora,
                            );
                        },
                    );
                    output.textures_delta.clear();
                    (output, answer)
                };
                for _ in 0..3 {
                    frame(vec![]);
                }
                let labels = [
                    (aurora_i18n::tr!("ui-notifications-accept"), Response::Accept),
                    (aurora_i18n::tr!("ui-notifications-discard"), Response::Decline),
                    (aurora_i18n::tr!("ui-notifications-mute-owner"), Response::Block),
                ];
                for (index, (label, expected)) in labels.iter().enumerate() {
                    let (output, _) = frame(vec![]);
                    assert!(
                        output
                            .shapes
                            .iter()
                            .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == body)),
                        "complete body visible"
                    );
                    let pos = output.shapes.iter().find_map(|s| match &s.shape {
                        egui::Shape::Text(t) if t.galley.text() == *label => Some(t.pos + t.galley.size() * 0.5),
                        _ => None,
                    });
                    if own_object && index == 2 {
                        assert!(pos.is_none(), "our own owner cannot be muted");
                        continue;
                    }
                    let pos = pos.expect("button visible");
                    frame(vec![egui::Event::PointerMoved(pos)]);
                    for pressed in [true, false] {
                        let (_, answer) = frame(vec![egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        }]);
                        if !pressed {
                            assert!(matches!(
                                (answer, expected),
                                (Some(Response::Accept), Response::Accept)
                                    | (Some(Response::Decline), Response::Decline)
                                    | (Some(Response::Block), Response::Block)
                            ));
                        }
                    }
                }
            }
        }
    }

    /// A Marketplace delivery's card names it and its one button shows it in
    /// the inventory, in both styles (#752).
    #[test]
    fn delivery_card_shows_its_name_and_opens_the_inventory_in_both_styles() {
        for (aurora, toast) in [(false, true), (true, true), (true, false)] {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            let p = theme.palette();
            let icons = Icons::default();
            let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let mut list = Notifications::default();
            list.push_deliveries(&[(Uuid::from_u128(7), "Lantern".into())], Uuid::from_u128(50));
            let mut time = 0.0;
            let mut frame = |events| {
                time += 0.1;
                let mut answer = None;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        ui.set_width(320.0);
                        answer = card(
                            ui,
                            &p,
                            &icons,
                            &world,
                            &HashMap::new(),
                            &mut HashSet::new(),
                            &mut list.list[0],
                            toast,
                            aurora,
                        );
                    },
                );
                output.textures_delta.clear();
                (output, answer)
            };
            for _ in 0..3 {
                frame(vec![]);
            }
            let (output, _) = frame(vec![]);
            let text = |wanted: &str| {
                output.shapes.iter().find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == wanted => Some(t.pos + t.galley.size() * 0.5),
                    _ => None,
                })
            };
            assert!(text(aurora_i18n::tr!("received-delivery-title")).is_some(), "title visible");
            assert!(text("Lantern").is_some(), "the delivery's name visible");
            let pos = text(aurora_i18n::tr!("ui-notifications-show-in-inventory")).expect("button visible");
            frame(vec![egui::Event::PointerMoved(pos)]);
            let mut clicked = None;
            for pressed in [true, false] {
                let (_, answer) = frame(vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }]);
                clicked = clicked.or(answer);
            }
            assert!(matches!(clicked, Some(Response::Open)), "Show in inventory answers Open");
        }
    }

    /// A friend's notice lives as long as its toast; a message's entry
    /// stays under the bell once its toast is gone when asked for, else it
    /// leaves with its toast too (#752).
    #[test]
    fn a_friend_toast_leaves_with_its_toast_and_a_message_stays_listed() {
        const JOHN: Uuid = Uuid::from_u128(3);
        let mut list = Notifications::default();
        let mut nui = NotifUi::default();
        let friend = list.push_presence(JANE, "Jane Doe is online.", true);
        let message = list.push_message(JANE, JANE, "Jane Doe", "hello", true);
        let toast_only = list.push_message(JOHN, JOHN, "John Doe", "hi", false);
        update_toasts(&mut list, &mut nui, 0.0, 30.0, TOAST_TIP, false, false);
        assert_eq!(
            nui.toasts.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![friend, message, toast_only]
        );
        assert_eq!(list.unread(), 1, "the toast alone is not counted");
        // the friend's toast ends after TOAST_TIP, the messages' after 30 s
        update_toasts(&mut list, &mut nui, TOAST_TIP + 1.0, 30.0, TOAST_TIP, false, false);
        assert!(!list.list.iter().any(|n| n.id == friend));
        assert!(list.list.iter().any(|n| n.id == message));
        update_toasts(&mut list, &mut nui, 31.0, 30.0, TOAST_TIP, false, false);
        assert!(nui.toasts.is_empty());
        assert_eq!(list.list.iter().map(|n| n.id).collect::<Vec<_>>(), vec![message]);
        assert_eq!(list.unread(), 1);
    }

    /// The friend's toast lasts NotificationTipToastLifeTime, whatever the
    /// message toasts' time.
    #[test]
    fn a_friend_toast_lasts_the_tip_lifetime() {
        let mut list = Notifications::default();
        let mut nui = NotifUi::default();
        let friend = list.push_presence(JANE, "Jane Doe is online.", true);
        update_toasts(&mut list, &mut nui, 0.0, 30.0, 3.0, false, false);
        update_toasts(&mut list, &mut nui, 2.9, 30.0, 3.0, false, false);
        assert!(nui.toasts.iter().any(|(id, _)| *id == friend));
        update_toasts(&mut list, &mut nui, 3.1, 30.0, 3.0, false, false);
        assert!(!nui.toasts.iter().any(|(id, _)| *id == friend));
        assert!(!list.list.iter().any(|n| n.id == friend), "it leaves with its toast");
    }

    /// In mouselook or the photo's full frame no toast starts: a friend's
    /// notice is dropped, a message waits under the bell.
    #[test]
    fn hidden_toasts_drop_friends_and_keep_messages() {
        let mut list = Notifications::default();
        let mut nui = NotifUi::default();
        list.push_presence(JANE, "Jane Doe is online.", true);
        let message = list.push_message(JANE, JANE, "Jane Doe", "hello", true);
        update_toasts(&mut list, &mut nui, 0.0, 30.0, TOAST_TIP, true, false);
        assert!(nui.toasts.is_empty());
        assert_eq!(list.list.iter().map(|n| n.id).collect::<Vec<_>>(), vec![message]);
    }

    /// In do not disturb only a script menu toasts (#491): a message waits in
    /// the list, and a toast already out leaves when the status is set.
    #[test]
    fn do_not_disturb_toasts_only_script_menus() {
        let mut list = Notifications::default();
        let mut nui = NotifUi::default();
        let early = list.push_message(JANE, JANE, "Jane Doe", "before", true);
        update_toasts(&mut list, &mut nui, 0.0, 30.0, TOAST_TIP, false, false);
        assert_eq!(nui.toasts.iter().map(|t| t.0).collect::<Vec<_>>(), vec![early]);
        let message = list.push_message(JANE, JANE, "Jane Doe", "hello", true);
        let menu = list.push(
            Kind::Script,
            "Fountain",
            "Pick a colour",
            Data::Dialog {
                sender: ([127, 0, 0, 1], 13000).into(),
                object: Uuid::from_u128(9),
                object_name: "Fountain".into(),
                own_object: false,
                channel: -5,
                buttons: vec!["Violet".into()],
            },
        );
        update_toasts(&mut list, &mut nui, 1.0, 30.0, TOAST_TIP, false, true);
        assert_eq!(nui.toasts.iter().map(|t| t.0).collect::<Vec<_>>(), vec![menu]);
        assert!(list.list.iter().any(|n| n.id == message));
    }

    /// Every window of a scripted object keeps its toast until answered,
    /// not only llDialog: no time runs out, the information arriving after
    /// it never pushes it out, and it is back after the open list, mouselook
    /// or do not disturb. Its cross hides it for good (it waits in the list).
    #[test]
    fn every_scripted_window_keeps_its_toast_until_answered() {
        let mut list = Notifications::default();
        let mut nui = NotifUi::default();
        let windows = super::super::notification_center::tests::scripted_windows(&mut list);
        update_toasts(&mut list, &mut nui, 0.0, 30.0, TOAST_TIP, false, false);
        let infos: Vec<u64> = (0..4).map(|i| list.push(Kind::Info, format!("n{i}"), "", Data::None)).collect();
        update_toasts(&mut list, &mut nui, 0.1, 30.0, TOAST_TIP, false, false);
        let ids = |nui: &NotifUi| nui.toasts.iter().map(|t| t.0).collect::<Vec<_>>();
        // the oldest information gave way, not a scripted window
        assert_eq!(ids(&nui), [windows.clone(), infos[1..].to_vec()].concat());
        // the newest three scripted windows drawn, the rest behind
        assert_eq!(shown_toasts(&list, &nui, false), (windows[1..].to_vec(), 4));
        // long after: only they are left
        update_toasts(&mut list, &mut nui, TOAST_OFFER * 4.0, 30.0, TOAST_TIP, false, false);
        assert_eq!(ids(&nui), windows);
        // the list opened (everything read), then closed: back
        nui.toasts.clear();
        list.mark_all_read();
        update_toasts(&mut list, &mut nui, 200.0, 30.0, TOAST_TIP, false, false);
        assert_eq!(ids(&nui), windows);
        // mouselook keeps them (drawn once it ends); do not disturb draws
        // only the text box
        update_toasts(&mut list, &mut nui, 201.0, 30.0, TOAST_TIP, true, false);
        assert_eq!(ids(&nui), windows);
        update_toasts(&mut list, &mut nui, 202.0, 30.0, TOAST_TIP, false, true);
        assert_eq!(shown_toasts(&list, &nui, true), (vec![windows[0]], 0));
        assert_eq!(shown_toasts(&list, &nui, false).0, windows[1..].to_vec());
        // hidden by its cross: stays in the list, not brought back
        nui.toasts.retain(|t| t.0 != windows[3]);
        nui.hidden.insert(windows[3]);
        update_toasts(&mut list, &mut nui, 203.0, 30.0, TOAST_TIP, false, false);
        assert_eq!(ids(&nui), windows[..3].to_vec());
        assert!(list.listed().any(|n| n.id == windows[3]));
        // answered: gone
        list.list.retain(|n| n.id != windows[0]);
        update_toasts(&mut list, &mut nui, 204.0, 30.0, TOAST_TIP, false, false);
        assert_eq!(ids(&nui), windows[1..3].to_vec());
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
                    super::super::widgets::Floater::new(
                        "conversations",
                        "Conversations",
                        egui::pos2(300.0, 200.0),
                        egui::vec2(440.0, 300.0),
                    )
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

    /// A message card shows the labels of its links and cuts after them,
    /// never inside an address (#947).
    #[test]
    fn a_message_card_shows_link_labels_and_cuts_after_them() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let (lune, group) = (Uuid::from_u128(0x51), Uuid::from_u128(0x52));
        world.social.names.insert(lune, "Lune Étoilée".into());
        world.land.group_names.insert(group, "Cercle des Brumes".into());
        let body = format!("ask secondlife:///app/agent/{lune}/about about secondlife:///app/group/{group}/about");
        assert_eq!(message_excerpt(&world, &body), "ask Lune Étoilée about Cercle des Brumes");
        // the cut counts the labels, not the addresses
        let long = format!("{} secondlife:///app/agent/{lune}/about", "é".repeat(MESSAGE_CHARS - 5));
        let cut = message_excerpt(&world, &long);
        assert_eq!(cut.chars().count(), MESSAGE_CHARS + 1, "{cut}");
        assert!(cut.ends_with(" Lune…") && !cut.contains("secondlife:"), "{cut}");
    }
}
