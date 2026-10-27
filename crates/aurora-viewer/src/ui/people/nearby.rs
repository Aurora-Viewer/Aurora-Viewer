//! "Near you" in Aurora's look (the People mock-up, radar A): the avatars
//! as rows with their picture, status, badges and range, grouped by the
//! ranges of the chat (whisper, say, shout, beyond); a compact density of
//! one line each for a crowd; sort pills; and the sheet of the person
//! held, at the right (`sheet.rs`). The data, rules, options and menus
//! are those of Firestorm's radar table (`radar.rs`: FSRadar,
//! fspanelradar.cpp, menu_fs_radar_options.xml), which Preferences › Aurora
//! › People brings back.

use super::radar::{self, Column, Payment, RadarData, Row};
use super::{PeopleAction, PeopleUi};
use crate::theme::Palette;
use crate::ui::context::{self, AvatarList, CtxAction};
use crate::ui::{menu, presence, widgets};
use crate::world::World;
use crate::world::radar::{CHAT_RANGE, SHOUT_RANGE, WHISPER_RANGE};
use egui::{Color32, FontId, Pos2, Rect, Sense, Vec2};
use uuid::Uuid;

/// Height of a rich row (a 30 px picture and its margins), of a compact
/// one, and the gap between two rich rows.
pub(super) const RICH_H: f32 = 42.0;
const COMPACT_H: f32 = 24.0;
pub(super) const RICH_GAP: f32 = 3.0;
/// Height of the header of a range.
const BAND_H: f32 = 24.0;
/// Width of the range and of the time since the arrival.
const DIST_W: f32 = 58.0;
const COMPACT_DIST_W: f32 = 46.0;
const SEEN_W: f32 = 44.0;
/// Height of the toolbar (search, density, options).
const TOOLBAR_H: f32 = 30.0;
/// Width of the sheet, and the width the list keeps beside it: a window
/// narrower than both grows by what is missing when a sheet opens.
pub(super) const SHEET_W: f32 = 330.0;
const LIST_W: f32 = 510.0;
/// The least width that still shows the list beside the sheet; below it
/// the sheet takes the whole room.
const LIST_MIN: f32 = 260.0;

/// The ranges of the chat the rows are grouped by (CHAT_WHISPER_RADIUS,
/// CHAT_NORMAL_RADIUS, CHAT_SHOUT_RADIUS), as the mock-up does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Band {
    Whisper,
    Say,
    Shout,
    Beyond,
}

impl Band {
    /// An unknown range (a coarse location without altitude) is beyond.
    pub(super) fn of(range: Option<f32>) -> Band {
        match range {
            Some(d) if d <= WHISPER_RANGE => Band::Whisper,
            Some(d) if d <= CHAT_RANGE => Band::Say,
            Some(d) if d <= SHOUT_RANGE => Band::Shout,
            _ => Band::Beyond,
        }
    }

    /// The color of the range figures: turquoise for a whisper, green within
    /// the chat, amber within a shout.
    pub(super) fn color(self, p: &Palette) -> Color32 {
        match self {
            Band::Whisper => p.teal,
            Band::Say => p.success,
            Band::Shout => p.amber,
            Band::Beyond => p.muted,
        }
    }

    /// "Whisper · 10 m".
    pub(super) fn title(self) -> String {
        let m = |r: f32| aurora_i18n::number(f64::from(r), 0);
        match self {
            Band::Whisper => aurora_i18n::tr!("people-band-whisper", range = m(WHISPER_RANGE)),
            Band::Say => aurora_i18n::tr!("people-band-say", range = m(CHAT_RANGE)),
            Band::Shout => aurora_i18n::tr!("people-band-shout", range = m(SHOUT_RANGE)),
            Band::Beyond => aurora_i18n::tr!("people-band-beyond").to_owned(),
        }
    }
}

/// "4.7 m", or beyond the draw distance when the range is unknown.
pub(super) fn distance_text(range: Option<f32>, draw_distance: f32) -> String {
    match range {
        Some(d) => format!("{} m", aurora_i18n::number(f64::from(d), 1)),
        None => format!(">{} m", aurora_i18n::number(f64::from(draw_distance), 0)),
    }
}

/// Time since the arrival, short: "< 1 min", "12 min", "2 h".
pub(super) fn seen_text(seconds: f64) -> String {
    let s = seconds.max(0.0) as i64;
    if s < 60 {
        aurora_i18n::tr!("people-seen-now").to_owned()
    } else if s < 3600 {
        aurora_i18n::tr!("people-seen-minutes", n = s / 60)
    } else {
        aurora_i18n::tr!("people-seen-hours", n = s / 3600)
    }
}

/// The badges of a row in the mock-up's order (friend, voice, typing,
/// sitting, payment info, note), with the text each shows on hover. Only
/// what is so: no payment info, Firestorm's grey card, is in the sheet.
fn badges(p: &Palette, row: &Row) -> Vec<(&'static str, Color32, String)> {
    let mut out = Vec::new();
    if row.muted {
        out.push(("ban", p.muted_dim, aurora_i18n::tr!("common-blocked").to_owned()));
    }
    if row.friend {
        out.push(("heart-filled", p.success, aurora_i18n::tr!("ui-profile-aurora-friend").to_owned()));
    }
    match row.voice {
        Some(true) => out.push(("microphone-filled", p.success, aurora_i18n::tr!("radar-voice-speaking").to_owned())),
        Some(false) => out.push(("microphone-filled", p.muted, aurora_i18n::tr!("radar-voice-connected").to_owned())),
        None => {}
    }
    if row.typing {
        out.push(("keyboard-filled", p.violet_light, aurora_i18n::tr!("radar-col-typing").to_owned()));
    }
    if row.sitting {
        out.push(("armchair", p.muted, aurora_i18n::tr!("radar-col-sitting").to_owned()));
    }
    match row.payment {
        Payment::OnFile => out.push(("credit-card-filled", p.amber, aurora_i18n::tr!("radar-payment-on-file").to_owned())),
        Payment::Used => out.push(("credit-card-filled", p.success, aurora_i18n::tr!("radar-payment-used").to_owned())),
        Payment::None | Payment::Unknown => {}
    }
    if !row.note.is_empty() {
        out.push(("note", p.teal, row.note.clone()));
    }
    out
}

