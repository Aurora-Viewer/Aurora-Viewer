//! Aurora's notification center (Preferences › Aurora › Notifications,
//! "Animated bell"): a new notification unrolls from the bell at the right
//! end of the navigation bar, stays a few seconds (a fine bar shows the time
//! left, hovering pauses it), then folds back into the bell, whose count
//! bounces. Clicking the bell opens the full list at the same place, with a
//! "To do" tab and the persistent "History" (world/notification_history.rs).
//! In do not disturb the bell is paused: nothing unrolls nor rings but the
//! script menus (`dnd_shows`), everything still arrives in "To do". The Firestorm behaviour (toasts stacked apart from
//! the bell) stays in `ui::notifications` as the Legacy style.

use super::icons::Icons;
use super::notifications::{self as cards, NotifUi, ToastEnv};
use crate::settings::PresencePosition;
use crate::theme::Palette;
use crate::world::World;
use crate::world::chat_log::format::Slt;
use crate::world::notification_history::{self as history, Entry, Follow, History, Outcome};
use crate::world::notifications::{Data, Notification, Notifications, Response};
use egui::{Rect, RichText, Vec2};

/// Width of the panel under the bell (the mockup's 360 px), with
/// Firestorm's bars; the islands give it theirs (`NotifUi::column`).
pub const WIDTH: f32 = 360.0;
/// What the panel's overlay adds around its content: 6 px of margin and the
/// 1 px border on each side.
const PANEL_FRAME: f32 = 14.0;
/// Seconds a card stays rolled out: information / waiting for an answer.
pub const SHOW_INFO: f64 = 6.0;
pub const SHOW_ANSWER: f64 = 12.0;
/// Unrolling and folding (the mockup's 220 and 260 ms).
pub const UNROLL: f32 = 0.22;
pub const FOLD: f64 = 0.26;
/// Cards shown rolled out at once; the others wait behind "+ N more".
pub const MAX_PEEK: usize = 3;
/// The bell's bounce and ring (380 and 520 ms).
const BUMP: f64 = 0.38;
const RING: f64 = 0.52;
/// A friend online / offline: a small toast apart from the center, then a
/// fade (the mockup's 3.5 s and 300 ms); at the top right it waits that
/// long at most for the bell's cards to fold (`presence`).
pub const PRESENCE: f64 = 3.5;
const PRESENCE_FADE: f64 = 0.3;

/// A card rolled out under the bell.
#[derive(Debug, Clone, PartialEq)]
pub struct Peek {
    pub id: u64,
    pub total: f64,
    pub left: f64,
    /// When it started to fold back into the bell.
    pub folding: Option<f64>,
    /// A scripted object's window (`Notification::stays_until_answered`):
    /// it stays out until answered, like the Legacy Viewer's script
    /// floaters (no time left, no fold).
    pub pinned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    ToDo,
    History,
}

/// What the center remembers from one frame to the next.
#[derive(Default)]
pub struct CenterUi {
    pub peek: Vec<Peek>,
    pub tab: Tab,
    pub query: String,
    last_seen: u64,
    last_time: Option<f64>,
    /// The panel's rectangle last frame (hovering it pauses the cards).
    panel: Option<Rect>,
    /// Height of each card last frame (folding shrinks it).
    heights: std::collections::HashMap<u64, f32>,
    /// The count the bell showed, and when it last bounced / rang.
    shown_count: usize,
    bump_at: Option<f64>,
    ring_at: Option<f64>,
    /// Friends' online / offline toasts, shown or waiting (`presence`).
    pub presence: Vec<presence::Toast>,
    /// The size each of them was drawn with last frame.
    presence_sizes: std::collections::HashMap<u64, Vec2>,
    /// The panel's height last time it was fully unrolled.
    unrolled_height: f32,
    /// The width of the panel's content this frame.
    inner_width: f32,
}

/// What the frame tells the center besides its notifications.
#[derive(Debug, Clone, Copy, Default)]
pub struct Moment {
    pub now: f64,
    /// The pointer is over the rolled-out panel.
    pub hovered: bool,
    /// Do not disturb: the bell is paused.
    pub dnd: bool,
    /// Mouselook or the photo's full frame: nothing over the view.
    pub hidden: bool,
    /// The full list is open.
    pub open: bool,
    /// The friends' toasts stand at the top right, where the cards unroll:
    /// they wait while the cards or the list are out.
    pub under_bell: bool,
}

/// How long `n` stays rolled out.
pub fn show_seconds(n: &Notification, message_seconds: f64) -> f64 {
    match n.data {
        // a message is information; a shorter IM toast setting shortens it
        Data::Message { .. } => message_seconds.clamp(1.0, SHOW_INFO),
        _ if n.interactive() => SHOW_ANSWER,
        _ => SHOW_INFO,
    }
}

