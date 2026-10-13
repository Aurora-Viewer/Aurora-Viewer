//! "Près de vous" tab of the People window: Firestorm's radar (columns,
//! sorting, options and names of panel_fs_radar.xml, fspanelradar.cpp,
//! fsradarlistctrl.cpp, fsradarentry.cpp and menu_fs_radar_options.xml,
//! originally LGPL 2.1), drawn the Aurora way.

use super::{PeopleAction, PeopleUi};
use crate::theme::Palette;
use crate::ui::context::{self, AvatarList, CtxAction};
use crate::ui::{menu, widgets};
use crate::world::World;
use crate::world::radar::{CHAT_RANGE, Radar, SHOUT_RANGE};
use aurora_net::profile::flags;
use egui::{Color32, Rect, Sense, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Columns of the list, in their order on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Name,
    Voice,
    Region,
    Typing,
    Sitting,
    Payment,
    Notes,
    Age,
    Seen,
    Range,
}

impl Column {
    const ALL: [Column; 10] = [
        Column::Name,
        Column::Voice,
        Column::Region,
        Column::Typing,
        Column::Sitting,
        Column::Payment,
        Column::Notes,
        Column::Age,
        Column::Seen,
        Column::Range,
    ];

    /// Bit of FSRadarColumnConfig.
    const fn bit(self) -> u32 {
        match self {
            Column::Name => 1,
            Column::Voice => 2,
            Column::Region => 4,
            Column::Typing => 8,
            Column::Sitting => 16,
            Column::Payment => 32,
            Column::Age => 64,
            Column::Seen => 128,
            Column::Range => 256,
            Column::Notes => 512,
        }
    }

    /// The name and the range cannot be hidden.
    const fn optional(self) -> bool {
        !matches!(self, Column::Name | Column::Range)
    }

    /// Width; the name takes what the others leave.
    const fn width(self) -> f32 {
        match self {
            Column::Name => 0.0,
            Column::Voice | Column::Typing | Column::Sitting | Column::Payment | Column::Notes => 20.0,
            Column::Region => 16.0,
            Column::Age => 46.0,
            Column::Seen => 56.0,
            Column::Range => 58.0,
        }
    }

    /// Icon of the header of a narrow column.
    const fn icon(self) -> &'static str {
        match self {
            Column::Voice => "microphone-filled",
            Column::Region => "map-pin-filled",
            Column::Typing => "keyboard-filled",
            Column::Sitting => "armchair",
            Column::Payment => "credit-card-filled",
            Column::Notes => "note",
            _ => "",
        }
    }

    /// Title in the header of a column of figures, short enough for it.
    fn short(self) -> &'static str {
        match self {
            Column::Seen => aurora_i18n::tr!("radar-col-seen-short"),
            Column::Range => aurora_i18n::tr!("radar-col-range-short"),
            _ => self.title(),
        }
    }

    fn title(self) -> &'static str {
        match self {
            Column::Name => aurora_i18n::tr!("common-name"),
            Column::Voice => aurora_i18n::tr!("common-voice"),
            Column::Region => aurora_i18n::tr!("radar-col-region"),
            Column::Typing => aurora_i18n::tr!("radar-col-typing"),
            Column::Sitting => aurora_i18n::tr!("radar-col-sitting"),
            Column::Payment => aurora_i18n::tr!("radar-col-payment"),
            Column::Notes => aurora_i18n::tr!("radar-col-notes"),
            Column::Age => aurora_i18n::tr!("radar-col-age"),
            Column::Seen => aurora_i18n::tr!("radar-col-seen"),
            Column::Range => aurora_i18n::tr!("common-distance"),
        }
    }
}