/// A semi-bold text format (the mock-up's weight 600).
fn semibold(ui: &egui::Ui, size: f32, color: Color32) -> egui::TextFormat {
    crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), size, color)
}

/// The second line of a rich row: the account name ("nova.exemple", as the
/// mock-up writes it: who they are, whatever the name preferences make of
/// the first line), or the display name when the first line is already the
/// username (name formats 1 and 3).
fn second_line(world: &World, id: &Uuid, format: u8) -> String {
    let names = &world.social.avatar_names;
    match names.get(id) {
        Some(n) if format == 1 || format == 3 => n.display(&names.options),
        Some(n) => n.username.clone(),
        None => String::new(),
    }
}

/// The sort pills: the columns of Firestorm's radar that a row shows.
const SORTS: [(Column, &str); 4] = [
    (Column::Range, "common-distance"),
    (Column::Name, "common-name"),
    (Column::Seen, "people-sort-arrival"),
    (Column::Age, "radar-col-age"),
];

/// Open the sheet of `id`; the window grows for it on the next frame if it
/// is too narrow.
pub(super) fn open_sheet(st: &mut PeopleUi, id: Uuid) {
    if st.sheet.is_none() {
        st.sheet_grow = true;
    }
    st.sheet = Some(id);
    st.sheet_group = false;
}

/// Close the sheet; the window gives back the width it took for it before
/// its next frame (`fit_window`).
pub(super) fn close_sheet(st: &mut PeopleUi) {
    st.sheet = None;
    st.sheet_grow = false;
}

/// Before the window is shown (a size given inside its body would be
/// overwritten by the one it ends with): widen it for a sheet that opened
/// beside a narrow list, or give back that width once the sheet closed.
pub(super) fn fit_window(ctx: &egui::Context, st: &mut PeopleUi) {
    if st.sheet.is_none() && st.sheet_grown > 0.0 {
        resize_window(ctx, -st.sheet_grown);
        st.sheet_grown = 0.0;
    }
    if st.sheet.is_none() || !st.sheet_grow {
        return;
    }
    let Some(width) = egui::AreaState::load(ctx, egui::Id::unique(super::WINDOW))
        .and_then(|s| s.size)
        .map(|s| s.x)
    else {
        // not laid out yet: next frame
        return;
    };
    // the list's width: the window's less its margins
    let missing = LIST_W + SHEET_W - (width - WINDOW_MARGINS);
    if missing > 0.0
        && let Some(took) = resize_window(ctx, missing)
    {
        st.sheet_grown += took;
    }
    st.sheet_grow = false;
}

/// What a floater's frame takes of its width around the body.
const WINDOW_MARGINS: f32 = 20.0;

/// Widen (or narrow) the People window by `dx`, at its place; returns what
/// it took, or None before the window was ever laid out.
fn resize_window(ctx: &egui::Context, dx: f32) -> Option<f32> {
    let state = egui::AreaState::load(ctx, egui::Id::unique(super::WINDOW))?;
    let size = state.size?;
    let pos = state.left_top_pos();
    let work = crate::ui::layout::work_area(ctx);
    // within the room right of the window
    let room = (work.right() - pos.x - size.x).max(0.0);
    let dx = dx.min(room);
    if dx.abs() < 0.5 {
        return Some(0.0);
    }
    widgets::place_floater(ctx, super::WINDOW, pos, Vec2::new((size.x + dx).max(LIST_MIN), size.y));
    Some(dx)
}

/// The "Near you" tab. `blocked` is set when the options ask for the block
/// list.
#[allow(clippy::too_many_arguments)]
pub fn tab(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    data: &mut RadarData,
    st: &mut PeopleUi,
    map: &mut dyn FnMut(&mut egui::Ui, Vec2),
    actions: &mut Vec<PeopleAction>,
    blocked: &mut bool,
) {
    st.radar_selected.retain(|id| data.radar.get(id).is_some());
    let (list, sheet) = areas(ui, st, false);
    if let Some(rect) = list {
        let mut child = list_ui(ui, rect, "people-nearby-list", sheet.is_some());
        list_part(&mut child, p, world, data, st, map, actions, blocked, sheet.is_some());
    }
    if let (Some(rect), Some(id)) = (sheet, st.sheet)
        && super::sheet::show(
            &mut list_ui(ui, rect, "people-sheet", false),
            p,
            world,
            data,
            st,
            id,
            true,
            true,
            actions,
        )
    {
        close_sheet(st);
    }
}

/// Whether the list and sheet can be shown beside each other.
pub(super) fn sheet_fits(width: f32) -> bool {
    width >= LIST_MIN + SHEET_W
}

/// The room left in `ui` taken whole, then shared between the list and the
/// sheet: the list alone, both side by side, or the sheet alone when the
/// window is too narrow for both (and the list alone on the frame the
/// window waits to grow for a sheet).
pub(super) fn areas(ui: &mut egui::Ui, st: &PeopleUi, persistent_sheet: bool) -> (Option<Rect>, Option<Rect>) {
    let body = Rect::from_min_max(ui.cursor().min, ui.max_rect().max);
    ui.allocate_rect(body, Sense::hover());
    let split = sheet_fits(body.width());
    if split && persistent_sheet {
        let (list, sheet) = body.split_left_right_at_x(body.right() - SHEET_W);
        return (Some(list), Some(sheet));
    }
    match st.sheet {
        Some(_) if st.sheet_grow => (Some(body), None),
        Some(_) if split => {
            let (l, s) = body.split_left_right_at_x(body.right() - SHEET_W);
            (Some(l), Some(s))
        }
        Some(_) => (None, Some(body)),
        None => (Some(body), None),
    }
}

/// A child of `ui` over `rect`, top-down; clipped to it beside a sheet.
pub(super) fn list_ui(ui: &mut egui::Ui, rect: Rect, salt: &str, clip: bool) -> egui::Ui {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(salt)
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    if clip {
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
    }
    child
}