/// Shows in do not disturb as well: a script menu or text box (#491). The
/// Legacy Viewer opens its script floater for both (ScriptDialog) whatever
/// the status (LLScriptHandler::processNotification) and keeps only the
/// other toasts back (LLScreenChannel::addToast).
pub fn dnd_shows(n: &Notification) -> bool {
    matches!(n.data, Data::Dialog { .. } | Data::TextBox { .. })
}

/// A scripted object's window out of the way of the list, the clear view or
/// do not disturb, which can be shown now.
fn may_show(n: &Notification, m: &Moment) -> bool {
    !m.open && !m.hidden && (!m.dnd || dnd_shows(n))
}

/// Rolls out as a card: what is listed, and a message kept out of the bell
/// (`ToastPrefs::im_in_bell` off), whose card leaves with its time.
fn card(n: &Notification) -> bool {
    !n.transient || matches!(n.data, Data::Message { .. })
}

/// The count on the bell: what waits in "To do", without the cards still
/// rolled out (they are counted when they fold in).
pub fn bell_count(list: &Notifications, c: &CenterUi) -> usize {
    list.listed().filter(|n| !n.read && !c.peek.iter().any(|p| p.id == n.id)).count()
}

/// Advance the cards: new arrivals unroll (unless the bell is paused, a
/// script menu excepted, the view must stay clear or the list is open), time
/// runs out unless the panel is hovered, a card that ran out (or was
/// answered) folds into the bell. A scripted object's window has no time: it
/// stays out until answered, and comes back after whatever kept it in.
/// Returns whether something moves (a repaint is needed).
pub fn update(list: &Notifications, c: &mut CenterUi, m: Moment, message_seconds: f64) -> bool {
    let dt = c.last_time.map_or(0.0, |t| (m.now - t).max(0.0));
    c.last_time = Some(m.now);
    // a friend online: its own small toast, never a card nor a count
    for n in list
        .list
        .iter()
        .filter(|n| n.id > c.last_seen && n.transient && matches!(n.data, Data::Presence { .. }))
    {
        if !m.hidden {
            c.presence.push(presence::Toast {
                id: n.id,
                arrived: m.now,
                shown: None,
            });
        }
    }
    for n in list.list.iter().filter(|n| n.id > c.last_seen && card(n)) {
        if !n.stays_until_answered() && !n.read && !n.quiet && may_show(n, &m) {
            let total = show_seconds(n, message_seconds);
            c.peek.push(Peek {
                id: n.id,
                total,
                left: total,
                folding: None,
                pinned: false,
            });
        }
    }
    if let Some(last) = list.list.iter().map(|n| n.id).max() {
        c.last_seen = c.last_seen.max(last);
    }
    if m.open || m.hidden {
        // the list shows everything / the view stays clear: no card stays out
        c.peek.clear();
    } else if m.dnd {
        // the bell is paused: only the script menus stay out (#491)
        c.peek
            .retain(|p| p.pinned && list.list.iter().find(|n| n.id == p.id).is_none_or(dnd_shows));
    }
    // a scripted object's window stays out until answered: when it arrives,
    // and again once the list closes, the view clears or do not disturb
    // ends (seen in the list or not, it is still waiting)
    let mut back = false;
    for n in list.list.iter().filter(|n| n.stays_until_answered() && !n.quiet && card(n)) {
        if may_show(n, &m) && !c.peek.iter().any(|p| p.id == n.id) {
            c.peek.push(Peek {
                id: n.id,
                total: SHOW_ANSWER,
                left: SHOW_ANSWER,
                folding: None,
                pinned: true,
            });
            back = true;
        }
    }
    if back {
        c.peek.sort_by_key(|p| p.id);
    }
    for p in c.peek.iter_mut() {
        let gone = !list.list.iter().any(|n| n.id == p.id && (p.pinned || !n.read) && card(n));
        if p.folding.is_none() {
            if !m.hovered && !p.pinned {
                p.left -= dt;
            }
            if (p.left <= 0.0 && !p.pinned) || gone {
                p.folding = Some(m.now);
            }
        }
    }
    let before = c.peek.len();
    c.peek.retain(|p| p.folding.is_none_or(|t| m.now - t < FOLD));
    let folded = before != c.peek.len();
    // the friends' toasts, once the cards of this frame are known
    presence::advance(&mut c.presence, m.now, m.under_bell && (m.open || !c.peek.is_empty()));
    c.presence.retain(|t| list.list.iter().any(|n| n.id == t.id));
    let kept: Vec<u64> = c.presence.iter().map(|t| t.id).collect();
    c.presence_sizes.retain(|id, _| kept.contains(id));
    let count = bell_count(list, c);
    if count > c.shown_count || (folded && count > 0) {
        c.bump_at = Some(m.now);
        if !m.dnd {
            c.ring_at = Some(m.now);
        }
    }
    c.shown_count = count;
    let bell_moves = c.bump_at.is_some_and(|t| m.now - t < RING.max(BUMP));
    !c.peek.is_empty() || !c.presence.is_empty() || bell_moves
}

