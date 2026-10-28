//! Aurora's top bar: glass islands laid over the world instead of a
//! full-width band (variant A of the bars mockup the human validated). From
//! the left: the Aurora menu, the bug report (when shown), the place
//! (stretched to the account island,
//! `place.rs`), the account (picture with the online status, balance), the
//! system (SLT clock, latency, frame rate, Firestorm's traffic indicator,
//! media and volume, as its right click chooses; a detailed tooltip) and the
//! bell.
//! The islands keep the windows' blur, sampled within their rectangles
//! (`widgets::island_shape`): no extra GPU pass. Preferences › Aurora ›
//! Appearance › "Top and bottom bars" brings Firestorm's bars back
//! (`top_bars`, unchanged).

use super::{BarAction, BellState, StatusInfo, clock, main_menus, paint_icon, status_color, status_menu_body, status_shape, status_tip};
use crate::theme::Palette;
use crate::ui::Panels;
use crate::ui::icons::Icons;
use crate::ui::menu;
use egui::{Color32, FontId, Id, Pos2, Rect, Sense, Stroke, Vec2};

/// Gap between the islands and to the screen's edges.
pub const MARGIN: f32 = 8.0;
/// Height of an island.
pub const HEIGHT: f32 = 36.0;
/// Height (and least width) of a slot in an island: a button, a readout.
pub const SLOT: f32 = 28.0;
/// Room between an island's edge and its first slot.
pub const PAD: f32 = 4.0;
/// Room between two slots of an island.
pub const GAP: f32 = 2.0;
/// Room around the icon and the text inside a slot with text.
const SLOT_PAD: f32 = 6.0;
/// Icons of the slots (15 in the system island, like the mockup).
pub const ICON: f32 = 16.0;
const SYSTEM_ICON: f32 = 15.0;

/// What the islands leave to the windows: below the islands (8 px under
/// them) and 8 px from the left and right edges, so that a window snapped
/// there lines up with the islands (`ui::layout`).
pub fn work_area(screen: Rect, bottom: f32) -> Rect {
    Rect::from_min_max(
        Pos2::new(screen.left() + MARGIN, screen.top() + MARGIN + HEIGHT + MARGIN),
        Pos2::new(screen.right() - MARGIN, bottom),
    )
}

/// The bottom of the favourites' row under the islands, when it is shown
/// (`top` this frame): the windows' work area starts 8 px under it.
pub fn favorites_row_bottom(ctx: &egui::Context) -> Option<f32> {
    ctx.data(|d| d.get_temp::<f32>(Id::unique("bars_favorites_row_bottom")))
}

/// The rectangle of an island between `left` and `right` on the row.
fn row(screen: Rect, left: f32, right: f32) -> Rect {
    let top = screen.top() + MARGIN;
    Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, top + HEIGHT))
}

/// The slot face: the raised face under the pointer or while its menu is
/// open, the accent while `active`, nothing at rest.
pub(crate) fn slot_face(ui: &egui::Ui, p: &Palette, rect: Rect, hot: bool, active: bool) {
    let fill = match (active, hot) {
        (true, true) => Some(p.violet.gamma_multiply(0.55)),
        (true, false) => Some(p.violet.gamma_multiply(0.40)),
        (false, true) => Some(p.raised),
        (false, false) => None,
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
    }
}

/// The one unread counter of the islands (the bell, the chat island's
/// button, a Conversations button added to the dock), the shared
/// `widgets::unread_count`: pinned by its top right corner 2 px right of the
/// slot's, `scale` for the bell's bounce; grey while do not disturb pauses
/// the bell.
pub fn count(ui: &egui::Ui, p: &Palette, slot: Rect, unread: usize, scale: f32, paused: bool) {
    let anchor = slot.right_top() + Vec2::new(2.0, 0.0);
    crate::ui::widgets::UnreadCount::new(unread, crate::ui::widgets::CountSize::Bar)
        .scale(scale)
        .paused(paused)
        .paint(ui.painter(), p, anchor, egui::Align2::RIGHT_TOP);
}

/// The ink of a slot's icon and text.
pub(super) fn slot_ink(p: &Palette, hot: bool, enabled: bool) -> Color32 {
    if !enabled {
        p.muted_dim.gamma_multiply(0.55)
    } else if hot {
        p.ink
    } else {
        p.muted
    }
}

/// An icon button of `SLOT` × `SLOT` at `rect`.
pub(super) fn icon_slot(ui: &mut egui::Ui, p: &Palette, icons: &Icons, rect: Rect, id: Id, icon: &str, enabled: bool) -> egui::Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let resp = ui.interact(rect, id, sense);
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let hot = enabled && (resp.hovered() || open);
    slot_face(ui, p, rect, hot, false);
    paint_icon(ui, icons, icon, rect, ICON, slot_ink(p, hot, enabled));
    resp
}

/// A text laid out in the font of the islands' readouts.
fn text(ui: &egui::Ui, s: String, size: f32, color: Color32) -> std::sync::Arc<egui::Galley> {
    ui.painter().layout_no_wrap(s, FontId::proportional(size), color)
}

/// The width a changing readout keeps: its text with every digit made the
/// widest one, so that the clock's seconds do not move the islands.
fn steady_width(ui: &egui::Ui, s: &str, size: f32) -> f32 {
    let wide: String = s.chars().map(|c| if c.is_ascii_digit() { '0' } else { c }).collect();
    let probe = |d: char| text(ui, wide.replace('0', &d.to_string()), size, Color32::PLACEHOLDER).size().x;
    ['0', '4', '8'].into_iter().map(probe).fold(0.0, f32::max).ceil()
}

/// How long a readout keeps a width it no longer needs, in seconds.
const HOLD: f64 = 10.0;

/// The width a readout takes (#1246): the widest steady width its text
/// needed in the last `HOLD` seconds, `id` remembering it. A frame rate
/// going back and forth across 100 (a latency across 100 ms, a 12-hour
/// clock across 10:00) does not move the bar, and no margin is kept for a
/// digit the value never reaches.
pub(super) fn held_width(ui: &egui::Ui, id: Id, s: &str, size: f32) -> f32 {
    let w = steady_width(ui, s, size);
    let ctx = ui.ctx();
    let now = ctx.input(|i| i.time);
    let (held, since) = ctx.data(|d| d.get_temp::<(f32, f64)>(id)).unwrap_or((w, now));
    if w >= held || now - since >= HOLD {
        // `since`: the last time the held width was needed
        ctx.data_mut(|d| d.insert_temp(id, (w, now)));
        w
    } else {
        held
    }
}

/// The latency's text.
pub(super) fn ping_text(ms: u32) -> String {
    format!("{ms} ms")
}

/// The frame rate's text.
pub(super) fn fps_text(fps: f32) -> String {
    aurora_i18n::number(f64::from(fps), 1)
}