/// The list side: toolbar, sort pills, counts, then the rows.
#[allow(clippy::too_many_arguments)]
fn list_part(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    data: &mut RadarData,
    st: &mut PeopleUi,
    map: &mut dyn FnMut(&mut egui::Ui, Vec2),
    actions: &mut Vec<PeopleAction>,
    blocked: &mut bool,
    beside_sheet: bool,
) {
    // a little room between the list and the sheet
    if beside_sheet {
        ui.set_max_width(ui.max_rect().width() - 10.0);
    }
    // the mock-up's spacing, every gap written below
    ui.spacing_mut().item_spacing.y = 0.0;
    let compact = data.settings.compact;
    toolbar(ui, p, data, st, blocked);
    ui.add_space(if compact { 6.0 } else { 4.0 });
    if data.settings.show_minimap {
        ui.vertical_centered(|ui| map(ui, Vec2::new(ui.available_width().min(320.0), 150.0)));
        ui.add_space(6.0);
    }
    if !compact {
        sort_pills(ui, p, data);
        ui.add_space(7.0);
    }
    // the demo's row "under the pointer" lasts until the real pointer moves
    if st.demo_hover.is_some() && ui.input(|i| i.pointer.delta() != Vec2::ZERO) {
        st.demo_hover = None;
    }
    let filter = st.filter.trim().to_lowercase();
    let list = radar::rows(world, data, &filter);
    summary(
        ui,
        p,
        data.radar.total,
        list.iter().filter(|r| r.range.is_some_and(|d| d <= CHAT_RANGE)).count(),
        list.iter().filter(|r| r.friend).count(),
    );
    ui.add_space(if compact { 4.0 } else { 3.0 });
    let order: Vec<Uuid> = list.iter().map(|r| r.id).collect();
    let by_range = Column::ALL.get(data.settings.sort as usize) == Some(&Column::Range);
    let now = ui.input(|i| i.time);
    egui::ScrollArea::vertical()
        .id_salt("people-nearby-rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = if compact { 0.0 } else { RICH_GAP };
            if list.is_empty() {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(aurora_i18n::tr!("radar-empty")).size(12.0).color(p.muted));
            }
            let mut band = None;
            for (i, row) in list.iter().enumerate() {
                if by_range && !compact && band != Some(Band::of(row.range)) {
                    band = Some(Band::of(row.range));
                    band_header(ui, p, Band::of(row.range));
                }
                let width = ui.available_width().min(ui.clip_rect().right() - ui.cursor().left()).max(0.0);
                let bounds = Rect::from_min_size(ui.cursor().min, Vec2::new(width, if compact { COMPACT_H } else { RICH_H }));
                let count = 4 + usize::from(!row.friend && row.id != world.agent_id);
                let badge_count = badges(p, row).len();
                let (rect, resp) = ui.allocate_exact_size(bounds.size(), Sense::click_and_drag());
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                if !data.pics.contains_key(&row.id) {
                    st.want_pic(row.id, now);
                }
                let selected = st.radar_selected.contains(&row.id) || st.sheet == Some(row.id);
                let opacity = ui.opacity();
                if row.left_behind {
                    ui.set_opacity(opacity * radar::LEFT_BEHIND_OPACITY);
                }
                let pointer = resp.hover_pos();
                let hot =
                    (ui.rect_contains_pointer(rect) && !resp.dragged()) || st.row_menu == Some(row.id) || st.demo_hover == Some(row.id);
                let end = right_end(rect, names_left(rect, compact), badge_count, hot.then_some(count), compact);
                let tip = if compact {
                    paint_compact(ui, p, world, data, st, row, rect, &end, i, selected, hot, pointer, actions)
                } else {
                    paint_rich(ui, p, world, data, st, row, rect, &end, selected, hot, pointer, actions)
                };
                ui.set_opacity(opacity);
                let action = super::rows::select(ui, &resp, &mut st.radar_selected, &mut st.radar_anchor, &order, row.id);
                if action.sheet {
                    open_sheet(st, row.id);
                }
                // double click: the camera goes to the avatar and follows it, when it is
                // drawn (LLAvatarActions::zoomIn, FSPanelRadar)
                if action.activate {
                    if row.loaded {
                        context::request(ui.ctx(), CtxAction::ZoomAvatar(row.id));
                    } else {
                        actions.push(PeopleAction::TooFarToZoom);
                    }
                }
                // menu_fs_radar.xml, menu_fs_radar_multiselect.xml for several
                let several = st.radar_selected.len() > 1 && st.radar_selected.contains(&row.id);
                let chosen = st.radar_selected.clone();
                menu::context_menu(&resp, p, |ui| {
                    if several {
                        context::avatars_menu(ui, p, world, &chosen);
                    } else {
                        context::avatar_list_menu(ui, p, world, row.id, AvatarList::Nearby);
                    }
                });
                super::rows::contact_tip(resp, tip.as_deref());
            }
        });
}

/// Search, density and the options of the list (menu_fs_radar_options.xml).
fn toolbar(ui: &mut egui::Ui, p: &Palette, data: &mut RadarData, st: &mut PeopleUi, blocked: &mut bool) {
    let line = Vec2::new(ui.available_width(), TOOLBAR_H);
    ui.allocate_ui_with_layout(line, egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let resp = super::rows::gear(ui, p, aurora_i18n::tr!("people-alerts-options"));
        if std::env::var_os("AURORA_DEMO").is_some() && std::env::var("AURORA_DEMO_BRIDGE_QUERY").is_ok_and(|m| m == "radar") {
            egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&resp));
        }
        menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            radar::options_menu(ui, p, data.settings, blocked);
        });
        density(ui, p, &mut data.settings.compact);
        let w = ui.available_width();
        widgets::search_box(ui, p, &mut st.filter, aurora_i18n::tr!("people-filter-by-name"), w);
    });
}