/// The cards shown rolled out (the newest three) and how many wait behind.
pub fn visible(c: &CenterUi) -> (Vec<Peek>, usize) {
    // the script menus waiting for an answer first (the newest three), the
    // newest other cards in the room left; the rest waits behind "+ N more"
    let mut keep: Vec<usize> = (0..c.peek.len()).rev().filter(|i| c.peek[*i].pinned).take(MAX_PEEK).collect();
    let room = MAX_PEEK - keep.len();
    keep.extend((0..c.peek.len()).rev().filter(|i| !c.peek[*i].pinned).take(room));
    keep.sort_unstable();
    let shown: Vec<Peek> = keep.iter().map(|i| c.peek[*i].clone()).collect();
    let behind = c.peek.len() - shown.len();
    (shown, behind)
}

/// Bell at the right end of the navigation bar: red count (grey when paused),
/// bounce when the count grows, a small ring; in do not disturb a grey bell
/// with a moon. Returns its response: a click opens the center, a right
/// click turns do not disturb on or off (#491).
/// In an island of the Aurora bars (`island`): a slot of the island, the
/// ringing bell while something waits, and the count in the violet of the
/// Conversations button's (one counter for both, grey while paused).
#[allow(clippy::too_many_arguments)]
pub fn bell(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    c: &CenterUi,
    unread: usize,
    open: bool,
    dnd: bool,
    island: bool,
) -> egui::Response {
    let now = ui.input(|i| i.time);
    let size = if island {
        Vec2::splat(super::bars::islands::SLOT)
    } else {
        Vec2::new(26.0, 22.0)
    };
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.hovered() || open {
        let radius = if island {
            ui.visuals().window_corner_radius
        } else {
            ui.visuals().widgets.inactive.corner_radius
        };
        ui.painter().rect_filled(rect, radius, p.raised);
    }
    let icon = if dnd {
        "bell-z-filled"
    } else if island && unread > 0 {
        "bell-ringing-filled"
    } else {
        "bell-filled"
    };
    let col = if dnd { p.muted } else { p.ink };
    // the ring: a damped swing around the top of the bell
    let angle = c
        .ring_at
        .filter(|_| !dnd)
        .map(|t| ((now - t) / RING) as f32)
        .filter(|t| *t < 1.0)
        .map_or(0.0, |t| (t * std::f32::consts::TAU * 2.0).sin() * 0.25 * (1.0 - t));
    if let Some(t) = icons.get(icon) {
        let r = Rect::from_center_size(rect.center(), Vec2::splat(16.0));
        egui::Image::new(t).tint(col).rotate(angle, Vec2::new(0.5, 0.1)).paint_at(ui, r);
    }
    // A full red dot: the moon read as the away status, and half of it
    // looked cut off by the bell (#491).
    if dnd {
        ui.painter().circle_filled(rect.left_bottom() + Vec2::new(5.0, -5.0), 3.5, p.danger);
    }
    // the bounce: 1 → 1.45 → 1 over 380 ms
    let s = c
        .bump_at
        .map(|t| ((now - t) / BUMP) as f32)
        .filter(|t| *t < 1.0)
        .map_or(1.0, |t| 1.0 + 0.45 * (t * std::f32::consts::PI).sin());
    if unread > 0 && island {
        super::bars::islands::count(ui, p, rect, unread, s, dnd);
    } else if unread > 0 {
        // the interface's one counter, pinned by its top right corner: the
        // bounce grows inside the bar
        let anchor = rect.right_top() + Vec2::new(3.0, -1.0);
        super::widgets::UnreadCount::new(unread, super::widgets::CountSize::Bar)
            .scale(s)
            .paused(dnd)
            .paint(ui.painter(), p, anchor, egui::Align2::RIGHT_TOP);
    }
    let tip = if dnd {
        aurora_i18n::tr!("ui-notifications-bell-paused").to_owned()
    } else {
        aurora_i18n::tr!("notifications-unread-count", count = unread)
    };
    resp.on_hover_text(super::notifications::bell_tip(tip, dnd))
}