/// A readout of the system island: icon then text, `width` wide.
#[allow(clippy::too_many_arguments)]
fn readout(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    rect: Rect,
    id: Id,
    icon: &str,
    galley: std::sync::Arc<egui::Galley>,
    color: Color32,
    clickable: bool,
) -> egui::Response {
    let resp = ui.interact(rect, id, if clickable { Sense::click() } else { Sense::hover() });
    let hot = clickable && resp.hovered();
    slot_face(ui, p, rect, hot, false);
    let icon_rect = Rect::from_center_size(
        Pos2::new(rect.left() + SLOT_PAD + SYSTEM_ICON / 2.0, rect.center().y),
        Vec2::splat(SYSTEM_ICON),
    );
    paint_icon(ui, icons, icon, icon_rect, SYSTEM_ICON, slot_ink(p, hot, true));
    let x = icon_rect.right() + SLOT_PAD;
    paint_readout_text(ui, galley, Pos2::new(x, rect.center().y), color);
    if clickable && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// A readout's text, its left at `left_center.x` and its digits' ink
/// centred on `left_center.y`, so that every readout of the island shares
/// one baseline: centring each text's own ink lifted the frame rate, whose
/// comma hangs below the line ("120,0" stood 2 px above "0 ms").
fn paint_readout_text(ui: &egui::Ui, galley: std::sync::Arc<egui::Galley>, left_center: Pos2, color: Color32) {
    let font = galley.job.sections.first().map(|s| s.format.font_id.clone()).unwrap_or_default();
    let digit = ui.painter().layout_no_wrap("0".to_owned(), font, color);
    let digits = crate::ui::widgets::ink_rect(&digit);
    let ink = crate::ui::widgets::ink_rect(&galley);
    // on the physical pixel, where epaint draws it (`widgets::ink_rect`)
    let pos = Pos2::new(left_center.x - ink.left(), left_center.y - digits.center().y);
    let pos = egui::emath::GuiRounding::round_to_pixels(pos, ui.painter().pixels_per_point());
    ui.painter().galley(pos, galley, color);
}

/// The width of a readout whose text is `text_w` wide.
fn readout_width(text_w: f32) -> f32 {
    SLOT_PAD + SYSTEM_ICON + SLOT_PAD + text_w + SLOT_PAD
}

/// Demo captures (`AURORA_DEMO_BARS`): the menu to open once, at frame 300.
pub(super) fn demo_open(ctx: &egui::Context, resp: &egui::Response, which: super::demo::Open) {
    if super::demo::get().open == Some(which) && ctx.cumulative_frame_nr() == 300 {
        egui::Popup::open_id(ctx, egui::Popup::default_response_id(resp));
    }
}

/// The top islands. Returns the bar's action and the audio controls' one.
#[allow(clippy::too_many_arguments)]
pub fn top(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    panels: &mut Panels,
    st: &StatusInfo,
    audio: &mut crate::settings::AudioSettings,
    audio_ui: &mut crate::ui::audio::AudioUi,
    media: crate::ui::audio::MediaState,
    notif: &mut crate::ui::notifications::NotifUi,
    bell: BellState,
) -> (BarAction, crate::ui::audio::AudioAction) {
    let mut action = BarAction::None;
    let ctx = ui.ctx().clone();
    let screen = ui.max_rect();

    // the rectangles first, from the right end: the bell, the system, the
    // account; the menu at the left; the place in what is left between
    let mut right = screen.right() - MARGIN;
    let bell_rect = row(screen, right - PAD * 2.0 - SLOT, right);
    right = bell_rect.left() - MARGIN;
    let sys_rect = row(screen, right - system_width(ui, st), right);
    right = sys_rect.left() - MARGIN;
    let acc_rect = row(screen, right - account_width(ui, p, st), right);
    right = acc_rect.left() - MARGIN;
    // the conversations' thumbnails in an island of their own, left of the
    // account, where the Legacy Viewer's chiclets stand (#1134); the place
    // keeps two thirds of the row
    let thumbs_w = crate::ui::chat::thumbnails::top_width(st.conversation_thumbnails, SLOT, PAD, (right - screen.left()) / 3.0);
    if thumbs_w > 0.0 {
        let thumbs = row(screen, right - thumbs_w, right);
        crate::ui::chat::thumbnails::reserve_top(&ctx, thumbs, SLOT, true);
        right = thumbs.left() - MARGIN;
    }
    let left = screen.left() + MARGIN;
    let menu_rect = row(screen, left, left + PAD * 2.0 + SLOT);
    // the bug report in an island of its own after the wolf's (the human's
    // request: in one island the two read as one button); hidden, the place
    // follows the wolf 8 px away
    let bug_rect = st
        .show_bug_report
        .then(|| row(screen, menu_rect.right() + MARGIN, menu_rect.right() + MARGIN + PAD * 2.0 + SLOT));
    let span = row(screen, bug_rect.unwrap_or(menu_rect).right() + MARGIN, right);
    let (place_rect, detail) = super::place::layout(ui, st, span);

    // each island is an area of its own: the pointer over it is over the
    // interface (`Context::is_pointer_over_egui`), between them over the world
    notif.column = Some(Rect::from_min_max(sys_rect.min, bell_rect.max));
    let bell_slot = Rect::from_center_size(bell_rect.center(), Vec2::splat(SLOT));
    area(&ctx, p, "bell", bell_rect, |ui| {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bell_slot));
        let resp = if bell.aurora {
            crate::ui::notification_center::bell(&mut child, p, icons, &notif.center, bell.unread, notif.open, bell.dnd, true)
        } else {
            crate::ui::notifications::bell(&mut child, p, icons, bell.unread, notif.open, bell.dnd, true)
        };
        let rect = resp.rect;
        notif.anchor = Some(Pos2::new(rect.left(), bell_rect.bottom()));
        notif.bell = Some(Rect::from_min_max(rect.min, Pos2::new(rect.right(), bell_rect.bottom())));
        if resp.clicked() {
            notif.open = !notif.open;
        }
        // do not disturb on or off (#491), as the status selector does
        if resp.secondary_clicked() {
            action = BarAction::SetStatus(crate::ui::notifications::bell_dnd_toggled(st.status));
        }
    });
    let (audio_action, readouts_hovered) = area(&ctx, p, "system", sys_rect, |ui| {
        system(ui, p, icons, panels, st, audio, audio_ui, media, sys_rect, &mut action)
    });
    system_tip(
        &ctx,
        p,
        icons,
        st,
        media,
        sys_rect,
        readouts_hovered && !audio_ui.panel_open,
        &mut action,
    );
    area(&ctx, p, "account", acc_rect, |ui| account(ui, p, st, acc_rect, &mut action));
    area(&ctx, p, "menu", menu_rect, |ui| {
        // a right click on the island or its buttons: show the bug button
        let back = ui.interact(menu_rect, Id::unique("bars_island_menu_back"), Sense::click());
        menu_options(&back, p, st, &mut action);
        let wolf = Rect::from_min_size(
            Pos2::new(menu_rect.left() + PAD, menu_rect.center().y - SLOT / 2.0),
            Vec2::splat(SLOT),
        );
        let resp = ui.interact(wolf, Id::unique("bars_island_menu"), Sense::click());
        menu_options(&resp, p, st, &mut action);
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
        slot_face(ui, p, wolf, resp.hovered() || open, false);
        paint_icon(ui, icons, "wolf", wolf, 18.0, p.ink);
        menu::dropdown_placed(
            &resp,
            p,
            egui::PopupCloseBehavior::CloseOnClick,
            Some((egui::RectAlign::BOTTOM_START, 10.0)),
            |ui| {
                main_menus(ui, p, panels, st, &mut action, true);
            },
        );
        resp.on_hover_text(aurora_i18n::tr!("ui-bars-aurora-menu"));
    });
    if let Some(bug_rect) = bug_rect {
        area(&ctx, p, "bug", bug_rect, |ui| {
            // its right click copies the version (the wolf's menu still
            // hides it)
            let back = ui.interact(bug_rect, Id::unique("bars_island_bug_back"), Sense::click());
            let bug = Rect::from_center_size(bug_rect.center(), Vec2::splat(SLOT));
            let resp = icon_slot(ui, p, icons, bug, Id::unique("bars_island_bug"), "bug-filled", true);
            if back.secondary_clicked() || resp.secondary_clicked() {
                action = BarAction::CopyVersion;
            }
            if resp.on_hover_text(aurora_i18n::tr!("bug-title")).clicked() {
                action = BarAction::ReportBug;
            }
        });
    }
    area(&ctx, p, "place", place_rect, |ui| {
        if let Some(a) = super::place::island(ui, p, icons, panels, st, place_rect, detail) {
            action = a;
        }
    });
    // the favourites' row (F2) under the place island: the windows stay
    // under it too
    let row_id = Id::unique("bars_favorites_row_bottom");
    if st.favorites.access == crate::settings::FavoritesAccess::Row {
        let (row, a) = super::favorites::row(&ctx, p, icons, st.favorites, place_rect);
        if let Some(a) = a {
            action = a;
        }
        ctx.data_mut(|d| d.insert_temp(row_id, row.bottom()));
    } else {
        ctx.data_mut(|d| d.remove_temp::<f32>(row_id));
    }
    (action, audio_action)
}

/// Right-click menu of the menu island: the bug report button, also in
/// Preferences › Interface ("Report a bug in the top bar", on by default
/// during the alpha).
fn menu_options(resp: &egui::Response, p: &Palette, st: &StatusInfo, action: &mut BarAction) {
    menu::context_menu_of_button(resp, p, |ui| {
        if menu::check(ui, p, "bug-filled", aurora_i18n::tr!("bug-toolbar"), st.show_bug_report, true) {
            *action = BarAction::ShowBugButton(!st.show_bug_report);
        }
    });
}

/// One island in an area of its own at `rect`, its surface under what
/// `body` draws there (in screen coordinates).
pub(crate) fn area<R>(ctx: &egui::Context, p: &Palette, name: &'static str, rect: Rect, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    // (the windows keep their title clear of it: `layout::clear_of_islands`)
    crate::ui::layout::island_drawn(ctx, rect);
    egui::Area::new(Id::unique(("bars_island", name)))
        .order(egui::Order::Middle)
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(ctx, |ui| {
            ui.painter().add(crate::ui::widgets::island_shape(ui.ctx(), p, rect));
            ui.set_min_size(rect.size());
            body(ui)
        })
        .inner
}

/// The account's picture.
const DISC: f32 = 22.0;

fn balance_galley(ui: &egui::Ui, p: &Palette, st: &StatusInfo) -> Option<std::sync::Arc<egui::Galley>> {
    let font = FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx()));
    st.balance.map(|b| {
        ui.painter()
            .layout_no_wrap(format!("{} L$", aurora_i18n::integer(i64::from(b))), font, p.ink)
    })
}

/// The account's slot: 4 before the picture, 6 between, 8 after the text.
fn account_slot_width(balance: Option<&std::sync::Arc<egui::Galley>>) -> f32 {
    4.0 + DISC + balance.map_or(0.0, |g| 6.0 + g.size().x.ceil()) + 8.0
}

fn account_width(ui: &egui::Ui, p: &Palette, st: &StatusInfo) -> f32 {
    PAD * 2.0 + account_slot_width(balance_galley(ui, p, st).as_ref())
}