/// The two densities of the rows, as icons in one track: rich rows, or one
/// line each.
fn density(ui: &mut egui::Ui, p: &Palette, compact: &mut bool) {
    const SEG: Vec2 = Vec2::new(30.0, 22.0);
    const PAD: f32 = 2.0;
    let (track, _) = ui.allocate_exact_size(Vec2::new(SEG.x * 2.0 + PAD * 2.0, SEG.y + PAD * 2.0), Sense::hover());
    ui.painter().rect_filled(track, ui.visuals().widgets.inactive.corner_radius, p.bar);
    for (i, (icon, tip, value)) in [
        ("list-filled", aurora_i18n::tr!("people-view-rich"), false),
        ("baseline-density-small", aurora_i18n::tr!("people-view-compact"), true),
    ]
    .into_iter()
    .enumerate()
    {
        let rect = Rect::from_min_size(Pos2::new(track.left() + PAD + SEG.x * i as f32, track.top() + PAD), SEG);
        let resp = ui.interact(rect, ui.scope_id().with(("people-density", i)), Sense::click());
        let on = *compact == value;
        let (face, ink) = if on {
            (Some(p.violet), p.on_accent)
        } else if resp.hovered() {
            (Some(p.raised.gamma_multiply(0.5)), p.ink)
        } else {
            (None, p.muted)
        };
        if let Some(face) = face {
            ui.painter().rect_filled(rect, widgets::inner_radius(ui.visuals()), face);
        }
        radar::icon(ui, icon, rect, 14.0, ink);
        if resp.on_hover_text(tip).clicked() {
            *compact = value;
        }
    }
}

/// "Sort" and the four pills; the lit one sorts the other way when clicked
/// again.
fn sort_pills(ui: &mut egui::Ui, p: &Palette, data: &mut RadarData) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        ui.label(egui::RichText::new(aurora_i18n::tr!("people-sort")).size(11.0).color(p.muted_dim));
        let width = SORTS
            .iter()
            .map(|(column, key)| {
                widgets::filter_pill_width(
                    ui,
                    if data.settings.sort == column.sort_index() {
                        "arrows-sort"
                    } else {
                        ""
                    },
                    aurora_i18n::text(key),
                )
            })
            .sum::<f32>()
            + 15.0;
        if width > ui.available_width() {
            let (_, key) = SORTS
                .iter()
                .find(|(column, _)| data.settings.sort == column.sort_index())
                .unwrap_or(&SORTS[0]);
            let sort = widgets::filter_pill(ui, p, "chevron-down", aurora_i18n::text(key), true);
            menu::dropdown(&sort, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                for (column, key) in SORTS {
                    if menu::check(
                        ui,
                        p,
                        column.icon(),
                        aurora_i18n::text(key),
                        data.settings.sort == column.sort_index(),
                        true,
                    ) {
                        choose_sort(data, column);
                    }
                }
            });
        } else {
            for (column, key) in SORTS {
                let index = column.sort_index();
                let on = data.settings.sort == index;
                let icon = if on { "arrows-sort" } else { "" };
                if widgets::filter_pill(ui, p, icon, aurora_i18n::text(key), on).clicked() {
                    choose_sort(data, column);
                }
            }
        }
    });
}

fn choose_sort(data: &mut RadarData, column: Column) {
    if data.settings.sort == column.sort_index() {
        data.settings.sort_descending = !data.settings.sort_descending;
    } else {
        data.settings.sort = column.sort_index();
        data.settings.sort_descending = false;
    }
}

/// "16 near you · 7 within earshot · 2 friends": the figures in the ink,
/// their words in the muted color.
fn summary(ui: &mut egui::Ui, p: &Palette, total: usize, chat: usize, friends: usize) {
    // a line of text, not of buttons
    ui.spacing_mut().interact_size.y = 16.0;
    ui.horizontal_wrapped(|ui| {
        ui.add_space(2.0);
        ui.spacing_mut().item_spacing.x = 12.0;
        for (n, words) in [
            (total, aurora_i18n::tr!("people-sum-nearby", count = total as i64)),
            (chat, aurora_i18n::tr!("people-sum-earshot", count = chat as i64)),
            (friends, aurora_i18n::tr!("people-sum-friends", count = friends as i64)),
        ] {
            let mut job = egui::text::LayoutJob::default();
            job.append(&aurora_i18n::integer(n as i64), 0.0, semibold(ui, 12.0, p.ink));
            job.append(&words, 4.0, egui::TextFormat::simple(FontId::proportional(12.0), p.muted));
            ui.label(job);
        }
    });
}

/// The header of a range: its name in small capitals, then a faint line.
fn band_header(ui: &mut egui::Ui, p: &Palette, band: Band) {
    section_header(ui, p, &band.title());
}

/// The header of a part of a list ("Whisper · 10 m", "Online · 2").
pub(super) fn section_header(ui: &mut egui::Ui, p: &Palette, title: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), BAND_H), Sense::hover());
    let galley = ui.painter().layout_no_wrap(
        title.to_uppercase(),
        FontId::new(10.5, crate::ui::fonts::semibold(ui.ctx())),
        p.muted_dim,
    );
    let y = rect.bottom() - 9.0;
    let w = galley.size().x;
    crate::ui::widgets::paint_row_text(ui.painter(), galley, rect.left() + 2.0, y, p.muted_dim);
    let from = rect.left() + 2.0 + w + 6.0;
    if from < rect.right() {
        ui.painter()
            .hline(from..=rect.right(), y, egui::Stroke::new(1.0, p.raised.gamma_multiply(0.5)));
    }
}

/// The most badges a row can carry: blocked, friend, voice, typing,
/// sitting, payment info, note.
#[cfg(test)]
const MAX_BADGES: usize = 7;

/// The measures of a row's right end, rich or compact.
struct Metrics {
    /// Room left of the row's right edge.
    pad: f32,
    /// Side of a badge, and the gap between two.
    badge: f32,
    gap: f32,
    /// Width of the "+2" of the badges that do not fit.
    more: f32,
    dist: f32,
    /// Width of the time since the arrival; none in the compact rows.
    seen: f32,
    /// One quick action's button.
    button: Vec2,
    /// What the names keep at least, and the gap after them.
    names_min: f32,
}

const fn metrics(compact: bool) -> Metrics {
    if compact {
        Metrics {
            pad: 6.0,
            badge: 11.0,
            gap: 4.0,
            more: 20.0,
            dist: COMPACT_DIST_W,
            seen: 0.0,
            button: Vec2::new(24.0, 20.0),
            names_min: 48.0,
        }
    } else {
        Metrics {
            pad: 8.0,
            badge: 13.0,
            gap: 4.0,
            more: 22.0,
            dist: DIST_W,
            seen: SEEN_W,
            button: Vec2::new(26.0, 24.0),
            names_min: 60.0,
        }
    }
}