/// Firestorm settings of the radar.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RadarSettings {
    /// FSRadarColumnConfig: bits of the columns shown.
    pub columns: u32,
    /// ShowRadarMinimap. Off by default, unlike Firestorm (asked by the
    /// human): a first launch opens the mini-map window beside the radar,
    /// which would show the same map twice. The options menu turns it on.
    pub show_minimap: bool,
    /// LimitRadarByRange, with NearMeRange.
    pub limit_range: bool,
    pub range: f32,
    /// FSRadarShowMutedAndDerendered.
    pub show_muted: bool,
    /// RadarNameFormat: 0 display name, 1 username, 2 "Display (username)",
    /// 3 "username (Display)".
    pub name_format: u8,
    /// FSRadarColorNamesByDistance.
    pub color_names: bool,
    /// FSMilkshakeRadarToasts: alerts as notifications instead of chat lines.
    pub toasts: bool,
    /// RadarReportChatRangeEnter / Leave, RadarReportDrawRangeEnter / Leave,
    /// RadarReportSimRangeEnter / Leave.
    pub chat_enter: bool,
    pub chat_leave: bool,
    pub draw_enter: bool,
    pub draw_leave: bool,
    pub region_enter: bool,
    pub region_leave: bool,
    /// RadarAvatarAgeAlert, with RadarAvatarAgeAlertValue (days).
    pub age_alert: bool,
    pub age_alert_days: u32,
    /// Sorted column (index in `Column::ALL`, the order of the enum) and
    /// direction.
    pub sort: u8,
    pub sort_descending: bool,
}

impl Default for RadarSettings {
    fn default() -> Self {
        RadarSettings {
            columns: 1023,
            show_minimap: false,
            limit_range: false,
            range: 162.0,
            show_muted: true,
            name_format: 0,
            color_names: false,
            toasts: false,
            chat_enter: false,
            chat_leave: false,
            draw_enter: false,
            draw_leave: false,
            region_enter: false,
            region_leave: false,
            age_alert: false,
            age_alert_days: 7,
            // nearest first: who is around matters more than the alphabet
            // (Firestorm sorts by name until a header is clicked)
            sort: Column::Range as u8,
            sort_descending: false,
        }
    }
}

impl RadarSettings {
    fn shows(&self, column: Column) -> bool {
        !column.optional() || self.columns & column.bit() != 0
    }

    /// The radar needs the profiles: a column shows them, or the age alert
    /// watches them.
    pub fn wants_profiles(&self) -> bool {
        self.age_alert || [Column::Payment, Column::Notes, Column::Age].into_iter().any(|c| self.shows(c))
    }
}

/// What the tab reads besides the world.
pub struct RadarData<'a> {
    pub radar: &'a Radar,
    pub settings: &'a mut RadarSettings,
    /// Voice participants: level and speaking.
    pub voice: &'a HashMap<Uuid, (f32, bool)>,
    pub draw_distance: f32,
    /// Clock of the radar's sweeps.
    pub now: f64,
    /// Profile pictures the viewer holds.
    pub pics: &'a HashMap<Uuid, egui::TextureHandle>,
}

