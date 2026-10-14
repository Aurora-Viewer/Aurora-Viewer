//! Aurora's top bar: glass islands laid over the world instead of a
//! full-width band (variant A of the bars mockup the human validated). From
//! the left: the Aurora menu, the place (stretched to the account island,
//! `place.rs`), the account (picture with the online status, balance), the
//! system (SLT clock, latency, frame rate, media and volume) and the bell.
//! The islands keep the windows' blur, sampled within their rectangles
//! (`widgets::island_shape`): no extra GPU pass. Preferences › Aurora ›
//! Appearance › "Top and bottom bars" brings Firestorm's bars back
//! (`top_bars`, unchanged).

use super::{BarAction, BellState, StatusInfo, main_menus, paint_icon, slt_time, status_color, status_menu_body, status_tip};
use crate::theme::Palette;
use crate::ui::Panels;
use crate::ui::icons::Icons;
use crate::ui::menu;
use egui::{Color32, FontId, Id, Pos2, Rect, Sense, Vec2};

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

/// The rectangle of an island between `left` and `right` on the row.
fn row(screen: Rect, left: f32, right: f32) -> Rect {
    let top = screen.top() + MARGIN;
    Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, top + HEIGHT))
}

/// The slot face: the raised face under the pointer or while its menu is
/// open, the accent while `active`, nothing at rest.
pub(super) fn slot_face(ui: &egui::Ui, p: &Palette, rect: Rect, hot: bool, active: bool) {
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

/// The one unread counter of the islands (the bell; the Conversations
/// button of the dock): the accent at the theme's radius, its number in
/// `on_accent`, pinned by its top right corner on the slot's, `scale` for
/// the bell's bounce; grey while do not disturb pauses the bell.
pub fn count(ui: &egui::Ui, p: &Palette, slot: Rect, unread: usize, scale: f32, paused: bool) {
    let text = if unread > 99 { "99+".to_owned() } else { unread.to_string() };
    let (fill, ink) = if paused { (p.raised, p.muted) } else { (p.violet, p.on_accent) };
    let font = FontId::new(9.5 * scale, crate::ui::fonts::semibold(ui.ctx()));
    let galley = ui.painter().layout_no_wrap(text, font, ink);
    let h = 13.0 * scale;
    let w = (galley.size().x + 7.0 * scale).max(h);
    let pill = Rect::from_min_size(Pos2::new(slot.right() + 2.0 - w, slot.top()), Vec2::new(w, h));
    ui.painter().rect_filled(pill, ui.visuals().window_corner_radius, fill);
    crate::ui::widgets::paint_ink_centered(ui.painter(), galley, pill.center(), ink);
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

/// Reserve the island's surface under what is drawn next; `set_island`
/// gives it its rectangle once the contents are laid out.
fn reserve(ui: &egui::Ui) -> egui::layers::ShapeIdx {
    ui.painter().add(egui::Shape::Noop)
}

fn set_island(ui: &egui::Ui, p: &Palette, slot: egui::layers::ShapeIdx, rect: Rect) {
    ui.painter().set(slot, crate::ui::widgets::island_shape(ui.ctx(), p, rect));
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
    crate::ui::widgets::paint_ink_left(ui.painter(), galley, Pos2::new(x, rect.center().y), color);
    if clickable && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
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
    let left = screen.left() + MARGIN;
    let n = if st.show_bug_report { 2.0 } else { 1.0 };
    let menu_rect = row(screen, left, left + PAD * 2.0 + SLOT * n + GAP * (n - 1.0));
    let span = row(screen, menu_rect.right() + MARGIN, right);
    let (place_rect, detail) = super::place::layout(ui, st, span);

    // each island is an area of its own: the pointer over it is over the
    // interface (`Context::is_pointer_over_egui`), between them over the world
    let bell_slot = Rect::from_center_size(bell_rect.center(), Vec2::splat(SLOT));
    area(&ctx, p, "bell", bell_rect, |ui| {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bell_slot));
        let (clicked, rect) = if bell.aurora {
            crate::ui::notification_center::bell(&mut child, p, icons, &notif.center, bell.unread, notif.open, bell.dnd, true)
        } else {
            crate::ui::notifications::bell(&mut child, p, icons, bell.unread, notif.open, true)
        };
        notif.anchor = Some(Pos2::new(rect.left(), bell_rect.bottom()));
        notif.bell = Some(Rect::from_min_max(rect.min, Pos2::new(rect.right(), bell_rect.bottom())));
        if clicked {
            notif.open = !notif.open;
        }
    });
    let audio_action = area(&ctx, p, "system", sys_rect, |ui| {
        system(ui, p, icons, panels, st, audio, audio_ui, media, sys_rect)
    });
    area(&ctx, p, "account", acc_rect, |ui| account(ui, p, st, acc_rect, &mut action));
    area(&ctx, p, "menu", menu_rect, |ui| {
        let wolf = Rect::from_min_size(
            Pos2::new(menu_rect.left() + PAD, menu_rect.center().y - SLOT / 2.0),
            Vec2::splat(SLOT),
        );
        let resp = ui.interact(wolf, Id::new("bars_island_menu"), Sense::click());
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
        slot_face(ui, p, wolf, resp.hovered() || open, false);
        paint_icon(ui, icons, "wolf", wolf, 18.0, p.ink);
        menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            main_menus(ui, p, panels, st, &mut action, true);
        });
        resp.on_hover_text(aurora_i18n::tr!("ui-bars-aurora-menu"));
        // the bug report next to it, when shown
        if st.show_bug_report {
            let bug = wolf.translate(Vec2::new(SLOT + GAP, 0.0));
            if icon_slot(ui, p, icons, bug, Id::new("bars_island_bug"), "bug-filled", true)
                .on_hover_text(aurora_i18n::tr!("bug-title"))
                .clicked()
            {
                action = BarAction::ReportBug;
            }
        }
    });
    area(&ctx, p, "place", place_rect, |ui| {
        if let Some(a) = super::place::island(ui, p, icons, panels, st, place_rect, detail) {
            action = a;
        }
    });
    (action, audio_action)
}