/// What a click in the center asks the app for, besides answering a card.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// Turn do not disturb off (the paused bell's banner).
    EndDnd,
    /// An entry of the history: put back in "To do", open what it was about.
    Follow(Entry),
}

/// The center: the rolled-out cards, or the full list when open. Returns the
/// answers to apply and the other requests.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    icons: &Icons,
    world: &World,
    list: &mut Notifications,
    hist: &mut History,
    nui: &mut NotifUi,
    env: &mut ToastEnv<'_>,
    dnd: bool,
) -> (Vec<(u64, Response)>, Vec<Ask>) {
    let mut answers = Vec::new();
    let mut asks = Vec::new();
    let now = ctx.input(|i| i.time);
    // published by the islands this frame (Firestorm's bars have none)
    let column = nui.column.take();
    let hovered = nui
        .center
        .panel
        .zip(ctx.pointer_hover_pos())
        .is_some_and(|(r, pos)| r.contains(pos));
    let moving = update(
        list,
        &mut nui.center,
        Moment {
            now,
            hovered,
            dnd,
            hidden: env.hidden,
            open: nui.open,
            under_bell: env.presence_at == PresencePosition::TopRight,
        },
        env.message_seconds,
    );
    // a friend's notice leaves the list with its toast, a message kept out
    // of the bell with its card
    let (presence, peek) = (&nui.center.presence, &nui.center.peek);
    list.list
        .retain(|n| !n.transient || presence.iter().any(|t| t.id == n.id) || peek.iter().any(|p| p.id == n.id));
    let Some(bell) = nui.bell else {
        return (answers, asks);
    };
    presence_toasts(ctx, p, list, nui, env, column, now, &mut answers);
    let screen = ctx.content_rect();
    // with the islands, exactly under the system and bell islands, the
    // overlay's margins and border within that width; with Firestorm's bars
    // its own width, ending under the bell
    let (x, inner) = match column {
        Some(c) => (c.left(), c.width() - PANEL_FRAME),
        None => (
            ((bell.right() + 4.0).min(screen.right() - 6.0) - WIDTH).max(screen.left() + 6.0),
            WIDTH - 12.0,
        ),
    };
    nui.center.inner_width = inner;
    let pos = egui::pos2(x, bell.bottom() + 6.0);
    let unroll = ctx.animate_bool_with_time_and_easing(
        egui::Id::unique("notification_center_unroll"),
        nui.open || !nui.center.peek.is_empty(),
        UNROLL,
        egui::emath::easing::cubic_out,
    );
    if unroll <= 0.0 {
        nui.center.panel = None;
        if moving {
            ctx.request_repaint();
        }
        return (answers, asks);
    }
    // the height it had last time it was fully out (a clip read back from the
    // panel itself would feed its own list's height)
    let height = Some(nui.center.unrolled_height)
        .filter(|h| *h > 1.0)
        .unwrap_or(screen.height() * 0.5);
    let area = egui::Area::new(egui::Id::unique("notification_center"))
        .fade_in(false)
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .constrain(false)
        .show(ctx, |ui| {
            // unroll from the bell: the panel is revealed from its top
            if unroll < 1.0 {
                let clip = Rect::from_min_size(pos, Vec2::new(inner + PANEL_FRAME, (height * unroll).max(1.0)));
                ui.set_clip_rect(clip.intersect(ui.clip_rect()));
                ui.multiply_opacity(unroll);
            }
            super::widgets::overlay(ui, p, egui::Margin::same(6), |ui| {
                ui.set_width(inner);
                if nui.open {
                    full_list(ui, p, icons, world, list, hist, nui, env, dnd, &mut answers, &mut asks);
                } else {
                    rolled_out(ui, p, icons, world, list, nui, env, now, &mut answers);
                }
            });
        });
    nui.center.panel = Some(area.response.rect);
    if unroll >= 1.0 {
        nui.center.unrolled_height = area.response.rect.height();
    }
    // the rolled-out cards wait while the pointer is on them: say so beside
    if hovered && !nui.open && !nui.center.peek.is_empty() {
        egui::Area::new(egui::Id::unique("notification_center_paused"))
            .fade_in(false)
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(pos + Vec2::new(-6.0, 4.0))
            .interactable(false)
            .show(ctx, |ui| {
                super::widgets::overlay(ui, p, egui::Margin::symmetric(7, 3), |ui| {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("ui-notifications-paused-hint"))
                            .size(11.0)
                            .color(p.muted),
                    );
                });
            });
    }
    if nui.open {
        // a click outside closes the list (not on the bell: it toggles itself)
        let out = ctx.input(|i| i.pointer.any_pressed())
            && ctx
                .pointer_interact_pos()
                .is_some_and(|q| !area.response.rect.contains(q) && !bell.contains(q));
        if out {
            nui.open = false;
        }
    }
    if moving || unroll < 1.0 {
        ctx.request_repaint();
    }
    (answers, asks)
}

