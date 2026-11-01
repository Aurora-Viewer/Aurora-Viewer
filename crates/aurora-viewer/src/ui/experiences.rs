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
            let labels = [
                (aurora_i18n::tr!("ui-experiences-search"), true),
                (aurora_i18n::tr!("ui-experiences-allowed"), true),
                (aurora_i18n::tr!("ui-experiences-blocked"), true),
                (aurora_i18n::tr!("ui-experiences-admin"), true),
                (aurora_i18n::tr!("ui-experiences-contributor"), true),
                (aurora_i18n::tr!("ui-experiences-owned"), true),
                (aurora_i18n::tr!("ui-experiences-log"), true),
            ];
            widgets::tabs(ui, p, &mut self.tab, &labels);
            ui.add_space(6.0);
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
                    None => muted(ui, p, aurora_i18n::tr!("ui-experiences-loading")),
                    Some(None) => muted(ui, p, aurora_i18n::tr!("ui-experiences-unavailable")),
                    Some(Some(ids)) if ids.is_empty() => muted(ui, p, aurora_i18n::tr!("ui-experiences-none")),
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
                None => muted(ui, p, aurora_i18n::tr!("ui-experiences-searching")),
                Some(None) => muted(ui, p, aurora_i18n::tr!("ui-experiences-unavailable")),
                Some(Some(list)) => {
                    let list = filter_found(list, self.max_rating);
                    if list.is_empty() {
                        muted(ui, p, &aurora_i18n::tr!("ui-experiences-search-none", query = searched));
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
        let footer = 110.0;
        let list_height = (ui.available_height() - footer).clamp(60.0, 600.0);
        let mut open_profile = None;
        egui::ScrollArea::vertical()
            .id_salt("experiences_log")
            .max_height(list_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if page.is_empty() {
                    muted(ui, p, aurora_i18n::tr!("ui-experiences-log-none"));
                }
                for (index, entry) in &page {
                    world.experiences.want(entry.experience);
                    let name = label(&world.experiences, entry.experience);
                    let selected = self.selected == Some(*index);
                    let fill = if selected { p.violet.gamma_multiply(0.35) } else { p.bar };
                    let frame = egui::Frame::new()
                        .fill(fill)
                        .corner_radius(ui.visuals().window_corner_radius)
                        .inner_margin(egui::Margin::same(6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let count = if entry.count > 1 {
                                    format!(" ×{}", entry.count)
                                } else {
                                    String::new()
                                };
                                ui.label(RichText::new(format!("{} {}", entry.day, entry.time)).size(11.5).color(p.muted));
                                ui.label(
                                    RichText::new(format!("{}{count}", experience_log::permission_short(entry.permission)))
                                        .size(12.5)
                                        .color(p.ink),
                                );
                            });
                            ui.label(RichText::new(name).size(12.5).strong().color(p.ink));
                            ui.label(RichText::new(&entry.object_name).size(11.5).color(p.muted));
                        });
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

/// One experience found: name, rating and scope, and "Profile". True when
/// Profile was clicked.
fn found_row(ui: &mut egui::Ui, p: &Palette, e: &aurora_net::land::FoundExperience) -> bool {
    let mut open = false;
    egui::Frame::new()
        .fill(p.bar)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(e.name.trim()).size(13.0).strong().color(p.ink));
                    ui.label(
                        RichText::new(crate::world::worldmap::access_label(e.maturity))
                            .size(11.5)
                            .color(p.muted),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    open = widgets::flat_button(ui, p, aurora_i18n::tr!("ui-group-experiences-profile")).clicked();
                });
            });
        });
    ui.add_space(4.0);
    open
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

/// One experience: its name, rating and scope, and "Profile". True when
/// Profile was clicked.
fn row(ui: &mut egui::Ui, p: &Palette, world: &mut World, id: Uuid) -> bool {
    world.experiences.want(id);
    let mut open = false;
    egui::Frame::new()
        .fill(p.bar)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(label(&world.experiences, id)).size(13.0).strong().color(p.ink));
                    if let Some(d) = world.experiences.details.get(&id).filter(|d| !d.invalid()) {
                        let scope = if d.grid_wide() {
                            aurora_i18n::tr!("ui-group-experiences-grid")
                        } else {
                            aurora_i18n::tr!("ui-group-experiences-land")
                        };
                        let rating = crate::world::worldmap::access_label(d.maturity);
                        ui.label(RichText::new(format!("{rating} · {scope}")).size(11.5).color(p.muted));
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("ui-group-experiences-profile")).clicked() {
                        open = true;
                    }
                });
            });
        });
    ui.add_space(4.0);
    open
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