/// One island in an area of its own at `rect`, its surface under what
/// `body` draws there (in screen coordinates).
fn area<R>(ctx: &egui::Context, p: &Palette, name: &'static str, rect: Rect, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Area::new(Id::new(("bars_island", name)))
        .order(egui::Order::Middle)
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(ctx, |ui| {
            let slot = reserve(ui);
            ui.set_min_size(rect.size());
            let out = body(ui);
            set_island(ui, p, slot, rect);
            out
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
    let status = ui.interact(status_rect, Id::new("bars_island_status"), Sense::click());
    let money = ui.interact(money_rect, Id::new("bars_island_balance"), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&status));
    slot_face(ui, p, rect, status.hovered() || money.hovered() || open, false);
    crate::ui::widgets::avatar_disc(ui, p, disc, st.own_pic, st.own_name, false);
    // the status dot on the picture's lower right, ringed with the island
    let dot = disc.right_bottom() - Vec2::splat(3.5);
    let col = status_color(p, &st.status);
    ui.painter().circle_filled(dot, 4.5, p.bar);
    if st.status.invisible {
        ui.painter().circle_stroke(dot, 2.4, egui::Stroke::new(1.2, col));
    } else {
        ui.painter().circle_filled(dot, 3.0, col);
    }
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
    menu::dropdown(&status, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        status_menu_body(ui, p, st.status, action);
    });
    status.on_hover_text(status_tip(&st.status));
}

/// The readouts of the system island: clock, latency, frame rate.
fn readouts(st: &StatusInfo) -> (String, String, String) {
    (slt_time(), format!("{} ms", st.ping_ms), aurora_i18n::number(f64::from(st.fps), 1))
}

/// The size of the readouts' text.
const READOUT: f32 = 12.0;
/// The speaker, radio and media buttons: three slots, 2 apart (`ui::audio`).
const AUDIO_W: f32 = SLOT * 3.0 + 2.0 * 2.0;

fn system_width(ui: &egui::Ui, st: &StatusInfo) -> f32 {
    let (time, ping, fps) = readouts(st);
    let readouts: f32 = [time, ping, fps]
        .iter()
        .map(|t| readout_width(steady_width(ui, t, READOUT)) + GAP)
        .sum();
    PAD + readouts + AUDIO_W + PAD
}

/// The system island: SLT clock with its seconds, region latency, frame
/// rate (a click opens Performance), then media, music and volume. Returns
/// the audio controls' action.
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
) -> crate::ui::audio::AudioAction {
    const SIZE: f32 = READOUT;
    let top = island.center().y - SLOT / 2.0;
    // media, music and volume: the audio controls in island slots
    let controls = Rect::from_min_max(
        Pos2::new(island.right() - PAD - AUDIO_W, top),
        Pos2::new(island.right() - PAD, top + SLOT),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(controls)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let audio_action = crate::ui::audio::top_controls(&mut child, p, icons, audio, audio_ui, media, Vec2::splat(SLOT));
    let mut x = controls.left() - GAP;
    let (time, ping_text, fps_text) = readouts(st);

    // frame rate, latency, clock, from the right
    let fps_col = if st.fps >= 50.0 {
        p.teal
    } else if st.fps >= 25.0 {
        p.amber
    } else {
        p.danger
    };
    let fps_w = readout_width(steady_width(ui, &fps_text, SIZE));
    let fps_rect = Rect::from_min_size(Pos2::new(x - fps_w, top), Vec2::new(fps_w, SLOT));
    let fps = readout(
        ui,
        p,
        icons,
        fps_rect,
        Id::new("bars_island_fps"),
        "gauge-filled",
        text(ui, fps_text, SIZE, fps_col),
        fps_col,
        true,
    );
    if fps
        .on_hover_text(aurora_i18n::tr!("ui-bars-frames-per-second-click-for-performance-details"))
        .clicked()
    {
        panels.perf = !panels.perf;
    }
    x = fps_rect.left() - GAP;

    let ping_w = readout_width(steady_width(ui, &ping_text, SIZE));
    let ping_rect = Rect::from_min_size(Pos2::new(x - ping_w, top), Vec2::new(ping_w, SLOT));
    readout(
        ui,
        p,
        icons,
        ping_rect,
        Id::new("bars_island_ping"),
        "antenna-bars-5",
        text(ui, ping_text, SIZE, p.muted),
        p.muted,
        false,
    )
    .on_hover_text(aurora_i18n::tr!("ui-bars-region-latency"));
    x = ping_rect.left() - GAP;

    let time_w = readout_width(steady_width(ui, &time, SIZE));
    let time_rect = Rect::from_min_size(Pos2::new(x - time_w, top), Vec2::new(time_w, SLOT));
    readout(
        ui,
        p,
        icons,
        time_rect,
        Id::new("bars_island_time"),
        "clock-filled",
        text(ui, time, SIZE, p.muted),
        p.muted,
        false,
    )
    .on_hover_text(aurora_i18n::tr!("ui-bars-second-life-time-pacific"));
    audio_action
}
