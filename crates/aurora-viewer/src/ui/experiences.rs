//! "Experiences" (Me menu): the lists of the agent's experiences, one tab
//! each for those it allowed, blocked, administers, contributes to and
//! owns, with the experience profile one click away. Port of
//! LLFloaterExperiences and LLPanelExperiences (indra/newview/
//! llfloaterexperiences.cpp, llpanelexperiences.cpp, originally LGPL 2.1):
//! opening the window asks every list again (onOpen / refreshContents), the
//! rows are sorted by name without regard to case (LLExperienceItemComparator)
//! and a permission changed in the profile moves the experience between the
//! Allowed and Blocked tabs (updatePermissions). The Search tab is
//! LLPanelExperiencePicker (llpanelexperiencepicker.cpp) and the Events tab
//! LLPanelExperienceLog (llpanelexperiencelog.cpp). The acquire button and
//! the search's next / previous page are not ported yet.

use super::widgets::{self, Floater};
use crate::theme::Palette;
use crate::world::World;
use crate::world::experience_log;
use crate::world::experiences::Experiences;
use aurora_net::experience::ExperienceListKind;
use egui::{RichText, Vec2};
use uuid::Uuid;

pub const WINDOW: &str = "experiences";

/// The tabs, in the order of LLFloaterExperiences::postBuild.
const TABS: [ExperienceListKind; 5] = [
    ExperienceListKind::Allowed,
    ExperienceListKind::Blocked,
    ExperienceListKind::Admin,
    ExperienceListKind::Contributor,
    ExperienceListKind::Owned,
];

/// Tab 0 is the search, 1..=5 the lists of `TABS`, 6 the event log.
const SEARCH_TAB: usize = 0;
const LOG_TAB: usize = TABS.len() + 1;

/// The search's content ratings, as the picker's combo (SIM_ACCESS_*).
const RATINGS: [u8; 3] = [13, 21, 42];

pub struct ExperiencesUi {
    tab: usize,
    /// The window was open on the previous frame (the lists are asked again
    /// each time it opens).
    shown: bool,
    query: String,
    /// The query the results on screen answer.
    searched: Option<String>,
    max_rating: u8,
    page: usize,
    /// The selected event: its index in the log.
    selected: Option<usize>,
}

impl Default for ExperiencesUi {
    fn default() -> Self {
        ExperiencesUi {
            tab: 1,
            shown: false,
            query: String::new(),
            searched: None,
            max_rating: 42,
            page: 0,
            selected: None,
        }
    }
}

impl ExperiencesUi {
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &mut World, open: &mut bool) {
        if !*open {
            self.shown = false;
            return;
        }
        if !std::mem::replace(&mut self.shown, true) {
            world.experiences.request_lists();
        }
        let screen = ctx.content_rect();
        Floater::new(
            WINDOW,
            aurora_i18n::tr!("ui-experiences-title"),
            egui::pos2(screen.center().x - 260.0, screen.center().y - 200.0),
            Vec2::new(520.0, 400.0),
        )
        .show(ctx, p, open, |ui| {
            let names = [
                aurora_i18n::tr!("ui-experiences-allowed"),
                aurora_i18n::tr!("ui-experiences-blocked"),
                aurora_i18n::tr!("ui-experiences-admin"),
                aurora_i18n::tr!("ui-experiences-contributor"),
                aurora_i18n::tr!("ui-experiences-owned"),
            ];
            let mut titles = vec![aurora_i18n::tr!("ui-experiences-search").to_owned()];
            for (kind, name) in TABS.iter().zip(names) {
                titles.push(match world.experiences.lists.get(kind) {
                    Some(Some(ids)) if !ids.is_empty() => format!("{name} ({})", ids.len()),
                    _ => name.to_owned(),
                });
            }
            titles.push(aurora_i18n::tr!("ui-experiences-log").to_owned());
            let labels: Vec<(&str, bool)> = titles.iter().map(|t| (t.as_str(), true)).collect();
            widgets::tabs(ui, p, &mut self.tab, &labels);
            ui.add_space(8.0);
            match self.tab {
                SEARCH_TAB => return self.search(ui, p, world),
                LOG_TAB => return self.log(ui, p, world),
                _ => {}
            }
            let kind = TABS[(self.tab - 1).min(TABS.len() - 1)];
            let mut open_profile = None;
            egui::ScrollArea::vertical()
                .id_salt("experiences_list")
                .auto_shrink([false, false])
                .show(ui, |ui| match world.experiences.lists.get(&kind).cloned() {
                    None => empty(ui, p, "flask-filled", aurora_i18n::tr!("ui-experiences-loading")),
                    Some(None) => empty(ui, p, "flask-filled", aurora_i18n::tr!("ui-experiences-unavailable")),
                    Some(Some(ids)) if ids.is_empty() => empty(ui, p, "flask-filled", aurora_i18n::tr!("ui-experiences-none")),
                    Some(Some(ids)) => {
                        for id in sorted(&ids, &world.experiences) {
                            if row(ui, p, world, id) {
                                open_profile = Some(id);
                            }
                        }
                    }
                });
            if let Some(id) = open_profile {
                world.experiences.open_profile(id);
            }
        });
    }
}