/// Gap between the badges and the time, the range or the quick actions;
/// between the names and the badges.
const BADGES_GAP: f32 = 8.0;
const NAMES_GAP: f32 = 6.0;

/// Quick actions replace the time and distance at the row's right edge.
/// Every action stays on one line; narrowing the list never changes row height.
#[derive(Debug)]
pub(super) struct RightEnd {
    /// The first badges of the row's list, left to right.
    pub badges: Vec<Rect>,
    /// The "+2" of the others, and how many it stands for.
    pub more: Option<(Rect, usize)>,
    pub seen: Option<Rect>,
    pub dist: Option<Rect>,
    /// The pill of the quick actions, with one rectangle per button.
    pub actions: Option<(Rect, Vec<Rect>)>,
    /// Normal text limit, independent of the hover actions.
    pub names_right: f32,
}

impl RightEnd {
    /// Badges may move left on hover. Clip the normal name galley behind
    /// them instead of laying out a shorter name with another ellipsis.
    fn name_painter(&self, ui: &egui::Ui) -> egui::Painter {
        let mut clip = ui.clip_rect();
        if let Some(left) = self.badges.first().map(|r| r.left()).or(self.more.map(|(r, _)| r.left())) {
            clip.max.x = clip.max.x.min(left - NAMES_GAP);
        }
        ui.painter().with_clip_rect(clip)
    }
}

/// The right end of `row` for `badges` badges, its names starting at
/// `names_left`; `actions` replaces the time and distance while the row
/// is under the pointer, without covering the avatar or changing row height.
pub(super) fn right_end(row: Rect, names_left: f32, badges: usize, actions: Option<usize>, compact: bool) -> RightEnd {
    let m = metrics(compact);
    let right = row.right() - m.pad;
    let y = row.center().y;
    let dist = Rect::from_min_max(Pos2::new(right - m.dist, row.top()), Pos2::new(right, row.bottom()));
    let seen = (m.seen > 0.0).then(|| {
        Rect::from_min_max(
            Pos2::new(dist.left() - 6.0 - m.seen, row.top()),
            Pos2::new(dist.left() - 6.0, row.bottom()),
        )
    });
    let actions = actions.map(|n| {
        let left = names_left.min(right - super::rows::actions_width_for(n, 14.0));
        let slot = super::rows::actions_slot(row, left, right, n, m.button);
        super::rows::action_rects(slot, n, m.button)
    });
    let zone = actions
        .as_ref()
        .map_or_else(|| seen.map_or(dist.left(), |s| s.left()), |(pill, _)| pill.left());
    let badges_right = zone - BADGES_GAP;
    let room = badges_right - (names_left + m.names_min + NAMES_GAP);
    let width = |k: usize| if k == 0 { 0.0 } else { k as f32 * m.badge + (k - 1) as f32 * m.gap };
    // all of them, else as many as fit beside the "+2"
    let (shown, more) = if width(badges) <= room {
        (badges, false)
    } else {
        let beside = room - m.more - m.gap;
        let k = (0..badges).rev().find(|k| width(*k) <= beside).unwrap_or(0);
        (k, room >= m.more)
    };
    let mut x = badges_right;
    let more = more.then(|| {
        let r = Rect::from_center_size(Pos2::new(x - m.more / 2.0, y), Vec2::new(m.more, m.badge + 4.0));
        x = r.left() - m.gap;
        (r, badges - shown)
    });
    let mut rects: Vec<Rect> = (0..shown)
        .map(|_| {
            let r = Rect::from_center_size(Pos2::new(x - m.badge / 2.0, y), Vec2::splat(m.badge));
            x = r.left() - m.gap;
            r
        })
        .collect();
    rects.reverse();
    let first = rects.first().map(|r| r.left()).or(more.map(|(r, _)| r.left())).unwrap_or(zone);
    // Hover controls cover the name instead of shortening its normal text layout.
    let names_right = if actions.is_some() {
        right_end(row, names_left, badges, None, compact).names_right
    } else {
        first - NAMES_GAP
    };
    RightEnd {
        badges: rects,
        more,
        seen: if actions.is_none() { seen } else { None },
        dist: actions.is_none().then_some(dist),
        actions,
        names_right,
    }
}

