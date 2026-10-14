//! Aurora's top bar: glass islands laid over the world instead of a
//! full-width band (variant A of the bars mockup the human validated). From
//! the left: the Aurora menu, the bug report (when shown), the place
//! (stretched to the account island,
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

/// The bottom of the favourites' row under the islands, when it is shown
/// (`top` this frame): the windows' work area starts 8 px under it.
pub fn favorites_row_bottom(ctx: &egui::Context) -> Option<f32> {
    ctx.data(|d| d.get_temp::<f32>(Id::new("bars_favorites_row_bottom")))
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
    let pos = Pos2::new(left_center.x - ink.left(), left_center.y - digits.center().y);
    ui.painter().galley(pos.round(), galley, color);
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
        // a right click on the island or its buttons: show the bug button
        let back = ui.interact(menu_rect, Id::new("bars_island_menu_back"), Sense::click());
        menu_options(&back, p, st, &mut action);
        let wolf = Rect::from_min_size(
            Pos2::new(menu_rect.left() + PAD, menu_rect.center().y - SLOT / 2.0),
            Vec2::splat(SLOT),
        );
        let resp = ui.interact(wolf, Id::new("bars_island_menu"), Sense::click());
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
            // its right click hides it, as the wolf's does
            let back = ui.interact(bug_rect, Id::new("bars_island_bug_back"), Sense::click());
            menu_options(&back, p, st, &mut action);
            let bug = Rect::from_center_size(bug_rect.center(), Vec2::splat(SLOT));
            let resp = icon_slot(ui, p, icons, bug, Id::new("bars_island_bug"), "bug-filled", true);
            menu_options(&resp, p, st, &mut action);
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
    let row_id = Id::new("bars_favorites_row_bottom");
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

/// Right-click menu of the menu and bug islands: the bug report button, also in
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
pub(super) fn area<R>(ctx: &egui::Context, p: &Palette, name: &'static str, rect: Rect, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Area::new(Id::new(("bars_island", name)))
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

#[cfg(test)]
mod tests {
    use super::super::{IslandsLook, StatusInfo};
    use super::*;

    fn status(stretch: bool, bug: bool) -> StatusInfo<'static> {
        StatusInfo {
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
            marketplace: false,
            parcel_landmark: None,
            classic_bars: false,
            islands: IslandsLook {
                stretch,
                rights_text: false,
            },
            own_name: "Aurora Demo",
            own_pic: None,
            recent: Vec::new(),
            favorites: Box::leak(Box::default()),
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
            c.memory(|m| m.area_rect(Id::new(("bars_island", name))))
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
}