/// The account island: our picture with the online status on it (a click
/// opens the status menu), the balance (a click buys L$).
fn account(ui: &mut egui::Ui, p: &Palette, st: &StatusInfo, island: Rect, action: &mut BarAction) {
    let balance = balance_galley(ui, p, st);
    let w = account_slot_width(balance.as_ref());
    let rect = Rect::from_min_size(Pos2::new(island.left() + PAD, island.center().y - SLOT / 2.0), Vec2::new(w, SLOT));
    let disc = Rect::from_center_size(Pos2::new(rect.left() + 4.0 + DISC / 2.0, rect.center().y), Vec2::splat(DISC));
    // the picture and its status: one target; the balance: another
    let status_rect = Rect::from_min_max(rect.min, Pos2::new(disc.right() + 3.0, rect.bottom()));
    let money_rect = Rect::from_min_max(Pos2::new(status_rect.right(), rect.top()), rect.max);
    let status = ui.interact(status_rect, Id::unique("bars_island_status"), Sense::click());
    let money = ui.interact(money_rect, Id::unique("bars_island_balance"), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&status));
    slot_face(ui, p, rect, status.hovered() || money.hovered() || open, false);
    crate::ui::widgets::avatar_disc(ui, p, disc, st.own_pic, st.own_name, false);
    // the status dot on the picture's lower right, ringed with the island
    let dot = disc.right_bottom() - Vec2::splat(3.5);
    let col = status_color(p, &st.status);
    ui.painter().circle_filled(dot, 4.5, p.bar);
    crate::ui::presence::paint_mark(ui.painter(), dot, 3.0, status_shape(&st.status), col, p.bar);
    if let Some(g) = balance {
        crate::ui::widgets::paint_ink_left(ui.painter(), g, Pos2::new(disc.right() + 6.0, rect.center().y), p.ink);
    }
    if money.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if money.on_hover_text(aurora_i18n::tr!("ui-bars-balance-click-to-buy-l")).clicked() {
        *action = BarAction::BuyCurrency;
    }
    demo_open(ui.ctx(), &status, super::demo::Open::Status);
    menu::dropdown_placed(
        &status,
        p,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        Some((egui::RectAlign::BOTTOM_START, 10.0)),
        |ui| {
            status_menu_body(ui, p, st.status, action);
        },
    );
    status.on_hover_text(status_tip(&st.status));
}

/// The readouts of the system island.
struct Readouts {
    clock: String,
    ping: String,
    fps: String,
}

/// The readouts of the system island: clock, latency, frame rate.
fn readouts(st: &StatusInfo) -> Readouts {
    let s = st.system_island;
    Readouts {
        clock: clock::text(s),
        ping: ping_text(st.ping_ms),
        fps: fps_text(st.fps),
    }
}

/// The size of the readouts' text.
const READOUT: f32 = 12.0;
/// The speaker, radio and media buttons: three slots, 2 apart (`ui::audio`).
const AUDIO_W: f32 = SLOT * 3.0 + 2.0 * 2.0;
/// The traffic indicator's curve.
const NET_W: f32 = 36.0;

/// What the system island shows, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Item {
    Clock,
    Ping,
    Fps,
    /// Firestorm's traffic indicator (ShowNetStats).
    Net,
    /// Media, music and volume.
    Media,
}

/// The items shown, in their order, as the island's right click (and
/// Preferences › Interface for the traffic indicator) set them; the clock
/// when a settings file hides them all, the menu keeping the last one.
fn items(st: &StatusInfo) -> Vec<Item> {
    let s = st.system_island;
    let mut shown: Vec<Item> = [
        (Item::Clock, s.clock),
        (Item::Ping, s.ping),
        (Item::Fps, s.fps),
        (Item::Net, st.net_stats),
        (Item::Media, s.media),
    ]
    .into_iter()
    .filter_map(|(item, on)| on.then_some(item))
    .collect();
    if shown.is_empty() {
        shown.push(Item::Clock);
    }
    shown
}

fn item_width(ui: &egui::Ui, texts: &Readouts, item: Item) -> f32 {
    match item {
        Item::Clock => readout_width(held_width(ui, Id::unique(("bars_readout", item)), &texts.clock, READOUT)),
        Item::Ping => readout_width(held_width(ui, Id::unique(("bars_readout", item)), &texts.ping, READOUT)),
        Item::Fps => readout_width(held_width(ui, Id::unique(("bars_readout", item)), &texts.fps, READOUT)),
        Item::Net => readout_width(NET_W),
        Item::Media => AUDIO_W,
    }
}

/// The island's width: its items 2 px apart, no hole where one is hidden.
fn system_width(ui: &egui::Ui, st: &StatusInfo) -> f32 {
    let texts = readouts(st);
    let items = items(st);
    let slots: f32 = items.iter().map(|item| item_width(ui, &texts, *item)).sum();
    PAD + slots + GAP * items.len().saturating_sub(1) as f32 + PAD
}

/// The frame rate's colour: teal from 50 images a second, amber from 25.
fn fps_color(p: &Palette, fps: f32) -> Color32 {
    if fps >= 50.0 {
        p.teal
    } else if fps >= 25.0 {
        p.amber
    } else {
        p.danger
    }
}

/// The colour of the packets lost, at the thresholds of Firestorm's
/// traffic indicator (LLStatGraph of the status bar): under 1.25 %, from
/// 1.25 % and from 3 %.
pub(super) fn loss_color(p: &Palette, loss: f32) -> Color32 {
    if loss < 1.25 {
        p.teal
    } else if loss < 3.0 {
        p.amber
    } else {
        p.danger
    }
}

/// The system island: as its right click chooses, the clock (SLT or local
/// time, which a left click switches; its seconds, 12 or 24 hours), region
/// latency, frame rate (a click opens Performance),
/// Firestorm's traffic indicator, then media, music and volume. A right
/// click anywhere on it, its buttons too, opens its menu. Returns the audio
/// controls' action and whether the pointer is over the readouts (the
/// detailed tooltip, `system_tip`).
#[allow(clippy::too_many_arguments)]
fn system(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    panels: &mut Panels,
    st: &StatusInfo,
    audio: &mut crate::settings::AudioSettings,
    audio_ui: &mut crate::ui::audio::AudioUi,
    media: crate::ui::audio::MediaState,
    island: Rect,
    action: &mut BarAction,
) -> (crate::ui::audio::AudioAction, bool) {
    const SIZE: f32 = READOUT;
    let top = island.center().y - SLOT / 2.0;
    // under the readouts, for the right click
    let back = ui.interact(island, Id::unique("bars_island_system_back"), Sense::click());
    let texts = readouts(st);
    let mut audio_action = crate::ui::audio::AudioAction::None;
    let mut readouts_zone = Rect::NOTHING;
    let mut x = island.left() + PAD;
    for item in items(st) {
        let w = item_width(ui, &texts, item);
        let rect = Rect::from_min_size(Pos2::new(x, top), Vec2::new(w, SLOT));
        x = rect.right() + GAP;
        match item {
            Item::Clock => {
                let time = readout(
                    ui,
                    p,
                    icons,
                    rect,
                    Id::unique("bars_island_time"),
                    "clock-filled",
                    text(ui, texts.clock.clone(), SIZE, p.muted),
                    p.muted,
                    true,
                );
                if time.clicked() {
                    let s = st.system_island;
                    *action = BarAction::SystemIsland(crate::settings::SystemIsland {
                        clock_zone: clock::next(s.clock_zone),
                        ..s
                    });
                }
            }
            Item::Ping => {
                readout(
                    ui,
                    p,
                    icons,
                    rect,
                    Id::unique("bars_island_ping"),
                    "antenna-bars-5",
                    text(ui, texts.ping.clone(), SIZE, p.muted),
                    p.muted,
                    false,
                );
            }
            Item::Fps => {
                let color = fps_color(p, st.fps);
                let fps = readout(
                    ui,
                    p,
                    icons,
                    rect,
                    Id::unique("bars_island_fps"),
                    "gauge-filled",
                    text(ui, texts.fps.clone(), SIZE, color),
                    color,
                    true,
                );
                if fps.clicked() {
                    crate::ui::perf::place_under(ui.ctx(), island);
                    panels.perf = !panels.perf;
                }
            }
            Item::Net => {
                if traffic(ui, p, icons, st, rect).clicked() {
                    crate::ui::perf::place_under(ui.ctx(), island);
                    *action = BarAction::NetworkGraph;
                }
            }
            Item::Media => {
                // the audio controls in island slots
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                );
                audio_action = crate::ui::audio::top_controls(&mut child, p, icons, audio, audio_ui, media, Vec2::splat(SLOT));
                continue;
            }
        }
        readouts_zone = readouts_zone.union(rect);
    }
    // a right click on a button of the island (they take it) opens the
    // island's menu too, where it was made
    let ctx = ui.ctx().clone();
    let on_button = ctx
        .input(|i| i.pointer.secondary_clicked().then_some(i.pointer.interact_pos()).flatten())
        .is_some_and(|pos| !back.secondary_clicked() && island.contains(pos) && ctx.layer_id_at(pos) == Some(ui.layer_id()));
    menu::context_menu_also(&back, p, on_button, |ui| system_menu(ui, p, panels, st, island, action));
    // the demo cannot right-click: the same menu where the pointer would be
    if super::demo::get().open == Some(super::demo::Open::System) && ctx.cumulative_frame_nr() >= 300 {
        let id = Id::unique("bars_system_demo_menu");
        let mut open = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(true);
        // under the island at its right end, as the mockup draws it
        let at = Pos2::new(island.right() - 270.0, island.bottom() + MARGIN);
        menu::popup_at(&ctx, id, at, &mut open, p, |ui| system_menu(ui, p, panels, st, island, action));
        ctx.data_mut(|d| d.insert_temp(id, open));
    }
    // the demo cannot click: Performance opened from the island at frame
    // 300, by its frame rate or its network
    let demo = super::demo::get().perf;
    if demo.is_some() && ctx.cumulative_frame_nr() == 300 && !panels.perf {
        crate::ui::perf::place_under(&ctx, island);
        if demo == Some(super::demo::Perf::Network) {
            *action = BarAction::NetworkGraph;
        } else {
            panels.perf = true;
        }
    }
    // gone under a menu or the volume panel
    let hovered = readouts_zone.is_positive() && ui.rect_contains_pointer(readouts_zone);
    (audio_action, hovered)
}