/// The new cards under the bell, each with its time left, folding away.
#[allow(clippy::too_many_arguments)]
fn rolled_out(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    world: &World,
    list: &mut Notifications,
    nui: &mut NotifUi,
    env: &mut ToastEnv<'_>,
    now: f64,
    answers: &mut Vec<(u64, Response)>,
) {
    ui.spacing_mut().item_spacing.y = 6.0;
    let (shown, behind) = visible(&nui.center);
    // newest on top
    for peek in shown.iter().rev() {
        let Some(n) = list.list.iter_mut().find(|n| n.id == peek.id) else {
            continue;
        };
        let fold = peek.folding.map_or(0.0, |t| (((now - t) / FOLD) as f32).clamp(0.0, 1.0));
        let full_h = nui.center.heights.get(&peek.id).copied().unwrap_or(0.0);
        let keep = 1.0 - fold;
        let top = ui.cursor().min;
        let width = nui.center.inner_width;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(top, Vec2::new(width, 4000.0))));
        if fold > 0.0 {
            child.set_clip_rect(Rect::from_min_size(top, Vec2::new(width + 12.0, full_h * keep)).intersect(ui.clip_rect()));
            child.multiply_opacity(keep);
        }
        let r = center_card(&mut child, p, icons, world, env, &mut nui.wanted_pics, n);
        let card = child.min_rect();
        nui.center.heights.insert(peek.id, card.height());
        // the time left, a fine violet bar along the card's bottom
        if peek.folding.is_none() && !peek.pinned {
            let frac = (peek.left / peek.total).clamp(0.0, 1.0) as f32;
            let bar = Rect::from_min_size(egui::pos2(card.left(), card.bottom() - 2.0), Vec2::new(card.width() * frac, 2.0));
            child.painter().rect_filled(bar, 0.0, p.violet_light.gamma_multiply(0.55));
        }
        let used = if fold > 0.0 { full_h * keep } else { card.height() };
        ui.advance_cursor_after_rect(Rect::from_min_size(top, Vec2::new(card.width(), used)));
        if let Some(r) = r {
            answers.push((peek.id, r));
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
}

/// The full list: "To do" and "History" tabs, "Mark all read", the paused
/// bell's banner.
#[allow(clippy::too_many_arguments)]
fn full_list(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    world: &World,
    list: &mut Notifications,
    hist: &mut History,
    nui: &mut NotifUi,
    env: &mut ToastEnv<'_>,
    dnd: bool,
    answers: &mut Vec<(u64, Response)>,
    asks: &mut Vec<Ask>,
) {
    ui.horizontal(|ui| {
        if let Some(t) = icons.get("bell-filled") {
            ui.add(egui::Image::new(t).tint(p.violet_light).fit_to_exact_size(Vec2::splat(14.0)));
        }
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-notifications-notifications"))
                .size(13.0)
                .strong()
                .color(p.ink),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let any = list.listed().any(|n| !n.interactive() || !n.read);
            if ui
                .add_enabled(
                    any,
                    egui::Button::new(
                        RichText::new(aurora_i18n::tr!("ui-notifications-mark-all-read"))
                            .size(11.5)
                            .color(p.muted),
                    )
                    .frame(false),
                )
                .on_hover_text(aurora_i18n::tr!("ui-notifications-mark-all-read-tip"))
                .clicked()
            {
                list.read_all();
            }
        });
    });
    if dnd {
        ui.add_space(2.0);
        egui::Frame::new()
            .fill(p.danger.gamma_multiply(0.12))
            .corner_radius(ui.visuals().widgets.inactive.corner_radius)
            .inner_margin(egui::Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if let Some(t) = icons.get("moon-filled") {
                        ui.add(egui::Image::new(t).tint(p.danger).fit_to_exact_size(Vec2::splat(12.0)));
                    }
                    // the text wraps before "Turn off"
                    let off_w = ui
                        .painter()
                        .layout_no_wrap(
                            aurora_i18n::tr!("ui-notifications-dnd-off").to_owned(),
                            egui::FontId::proportional(11.5),
                            p.ink,
                        )
                        .size()
                        .x;
                    let text_w = (ui.available_width() - off_w - 12.0).max(40.0);
                    ui.allocate_ui(Vec2::new(text_w, 0.0), |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(aurora_i18n::tr!("ui-notifications-dnd-banner"))
                                    .size(11.5)
                                    .strong()
                                    .color(p.danger),
                            )
                            .wrap(),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let off = ui
                            .add(
                                egui::Label::new(
                                    RichText::new(aurora_i18n::tr!("ui-notifications-dnd-off"))
                                        .size(11.5)
                                        .strong()
                                        .color(p.ink),
                                )
                                .sense(egui::Sense::click()),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        if off.clicked() {
                            asks.push(Ask::EndDnd);
                        }
                    });
                });
            });
    }
    ui.add_space(4.0);
    tabs(ui, p, &mut nui.center.tab);
    ui.add_space(4.0);
    let max_h = ui.ctx().content_rect().height() * 0.65;
    match nui.center.tab {
        Tab::ToDo => {
            if list.listed().next().is_none() {
                empty(ui, p, aurora_i18n::tr!("ui-notifications-nothing-to-do"));
                return;
            }
            egui::ScrollArea::vertical()
                .max_height(max_h)
                .min_scrolled_height(max_h)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    // the region restart pinned at the top, then newest first
                    let pinned = |n: &Notification| matches!(n.data, Data::RegionRestart);
                    for first in [true, false] {
                        for n in list.list.iter_mut().rev().filter(|n| !n.transient && pinned(n) == first) {
                            if let Some(r) = center_card(ui, p, icons, world, env, &mut nui.wanted_pics, n) {
                                answers.push((n.id, r));
                            }
                        }
                    }
                });
        }
        Tab::History => history_list(ui, p, icons, world, hist, &mut nui.center.query, max_h, asks, &nui.privacy),
    }
}