impl ExperiencesUi {
    /// LLPanelExperiencePicker: a name, Go, the highest rating, and the
    /// experiences found (rows of the same kind as the lists').
    fn search(&mut self, ui: &mut egui::Ui, p: &Palette, world: &mut World) {
        let mut go = false;
        ui.horizontal(|ui| {
            let field = widgets::search_field(ui, &mut self.query, aurora_i18n::tr!("ui-experiences-search-hint"), 260.0);
            go |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            go |= widgets::flat_button(ui, p, aurora_i18n::tr!("ui-experiences-search-go")).clicked();
        });
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-experiences-search-rating"))
                    .size(12.0)
                    .color(p.muted),
            );
            for rating in RATINGS {
                widgets::radio_value(ui, p, &mut self.max_rating, rating, crate::world::worldmap::access_label(rating));
            }
        });
        if go {
            let query = self.query.trim().to_owned();
            world.land.search_experiences(&query);
            self.searched = Some(query);
        }
        ui.add_space(6.0);
        let Some(searched) = self.searched.clone() else {
            return;
        };
        let found = match world.land.experience_search.as_ref() {
            Some((q, found)) if *q == searched => found.clone(),
            _ => None,
        };
        let mut open_profile = None;
        egui::ScrollArea::vertical()
            .id_salt("experiences_search")
            .auto_shrink([false, false])
            .show(ui, |ui| match found {
                None => empty(ui, p, "search-filled", aurora_i18n::tr!("ui-experiences-searching")),
                Some(None) => empty(ui, p, "search-filled", aurora_i18n::tr!("ui-experiences-unavailable")),
                Some(Some(list)) => {
                    let list = filter_found(list, self.max_rating);
                    if list.is_empty() {
                        empty(
                            ui,
                            p,
                            "search-filled",
                            &aurora_i18n::tr!("ui-experiences-search-none", query = searched),
                        );
                    }
                    for e in list {
                        if found_row(ui, p, &e) {
                            open_profile = Some(e.id);
                        }
                    }
                }
            });
        if let Some(id) = open_profile {
            world.experiences.open_profile(id);
        }
    }

    /// LLPanelExperienceLog: the events newest first, a page at a time.
    fn log(&mut self, ui: &mut egui::Ui, p: &Palette, world: &mut World) {
        let total = world.experiences.log.events.len();
        let pages = world.experiences.log.pages();
        self.page = self.page.min(pages - 1);
        let size = world.experiences.log.page_size.max(1);
        let page: Vec<(usize, experience_log::LogEntry)> = world
            .experiences
            .log
            .events
            .iter()
            .cloned()
            .enumerate()
            .rev()
            .skip(self.page * size)
            .take(size)
            .collect();
        // The footer's height as laid out on the previous frame: reserving a
        // guess either grows the window each frame or leaves a gap.
        let footer_id = ui.make_persistent_id("xp_log_footer");
        let footer: f32 = ui.data(|d| d.get_temp(footer_id)).unwrap_or(70.0);
        let list_height = (ui.available_height() - footer - ui.spacing().item_spacing.y).max(60.0);
        let mut open_profile = None;
        egui::ScrollArea::vertical()
            .id_salt("experiences_log")
            .max_height(list_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if page.is_empty() {
                    empty(ui, p, "history", aurora_i18n::tr!("ui-experiences-log-none"));
                }
                for (index, entry) in &page {
                    world.experiences.want(entry.experience);
                    let name = label(&world.experiences, entry.experience);
                    let selected = self.selected == Some(*index);
                    let fill = if selected { p.violet.gamma_multiply(0.35) } else { p.bar };
                    let frame = egui::Frame::new()
                        .fill(fill)
                        .corner_radius(ui.visuals().window_corner_radius)
                        .inner_margin(egui::Margin::same(8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                tile(ui, p, permission_icon(entry.permission));
                                ui.vertical(|ui| {
                                    let count = if entry.count > 1 {
                                        format!(" ×{}", entry.count)
                                    } else {
                                        String::new()
                                    };
                                    ui.label(
                                        RichText::new(format!("{}{count}", experience_log::permission_short(entry.permission)))
                                            .size(13.0)
                                            .strong()
                                            .color(p.ink),
                                    );
                                    ui.label(RichText::new(format!("{name} · {}", entry.object_name)).size(11.5).color(p.muted));
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                    ui.label(RichText::new(format!("{} {}", entry.day, entry.time)).size(11.5).color(p.muted));
                                });
                            });
                        });
                    hover_tint(ui, p, frame.response.rect, selected);
                    let click = ui.interact(
                        frame.response.rect,
                        ui.make_persistent_id(("xp_log_row", index)),
                        egui::Sense::click(),
                    );
                    if click.clicked() {
                        self.selected = Some(*index);
                    }
                    if click.double_clicked() {
                        open_profile = Some(entry.experience);
                    }
                    ui.add_space(3.0);
                }
            });
        let footer_top = ui.cursor().min.y;
        widgets::divider(ui, p);
        let entry = self.selected.and_then(|i| world.experiences.log.events.get(i).cloned());
        ui.horizontal(|ui| {
            if widgets::flat_button(ui, p, "‹").clicked() && self.page > 0 {
                self.page -= 1;
            }
            ui.label(RichText::new(format!("{} / {pages}", self.page + 1)).size(12.0).color(p.muted));
            if widgets::flat_button(ui, p, "›").clicked() && self.page + 1 < pages {
                self.page += 1;
            }
            ui.add_enabled_ui(entry.is_some(), |ui| {
                if widgets::flat_button(ui, p, aurora_i18n::tr!("ui-experiences-log-notify")).clicked()
                    && let Some(entry) = entry.as_ref()
                {
                    world.notify_experience_event(entry);
                }
                if widgets::flat_button(ui, p, aurora_i18n::tr!("ui-group-experiences-profile")).clicked()
                    && let Some(entry) = entry.as_ref()
                {
                    open_profile = Some(entry.experience);
                }
                if widgets::flat_button(ui, p, aurora_i18n::tr!("ui-experiences-log-report")).clicked()
                    && let Some(entry) = entry.as_ref()
                {
                    crate::ui::context::request(
                        ui.ctx(),
                        crate::ui::context::CtxAction::ReportAbuse(crate::report_abuse::ReportTarget::Experience(entry.experience)),
                    );
                }
            });
            ui.add_enabled_ui(total > 0, |ui| {
                if widgets::flat_button(ui, p, aurora_i18n::tr!("ui-experiences-log-clear")).clicked() {
                    world.experiences.log.clear();
                    self.selected = None;
                    self.page = 0;
                }
            });
        });
        ui.horizontal(|ui| {
            let log = &mut world.experiences.log;
            let mut changed = widgets::checkbox(ui, p, &mut log.notify, aurora_i18n::tr!("ui-experiences-log-notify-all")).changed();
            ui.label(RichText::new(aurora_i18n::tr!("ui-experiences-log-days")).size(12.0).color(p.muted));
            changed |= ui.add(egui::DragValue::new(&mut log.max_days).range(1..=365)).changed();
            if changed {
                log.save();
            }
        });
        let measured = ui.min_rect().bottom() - footer_top;
        ui.data_mut(|d| d.insert_temp(footer_id, measured));
        if let Some(id) = open_profile {
            world.experiences.open_profile(id);
        }
    }
}