/// Firestorm's traffic indicator (ShowNetStats, off by default): the
/// network's last minute as the Performance window draws it
/// (`perf::net_curve`, the same data), its icon in the colour of the
/// packets lost at Firestorm's thresholds; a click opens Performance on its
/// Network section. Firestorm draws two instant gauges (bandwidth, losses):
/// the curve tells the same over a minute.
fn traffic(ui: &mut egui::Ui, p: &Palette, icons: &Icons, st: &StatusInfo, rect: Rect) -> egui::Response {
    let resp = ui.interact(rect, Id::unique("bars_island_net"), Sense::click());
    let hot = resp.hovered();
    slot_face(ui, p, rect, hot, false);
    let icon_rect = Rect::from_center_size(
        Pos2::new(rect.left() + SLOT_PAD + SYSTEM_ICON / 2.0, rect.center().y),
        Vec2::splat(SYSTEM_ICON),
    );
    paint_icon(ui, icons, "arrows-down-up", icon_rect, SYSTEM_ICON, loss_color(p, st.net_loss));
    let curve = Rect::from_min_max(
        Pos2::new(icon_rect.right() + SLOT_PAD, rect.top() + 7.0),
        Pos2::new(rect.right() - SLOT_PAD, rect.bottom() - 7.0),
    );
    crate::ui::perf::net_curve(&ui.painter().with_clip_rect(curve.expand(1.0)), p, curve, 0.0, st.perf);
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// The system island's right click (the mockup's): Firestorm's graphics
/// and camera presets of the status bar (LLPanelPresetsPulldown,
/// LLPanelPresetsCameraPulldown), what the island shows (its status bar's
/// menu, menu_hide_navbar.xml, the last item shown staying), hiding the
/// interface, the bars' buttons.
fn system_menu(ui: &mut egui::Ui, p: &Palette, panels: &mut Panels, st: &StatusInfo, island: Rect, action: &mut BarAction) {
    use crate::camera::CameraPreset;
    use crate::graphics_presets::Preset;
    menu::submenu(ui, p, "photo-filled", aurora_i18n::tr!("ui-bars-graphics-presets"), true, |ui| {
        for preset in Preset::ALL {
            if menu::check(
                ui,
                p,
                "device-desktop-filled",
                preset.label(),
                st.graphics_preset == Some(preset),
                true,
            ) {
                *action = BarAction::GraphicsPreset(preset);
            }
        }
        menu::separator(ui, p);
        if menu::item(ui, p, "adjustments-filled", aurora_i18n::tr!("ui-bars-open-graphics-preferences")) {
            *action = BarAction::GraphicsPreferences;
        }
        if menu::item(ui, p, "gauge-filled", aurora_i18n::tr!("ui-bars-performance-window")) {
            crate::ui::perf::place_under(ui.ctx(), island);
            crate::ui::widgets::open_floater(ui.ctx(), "perf", &mut panels.perf);
        }
        if menu::item(ui, p, "chart-area-line-filled", aurora_i18n::tr!("ui-bars-network-window")) {
            crate::ui::perf::place_under(ui.ctx(), island);
            *action = BarAction::NetworkGraph;
        }
    });
    menu::submenu(ui, p, "camera-filled", aurora_i18n::tr!("ui-bars-camera-presets"), true, |ui| {
        for preset in CameraPreset::ALL.into_iter().filter(|c| *c != CameraPreset::Custom) {
            if menu::check(ui, p, "camera-filled", preset.label(), st.camera_preset == preset, true) {
                *action = BarAction::CameraPreset(preset);
            }
        }
        menu::separator(ui, p);
        if menu::item(ui, p, "video-filled", aurora_i18n::tr!("ui-bars-camera-controls")) {
            *action = BarAction::CameraControls;
        }
    });
    menu::separator(ui, p);
    menu::section(ui, p, aurora_i18n::tr!("ui-bars-show-in-island"));
    let s = st.system_island;
    let shown = [s.clock, s.ping, s.fps, s.media, st.net_stats].into_iter().filter(|on| *on).count();
    // the last item shown stays: the island keeps something to right-click
    let can = |on: bool| !on || shown > 1;
    let checks: [(&str, &str, bool, crate::settings::SystemIsland); 4] = [
        (
            "clock-filled",
            aurora_i18n::tr!("ui-bars-show-clock"),
            s.clock,
            crate::settings::SystemIsland { clock: !s.clock, ..s },
        ),
        (
            "antenna-bars-5",
            aurora_i18n::tr!("ui-bars-show-latency"),
            s.ping,
            crate::settings::SystemIsland { ping: !s.ping, ..s },
        ),
        (
            "gauge-filled",
            aurora_i18n::tr!("ui-bars-show-fps"),
            s.fps,
            crate::settings::SystemIsland { fps: !s.fps, ..s },
        ),
        (
            "volume",
            aurora_i18n::tr!("ui-bars-show-media"),
            s.media,
            crate::settings::SystemIsland { media: !s.media, ..s },
        ),
    ];
    for (icon, label, on, next) in checks {
        if menu::check(ui, p, icon, label, on, can(on)) {
            *action = BarAction::SystemIsland(next);
        }
        // the clock's seconds, under the clock
        if icon == "clock-filled"
            && menu::check(
                ui,
                p,
                "stopwatch",
                aurora_i18n::tr!("ui-bars-clock-seconds"),
                s.clock_seconds,
                s.clock,
            )
        {
            *action = BarAction::SystemIsland(crate::settings::SystemIsland {
                clock_seconds: !s.clock_seconds,
                ..s
            });
        }
        // and its 12 / 24 hours (Preferences › Layout too)
        if icon == "clock-filled" {
            use crate::settings::ClockFormat;
            menu::submenu(ui, p, "clock-filled", aurora_i18n::tr!("ui-bars-clock-format"), s.clock, |ui| {
                for (format, label) in [
                    (ClockFormat::Auto, aurora_i18n::tr!("ui-options-clock-format-auto")),
                    (ClockFormat::H12, aurora_i18n::tr!("ui-options-clock-format-12")),
                    (ClockFormat::H24, aurora_i18n::tr!("ui-options-clock-format-24")),
                ] {
                    if menu::check(ui, p, "clock-filled", label, s.clock_format == format, true) {
                        *action = BarAction::SystemIsland(crate::settings::SystemIsland { clock_format: format, ..s });
                    }
                }
            });
        }
    }
    if menu::check(
        ui,
        p,
        "chart-area-line-filled",
        aurora_i18n::tr!("ui-bars-network-graph"),
        st.net_stats,
        can(st.net_stats),
    ) {
        *action = BarAction::NetStats(!st.net_stats);
    }
    menu::separator(ui, p);
    if super::interface_item(ui, p, st) {
        *action = BarAction::ToggleInterface;
    }
    if super::immersive_item(ui, p) {
        *action = BarAction::ToggleImmersive;
    }
    if menu::item(ui, p, "layout-bottombar-filled", aurora_i18n::tr!("ui-bars-customize-bars")) {
        *action = BarAction::CustomizeBars;
    }
}

/// Width of the system island's detailed tooltip (the mockup's).
const TIP_W: f32 = 330.0;
/// Height of its rows (12 px text at a line height of 1.5).
const TIP_ROW: f32 = 18.0;
/// Height of the network's curve under its row.
const TIP_CURVE: f32 = 28.0;

/// The system island's detailed tooltip (the mockup's hover state), under
/// the island at its right end: SLT and local time, date, latency, frame rate, losses
/// and bandwidth with the network's last minute (`perf::net_curve`, the
/// Performance window's data), parcel music, what the clicks do. It shows
/// after egui's tooltip delay over the readouts and stays while the
/// pointer goes down into it, whose network row opens Performance on its
/// Network section; a menu or the volume panel closes it.
#[allow(clippy::too_many_arguments)]
fn system_tip(
    ctx: &egui::Context,
    p: &Palette,
    icons: &Icons,
    st: &StatusInfo,
    media: crate::ui::audio::MediaState,
    island: Rect,
    hovered: bool,
    action: &mut BarAction,
) {
    let id = Id::unique("bars_system_tip");
    let (open_id, since_id) = (id.with("open"), id.with("since"));
    let now = ctx.input(|i| i.time);
    let was_open = ctx.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    // from the readouts down into the tooltip, over the 8 px between them
    let keep = was_open
        && ctx.memory(|m| m.area_rect(id)).is_some_and(|tip| {
            let bridge = Rect::from_min_max(Pos2::new(tip.left(), island.center().y), Pos2::new(tip.right(), tip.top() + 1.0));
            ctx.pointer_latest_pos()
                .is_some_and(|pos| tip.contains(pos) || bridge.contains(pos))
        });
    let blocked = egui::Popup::is_any_open(ctx);
    let open = if super::demo::get().tip {
        true
    } else if blocked {
        false
    } else if hovered || keep {
        let since = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(since_id, || now));
        let delay = f64::from(ctx.global_style().interaction.tooltip_delay);
        if was_open || now - since >= delay {
            true
        } else {
            ctx.request_repaint_after_secs((delay - (now - since)) as f32);
            false
        }
    } else {
        false
    };
    if !open && !hovered {
        ctx.data_mut(|d| d.remove_temp::<f64>(since_id));
    }
    ctx.data_mut(|d| d.insert_temp(open_id, open));
    if !open {
        return;
    }
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::new(island.right() - TIP_W, island.bottom() + MARGIN))
        .constrain(true)
        .fade_in(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(p.panel)
                .stroke(Stroke::new(1.0, p.raised))
                .corner_radius(ui.visuals().window_corner_radius)
                .inner_margin(egui::Margin::symmetric(9, 6))
                .shadow(ui.visuals().popup_shadow)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    ui.set_width(TIP_W - 2.0 - 18.0);
                    tip_body(ui, p, icons, st, media, island, action);
                });
        });
}