/// The "To do" / "History" tabs of a list (Aurora's center and Firestorm's
/// well); returns whether the history is shown.
pub(super) fn tabs(ui: &mut egui::Ui, p: &Palette, tab: &mut Tab) -> bool {
    let options = [
        (aurora_i18n::tr!("ui-notifications-to-do"), false),
        (aurora_i18n::tr!("ui-notifications-history"), false),
    ];
    let mut i = usize::from(*tab == Tab::History);
    if super::widgets::segmented(ui, p, &mut i, &options) {
        *tab = if i == 1 { Tab::History } else { Tab::ToDo };
    }
    *tab == Tab::History
}

/// The history with its search, by day, and the action left on each entry
/// (shared by both styles).
#[allow(clippy::too_many_arguments)]
pub(super) fn history_list(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    world: &World,
    hist: &mut History,
    query: &mut String,
    max_h: f32,
    asks: &mut Vec<Ask>,
    privacy: &crate::world::rlva::privacy::TextFilter,
) {
    ui.add(
        egui::TextEdit::singleline(query)
            .hint_text(aurora_i18n::tr!("ui-notifications-search-history"))
            .desired_width(f32::INFINITY),
    );
    ui.add_space(4.0);
    let query = query.trim().to_lowercase();
    let found: Vec<(usize, Entry)> = history::search(hist, "")
        .into_iter()
        .filter_map(|(i, e)| {
            let mut e = e.clone();
            privacy.redact_notice(&mut e.title, &mut e.body, &mut e.name_links);
            (query.is_empty() || e.title.to_lowercase().contains(&query) || e.body.to_lowercase().contains(&query)).then_some((i, e))
        })
        .collect();
    if found.is_empty() {
        empty(ui, p, aurora_i18n::tr!("ui-notifications-history-empty"));
        return;
    }
    let today = history::now_secs();
    egui::ScrollArea::vertical()
        .id_salt("notification_history")
        .max_height(max_h)
        .min_scrolled_height(max_h)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let mut last_day = None;
            for (index, e) in found {
                let day = day_label(e.time, today);
                if last_day.as_ref() != Some(&day) {
                    ui.label(RichText::new(day.to_uppercase()).size(10.5).strong().color(p.muted_dim));
                    last_day = Some(day);
                }
                if let Some(follow) = history_card(ui, p, icons, world, &e) {
                    if follow == Follow::Restore {
                        if let Some(e) = hist.take(index) {
                            asks.push(Ask::Follow(e));
                        }
                    } else {
                        asks.push(Ask::Follow(e));
                    }
                }
            }
        });
}