/// LLPanelExperiencePicker::FilterOverRating, then the picker's sort by
/// name.
fn filter_found(mut list: Vec<aurora_net::land::FoundExperience>, max_rating: u8) -> Vec<aurora_net::land::FoundExperience> {
    list.retain(|e| e.maturity <= max_rating);
    list.sort_by_key(|e| e.name.to_uppercase());
    list
}

/// One experience found: name and rating; the card opens the profile. True
/// when it was clicked.
fn found_row(ui: &mut egui::Ui, p: &Palette, e: &aurora_net::land::FoundExperience) -> bool {
    card(ui, p, ("xp_found", e.id), e.name.trim(), e.maturity, None)
}

fn muted(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text).size(12.5).color(p.muted));
}

/// What the row shows for an experience: its name once known, else its id.
fn label(xp: &Experiences, id: Uuid) -> String {
    match xp.details.get(&id) {
        Some(d) if !d.invalid() && !d.name.trim().is_empty() => d.name.trim().to_owned(),
        Some(_) => id.to_string(),
        None => aurora_i18n::tr!("common-loading-2").to_owned(),
    }
}

/// LLExperienceItemComparator: by name, upper-cased.
fn sorted(ids: &[Uuid], xp: &Experiences) -> Vec<Uuid> {
    let mut keyed: Vec<(String, Uuid)> = ids.iter().map(|id| (label(xp, *id).to_uppercase(), *id)).collect();
    keyed.sort();
    keyed.into_iter().map(|(_, id)| id).collect()
}