/// The rows of the system island's tooltip.
fn tip_body(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    st: &StatusInfo,
    media: crate::ui::audio::MediaState,
    island: Rect,
    action: &mut BarAction,
) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (_, month, day, weekday, ..) = crate::ui::land::slt_parts(secs);
    let date = aurora_i18n::tr!(
        "ui-bars-slt-date",
        weekday = crate::ui::land::weekdays()[weekday],
        day = day.to_string(),
        month = crate::ui::land::months()[month - 1]
    );
    let Readouts { ping, fps, .. } = readouts(st);
    let [slt, local] = clock::tip_values(st.system_island);
    tip_row(ui, p, icons, "clock-filled", aurora_i18n::tr!("ui-bars-tip-slt"), &slt, p.ink);
    tip_row(ui, p, icons, "home-filled", aurora_i18n::tr!("ui-bars-tip-local"), &local, p.ink);
    tip_row(ui, p, icons, "calendar-filled", aurora_i18n::tr!("ui-bars-tip-date"), &date, p.ink);
    tip_row(
        ui,
        p,
        icons,
        "antenna-bars-5",
        aurora_i18n::tr!("ui-bars-region-latency"),
        &ping,
        p.ink,
    );
    tip_row(
        ui,
        p,
        icons,
        "gauge-filled",
        aurora_i18n::tr!("ui-bars-tip-fps"),
        &fps,
        fps_color(p, st.fps),
    );

    // the network: its row and the Performance window's curve, one button
    let (rate_in, _) = crate::ui::perf::last_rates(st.perf);
    let network = format!("{} · {}", loss_text(st.net_loss), rate_text(rate_in));
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, TIP_ROW + 2.0 + TIP_CURVE + 3.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(
            rect.expand2(Vec2::new(4.0, 0.0)),
            ui.visuals().window_corner_radius,
            p.violet.gamma_multiply(0.18),
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let row_rect = Rect::from_min_size(rect.min, Vec2::new(w, TIP_ROW));
    // in the losses' colour once they pass Firestorm's first threshold
    let color = if st.net_loss < 1.25 { p.ink } else { loss_color(p, st.net_loss) };
    paint_tip_row(
        ui,
        p,
        icons,
        row_rect,
        "arrows-down-up",
        aurora_i18n::tr!("ui-bars-tip-network"),
        &network,
        color,
        12.0,
    );
    let curve = Rect::from_min_size(Pos2::new(rect.left(), row_rect.bottom() + 2.0), Vec2::new(w, TIP_CURVE));
    crate::ui::perf::net_curve(&ui.painter().with_clip_rect(curve.expand(1.0)), p, curve, 2.0, st.perf);
    if resp.clicked() {
        crate::ui::perf::place_under(ui.ctx(), island);
        *action = BarAction::NetworkGraph;
    }

    let music = if !media.music_available {
        aurora_i18n::tr!("ui-bars-tip-music-none")
    } else if media.music_playing {
        aurora_i18n::tr!("ui-bars-tip-music-playing")
    } else {
        aurora_i18n::tr!("ui-bars-tip-music-stopped")
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, TIP_ROW), Sense::hover());
    paint_tip_row(
        ui,
        p,
        icons,
        rect,
        "radio",
        aurora_i18n::tr!("ui-bars-tip-music"),
        music,
        p.muted,
        11.5,
    );

    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 11.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, crate::ui::widgets::divider_stroke(p));
    let hints = ui.painter().layout(
        aurora_i18n::tr!("ui-bars-tip-hints").to_owned(),
        FontId::proportional(11.5),
        p.muted,
        w,
    );
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, hints.size().y), Sense::hover());
    ui.painter().galley(rect.min, hints, p.muted);
}

/// A row of the tooltip: icon, label, the value in semibold at the right.
#[allow(clippy::too_many_arguments)]
fn tip_row(ui: &mut egui::Ui, p: &Palette, icons: &Icons, icon: &str, label: &str, value: &str, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), TIP_ROW), Sense::hover());
    paint_tip_row(ui, p, icons, rect, icon, label, value, color, 12.0);
}

#[allow(clippy::too_many_arguments)]
fn paint_tip_row(ui: &egui::Ui, p: &Palette, icons: &Icons, rect: Rect, icon: &str, label: &str, value: &str, color: Color32, size: f32) {
    const ICON: f32 = 14.0;
    let icon_rect = Rect::from_center_size(Pos2::new(rect.left() + ICON / 2.0, rect.center().y), Vec2::splat(ICON));
    paint_icon(ui, icons, icon, icon_rect, ICON, p.muted);
    let label = text(ui, label.to_owned(), 12.0, p.ink);
    crate::ui::widgets::paint_row_text(ui.painter(), label, icon_rect.right() + 8.0, rect.center().y, p.ink);
    let value = ui
        .painter()
        .layout_no_wrap(value.to_owned(), FontId::new(size, crate::ui::fonts::semibold(ui.ctx())), color);
    let left = rect.right() - value.size().x;
    crate::ui::widgets::paint_row_text(ui.painter(), value, left, rect.center().y, color);
}

/// The share of packets lost as the tooltip reads it: "0 %", else one
/// decimal.
fn loss_text(loss: f32) -> String {
    if loss <= 0.0 {
        "0 %".to_owned()
    } else {
        format!("{} %", aurora_i18n::number(f64::from(loss), 1))
    }
}