/// The badges of a row in their places, and "+2" for the others; returns
/// the text of the one under the pointer.
fn paint_badges(
    ui: &egui::Ui,
    p: &Palette,
    list: &[(&'static str, Color32, String)],
    end: &RightEnd,
    compact: bool,
    pointer: Option<Pos2>,
) -> Option<String> {
    let size = metrics(compact).badge;
    let mut tip = None;
    for ((icon, tint, text), cell) in list.iter().zip(&end.badges) {
        radar::icon(ui, icon, *cell, size, *tint);
        if pointer.is_some_and(|at| cell.expand(2.0).contains(at)) {
            tip = Some(text.clone());
        }
    }
    if let Some((cell, hidden)) = end.more {
        let galley = ui.painter().layout_job(egui::text::LayoutJob::single_section(
            format!("+{hidden}"),
            semibold(ui, size - 1.0, p.muted),
        ));
        crate::ui::widgets::paint_ink_centered(ui.painter(), galley, cell.center(), p.muted);
        if pointer.is_some_and(|at| cell.contains(at)) {
            let rest: Vec<&str> = list[end.badges.len()..].iter().map(|(_, _, text)| text.as_str()).collect();
            tip = Some(rest.join("\n"));
        }
    }
    tip
}

/// The quick actions of a row: profile, instant message, a friendship
/// offer to a stranger, contact details, and the row's whole menu.
fn quick_actions(world: &World, row: &Row) -> Vec<(&'static str, &'static str)> {
    let mut buttons = vec![
        ("user-filled", aurora_i18n::tr!("common-view-profile")),
        ("message-circle-filled", aurora_i18n::tr!("ui-context-send-im")),
    ];
    if !row.friend && row.id != world.agent_id {
        buttons.push(("user-plus", aurora_i18n::tr!("radar-add-friend")));
    }
    buttons.push(("info-circle-filled", aurora_i18n::tr!("people-show-contact-sheet")));
    buttons.push(("dots-filled", aurora_i18n::tr!("ui-chat-more")));
    buttons
}

/// The names of a row start there.
fn names_left(rect: Rect, compact: bool) -> f32 {
    if compact {
        rect.left() + 6.0 + 20.0 + 7.0
    } else {
        rect.left() + 8.0 + 30.0 + 10.0
    }
}

/// A rich identity with badges. Quick actions replace the time and range
/// at the right. Every list width keeps the same row height.
#[allow(clippy::too_many_arguments)]
fn paint_rich(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    data: &RadarData,
    st: &mut PeopleUi,
    row: &Row,
    rect: Rect,
    end: &RightEnd,
    selected: bool,
    hot: bool,
    pointer: Option<Pos2>,
    actions: &mut Vec<PeopleAction>,
) -> Option<String> {
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let face = if selected {
        p.violet.gamma_multiply(0.18)
    } else if hot {
        p.raised.gamma_multiply(0.45)
    } else {
        p.field
    };
    ui.painter().rect_filled(rect, radius, face);
    // the picture, round, with its status dot
    let identity = Rect::from_min_size(rect.min, Vec2::new(rect.width(), RICH_H));
    let disc = Rect::from_center_size(Pos2::new(rect.left() + 8.0 + 15.0, identity.center().y), Vec2::splat(30.0));
    widgets::avatar_disc(ui, p, disc, data.pics.get(&row.id), &row.name, row.muted);
    let status = presence::of(world, &row.id);
    if status != presence::Status::Unknown {
        presence::paint_dot(ui.painter(), p, disc, status, p.field);
    }
    let list = badges(p, row);
    let left = names_left(rect, false);
    // Quick actions take the place of the range and arrival time.
    let band = Band::of(row.range);
    if let Some(dist) = end.dist {
        let color = if row.range.is_some() { band.color(p) } else { p.muted };
        let galley = ui.painter().layout_job(egui::text::LayoutJob::single_section(
            distance_text(row.range, data.draw_distance),
            semibold(ui, 13.0, color),
        ));
        let left = dist.right() - galley.size().x;
        crate::ui::widgets::paint_row_text(ui.painter(), galley, left, dist.center().y, color);
    }
    if let Some(seen) = end.seen {
        let galley = ui
            .painter()
            .layout_no_wrap(seen_text(row.seen), FontId::proportional(11.0), p.muted_dim);
        let left = seen.right() - galley.size().x;
        crate::ui::widgets::paint_row_text(ui.painter(), galley, left, seen.center().y, p.muted_dim);
    }
    // Keep the normal name layout underneath the hover controls.
    let width = (end.names_right - left).max(0.0);
    let name_color = if data.settings.color_names {
        band.color(p)
    } else if row.muted {
        p.muted
    } else {
        p.ink
    };
    let name_color = crate::ui::colors::resident_name(world, &row.id, name_color);
    let mut name = semibold(ui, 13.0, name_color);
    name.italics = row.muted;
    let mut job = egui::text::LayoutJob::single_section(row.name.clone(), name);
    let mark = crate::world::staff::mark(world, &row.id);
    let staff = staff_room(ui, p, mark, 13.0, width);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width - staff);
    let name_g = ui.painter().layout_job(job);
    let cut = name_g.elided;
    let user = second_line(world, &row.id, data.settings.name_format);
    let mut job = egui::text::LayoutJob::single_section(user, egui::TextFormat::simple(FontId::proportional(11.5), p.muted));
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    let user_g = ui.painter().layout_job(job);
    let total = name_g.size().y + user_g.size().y - 2.0;
    let top = identity.center().y - total / 2.0;
    let (name_w, name_h) = (name_g.size().x, name_g.size().y);
    let painter = end.name_painter(ui);
    painter.galley(Pos2::new(left, top), name_g, name_color);
    painter.galley(Pos2::new(left, top + name_h - 2.0), user_g, p.muted);
    if staff > 0.0 {
        crate::ui::staff::paint_at(ui, &painter, p, mark, Pos2::new(left + name_w + 6.0, top + name_h / 2.0), 13.0);
    }
    let tip = paint_badges(ui, p, &list, end, false, pointer);
    if hot && let Some((pill, cells)) = &end.actions {
        hover_actions(ui, p, world, st, row, *pill, cells, &quick_actions(world, row), actions);
    }
    tip.or_else(|| (cut && pointer.is_some_and(|at| at.x >= left && at.x <= left + width)).then(|| row.name.clone()))
}

/// Points a member of the Aurora team's tag takes after a name `font`
/// points high in `width` (the gap included), or 0: the name is cut before
/// the tag is, as in the other lists (`rows::person_names`).
fn staff_room(ui: &egui::Ui, p: &Palette, mark: Option<crate::world::staff::Mark>, font: f32, width: f32) -> f32 {
    let tag = crate::ui::staff::width(ui, p, mark, font);
    if tag > 0.0 && width - tag - 6.0 >= 40.0 { tag + 6.0 } else { 0.0 }
}

/// A compact identity with the complete action strip replacing the range.
#[allow(clippy::too_many_arguments)]
fn paint_compact(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    data: &RadarData,
    st: &mut PeopleUi,
    row: &Row,
    rect: Rect,
    end: &RightEnd,
    index: usize,
    selected: bool,
    hot: bool,
    pointer: Option<Pos2>,
    actions: &mut Vec<PeopleAction>,
) -> Option<String> {
    let radius = widgets::inner_radius(ui.visuals());
    if selected {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.18));
    } else if hot {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.45));
    } else if index.is_multiple_of(2) {
        // every other line, faintly, to follow a line across
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.12));
    }
    let identity = Rect::from_min_size(rect.min, Vec2::new(rect.width(), COMPACT_H));
    let disc = Rect::from_center_size(Pos2::new(rect.left() + 6.0 + 10.0, identity.center().y), Vec2::splat(20.0));
    widgets::avatar_disc(ui, p, disc, data.pics.get(&row.id), &row.name, row.muted);
    let status = presence::of(world, &row.id);
    if status != presence::Status::Unknown {
        presence::paint_dot(ui.painter(), p, disc, status, p.panel);
    }
    let list = badges(p, row);
    let left = names_left(rect, true);
    let band = Band::of(row.range);
    if let Some(dist) = end.dist {
        let color = if row.range.is_some() { band.color(p) } else { p.muted };
        let galley = ui.painter().layout_job(egui::text::LayoutJob::single_section(
            distance_text(row.range, data.draw_distance),
            semibold(ui, 11.5, color),
        ));
        let left = dist.right() - galley.size().x;
        crate::ui::widgets::paint_row_text(ui.painter(), galley, left, dist.center().y, color);
    }
    let width = (end.names_right - left).max(0.0);
    let name_color = if data.settings.color_names {
        band.color(p)
    } else if row.muted {
        p.muted
    } else {
        p.ink
    };
    let name_color = crate::ui::colors::resident_name(world, &row.id, name_color);
    let mut format = egui::TextFormat::simple(FontId::proportional(12.0), name_color);
    format.italics = row.muted;
    let mut job = egui::text::LayoutJob::single_section(row.name.clone(), format);
    let mark = crate::world::staff::mark(world, &row.id);
    let staff = staff_room(ui, p, mark, 12.0, width);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width - staff);
    let g = ui.painter().layout_job(job);
    let cut = g.elided;
    let name_w = g.size().x;
    let painter = end.name_painter(ui);
    crate::ui::widgets::paint_row_text(&painter, g, left, identity.center().y, name_color);
    if staff > 0.0 {
        crate::ui::staff::paint_at(ui, &painter, p, mark, Pos2::new(left + name_w + 6.0, identity.center().y), 12.0);
    }
    let tip = paint_badges(ui, p, &list, end, true, pointer);
    if hot && let Some((pill, cells)) = &end.actions {
        hover_actions(ui, p, world, st, row, *pill, cells, &quick_actions(world, row), actions);
    }
    tip.or_else(|| (cut && pointer.is_some_and(|at| at.x >= left && at.x <= left + width)).then(|| row.name.clone()))
}