/// One experience: its name, rating and scope; the card opens the profile.
/// True when it was clicked.
fn row(ui: &mut egui::Ui, p: &Palette, world: &mut World, id: Uuid) -> bool {
    world.experiences.want(id);
    let name = label(&world.experiences, id);
    match world.experiences.details.get(&id).filter(|d| !d.invalid()) {
        Some(d) => {
            let scope = if d.grid_wide() {
                aurora_i18n::tr!("ui-group-experiences-grid")
            } else {
                aurora_i18n::tr!("ui-group-experiences-land")
            };
            card(ui, p, ("xp_row", id), &name, d.maturity, Some(scope))
        }
        None => card(ui, p, ("xp_row", id), &name, 0, None),
    }
}

/// A clickable card: an icon tile, the name, the rating badge (when known)
/// and the scope, and a chevron. True when clicked.
fn card(ui: &mut egui::Ui, p: &Palette, id: (&str, Uuid), name: &str, maturity: u8, scope: Option<&str>) -> bool {
    let frame = egui::Frame::new()
        .fill(p.bar)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                tile(ui, p, "flask-filled");
                ui.vertical(|ui| {
                    ui.label(RichText::new(name).size(13.0).strong().color(p.ink));
                    ui.horizontal(|ui| {
                        if maturity != 0 {
                            widgets::maturity_badge_of(ui, p, maturity, 14.0);
                        }
                        if let Some(scope) = scope {
                            ui.label(RichText::new(scope).size(11.5).color(p.muted));
                        }
                    });
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
                    paint_icon(ui, "chevron-right", rect, p.muted);
                });
            });
        });
    let rect = frame.response.rect;
    let click = ui.interact(rect, ui.make_persistent_id(id), egui::Sense::click());
    hover_tint(ui, p, rect, false);
    ui.add_space(4.0);
    click.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A rounded square holding an icon, the leading mark of a row.
fn tile(ui: &mut egui::Ui, p: &Palette, icon: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(34.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.violet.gamma_multiply(0.22));
    paint_icon(ui, icon, rect.shrink(8.0), p.violet);
}

fn paint_icon(ui: &egui::Ui, icon: &str, rect: egui::Rect, tint: egui::Color32) {
    if let Some(t) = super::icons::global(icon) {
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(t.id(), rect, uv, tint);
    }
}

/// A light wash over a row the pointer is on.
fn hover_tint(ui: &egui::Ui, p: &Palette, rect: egui::Rect, selected: bool) {
    if !selected && ui.rect_contains_pointer(rect) {
        ui.painter()
            .rect_filled(rect, ui.visuals().window_corner_radius, p.violet.gamma_multiply(0.12));
    }
}

/// What a list says when it has nothing to show: a large icon and a line.
fn empty(ui: &mut egui::Ui, p: &Palette, icon: &str, text: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(36.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), egui::Sense::hover());
        paint_icon(ui, icon, rect, p.muted.gamma_multiply(0.6));
        ui.add_space(6.0);
        muted(ui, p, text);
    });
}

/// The icon of an event, by the permission it records.
fn permission_icon(permission: i32) -> &'static str {
    match permission {
        1 => "hand-move",
        3 => "run",
        4 => "paperclip",
        9 => "video-filled",
        10 => "camera-filled",
        11 => "door-enter",
        12 => "shield-filled",
        16 => "armchair",
        17 => "sunset-2-filled",
        _ => "help-circle-filled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::experience::ExperienceDetails;

    #[test]
    fn found_experiences_are_capped_by_rating_and_sorted_by_name() {
        let found = |name: &str, maturity| aurora_net::land::FoundExperience {
            id: Uuid::nil(),
            name: name.into(),
            owner: Uuid::nil(),
            maturity,
            properties: 0,
        };
        let list = filter_found(
            vec![found("zed", 13), found("Adult club", 42), found("beta", 21), found("Alpha", 13)],
            21,
        );
        let names: Vec<_> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "beta", "zed"]);
    }

    #[test]
    fn rows_are_sorted_by_name_without_regard_to_case() {
        let mut xp = Experiences::default();
        let (a, b, c) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        for (id, name) in [(a, "zebra"), (b, "Apple"), (c, "mango")] {
            xp.details.insert(
                id,
                ExperienceDetails {
                    id,
                    name: name.into(),
                    ..Default::default()
                },
            );
        }
        assert_eq!(sorted(&[a, b, c], &xp), vec![b, c, a]);
    }
}