/// Name of an avatar in the radar and its alerts (FSRadarEntry::getRadarName):
/// with both names, a display name that is only the username is not repeated.
pub fn radar_name(world: &World, id: &Uuid, format: u8) -> String {
    let names = &world.social.avatar_names;
    let Some(name) = names.get(id) else {
        return world.person_name(id).unwrap_or_else(|| "…".into());
    };
    let user = name.user_name_for_display(&names.options);
    let display = &name.display_name;
    match format {
        1 => user,
        2 if !name.is_default => format!("{display} ({user})"),
        3 if !name.is_default => format!("{user} ({display})"),
        2 | 3 => user,
        _ => display.clone(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Payment {
    Unknown,
    None,
    OnFile,
    Used,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Age {
    Unknown,
    Hidden,
    Days(i64),
}

struct Row {
    id: Uuid,
    name: String,
    friend: bool,
    muted: bool,
    /// In the voice channel: speaking or not.
    voice: Option<bool>,
    same_region: bool,
    typing: bool,
    sitting: bool,
    payment: Payment,
    note: String,
    age: Age,
    /// Seconds since the first sighting.
    seen: f64,
    range: Option<f32>,
    loaded: bool,
    position: glam::Vec3,
    /// In a region a teleport left behind: faded until it leaves the list.
    left_behind: bool,
}

/// Age in days of a profile, when it tells it (also for the age alert).
pub fn age_days(profile: &aurora_net::AvatarProfile, unix_now: f64) -> Option<i64> {
    if profile.hide_age == Some(true) {
        return None;
    }
    profile.born.map(|born| ((unix_now - born) / 86_400.0).floor() as i64)
}

fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

fn rows(world: &World, data: &RadarData, filter: &str) -> Vec<Row> {
    let settings = &*data.settings;
    let today = unix_now();
    let mut rows: Vec<Row> = data
        .radar
        .entries()
        .filter_map(|e| {
            let muted = world.mutes.is_muted_id(&e.id);
            if muted && !settings.show_muted {
                return None;
            }
            if settings.limit_range && !e.range.is_some_and(|d| d <= settings.range) {
                return None;
            }
            let name = radar_name(world, &e.id, settings.name_format);
            if !filter.is_empty() && !name.to_lowercase().contains(filter) {
                return None;
            }
            let profile = world.profiles.get(&e.id);
            let payment = match profile {
                None => Payment::Unknown,
                Some(p) if p.flags & flags::TRANSACTED != 0 => Payment::Used,
                Some(p) if p.flags & flags::IDENTIFIED != 0 => Payment::OnFile,
                Some(_) => Payment::None,
            };
            let age = match profile {
                None => Age::Unknown,
                Some(p) if p.hide_age == Some(true) => Age::Hidden,
                Some(p) => age_days(p, today).map_or(Age::Unknown, Age::Days),
            };
            Some(Row {
                id: e.id,
                name,
                friend: world.social.is_friend(&e.id),
                muted,
                // a blocked resident shows no voice state (FSRadarEntry)
                voice: data.voice.get(&e.id).filter(|_| !muted).map(|(_, speaking)| *speaking),
                same_region: e.same_region,
                typing: e.typing,
                sitting: e.sitting,
                payment,
                note: profile.and_then(|p| p.notes.clone()).unwrap_or_default(),
                age,
                seen: (data.now - e.first_seen).max(0.0),
                range: e.range,
                loaded: e.loaded,
                position: e.position,
                left_behind: e.left_behind,
            })
        })
        .collect();
    let column = Column::ALL.get(settings.sort as usize).copied().unwrap_or(Column::Name);
    rows.sort_by(|a, b| {
        use std::cmp::Ordering;
        let by_name = || a.name.to_lowercase().cmp(&b.name.to_lowercase());
        // the lit rows of a flag column come first
        let flag = |x: bool, y: bool| y.cmp(&x);
        let order = match column {
            Column::Name => Ordering::Equal,
            Column::Voice => b.voice.cmp(&a.voice),
            Column::Region => flag(a.same_region, b.same_region),
            Column::Typing => flag(a.typing, b.typing),
            Column::Sitting => flag(a.sitting, b.sitting),
            Column::Payment => b.payment.cmp(&a.payment),
            Column::Notes => flag(!a.note.is_empty(), !b.note.is_empty()),
            Column::Age => {
                let days = |r: &Row| match r.age {
                    Age::Days(d) => d,
                    _ => i64::MAX,
                };
                days(a).cmp(&days(b))
            }
            Column::Seen => a.seen.total_cmp(&b.seen),
            Column::Range => a.range.unwrap_or(f32::MAX).total_cmp(&b.range.unwrap_or(f32::MAX)),
        };
        let order = order.then_with(by_name);
        if settings.sort_descending { order.reverse() } else { order }
    });
    rows
}

/// Color of a range (and of a name colored by distance): within the chat,
/// within a shout, farther.
fn range_color(p: &Palette, range: Option<f32>) -> Color32 {
    match range {
        Some(d) if d <= CHAT_RANGE => p.success,
        Some(d) if d <= SHOUT_RANGE => p.ink,
        _ => p.muted,
    }
}

/// "h:mm:ss" since the first sighting.
fn seen_text(seconds: f64) -> String {
    let s = seconds as u64;
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn icon(ui: &egui::Ui, name: &str, cell: Rect, size: f32, tint: Color32) {
    if let Some(texture) = crate::ui::icons::global(name) {
        let rect = Rect::from_center_size(cell.center(), Vec2::splat(size));
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(texture.id(), rect, uv, tint);
    }
}

/// Write `text` against the right edge of `cell`.
fn text_right(ui: &egui::Ui, text: String, cell: Rect, size: f32, color: Color32, bold: bool) {
    let galley = ui.painter().layout_no_wrap(text, egui::FontId::proportional(size), color);
    let left = egui::pos2(cell.right() - 6.0 - galley.size().x, cell.center().y);
    if bold {
        widgets::paint_ink_left(ui.painter(), galley.clone(), left + widgets::bold_text_offset(ui), color);
    }
    widgets::paint_ink_left(ui.painter(), galley, left, color);
}

/// The cells of the visible columns over `row`, the name taking the rest.
fn cells(settings: &RadarSettings, row: Rect) -> Vec<(Column, Rect)> {
    let visible: Vec<Column> = Column::ALL.into_iter().filter(|c| settings.shows(*c)).collect();
    let fixed: f32 = visible.iter().map(|c| c.width()).sum();
    let name = (row.width() - fixed).max(60.0);
    let mut x = row.left();
    visible
        .into_iter()
        .map(|c| {
            let w = if c == Column::Name { name } else { c.width() };
            let cell = Rect::from_min_max(egui::pos2(x, row.top()), egui::pos2(x + w, row.bottom()));
            x += w;
            (c, cell)
        })
        .collect()
}

fn header(ui: &mut egui::Ui, p: &Palette, settings: &mut RadarSettings, counts: (usize, usize, usize)) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
    for (column, cell) in cells(settings, rect) {
        let index = Column::ALL.iter().position(|c| *c == column).unwrap_or(0) as u8;
        let resp = ui.interact(cell, ui.id().with(("radar_header", index)), Sense::click());
        let sorted = settings.sort == index;
        let color = if sorted || resp.hovered() { p.ink } else { p.muted };
        let font = egui::FontId::proportional(12.0);
        let mut tip = column.title().to_owned();
        match column {
            Column::Name => {
                let (total, region, chat) = counts;
                let text = aurora_i18n::tr!(
                    "radar-name-header",
                    total = format!("{total}"),
                    region = format!("{region}"),
                    chat = format!("{chat}")
                );
                let left = egui::pos2(cell.left() + 8.0, cell.center().y);
                widgets::paint_elided(ui, &text, left, cell.width() - 24.0, font.clone(), color);
                tip = aurora_i18n::tr!("radar-name-header-tip").to_owned();
            }
            Column::Age | Column::Seen | Column::Range => text_right(ui, column.short().to_owned(), cell, 12.0, color, false),
            _ => icon(ui, column.icon(), cell, 14.0, color),
        }
        if sorted {
            // the arrow of the sorted column: after the name, before the
            // title of a column of figures, under an icon
            let title = ui.painter().layout_no_wrap(column.short().to_owned(), font.clone(), color).size().x;
            let arrow = if settings.sort_descending {
                "caret-down-filled"
            } else {
                "caret-up-filled"
            };
            let at = match column {
                Column::Name => egui::pos2(cell.right() - 10.0, cell.center().y),
                Column::Age | Column::Seen | Column::Range => egui::pos2(cell.right() - 6.0 - title - 7.0, cell.center().y),
                _ => egui::pos2(cell.center().x, cell.bottom() - 3.0),
            };
            icon(ui, arrow, Rect::from_center_size(at, Vec2::splat(8.0)), 8.0, p.violet_light);
        }
        if resp.clicked() {
            if sorted {
                settings.sort_descending = !settings.sort_descending;
            } else {
                settings.sort = index;
                settings.sort_descending = false;
            }
        }
        resp.on_hover_text(tip);
    }
}

/// Paint one avatar; returns the text of the cell under the pointer.
fn paint_row(ui: &egui::Ui, p: &Palette, data: &RadarData, row: &Row, rect: Rect, pointer: Option<egui::Pos2>) -> Option<String> {
    let mut tip = None;
    for (column, cell) in cells(data.settings, rect) {
        let under = pointer.is_some_and(|at| cell.contains(at));
        let mut say = |text: String| {
            if under {
                tip = Some(text);
            }
        };
        match column {
            Column::Name => {
                let color = if data.settings.color_names {
                    range_color(p, row.range)
                } else if row.muted {
                    p.muted
                } else {
                    p.ink
                };
                // the profile picture first (not in Firestorm's radar, asked
                // by the human), then the name
                let picture = Rect::from_min_size(egui::pos2(cell.left() + 4.0, cell.center().y - PICTURE / 2.0), Vec2::splat(PICTURE));
                widgets::avatar_picture(ui, p, picture, data.pics.get(&row.id), &row.name, row.muted);
                let cell = cell.with_min_x(picture.right());
                // friends in bold, blocked residents in italics
                let mut job = egui::text::LayoutJob::single_section(
                    row.name.clone(),
                    egui::TextFormat {
                        font_id: egui::FontId::proportional(13.0),
                        color,
                        italics: row.muted,
                        ..Default::default()
                    },
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width((cell.width() - 12.0).max(0.0));
                let galley = ui.painter().layout_job(job);
                if galley.elided {
                    say(row.name.clone());
                }
                let left = egui::pos2(cell.left() + 8.0, cell.center().y);
                if row.friend {
                    widgets::paint_ink_left(ui.painter(), galley.clone(), left + widgets::bold_text_offset(ui), color);
                }
                widgets::paint_ink_left(ui.painter(), galley, left, color);
            }
            Column::Voice => {
                if let Some(speaking) = row.voice {
                    icon(ui, "microphone-filled", cell, 14.0, if speaking { p.success } else { p.ink });
                    say(if speaking {
                        aurora_i18n::tr!("radar-voice-speaking").to_owned()
                    } else {
                        aurora_i18n::tr!("radar-voice-connected").to_owned()
                    });
                }
            }
            Column::Region => {
                if row.same_region {
                    let dot = Rect::from_center_size(cell.center(), Vec2::splat(8.0));
                    ui.painter()
                        .rect_filled(dot, crate::ui::widgets::inner_radius(ui.visuals()), p.violet_light);
                    say(aurora_i18n::tr!("radar-col-region").to_owned());
                }
            }
            Column::Typing => {
                if row.typing {
                    icon(ui, "keyboard-filled", cell, 14.0, p.teal);
                    say(aurora_i18n::tr!("radar-col-typing").to_owned());
                }
            }
            Column::Sitting => {
                if row.sitting {
                    icon(ui, "armchair", cell, 14.0, p.ink);
                    say(aurora_i18n::tr!("radar-col-sitting").to_owned());
                }
            }
            Column::Payment => match row.payment {
                Payment::Unknown => {}
                Payment::None => {
                    icon(ui, "credit-card-off", cell, 14.0, p.muted_dim);
                    say(aurora_i18n::tr!("radar-payment-none").to_owned());
                }
                Payment::OnFile => {
                    icon(ui, "credit-card-filled", cell, 14.0, p.amber);
                    say(aurora_i18n::tr!("radar-payment-on-file").to_owned());
                }
                Payment::Used => {
                    icon(ui, "credit-card-filled", cell, 14.0, p.success);
                    say(aurora_i18n::tr!("radar-payment-used").to_owned());
                }
            },
            Column::Notes => {
                if !row.note.is_empty() {
                    icon(ui, "note", cell, 14.0, p.violet_light);
                    say(row.note.clone());
                }
            }
            Column::Age => match row.age {
                Age::Unknown => {}
                Age::Hidden => text_right(ui, aurora_i18n::tr!("radar-age-hidden").to_owned(), cell, 12.0, p.muted_dim, false),
                Age::Days(days) => {
                    let young = (0..=i64::from(data.settings.age_alert_days)).contains(&days);
                    let color = if young { p.danger } else { p.muted };
                    text_right(ui, format!("{days}"), cell, 12.0, color, young);
                    say(aurora_i18n::tr!("radar-age-days", days = days));
                }
            },
            Column::Seen => text_right(ui, seen_text(row.seen), cell, 12.0, p.muted, false),
            Column::Range => {
                let color = range_color(p, row.range);
                match row.range {
                    // bold while the avatar itself is drawn
                    Some(d) => text_right(
                        ui,
                        aurora_i18n::number(f64::from(d), 2),
                        cell,
                        12.0,
                        color,
                        row.loaded && d <= data.draw_distance,
                    ),
                    None => text_right(
                        ui,
                        format!(">{}", aurora_i18n::number(f64::from(data.draw_distance), 0)),
                        cell,
                        12.0,
                        color,
                        false,
                    ),
                }
            }
        }
    }
    tip
}

/// Side of the buttons beside the filter.
pub(super) const TOOL: f32 = 24.0;
/// How much of a row shows while its avatar is in a region left behind.
const LEFT_BEHIND_OPACITY: f32 = 0.35;
/// Side of the profile picture of a row.
pub(super) const PICTURE: f32 = 18.0;

/// Face of a button of the list's toolbar: the theme's button around an icon.
pub(super) fn tool_button<'a>(p: &Palette, icon: &str, enabled: bool) -> egui::Button<'a> {
    let tint = if enabled { p.ink } else { p.muted_dim };
    let button = match crate::ui::icons::global(icon) {
        Some(texture) => {
            egui::Button::image(egui::Image::from_texture(egui::load::SizedTexture::new(texture.id(), Vec2::splat(14.0))).tint(tint))
        }
        None => egui::Button::new(""),
    };
    button.min_size(Vec2::splat(TOOL))
}

/// A toolbar button opening a menu.
pub(super) fn tool_menu(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, enabled: bool, content: impl FnOnce(&mut egui::Ui)) {
    if !enabled {
        ui.add_enabled(false, tool_button(p, icon, false)).on_disabled_hover_text(tip);
        return;
    }
    let (resp, _) = egui::containers::menu::MenuButton::from_button(tool_button(p, icon, true)).ui(ui, content);
    resp.on_hover_text(tip);
}

/// Options of the list (menu_fs_radar_options.xml). `blocked` asks for the
/// block list.
fn options_menu(ui: &mut egui::Ui, p: &Palette, s: &mut RadarSettings, blocked: &mut bool) {
    menu::toggle(ui, p, "", aurora_i18n::tr!("radar-opt-minimap"), &mut s.show_minimap);
    menu::toggle(ui, p, "", aurora_i18n::tr!("radar-opt-limit-range"), &mut s.limit_range);
    ui.horizontal(|ui| {
        ui.add_space(26.0);
        ui.add_enabled(
            s.limit_range,
            egui::Label::new(egui::RichText::new(aurora_i18n::tr!("radar-opt-range")).size(12.5).color(p.muted)),
        );
        ui.add_enabled(
            s.limit_range,
            egui::DragValue::new(&mut s.range).range(20.0..=4096.0).speed(2.0).suffix(" m"),
        );
    });
    menu::toggle(ui, p, "", aurora_i18n::tr!("radar-opt-show-muted"), &mut s.show_muted);
    menu::submenu(ui, p, "", aurora_i18n::tr!("radar-opt-name-format"), true, |ui| {
        for (format, label) in [
            (0, aurora_i18n::tr!("radar-name-display")),
            (1, aurora_i18n::tr!("radar-name-user")),
            (2, aurora_i18n::tr!("radar-name-display-user")),
            (3, aurora_i18n::tr!("radar-name-user-display")),
        ] {
            if menu::check(ui, p, "", label, s.name_format == format, true) {
                s.name_format = format;
            }
        }
    });
    menu::submenu(ui, p, "", aurora_i18n::tr!("radar-opt-columns"), true, |ui| {
        for column in Column::ALL.into_iter().filter(|c| c.optional()) {
            let mut on = s.columns & column.bit() != 0;
            if menu::toggle(ui, p, column.icon(), column.title(), &mut on) {
                s.columns ^= column.bit();
            }
        }
    });
    menu::toggle(ui, p, "", aurora_i18n::tr!("radar-opt-color-names"), &mut s.color_names);
    menu::separator(ui, p);
    menu::submenu(ui, p, "", aurora_i18n::tr!("radar-opt-report-to"), true, |ui| {
        if menu::check(ui, p, "", aurora_i18n::tr!("radar-report-chat"), !s.toasts, true) {
            s.toasts = false;
        }
        if menu::check(ui, p, "", aurora_i18n::tr!("radar-report-toasts"), s.toasts, true) {
            s.toasts = true;
        }
    });
    menu::toggle(ui, p, "", aurora_i18n::tr!("radar-opt-draw-enter"), &mut s.draw_enter);
    menu::toggle(ui, p, "", aurora_i18n::tr!("radar-opt-draw-leave"), &mut s.draw_leave);
    menu::toggle(ui, p, "", aurora_i18n::tr!("radar-opt-chat-enter"), &mut s.chat_enter);
    menu::toggle(ui, p, "", aurora_i18n::tr!("radar-opt-chat-leave"), &mut s.chat_leave);
    menu::toggle(ui, p, "", aurora_i18n::tr!("radar-opt-region-enter"), &mut s.region_enter);
    menu::toggle(ui, p, "", aurora_i18n::tr!("radar-opt-region-leave"), &mut s.region_leave);
    menu::toggle(ui, p, "", aurora_i18n::tr!("radar-opt-age-alert"), &mut s.age_alert);
    ui.horizontal(|ui| {
        ui.add_space(26.0);
        ui.add_enabled(
            s.age_alert,
            egui::Label::new(
                egui::RichText::new(aurora_i18n::tr!("radar-opt-age-days"))
                    .size(12.5)
                    .color(p.muted),
            ),
        );
        ui.add_enabled(s.age_alert, egui::DragValue::new(&mut s.age_alert_days).range(0..=365).speed(0.2));
    });
    menu::separator(ui, p);
    if menu::item(ui, p, "user-off", aurora_i18n::tr!("radar-opt-show-blocked")) {
        *blocked = true;
    }
}

/// The radar tab. `blocked` is set when the options ask for the block list.
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
    let first = st.radar_selected.first().copied();
    // one line: the filter, then the buttons of the list (its options, the
    // menu of the selection, a friendship offer). Laid out from the right,
    // the filter takes exactly what the buttons leave
    let line = Vec2::new(ui.available_width(), TOOL);
    ui.allocate_ui_with_layout(line, egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let can_befriend = first.is_some_and(|id| !world.social.is_friend(&id));
        let add = ui
            .add_enabled(can_befriend, tool_button(p, "user-plus", can_befriend))
            .on_hover_text(aurora_i18n::tr!("radar-add-friend"));
        if add.clicked()
            && let Some(id) = first
        {
            context::request(ui.ctx(), CtxAction::AddFriend(id));
        }
        tool_menu(
            ui,
            p,
            "settings-filled",
            aurora_i18n::tr!("radar-selection-menu"),
            first.is_some(),
            |ui| {
                if let Some(id) = first {
                    context::avatar_list_menu(ui, p, world, id, AvatarList::Nearby);
                }
            },
        );
        tool_menu(ui, p, "adjustments-filled", aurora_i18n::tr!("radar-options"), true, |ui| {
            options_menu(ui, p, data.settings, blocked);
        });
        widgets::search_field(ui, &mut st.filter, aurora_i18n::tr!("ui-people-filter"), ui.available_width());
    });
    ui.add_space(4.0);
    if data.settings.show_minimap {
        ui.vertical_centered(|ui| map(ui, Vec2::new(ui.available_width().min(320.0), 150.0)));
        ui.add_space(4.0);
    }
    let filter = st.filter.trim().to_lowercase();
    let list = rows(world, data, &filter);
    let counts = (
        data.radar.total,
        list.iter().filter(|r| r.same_region).count(),
        list.iter().filter(|r| r.range.is_some_and(|d| d <= CHAT_RANGE)).count(),
    );
    header(ui, p, data.settings, counts);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        if list.is_empty() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(aurora_i18n::tr!("radar-empty")).size(12.0).color(p.muted));
        }
        let radius = ui.visuals().window_corner_radius;
        for row in &list {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::click());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let selected = st.radar_selected.contains(&row.id);
            if selected {
                ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.30));
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.6));
            }
            if !data.pics.contains_key(&row.id) {
                st.want_pic(row.id, ui.input(|i| i.time));
            }
            let pointer = resp.hover_pos();
            // who stayed in the region we teleported from is on the way out
            let opacity = ui.opacity();
            if row.left_behind {
                ui.set_opacity(opacity * LEFT_BEHIND_OPACITY);
            }
            let tip = paint_row(ui, p, data, row, rect, pointer);
            ui.set_opacity(opacity);
            if resp.clicked() {
                // Ctrl adds to the selection or takes away from it
                if ui.input(|i| i.modifiers.command) {
                    if selected {
                        st.radar_selected.retain(|id| *id != row.id);
                    } else {
                        st.radar_selected.push(row.id);
                    }
                } else {
                    st.radar_selected = vec![row.id];
                }
            }
            if resp.secondary_clicked() && !selected {
                st.radar_selected = vec![row.id];
            }
            // double click: the camera goes to the avatar, when it is drawn
            // (LLAvatarActions::zoomIn)
            if resp.double_clicked() {
                if row.loaded {
                    context::request(ui.ctx(), CtxAction::Zoom(row.position + glam::Vec3::Z * 0.6));
                } else {
                    actions.push(PeopleAction::TooFarToZoom);
                }
            }
            // menu_fs_radar.xml
            menu::context_menu(&resp, p, |ui| context::avatar_list_menu(ui, p, world, row.id, AvatarList::Nearby));
            if let Some(tip) = tip {
                resp.on_hover_text(tip);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_bits_follow_firestorm_and_the_default_shows_them_all() {
        let all: u32 = Column::ALL.into_iter().map(Column::bit).sum();
        assert_eq!(all, 1023);
        let settings = RadarSettings::default();
        assert!(Column::ALL.into_iter().all(|c| settings.shows(c)));
        // the name and the range stay whatever the setting says
        let none = RadarSettings {
            columns: 0,
            ..Default::default()
        };
        let shown: Vec<Column> = Column::ALL.into_iter().filter(|c| none.shows(*c)).collect();
        assert_eq!(shown, vec![Column::Name, Column::Range]);
        assert!(!none.wants_profiles());
        assert!(
            RadarSettings {
                columns: 0,
                age_alert: true,
                ..Default::default()
            }
            .wants_profiles()
        );
    }

    #[test]
    fn the_name_takes_the_width_the_other_columns_leave() {
        let settings = RadarSettings::default();
        let row = Rect::from_min_size(egui::pos2(10.0, 0.0), Vec2::new(400.0, 22.0));
        let cells = cells(&settings, row);
        assert_eq!(cells.len(), 10);
        assert_eq!(cells[0].0, Column::Name);
        assert_eq!(cells[0].1.left(), 10.0);
        let last = cells.last().expect("ten columns");
        assert_eq!(last.0, Column::Range);
        assert!((last.1.right() - 410.0).abs() < 0.01);
        // consecutive, without holes
        for pair in cells.windows(2) {
            assert!((pair[0].1.right() - pair[1].1.left()).abs() < 0.01);
        }
    }

    /// The People window opens wide enough for every column, the name
    /// keeping some eighteen characters (13 px font, ellipsis beyond).
    #[test]
    fn the_default_window_shows_every_column_beside_a_readable_name() {
        // the floater keeps a 10 px margin on each side of the list
        let width = super::super::DEFAULT_SIZE.x - 20.0;
        let row = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(width, 22.0));
        let settings = RadarSettings::default();
        assert!(!settings.show_minimap, "the mini-map window shows the map on a first launch");
        let cells = cells(&settings, row);
        assert_eq!(cells.len(), Column::ALL.len());
        let last = cells.last().expect("ten columns");
        assert!(last.1.right() <= row.right() + 0.01, "the range is not pushed out of the list");
        assert!(cells[0].1.width() >= 140.0, "name column: {}", cells[0].1.width());
    }

    #[test]
    fn seen_time_reads_as_hours_minutes_seconds() {
        assert_eq!(seen_text(0.0), "0:00:00");
        assert_eq!(seen_text(75.9), "0:01:15");
        assert_eq!(seen_text(3_725.0), "1:02:05");
    }

    #[test]
    fn age_in_days_respects_a_hidden_age() {
        let day = 86_400.0;
        let mut profile = aurora_net::AvatarProfile {
            born: Some(10.0 * day),
            ..Default::default()
        };
        assert_eq!(age_days(&profile, 13.5 * day), Some(3));
        profile.hide_age = Some(true);
        assert_eq!(age_days(&profile, 13.5 * day), None);
        profile.hide_age = Some(false);
        profile.born = None;
        assert_eq!(age_days(&profile, 13.5 * day), None);
    }
}