/// A card of the center; unread, it has the violet edge.
fn center_card(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    world: &World,
    env: &mut ToastEnv<'_>,
    wanted: &mut std::collections::HashSet<uuid::Uuid>,
    n: &mut Notification,
) -> Option<Response> {
    if matches!(n.data, Data::RegionRestart)
        && let Some(info) = &env.restart
    {
        return super::region_restart::bell_card(ui, p, info);
    }
    let read = n.read;
    let inner = ui.scope(|ui| {
        if matches!(n.data, Data::Message { .. }) {
            cards::person_card(ui, p, icons, world, env, wanted, n, false)
        } else {
            cards::card(ui, p, icons, world, n, false, true)
        }
    });
    if !read {
        let r = inner.response.rect;
        ui.painter().rect_filled(
            Rect::from_min_max(r.left_top(), egui::pos2(r.left() + 2.0, r.bottom())),
            0.0,
            p.violet,
        );
    }
    inner.inner
}

/// The nearby chat lines over the chat bar (`ui::chat::toasts`), when
/// they show: the friends' toasts stack above them.
fn chat_lines(ctx: &egui::Context) -> Option<Rect> {
    let layer = egui::LayerId::new(egui::Order::Background, egui::Id::unique("chat_toasts"));
    ctx.memory(|m| m.areas().is_visible(&layer).then(|| m.area_rect(layer.id)).flatten())
}

/// The friends' online / offline toasts at their place in the room the
/// bars leave (`presence`); clicking one opens an IM. At the top right with
/// the islands they take the center's width under the system and bell
/// islands (`column`), their text cut short if it does not fit.
#[allow(clippy::too_many_arguments)]
fn presence_toasts(
    ctx: &egui::Context,
    p: &Palette,
    list: &Notifications,
    nui: &mut NotifUi,
    env: &mut ToastEnv<'_>,
    column: Option<Rect>,
    now: f64,
    answers: &mut Vec<(u64, Response)>,
) {
    let shown: Vec<(u64, f64)> = nui.center.presence.iter().filter_map(|t| Some((t.id, t.shown?))).collect();
    if shown.is_empty() || env.hidden {
        return;
    }
    let mut room = presence::room(super::layout::work_area(ctx), env.classic_bars);
    let fixed = column.filter(|_| env.presence_at == PresencePosition::TopRight);
    if let Some(c) = fixed {
        room.max.x = c.right();
    }
    let sizes: Vec<(Vec2, bool)> = shown
        .iter()
        .map(|(id, _)| {
            let (size, measured) = presence::size_of(&nui.center.presence_sizes, *id);
            (fixed.map_or(size, |c| Vec2::new(c.width(), size.y)), measured)
        })
        .collect();
    let avoid: Vec<Rect> = chat_lines(ctx).into_iter().collect();
    let only: Vec<Vec2> = sizes.iter().map(|(s, _)| *s).collect();
    let rects = presence::stack(room, env.presence_at, &only, &avoid);
    let Some(min) = rects.iter().flatten().map(|r| r.min).reduce(|a, b| a.min(b)) else {
        return;
    };
    egui::Area::new(egui::Id::unique("notification_presence"))
        .fade_in(false)
        .order(egui::Order::Foreground)
        .fixed_pos(min)
        .constrain(false)
        .show(ctx, |ui| {
            for (((id, since), rect), (_, measured)) in shown.into_iter().zip(rects).zip(sizes) {
                let (Some(rect), Some(n)) = (rect, list.list.iter().find(|n| n.id == id)) else {
                    continue;
                };
                let Data::Presence { who, online } = n.data else {
                    continue;
                };
                let fade = ((now - since - PRESENCE) / PRESENCE_FADE).clamp(0.0, 1.0) as f32;
                let mut name_hover = None;
                let frame = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                        // laid out with a guess on its first frame: unseen, measured
                        if !measured {
                            ui.set_invisible();
                        }
                        ui.multiply_opacity(1.0 - fade);
                        super::widgets::overlay(ui, p, egui::Margin::symmetric(12, 8), |ui| {
                            if fixed.is_some() {
                                ui.set_width(ui.available_width());
                            }
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 9.0;
                                let (pic, _) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::hover());
                                nui.wanted_pics.insert(who);
                                // the initials of the name, not of the notice's quote
                                let name = n.title.trim_start_matches(['\'', '"']);
                                super::widgets::avatar_disc(ui, p, pic, env.pics.get(&who), name, false);
                                let status = if online {
                                    super::presence::Status::Online
                                } else {
                                    super::presence::Status::Offline
                                };
                                super::presence::paint_dot(ui.painter(), p, pic, status, p.panel);
                                name_hover = cards::text::show(ui, p, &n.title, &n.name_links, 12.5, p.ink, true, false);
                            });
                        })
                        .response
                        .rect
                    })
                    .inner;
                nui.center.presence_sizes.insert(id, frame.size());
                if !measured {
                    continue;
                }
                let click = ui.interact(frame, egui::Id::unique(("notification_presence", id)), egui::Sense::click());
                if name_hover.is_some() {
                    let click = click
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(aurora_i18n::tr!("common-view-profile"));
                    if click.clicked()
                        && let Some(target) = name_hover
                    {
                        cards::text::open(ctx, target);
                    }
                    continue;
                }
                if n.opens() {
                    let click = click
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(aurora_i18n::tr!("ui-notifications-send-im"));
                    if click.clicked() {
                        answers.push((id, Response::Open));
                    }
                }
            }
        });
}