/// The quick actions of the row under the pointer, in the pill `right_end`
/// placed at the right, where the time and range were.
#[allow(clippy::too_many_arguments)]
fn hover_actions(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut PeopleUi,
    row: &Row,
    pill: Rect,
    cells: &[Rect],
    buttons: &[(&'static str, &'static str)],
    actions: &mut Vec<PeopleAction>,
) {
    ui.painter()
        .rect_filled(pill, ui.visuals().widgets.inactive.corner_radius, p.raised);
    let mut menu_open = false;
    for ((icon, tip), r) in buttons.iter().zip(cells) {
        let resp = ui.interact(*r, ui.scope_id().with(("people-row-act", row.id, *icon)), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(*r, widgets::inner_radius(ui.visuals()), p.bar);
        }
        radar::icon(ui, icon, *r, 14.0, p.ink);
        let resp = resp.on_hover_text(*tip).on_hover_cursor(egui::CursorIcon::PointingHand);
        match *icon {
            "user-filled" if resp.clicked() => actions.push(PeopleAction::Profile(row.id)),
            "message-circle-filled" if resp.clicked() => actions.push(PeopleAction::OpenIm(row.id)),
            "user-plus" if resp.clicked() => context::request(ui.ctx(), CtxAction::AddFriend(row.id)),
            "info-circle-filled" if resp.clicked() => {
                st.radar_selected = vec![row.id];
                st.radar_anchor = Some(row.id);
                open_sheet(st, row.id);
            }
            "dots-filled" => {
                menu_open = menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    context::avatar_list_menu(ui, p, world, row.id, AvatarList::Nearby);
                });
            }
            _ => {}
        }
    }
    if menu_open {
        st.row_menu = Some(row.id);
    } else if st.row_menu == Some(row.id) {
        st.row_menu = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_nearby_action_stays_visible_and_details_opens_the_sheet() {
        let theme = crate::theme::Theme::default();
        let p = theme.palette();
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        theme.apply(&frames.ctx, 1.0);
        let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let radar = crate::world::radar::Radar::default();
        let mut settings = super::super::RadarSettings::default();
        let (voice, pics) = (std::collections::HashMap::new(), std::collections::HashMap::new());
        let data = RadarData {
            radar: &radar,
            settings: &mut settings,
            voice: &voice,
            draw_distance: 96.0,
            now: 0.0,
            pics: &pics,
        };
        for compact in [false, true] {
            for friend in [false, true] {
                let row = Row {
                    id: Uuid::from_u128(0xF100),
                    name: "A resident with a long display name".into(),
                    friend,
                    muted: true,
                    voice: Some(true),
                    same_region: true,
                    typing: true,
                    sitting: true,
                    payment: Payment::Used,
                    note: "A contact note".into(),
                    age: radar::Age::Unknown,
                    seen: 125.0,
                    range: Some(4.7),
                    loaded: true,
                    left_behind: false,
                };
                let buttons = quick_actions(&world, &row);
                assert_eq!(buttons.len(), if friend { 4 } else { 5 });
                assert_eq!(buttons[buttons.len() - 2].0, "info-circle-filled");
                for width in [220.0, 250.0, 260.0, 360.0, 520.0] {
                    let base = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(width, if compact { COMPACT_H } else { RICH_H }));
                    let end = right_end(
                        base,
                        names_left(base, compact),
                        badges(&p, &row).len(),
                        Some(buttons.len()),
                        compact,
                    );
                    let full = base;
                    let (pill, cells) = end.actions.as_ref().expect("every action has a slot");
                    assert_eq!(cells.len(), buttons.len());
                    assert!(cells.iter().all(|cell| full.contains_rect(*cell)));
                    assert!(
                        cells.iter().all(|cell| cell.center().y == base.center().y),
                        "every action stays on one line"
                    );
                    assert!(
                        cells.iter().all(|cell| cell.left() >= names_left(base, compact)),
                        "the avatar remains clickable"
                    );
                    assert_eq!(pill.right(), base.right() - metrics(compact).pad);
                    assert!(end.dist.is_none() && end.seen.is_none(), "actions replace the time and distance");
                    let info = cells[cells.len() - 2].center();
                    let mut st = PeopleUi::default();
                    let mut actions = Vec::new();
                    let mut draw = |ui: &mut egui::Ui| {
                        if compact {
                            paint_compact(ui, &p, &world, &data, &mut st, &row, full, &end, 0, false, true, None, &mut actions);
                        } else {
                            paint_rich(ui, &p, &world, &data, &mut st, &row, full, &end, false, true, None, &mut actions);
                        }
                    };
                    for _ in 0..3 {
                        frames.run(Vec::new(), &mut draw);
                    }
                    let shapes = frames.shapes(Vec::new(), &mut draw);
                    let name_index = shapes
                        .iter()
                        .position(|shape| matches!(shape, egui::Shape::Text(t) if t.galley.text() == row.name))
                        .expect("the resident's name is painted");
                    let pill_index = shapes
                        .iter()
                        .position(|shape| matches!(shape, egui::Shape::Rect(r) if r.rect == *pill && r.fill == p.raised))
                        .expect("the action strip is painted");
                    assert!(pill_index > name_index, "the hover strip covers the name");
                    frames.run(
                        vec![
                            egui::Event::PointerMoved(info),
                            egui::Event::PointerButton {
                                pos: info,
                                button: egui::PointerButton::Primary,
                                pressed: true,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                        &mut draw,
                    );
                    frames.run(
                        vec![egui::Event::PointerButton {
                            pos: info,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: egui::Modifiers::NONE,
                        }],
                        &mut draw,
                    );
                    assert_eq!(st.sheet, Some(row.id), "{width}, compact={compact}, friend={friend}");
                    assert_eq!(st.radar_selected, vec![row.id]);
                    assert!(st.sheet_grow, "People retains its sheet window expansion");
                    assert!(actions.is_empty(), "details does not start a profile, IM or friendship offer");
                }
            }
        }
    }

    #[test]
    fn the_rows_are_grouped_by_the_ranges_of_the_chat() {
        assert_eq!(Band::of(Some(4.7)), Band::Whisper);
        assert_eq!(Band::of(Some(10.0)), Band::Whisper);
        assert_eq!(Band::of(Some(10.1)), Band::Say);
        assert_eq!(Band::of(Some(20.0)), Band::Say);
        assert_eq!(Band::of(Some(42.0)), Band::Shout);
        assert_eq!(Band::of(Some(100.5)), Band::Beyond);
        // a coarse location without altitude
        assert_eq!(Band::of(None), Band::Beyond);
    }

    /// Under the pointer or not, rich or compact, from the narrowest window
    /// (a floater is 220 points wide at least, the list beside a sheet 250)
    /// to a wide one, with up to every badge: nothing of the right end
    /// overlaps, it stays in the row, and the names keep their least width
    /// whenever a badge or a "+2" is shown.
    #[test]
    fn the_quick_actions_never_cover_the_badges() {
        let overlap = |a: &Rect, b: &Rect| {
            a.left() < b.right() - 0.01 && b.left() < a.right() - 0.01 && a.top() < b.bottom() - 0.01 && b.top() < a.bottom() - 0.01
        };
        let mut narrowest_cut = false;
        for compact in [false, true] {
            let m = metrics(compact);
            for width in (160..=620).step_by(5) {
                let row = Rect::from_min_size(Pos2::ZERO, Vec2::new(width as f32, if compact { COMPACT_H } else { RICH_H }));
                let left = names_left(row, compact);
                for badges in 0..=MAX_BADGES {
                    for actions in [None, Some(3), Some(4), Some(5)] {
                        let end = right_end(row, left, badges, actions, compact);
                        assert_eq!(
                            end.names_right,
                            right_end(row, left, badges, None, compact).names_right,
                            "hover actions never shorten the name's normal layout"
                        );
                        let mut parts: Vec<Rect> = end.badges.clone();
                        parts.extend(end.more.map(|(r, _)| r));
                        let shown_badges = parts.len();
                        parts.extend(end.seen);
                        parts.extend(end.dist);
                        if let Some((pill, buttons)) = &end.actions {
                            parts.push(*pill);
                            assert_eq!(buttons.len(), actions.unwrap_or(0));
                            assert!(buttons.iter().all(|b| pill.contains_rect(*b)), "buttons in their pill");
                            assert!(buttons.iter().all(|b| b.width() >= 14.0), "icons retain their full width");
                            assert!(end.dist.is_none() && end.seen.is_none());
                            assert_eq!(
                                pill.right(),
                                row.right() - m.pad,
                                "actions stay over the distance at the right edge"
                            );
                            assert!(row.contains_rect(*pill), "actions never need a taller row");
                            assert!(buttons.iter().all(|button| button.center().y == row.center().y));
                        }
                        for (i, a) in parts.iter().enumerate() {
                            assert!(
                                a.left() >= row.left() && a.right() <= row.right(),
                                "{width} {compact}: {a:?} leaves the row"
                            );
                            for b in &parts[i + 1..] {
                                assert!(
                                    !overlap(a, b),
                                    "{width} pt, compact {compact}, {badges} badges, {actions:?}: {a:?} over {b:?}"
                                );
                            }
                        }
                        // every badge is shown or counted in the "+2"
                        let counted = end.badges.len() + end.more.map_or(0, |(_, n)| n);
                        if end.more.is_some() || badges == end.badges.len() {
                            assert_eq!(counted, badges);
                        } else {
                            narrowest_cut = true;
                        }
                        if shown_badges > 0 && actions.is_none() {
                            assert!(end.names_right - left >= m.names_min - 0.01, "the names keep their room");
                        }
                    }
                }
            }
            // a usual window shows all seven badges beside the actions
            let row = Rect::from_min_size(Pos2::ZERO, Vec2::new(500.0, RICH_H));
            if !compact {
                let end = right_end(row, names_left(row, false), MAX_BADGES, Some(4), false);
                assert_eq!((end.badges.len(), end.more.is_none()), (MAX_BADGES, true));
            }
        }
        // only a window too narrow even for the "+2" shows no badge at all
        assert!(narrowest_cut);
    }

    #[test]
    fn ranges_read_in_meters_with_one_decimal() {
        // in the language of the machine: the decimal mark is its own
        assert_eq!(distance_text(Some(4.72), 96.0), format!("{} m", aurora_i18n::number(4.7, 1)));
        assert_eq!(distance_text(None, 96.0), ">96 m");
        assert_eq!(seen_text(12.0), aurora_i18n::text("people-seen-now"));
        assert_eq!(seen_text(125.0), aurora_i18n::tr!("people-seen-minutes", n = 2_i64));
        assert_eq!(seen_text(7300.0), aurora_i18n::tr!("people-seen-hours", n = 2_i64));
    }
}