/// A rate as Performance writes it (`perf::fmt_rate`), with the language's
/// decimal separator.
fn rate_text(kbps: f32) -> String {
    let rate = crate::ui::perf::fmt_rate(kbps);
    match rate.split_once(' ') {
        Some((number, unit)) => format!("{} {unit}", aurora_i18n::number_text(number)),
        None => rate,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{IslandsLook, StatusInfo};
    use super::*;

    fn status(stretch: bool, bug: bool) -> StatusInfo<'static> {
        StatusInfo {
            location_hidden: false,
            show_bug_report: bug,
            show_huds: true,
            hud_zoom: 1.0,
            region: "Aurora Demo",
            parcel: "Aurora Plaza",
            tp_back: None,
            tp_forward: None,
            maturity: "",
            position: glam::Vec3::new(132.0, 126.0, 33.0),
            fps: 60.0,
            flying: false,
            seated: false,
            ping_ms: 38,
            balance: Some(1000),
            voice: (crate::voice::VoiceLight::Off, String::new()),
            slurl: String::new(),
            status: Default::default(),
            ban_lines: 0,
            parcel_icons: Vec::new(),
            local_env: false,
            double_click: Default::default(),
            quickjump: false,
            quiet_snapshots: false,
            marketplace: false,
            parcel_landmark: None,
            can_set_home: false,
            release_notes: false,
            region_console: false,
            motd: false,
            grid_help: None,
            classic_bars: false,
            islands: IslandsLook {
                stretch,
                rights_text: false,
            },
            own_name: "Aurora Demo",
            own_pic: None,
            recent: Vec::new(),
            favorites: Box::leak(Box::default()),
            system_island: Default::default(),
            net_stats: false,
            perf: Box::leak(Box::default()),
            net_loss: 0.0,
            graphics_preset: None,
            camera_preset: Default::default(),
            interface_hidden: false,
            interface_keys: String::new(),
            ik_keys: String::new(),
            edit_particles: false,
            health: Default::default(),
            hover_tips: Default::default(),
            world_overlays: Default::default(),
            show_transparency: false,
            conversation_thumbnails: 0,
        }
    }

    /// Two frames of the islands in a screen `width` wide (16:9), the
    /// pointer at `pointer`; the areas' rectangles by name, and what the
    /// last frame painted.
    fn frames(
        width: f32,
        stretch: bool,
        bug: bool,
        pointer: Pos2,
    ) -> (egui::Context, impl Fn(&str) -> Rect, Vec<egui::epaint::ClippedShape>) {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let icons = Icons::default();
        let settings = crate::settings::Settings::default();
        let mut panels = Panels::from_settings(&settings);
        let mut audio = settings.audio.clone();
        let mut audio_ui = crate::ui::audio::AudioUi::default();
        let mut notif = crate::ui::notifications::NotifUi::default();
        let st = status(stretch, bug);
        let mut shapes = Vec::new();
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, width * 9.0 / 16.0))),
                events: vec![egui::Event::PointerMoved(pointer)],
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                top(
                    ui,
                    &p,
                    &icons,
                    &mut panels,
                    &st,
                    &mut audio,
                    &mut audio_ui,
                    Default::default(),
                    &mut notif,
                    BellState::default(),
                );
            });
            out.textures_delta.clear();
            shapes = out.shapes;
        }
        let c = ctx.clone();
        let rect = move |name: &str| {
            c.memory(|m| m.area_rect(Id::unique(("bars_island", name))))
                .unwrap_or_else(|| panic!("no {name} island"))
        };
        (ctx, rect, shapes)
    }

    /// The islands stand 8 px from the edges and from each other, with or
    /// without the bug button (no hole where it was), at the three widths of
    /// the mockup; stretched, the place reaches the account island, and it
    /// follows the menu island 8 px away either way.
    #[test]
    fn the_islands_line_up_8_px_apart() {
        for width in [1280.0, 1536.0, 1920.0] {
            for bug in [true, false] {
                for stretch in [true, false] {
                    let (_, rect, _) = frames(width, stretch, bug, Pos2::new(700.0, 400.0));
                    let (menu, place, account, system, bell) = (rect("menu"), rect("place"), rect("account"), rect("system"), rect("bell"));
                    let case = format!("{width} bug {bug} stretch {stretch}");
                    assert_eq!((menu.left(), menu.top(), menu.height()), (MARGIN, MARGIN, HEIGHT), "{case}");
                    assert_eq!(menu.width(), PAD * 2.0 + SLOT, "{case}");
                    assert_eq!(bell.right(), width - MARGIN, "{case}");
                    // the bug's island between the wolf's and the place, or
                    // the place right after the wolf
                    let before_place = if bug {
                        let b = rect("bug");
                        assert_eq!((b.top(), b.height(), b.width()), (MARGIN, HEIGHT, PAD * 2.0 + SLOT), "{case}");
                        assert_eq!(b.left() - menu.right(), MARGIN, "{case}");
                        b
                    } else {
                        menu
                    };
                    assert_eq!(place.left() - before_place.right(), MARGIN, "{case}");
                    let pairs = [(account, system), (system, bell)];
                    for (a, b) in pairs {
                        assert_eq!(b.left() - a.right(), MARGIN, "{case}: {a:?} {b:?}");
                    }
                    if stretch {
                        assert_eq!(account.left() - place.right(), MARGIN, "{case}");
                    } else {
                        assert!(account.left() - place.right() >= MARGIN, "{case}");
                    }
                }
            }
        }
        assert_eq!(
            work_area(Rect::from_min_size(Pos2::ZERO, Vec2::new(1536.0, 864.0)), 834.0).top(),
            52.0
        );
    }

    /// The pointer over an island is over the interface; between the
    /// islands (the place not stretched), over the world.
    #[test]
    fn an_island_takes_the_pointer_the_gap_between_them_does_not() {
        let (ctx, rect, _) = frames(1536.0, true, true, Pos2::new(20.0, 20.0));
        assert!(ctx.is_pointer_over_egui(), "the menu island");
        let account = rect("account");
        let (ctx, rect, _) = frames(1536.0, false, true, Pos2::new(account.left() - 30.0, 20.0));
        assert!(rect("place").right() < account.left() - 30.0);
        assert!(!ctx.is_pointer_over_egui(), "between the place and the account");
    }
    /// The texts of the system island (clock, latency, frame rate) share
    /// one baseline: their digits' ink is centred on the island's middle,
    /// within half a pixel, at the three widths, whatever hangs below the
    /// line (the frame rate's comma).
    #[test]
    fn the_system_readouts_share_one_baseline() {
        fn texts(shape: &egui::Shape, out: &mut Vec<(String, Pos2)>) {
            match shape {
                egui::Shape::Text(t) => {
                    let mut ink = Rect::NOTHING;
                    for row in &t.galley.rows {
                        for g in row.row.glyphs.iter().filter(|g| g.chr.is_ascii_digit()) {
                            let min = t.pos + row.pos.to_vec2() + g.pos.to_vec2() + g.uv_rect.offset;
                            ink = ink.union(Rect::from_min_size(min, g.uv_rect.size));
                        }
                    }
                    if ink.is_positive() {
                        out.push((t.galley.text().to_owned(), ink.center()));
                    }
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        for width in [1280.0, 1536.0, 1920.0] {
            let (_, rect, shapes) = frames(width, true, true, Pos2::new(700.0, 400.0));
            let system = rect("system");
            let mut found = Vec::new();
            for clipped in &shapes {
                texts(&clipped.shape, &mut found);
            }
            let found: Vec<_> = found.into_iter().filter(|(_, at)| system.contains(*at)).collect();
            assert!(found.len() >= 3, "{width}: {found:?}");
            for (text, at) in &found {
                assert!(
                    (at.y - system.center().y).abs() <= 0.5,
                    "{width}: {text:?} at {}, island centre {}",
                    at.y,
                    system.center().y
                );
            }
        }
    }

    /// The place island with each interface font, at the interface scales
    /// 1, 1.25 and 1.5, its rating badge on: the badge centred in the
    /// island, its letter in the badge, the parcel's capitals on the
    /// badge's centre, all within half a pixel (an odd height in an even
    /// one cannot do better), and the region on the parcel's baseline to
    /// the pixel. With Inter, the badge's letter stood a pixel below the
    /// parcel's capitals, and the region a pixel below the parcel at 1.25 and
    /// 1.5: the shift of each face was measured where egui lays it out, not
    /// where epaint draws it (rounded to the pixel).
    #[test]
    fn the_place_island_centres_its_badge_and_texts_with_every_font() {
        let p = crate::theme::Theme::default().palette();
        let icons = Icons::default();
        let settings = crate::settings::Settings::default();
        let mut st = status(true, false);
        st.maturity = aurora_i18n::text("common-general");
        for font in crate::ui::widgets::alignment_tests::fonts() {
            for zoom in [1.0f32, 1.25, 1.5] {
                let ctx = egui::Context::default();
                crate::ui::fonts::apply(&ctx, &font);
                ctx.set_zoom_factor(zoom);
                let mut panels = Panels::from_settings(&settings);
                let mut audio = settings.audio.clone();
                let mut audio_ui = crate::ui::audio::AudioUi::default();
                let mut notif = crate::ui::notifications::NotifUi::default();
                let mut shapes = Vec::new();
                // the fonts and the zoom take effect at the next pass
                for _ in 0..3 {
                    let input = egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1536.0, 864.0))),
                        ..Default::default()
                    };
                    let mut out = ctx.run_ui(input, |ui| {
                        top(
                            ui,
                            &p,
                            &icons,
                            &mut panels,
                            &st,
                            &mut audio,
                            &mut audio_ui,
                            Default::default(),
                            &mut notif,
                            BellState::default(),
                        );
                    });
                    out.textures_delta.clear();
                    shapes = out.shapes;
                }
                let ppp = ctx.pixels_per_point();
                assert_eq!(ppp, zoom);
                let case = format!("{font} at {zoom}");
                let island = ctx
                    .memory(|m| m.area_rect(Id::unique(("bars_island", "place"))))
                    .expect("the place island");
                let centre = island.center().y * ppp;
                let badge = shapes
                    .iter()
                    .find_map(|c| match &c.shape {
                        egui::Shape::Rect(r) if r.fill == p.violet && r.rect.width() < 20.0 => Some(r.rect),
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("{case}: no badge"));
                // where epaint fills it
                let badge = egui::emath::GuiRounding::round_to_pixels(badge, ppp);
                let badge_c = badge.center().y * ppp;
                let texts = crate::ui::widgets::alignment_tests::capitals_ink(&shapes, ppp);
                let text = |s: &str| {
                    texts
                        .iter()
                        .find(|(t, ink)| t.starts_with(s) && island.contains(ink.center() / ppp))
                        .map(|(_, ink)| *ink)
                        .unwrap_or_else(|| panic!("{case}: no {s:?} in {texts:?}"))
                };
                let (letter, parcel, region) = (text("G"), text(st.parcel), text(st.region));
                let report = format!(
                    "{case}: island {centre}, badge {badge_c}, letter {}, parcel {} (foot {}), region foot {}",
                    letter.center().y,
                    parcel.center().y,
                    parcel.bottom(),
                    region.bottom()
                );
                eprintln!("{report}");
                assert!((badge_c - centre).abs() <= 0.5, "{report}");
                assert!((letter.center().y - badge_c).abs() <= 0.5, "{report}");
                assert!((parcel.center().y - badge_c).abs() <= 0.5, "{report}");
                assert!((parcel.bottom() - region.bottom()).abs() < 0.01, "{report}");
            }
        }
    }

    /// One frame of the islands at 1536 px wide, at `time`, the pointer's
    /// `events`; the bar's action.
    fn frame_at(ctx: &egui::Context, st: &StatusInfo, time: f64, events: Vec<egui::Event>) -> BarAction {
        let p = crate::theme::Theme::default().palette();
        let icons = Icons::default();
        let settings = crate::settings::Settings::default();
        let mut panels = Panels::from_settings(&settings);
        let mut audio = settings.audio.clone();
        let mut audio_ui = crate::ui::audio::AudioUi::default();
        let mut notif = crate::ui::notifications::NotifUi::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1536.0, 864.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut action = BarAction::None;
        let mut out = ctx.run_ui(input, |ui| {
            action = top(
                ui,
                &p,
                &icons,
                &mut panels,
                st,
                &mut audio,
                &mut audio_ui,
                Default::default(),
                &mut notif,
                BellState::default(),
            )
            .0;
        });
        out.textures_delta.clear();
        action
    }

    fn press(pos: Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Default::default(),
        }
    }

    fn system_rect(ctx: &egui::Context) -> Rect {
        ctx.memory(|m| m.area_rect(Id::unique(("bars_island", "system"))))
            .expect("the system island")
    }

    /// The system island shows what its right click keeps, 2 px apart with
    /// no hole where an item was, and stays 8 px from the account and the
    /// bell; the traffic indicator comes in before the audio controls.
    #[test]
    fn the_system_island_shows_what_its_menu_keeps() {
        let ctx = egui::Context::default();
        let mut st = status(true, true);
        frame_at(&ctx, &st, 0.0, vec![]);
        frame_at(&ctx, &st, 0.1, vec![]);
        let full = system_rect(&ctx).width();
        let texts = readouts(&st);
        let ui_width = |st: &StatusInfo| {
            let mut w = 0.0;
            let mut out = ctx.run_ui(Default::default(), |ui| w = system_width(ui, st));
            out.textures_delta.clear();
            w
        };
        assert_eq!(ui_width(&st), full);
        let mut fps_w = 0.0;
        let mut out = ctx.run_ui(Default::default(), |ui| fps_w = item_width(ui, &texts, Item::Fps));
        out.textures_delta.clear();
        st.system_island.fps = false;
        assert_eq!(ui_width(&st), full - fps_w - GAP);
        st.net_stats = true;
        assert_eq!(ui_width(&st), full - fps_w + readout_width(NET_W));
        assert_eq!(items(&st), vec![Item::Clock, Item::Ping, Item::Net, Item::Media]);
        frame_at(&ctx, &st, 0.2, vec![]);
        frame_at(&ctx, &st, 0.3, vec![]);
        let (system, account, bell) = (
            system_rect(&ctx),
            ctx.memory(|m| m.area_rect(Id::unique(("bars_island", "account")))).unwrap(),
            ctx.memory(|m| m.area_rect(Id::unique(("bars_island", "bell")))).unwrap(),
        );
        assert_eq!(system.width(), full - fps_w + readout_width(NET_W));
        assert_eq!((system.left() - account.right(), bell.left() - system.right()), (MARGIN, MARGIN));
        // everything hidden by a settings file: the clock stays
        st.system_island = crate::settings::SystemIsland {
            clock: false,
            ping: false,
            fps: false,
            media: false,
            ..Default::default()
        };
        st.net_stats = false;
        assert_eq!(items(&st), vec![Item::Clock]);
    }

    /// A readout keeps the widest width it needed in the last `HOLD`
    /// seconds (#1246): one digit is as wide as another, the frame rate
    /// going back and forth across 100 does not move the island, which
    /// narrows again once it stays under 100; the latency alike.
    #[test]
    fn the_readouts_hold_their_width() {
        let ctx = egui::Context::default();
        let st = status(true, true);
        let width_at = |time: f64, item: Item, fps: f32, ping: u32| {
            let mut w = 0.0;
            let input = egui::RawInput {
                time: Some(time),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let texts = Readouts {
                    ping: ping_text(ping),
                    fps: fps_text(fps),
                    ..readouts(&st)
                };
                w = item_width(ui, &texts, item);
            });
            out.textures_delta.clear();
            w
        };
        let narrow = width_at(0.0, Item::Fps, 99.9, 0);
        assert_eq!(width_at(0.5, Item::Fps, 15.0, 0), narrow, "one digit as wide as another");
        let wide = width_at(1.0, Item::Fps, 100.0, 0);
        assert!(wide > narrow);
        assert_eq!(width_at(2.0, Item::Fps, 99.9, 0), wide);
        assert_eq!(width_at(3.0, Item::Fps, 144.0, 0), wide);
        assert_eq!(width_at(3.0 + HOLD - 0.5, Item::Fps, 99.9, 0), wide);
        assert_eq!(width_at(3.0 + HOLD, Item::Fps, 99.9, 0), narrow);
        let ping = width_at(20.0, Item::Ping, 0.0, 99);
        assert!(width_at(21.0, Item::Ping, 0.0, 100) > ping);
        assert!(width_at(22.0, Item::Ping, 0.0, 5) > ping);
        assert_eq!(width_at(21.0 + HOLD, Item::Ping, 0.0, 99), ping);
    }

    /// Firestorm's thresholds of the traffic indicator: under 1.25 % of
    /// packets lost, up to 3 %, beyond.
    #[test]
    fn the_losses_take_firestorms_colours() {
        let p = crate::theme::Theme::default().palette();
        assert_eq!(loss_color(&p, 0.0), p.teal);
        assert_eq!(loss_color(&p, 1.2), p.teal);
        assert_eq!(loss_color(&p, 1.25), p.amber);
        assert_eq!(loss_color(&p, 2.9), p.amber);
        assert_eq!(loss_color(&p, 3.0), p.danger);
        assert_eq!(loss_text(0.0), "0 %");
    }

    /// The detailed tooltip opens after egui's delay over the readouts,
    /// under the island at its right end, stays while the pointer goes down
    /// into it, and its network row opens Performance on its Network
    /// section; a right click on the island's buttons opens its menu.
    #[test]
    fn the_system_tooltip_and_menu_answer_the_pointer() {
        let ctx = egui::Context::default();
        let st = status(true, true);
        frame_at(&ctx, &st, 0.0, vec![]);
        frame_at(&ctx, &st, 0.1, vec![]);
        let system = system_rect(&ctx);
        let tip = || ctx.memory(|m| m.area_rect(Id::unique("bars_system_tip")));
        let open = || {
            ctx.data(|d| d.get_temp::<bool>(Id::unique("bars_system_tip").with("open")))
                .unwrap_or(false)
        };
        let clock = Pos2::new(system.left() + PAD + 20.0, system.center().y);
        frame_at(&ctx, &st, 0.2, vec![egui::Event::PointerMoved(clock)]);
        assert!(!open(), "not before the delay");
        frame_at(&ctx, &st, 0.8, vec![egui::Event::PointerMoved(clock)]);
        frame_at(&ctx, &st, 0.85, vec![egui::Event::PointerMoved(clock)]);
        assert!(open(), "after the delay");
        let rect = tip().expect("the tooltip's area");
        assert_eq!(
            (rect.right(), rect.top(), rect.width()),
            (system.right(), system.bottom() + MARGIN, TIP_W)
        );
        // the network row: the sixth row (SLT, local time, date, latency,
        // frame rate before it) and its curve
        let network = Pos2::new(rect.center().x, rect.top() + 6.0 + TIP_ROW * 5.0 + TIP_ROW);
        frame_at(&ctx, &st, 0.9, vec![egui::Event::PointerMoved(network)]);
        assert!(open(), "kept while the pointer goes into it");
        frame_at(&ctx, &st, 0.95, vec![press(network, egui::PointerButton::Primary, true)]);
        let action = frame_at(&ctx, &st, 1.0, vec![press(network, egui::PointerButton::Primary, false)]);
        assert!(matches!(action, BarAction::NetworkGraph), "the network row opens Performance");
        // away: closed
        frame_at(&ctx, &st, 1.1, vec![egui::Event::PointerMoved(Pos2::new(700.0, 500.0))]);
        assert!(!open(), "closed once the pointer left");
        // the speaker takes its clicks, its right click opens the island's menu
        let speaker = Pos2::new(system.right() - PAD - SLOT / 2.0, system.center().y);
        frame_at(&ctx, &st, 1.2, vec![egui::Event::PointerMoved(speaker)]);
        frame_at(&ctx, &st, 1.25, vec![press(speaker, egui::PointerButton::Secondary, true)]);
        frame_at(&ctx, &st, 1.3, vec![press(speaker, egui::PointerButton::Secondary, false)]);
        frame_at(&ctx, &st, 1.35, vec![]);
        assert!(egui::Popup::is_any_open(&ctx), "the island's menu");
        assert!(!open(), "no tooltip under the menu");
    }

    /// The bug button's right click copies the version, silently: no menu
    /// opens, the left click still reports a bug.
    #[test]
    fn the_bug_button_right_click_copies_the_version() {
        let ctx = egui::Context::default();
        let st = status(true, true);
        frame_at(&ctx, &st, 0.0, vec![]);
        frame_at(&ctx, &st, 0.1, vec![]);
        let bug = ctx
            .memory(|m| m.area_rect(Id::unique(("bars_island", "bug"))))
            .expect("the bug island")
            .center();
        frame_at(&ctx, &st, 0.2, vec![egui::Event::PointerMoved(bug)]);
        frame_at(&ctx, &st, 0.25, vec![press(bug, egui::PointerButton::Secondary, true)]);
        let action = frame_at(&ctx, &st, 0.3, vec![press(bug, egui::PointerButton::Secondary, false)]);
        assert!(matches!(action, BarAction::CopyVersion), "the right click copies the version");
        frame_at(&ctx, &st, 0.35, vec![]);
        assert!(!egui::Popup::is_any_open(&ctx), "and opens no menu");
        frame_at(&ctx, &st, 0.4, vec![press(bug, egui::PointerButton::Primary, true)]);
        let action = frame_at(&ctx, &st, 0.45, vec![press(bug, egui::PointerButton::Primary, false)]);
        assert!(matches!(action, BarAction::ReportBug), "the left click reports a bug");
    }

    /// The home button's right click offers Set Home to Here (instead of the
    /// island's own menu), greyed where the region does not allow it.
    #[test]
    fn the_home_button_right_click_sets_home_here() {
        fn find(shape: &egui::Shape, text: &str) -> Option<Rect> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == text => Some(Rect::from_min_size(t.pos, t.galley.size())),
                egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, text)),
                _ => None,
            }
        }
        for allowed in [true, false] {
            let ctx = egui::Context::default();
            let mut st = status(true, true);
            st.can_set_home = allowed;
            frame_at(&ctx, &st, 0.0, vec![]);
            frame_at(&ctx, &st, 0.1, vec![]);
            let place = ctx
                .memory(|m| m.area_rect(Id::unique(("bars_island", "place"))))
                .expect("the place island");
            // back, forward, then home
            let home = Pos2::new(place.left() + PAD + 2.0 * (SLOT + GAP) + SLOT / 2.0, place.center().y);
            frame_at(&ctx, &st, 0.2, vec![egui::Event::PointerMoved(home)]);
            frame_at(&ctx, &st, 0.25, vec![press(home, egui::PointerButton::Secondary, true)]);
            let action = frame_at(&ctx, &st, 0.3, vec![press(home, egui::PointerButton::Secondary, false)]);
            assert!(matches!(action, BarAction::None), "a right click does not go home");
            // the menu's row, as the next frame paints it
            let shapes = {
                let p = crate::theme::Theme::default().palette();
                let settings = crate::settings::Settings::default();
                let mut panels = Panels::from_settings(&settings);
                let mut audio = settings.audio.clone();
                let input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1536.0, 864.0))),
                    time: Some(0.35),
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |ui| {
                    top(
                        ui,
                        &p,
                        &Icons::default(),
                        &mut panels,
                        &st,
                        &mut audio,
                        &mut crate::ui::audio::AudioUi::default(),
                        Default::default(),
                        &mut crate::ui::notifications::NotifUi::default(),
                        BellState::default(),
                    );
                });
                out.textures_delta.clear();
                out.shapes
            };
            let label = aurora_i18n::tr!("world-set-home-here");
            let row = shapes
                .iter()
                .find_map(|c| find(&c.shape, label))
                .expect("Set Home to Here in the menu");
            let island_menu = aurora_i18n::tr!("ui-options-place-bar-stretch");
            assert!(
                shapes.iter().all(|c| find(&c.shape, island_menu).is_none()),
                "not the island's menu"
            );
            let at = row.center();
            frame_at(&ctx, &st, 0.4, vec![egui::Event::PointerMoved(at)]);
            frame_at(&ctx, &st, 0.45, vec![press(at, egui::PointerButton::Primary, true)]);
            let action = frame_at(&ctx, &st, 0.5, vec![press(at, egui::PointerButton::Primary, false)]);
            assert_eq!(matches!(action, BarAction::SetHome), allowed, "allowed {allowed}");
        }
    }

    /// One frame of the islands and of Performance (`open`), at `width` ×
    /// 9/16, the work area under the islands and 60 px above the bottom
    /// (the dock), at `time`, the pointer's `events`.
    fn perf_frame(ctx: &egui::Context, width: f32, open: &mut bool, time: f64, events: Vec<egui::Event>) {
        let p = crate::theme::Theme::default().palette();
        let icons = Icons::default();
        let settings = crate::settings::Settings::default();
        let mut panels = Panels::from_settings(&settings);
        let mut audio = settings.audio.clone();
        let mut audio_ui = crate::ui::audio::AudioUi::default();
        let mut notif = crate::ui::notifications::NotifUi::default();
        let st = status(true, true);
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, width * 9.0 / 16.0));
        let data = crate::ui::perf::PerfData::default();
        let gpu = aurora_render::GpuInfo {
            name: String::new(),
            backend: String::new(),
            driver: String::new(),
            bindless: false,
            timestamps: false,
            max_textures: 0,
            vram_mb: None,
            integrated: false,
            vendor_id: 0,
            max_texture_size: 0,
            adapters: Vec::new(),
            notice: None,
        };
        let input = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut monitor = crate::ui::monitor::Monitor::default();
        let mut out = ctx.run_ui(input, |ui| {
            top(
                ui,
                &p,
                &icons,
                &mut panels,
                &st,
                &mut audio,
                &mut audio_ui,
                Default::default(),
                &mut notif,
                BellState::default(),
            );
            crate::ui::layout::set_work_area(ui.ctx(), work_area(screen, screen.bottom() - 60.0));
            let view = crate::ui::perf::PerfView { data: &data, gpu: &gpu };
            crate::ui::perf::show(ui.ctx(), &p, &view, &mut monitor, open);
        });
        out.textures_delta.clear();
    }

    fn perf_rect(ctx: &egui::Context) -> Rect {
        ctx.memory(|m| m.area_rect(Id::unique("perf"))).expect("the Performance window")
    }

    /// Performance opened from the system island: its full view (the
    /// resource monitor) stands 8 px under the island, its right edge on
    /// the island's (within half a pixel), at its own size and within the
    /// work area, at the three widths; its edges resize it both ways. The
    /// compact view keeps its own width and place.
    #[test]
    fn performance_opens_under_the_system_island() {
        for width in [1280.0, 1536.0, 1920.0] {
            let ctx = egui::Context::default();
            let mut open = false;
            let mut t = 0.0;
            let mut frame = |ctx: &egui::Context, open: &mut bool, events: Vec<egui::Event>| {
                t += 0.05;
                perf_frame(ctx, width, open, t, events);
            };
            frame(&ctx, &mut open, vec![]);
            frame(&ctx, &mut open, vec![]);
            let island = system_rect(&ctx);
            let work = crate::ui::layout::work_area(&ctx);

            // full, the window's first opening: under the island
            crate::ui::perf::set_full(&ctx, true);
            crate::ui::perf::place_under(&ctx, island);
            open = true;
            for _ in 0..4 {
                frame(&ctx, &mut open, vec![]);
            }
            let full = perf_rect(&ctx);
            let case = format!("{width}: {full:?} under {island:?}");
            assert!((full.right() - island.right()).abs() <= 0.5, "{case}");
            assert!((full.top() - (island.bottom() + MARGIN)).abs() <= 0.5, "{case}");
            assert!(full.bottom() <= work.bottom() + 0.5, "{case}: within the work area {work:?}");
            assert!(full.width() <= work.width() + 0.5, "{case}: within the work area {work:?}");
            assert!(full.width() >= crate::ui::monitor::MIN_SIZE.x.min(work.width()) - 0.5, "{case}");

            let drag = |ctx: &egui::Context,
                        open: &mut bool,
                        from: Pos2,
                        by: Vec2,
                        frame: &mut dyn FnMut(&egui::Context, &mut bool, Vec<egui::Event>)| {
                frame(ctx, open, vec![egui::Event::PointerMoved(from)]);
                frame(ctx, open, vec![press(from, egui::PointerButton::Primary, true)]);
                for k in 1..=4 {
                    frame(ctx, open, vec![egui::Event::PointerMoved(from + by * k as f32 / 4.0)]);
                }
                frame(ctx, open, vec![press(from + by, egui::PointerButton::Primary, false)]);
                frame(ctx, open, vec![]);
            };
            // the left side resizes it, as the bottom does
            drag(
                &ctx,
                &mut open,
                Pos2::new(full.left() + 1.0, full.center().y),
                Vec2::new(60.0, 0.0),
                &mut frame,
            );
            let narrower = perf_rect(&ctx);
            assert!(
                narrower.width() < full.width() - 30.0,
                "{width}: the left side resizes it, {narrower:?}"
            );
            drag(
                &ctx,
                &mut open,
                Pos2::new(narrower.center().x, narrower.bottom() - 1.0),
                Vec2::new(0.0, -80.0),
                &mut frame,
            );
            let shorter = perf_rect(&ctx);
            assert!(
                shorter.height() < narrower.height() - 40.0,
                "{width}: the bottom resizes it, {shorter:?}"
            );
            assert!(shorter.bottom() <= work.bottom() + 0.5, "{width}: {shorter:?} within {work:?}");

            // compact: its own width, wherever it stands
            open = false;
            frame(&ctx, &mut open, vec![]);
            crate::ui::perf::set_full(&ctx, false);
            crate::ui::perf::place_under(&ctx, island);
            open = true;
            for _ in 0..3 {
                frame(&ctx, &mut open, vec![]);
            }
            let compact = perf_rect(&ctx);
            assert_eq!(
                compact.width(),
                crate::ui::perf::COMPACT_W + crate::ui::perf::SIDES,
                "{width}: compact"
            );
            assert!(
                (compact.right() - island.right()).abs() > 0.5,
                "{width}: the compact view stands where it stood"
            );
        }
    }

    /// The full view by default; a network opening (the tooltip's network
    /// row, the traffic indicator, "Network graph…") shows it for itself
    /// alone when the compact view was chosen, and the choice saved stays
    /// as it was.
    #[test]
    fn a_network_opening_does_not_change_the_view_chosen() {
        let ctx = egui::Context::default();
        let mut open = false;
        let mut t = 0.0;
        let mut frame = |ctx: &egui::Context, open: &mut bool| {
            t += 0.05;
            perf_frame(ctx, 1536.0, open, t, vec![]);
        };
        frame(&ctx, &mut open);
        frame(&ctx, &mut open);
        let island = system_rect(&ctx);
        let compact_w = crate::ui::perf::COMPACT_W + crate::ui::perf::SIDES;
        let saved = |ctx: &egui::Context| ctx.data_mut(|d| d.get_persisted::<bool>(Id::unique(("perf", "monitor"))));
        // nothing chosen: the full view
        assert!(crate::ui::perf::is_full(&ctx), "the full view by default");
        // the compact view chosen, then a network opening: full, this time
        crate::ui::perf::set_full(&ctx, false);
        crate::ui::perf::place_under(&ctx, island);
        crate::ui::perf::show_network(&ctx);
        open = true;
        for _ in 0..3 {
            frame(&ctx, &mut open);
        }
        assert!(perf_rect(&ctx).width() > compact_w + 100.0, "the full view for the network");
        // closed during that full view
        open = false;
        frame(&ctx, &mut open);
        assert_eq!(saved(&ctx), Some(false), "the network opening kept the compact view chosen");
        // the frame rate's click: compact
        crate::ui::perf::place_under(&ctx, island);
        open = true;
        for _ in 0..3 {
            frame(&ctx, &mut open);
        }
        assert_eq!(perf_rect(&ctx).width(), compact_w, "the frame rate opens the compact view");
    }
}