fn empty(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add_space(12.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(text).size(12.0).color(p.muted_dim));
    });
    ui.add_space(12.0);
}

fn at(secs: u64) -> std::time::SystemTime {
    std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs)
}

/// "Today", "Yesterday" or the date of a history entry, in Second Life time
/// like the days of the conversations.
pub fn day_label(time: u64, now: u64) -> String {
    let day = |t: u64| {
        let s = Slt::of(at(t));
        (s.year, s.month, s.day)
    };
    let d = day(time);
    if d == day(now) {
        aurora_i18n::tr!("ui-chat-today").to_owned()
    } else if d == day(now.saturating_sub(86_400)) {
        aurora_i18n::tr!("ui-chat-yesterday").to_owned()
    } else {
        let month = crate::ui::land::months()[(d.1.clamp(1, 12) - 1) as usize];
        aurora_i18n::tr!(
            "ui-chat-day",
            day = d.2.to_string(),
            month = month.to_string(),
            year = d.0.to_string()
        )
        .to_owned()
    }
}

fn outcome_label(o: Outcome) -> &'static str {
    match o {
        Outcome::Answered => aurora_i18n::tr!("ui-notifications-outcome-answered"),
        Outcome::Declined => aurora_i18n::tr!("ui-notifications-outcome-declined"),
        Outcome::Kept => aurora_i18n::tr!("ui-notifications-outcome-kept"),
        Outcome::Ignored => aurora_i18n::tr!("ui-notifications-outcome-ignored"),
        Outcome::Read => aurora_i18n::tr!("ui-notifications-outcome-read"),
    }
}

fn follow_label(f: &Follow) -> Option<&'static str> {
    Some(match f {
        Follow::None => return None,
        Follow::Conversation(_) => aurora_i18n::tr!("ui-notifications-open-conversation"),
        Follow::Place(_) => aurora_i18n::tr!("ui-notifications-show-place"),
        Follow::Inventory(_) => aurora_i18n::tr!("ui-notifications-show-in-inventory"),
        Follow::Restore => aurora_i18n::tr!("ui-notifications-restore"),
    })
}

/// A greyed card of the history: its state, and the useful action left.
fn history_card(ui: &mut egui::Ui, p: &Palette, icons: &Icons, world: &World, e: &Entry) -> Option<Follow> {
    let mut out = None;
    let (icon, accent) = cards::look_style(p, e.look);
    egui::Frame::new()
        .fill(p.field.gamma_multiply(0.8))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.multiply_opacity(0.75);
            ui.horizontal_top(|ui| {
                if let Some(t) = icons.get(icon) {
                    ui.add(egui::Image::new(t).tint(accent).fit_to_exact_size(Vec2::splat(16.0)));
                }
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        // keep room for the time
                        let w = (ui.available_width() - 40.0).max(40.0);
                        ui.allocate_ui(Vec2::new(w, 18.0), |ui| {
                            cards::text::show(ui, p, &e.title, &e.name_links, 12.5, p.ink, true, true);
                        });
                        ui.label(RichText::new(hhmm(e.time)).size(10.5).color(p.muted_dim));
                    });
                    if !e.body.is_empty() {
                        super::chat::text_with_links(ui, p, world, &e.body, &e.name_links, 12.0, p.muted);
                    }
                    ui.horizontal(|ui| {
                        egui::Frame::new()
                            .fill(p.raised)
                            .corner_radius(ui.visuals().window_corner_radius)
                            .inner_margin(egui::Margin::symmetric(7, 1))
                            .show(ui, |ui| {
                                ui.label(RichText::new(outcome_label(e.outcome)).size(10.5).strong().color(p.muted));
                            });
                        if let Some(label) = follow_label(&e.follow) {
                            let link = ui
                                .add(
                                    egui::Label::new(RichText::new(label).size(11.0).strong().color(p.violet_light))
                                        .sense(egui::Sense::click()),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if link.clicked() {
                                out = Some(e.follow.clone());
                            }
                        }
                    });
                });
            });
        });
    out
}

fn hhmm(secs: u64) -> String {
    super::chat::hhmm(at(secs))
}

pub mod presence;

#[cfg(test)]
pub(super) mod tests;
