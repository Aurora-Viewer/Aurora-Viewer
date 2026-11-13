//! Aurora's Destinations window (Preferences › Aurora › Destinations): the
//! Destination Guide's places drawn by the viewer, from the feed the guide's
//! page reads (`destination_guide.rs`). It opens compact (search, category
//! chips, G / M / A, order, one list whose rows open in place) and widens
//! into a rail of categories, a grid of cards and a preview, like Aurora's
//! Places.
//!
//! Every action is one the guide's page or the viewer already offers: a
//! teleport goes the way of the page's `secondlife:///app/teleport/` links
//! (the web view's `clicked_link_action`, with the teleport confirmation),
//! the map, About Land and the SLurl as for any place link. A landmark is
//! made where the agent stands (LLFloaterCreateLandmark, no place of one's
//! choice in the protocol): its two buttons wait until the agent is in the
//! place's region.
//!
//! The lists are built again only when the guide, the filters, the order or
//! the teleport history change; a frame only draws them, and asks for the
//! pictures of the rows and cards on screen.

use crate::destination_guide::{self as guide, Category, DestinationGuide, Load, Place};
use crate::theme::Palette;
use crate::ui::context::{CtxAction, request};
use crate::ui::menu;
use crate::ui::place_card::{self, Extras, Row, RowSize, Thumb};
use crate::ui::widgets::{self, ActionKind};
use crate::world::tphistory::HistoryItem;
use aurora_net::search::AgentAccess;
use egui::{Color32, FontId, Pos2, Rect, RichText, Sense, Vec2};
use std::collections::HashSet;
use std::time::Instant;

/// The mockup's compact window, and its widened one.
pub const COMPACT_SIZE: Vec2 = Vec2::new(440.0, 680.0);
pub const WIDE_SIZE: Vec2 = Vec2::new(1120.0, 720.0);
pub const MIN_SIZE: Vec2 = Vec2::new(360.0, 420.0);
/// From this body width the window lays out its rail, cards and preview.
pub const WIDE: f32 = 860.0;
const RAIL_W: f32 = 200.0;
const PANEL_W: f32 = 340.0;
const TOOLBAR_H: f32 = 52.0;
const COLUMNS: usize = 3;
const CARD_GAP: f32 = 10.0;
const SAME_CATEGORY: usize = 3;

/// What the window needs of the viewer.
pub struct Env<'a> {
    pub history: &'a [HistoryItem],
    pub access: AgentAccess,
    /// The agent's region, None before it has one.
    pub here: Option<&'a str>,
}

/// What the app does for the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// A landmark where the agent stands, in Favorites when `favorite`.
    Landmark { favorite: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    #[default]
    Population,
    Name,
    Newest,
}

impl Sort {
    fn label(self) -> &'static str {
        match self {
            Sort::Population => aurora_i18n::tr!("destinations-sort-population"),
            Sort::Name => aurora_i18n::tr!("destinations-sort-name"),
            Sort::Newest => aurora_i18n::tr!("destinations-sort-newest"),
        }
    }
}

/// A place of the list, with what its row shows made once.
struct Item {
    /// `categories[cat].places[idx]`.
    cat: usize,
    idx: usize,
    /// The category it is shown under: its own, also for What's Hot Now
    /// and the search results.
    home: usize,
    visited: bool,
    people: String,
    /// "Until 14 Nov", for a listing that ends.
    until: Option<String>,
    /// The third line of a compact row: the end date, a past visit, or both.
    third: Option<(&'static str, String, bool)>,
    /// "Region · 128, 96, 23".
    location: String,
    /// "Category · Moderate".
    kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Key {
    generation: u64,
    category: usize,
    ratings: [bool; 3],
    sort: Sort,
    hide_visited: bool,
    visited: (usize, u64),
    language: &'static str,
}

struct Cache {
    key: Key,
    filter: String,
    /// By category: its name, and its places the ratings let through.
    labels: Vec<String>,
    counts: Vec<usize>,
    /// The categories that have places, in the feed's order.
    shown: Vec<usize>,
    items: Vec<Item>,
    count_line: String,
}

/// The window's state; the guide's data is `DestinationGuide`'s.
pub struct AuroraState {
    filter: String,
    /// The chosen category's key; None: What's Hot Now, else the first.
    category: Option<String>,
    /// G, M, A as toggled; a rating above the account's never counts.
    ratings: [bool; 3],
    sort: Sort,
    hide_visited: bool,
    /// The wide window's previewed place and the compact list's opened
    /// row, by slug.
    selected: Option<String>,
    opened: Option<String>,
    cache: Option<Cache>,
    visited: ((usize, u64), HashSet<String>),
    footer: (Option<u64>, Load, &'static str, String),
    people_line: (u32, &'static str, String),
    /// The demo opens a row (the first one visited) once the guide came.
    pub demo_open: bool,
}

impl Default for AuroraState {
    fn default() -> Self {
        Self {
            filter: String::new(),
            category: None,
            // LLFloaterSearch's defaults: General and Moderate
            ratings: [true, true, false],
            sort: Sort::Population,
            hide_visited: false,
            selected: None,
            opened: None,
            cache: None,
            visited: ((usize::MAX, 0), HashSet::new()),
            footer: (None, Load::Idle, "", String::new()),
            people_line: (u32::MAX, "", String::new()),
            demo_open: false,
        }
    }
}

/// The ratings that count: as toggled, never above the account's access.
fn effective(ratings: [bool; 3], access: AgentAccess) -> [bool; 3] {
    [
        ratings[0],
        ratings[1] && access.can_access_mature(),
        ratings[2] && access.can_access_adult(),
    ]
}

fn rating_ok(rating: u8, on: [bool; 3]) -> bool {
    match rating {
        42 => on[2],
        21 => on[1],
        _ => on[0],
    }
}

/// The category a place is shown under: the first of the server's own
/// that lists it, else the one it was found in.
fn home_of(categories: &[Category], found: usize, slug: &str) -> usize {
    if categories[found].key != guide::HOT {
        return found;
    }
    categories
        .iter()
        .position(|c| c.key != guide::HOT && c.places.iter().any(|p| p.slug == slug))
        .unwrap_or(found)
}

/// The regions of the teleport history, from its "Parcel, Region" titles.
fn visited_regions(history: &[HistoryItem]) -> HashSet<String> {
    history
        .iter()
        .map(|h| {
            h.title
                .rsplit_once(", ")
                .map_or(h.title.as_str(), |(_, region)| region)
                .trim()
                .to_lowercase()
        })
        .filter(|r| !r.is_empty())
        .collect()
}

fn build(categories: &[Category], key: Key, filter: &str, visited: &HashSet<String>) -> Cache {
    let on = key.ratings;
    let labels: Vec<String> = categories
        .iter()
        .map(|c| {
            if c.key == guide::HOT {
                aurora_i18n::tr!("destinations-hot").to_owned()
            } else {
                c.name.clone()
            }
        })
        .collect();
    let counts: Vec<usize> = categories
        .iter()
        .map(|c| c.places.iter().filter(|p| rating_ok(p.rating, on)).count())
        .collect();
    let shown: Vec<usize> = (0..categories.len()).filter(|&i| !categories[i].places.is_empty()).collect();
    let needle = filter.trim().to_lowercase();
    let mut found: Vec<(usize, usize)> = Vec::new();
    if needle.is_empty() {
        if let Some(c) = categories.get(key.category) {
            found.extend((0..c.places.len()).map(|i| (key.category, i)));
        }
    } else {
        // the search covers the whole guide, each place once
        let mut seen = HashSet::new();
        for (ci, c) in categories.iter().enumerate() {
            for (pi, p) in c.places.iter().enumerate() {
                let hit = p.name.to_lowercase().contains(&needle)
                    || p.region.to_lowercase().contains(&needle)
                    || p.description.to_lowercase().contains(&needle);
                if hit && seen.insert(p.slug.as_str()) {
                    found.push((ci, pi));
                }
            }
        }
    }
    let place = |&(c, i): &(usize, usize)| &categories[c].places[i];
    found.retain(|f| rating_ok(place(f).rating, on));
    let is_visited = |p: &Place| visited.contains(&p.region.to_lowercase());
    if key.hide_visited {
        found.retain(|f| !is_visited(place(f)));
    }
    match key.sort {
        Sort::Population => found.sort_by(|a, b| {
            place(b)
                .population
                .cmp(&place(a).population)
                .then_with(|| place(a).name.to_lowercase().cmp(&place(b).name.to_lowercase()))
        }),
        Sort::Name => found.sort_by_cached_key(|f| place(f).name.to_lowercase()),
        Sort::Newest => found.sort_by(|a, b| place(b).published.cmp(&place(a).published)),
    }
    let visited_label = aurora_i18n::tr!("destinations-visited");
    let items: Vec<Item> = found
        .iter()
        .map(|&(cat, idx)| {
            let p = &categories[cat].places[idx];
            let home = home_of(categories, cat, &p.slug);
            let visited = is_visited(p);
            let until = p
                .until
                .map(|(_, m, d)| aurora_i18n::tr!("destinations-until", date = crate::ui::land::day_month(None, m as usize, d as i64)));
            let third = match (&until, visited) {
                (Some(u), true) => Some(("calendar-event-filled", format!("{u} · {visited_label}"), true)),
                (Some(u), false) => Some(("calendar-event-filled", u.clone(), true)),
                (None, true) => Some(("history", visited_label.to_owned(), false)),
                (None, false) => None,
            };
            let rating = crate::ui::bars::maturity_name(p.rating);
            Item {
                cat,
                idx,
                home,
                visited,
                people: p.population.to_string(),
                until,
                third,
                location: format!("{} · {}, {}, {}", p.region, p.pos.x.round(), p.pos.y.round(), p.pos.z.round()),
                kind: format!("{} · {rating}", labels[home]),
            }
        })
        .collect();
    let category = if needle.is_empty() {
        labels.get(key.category).cloned().unwrap_or_default()
    } else {
        aurora_i18n::tr!("destinations-results").to_owned()
    };
    let count_line = aurora_i18n::tr!("destinations-count", category = category, count = items.len() as i64);
    Cache {
        key,
        filter: filter.to_owned(),
        labels,
        counts,
        shown,
        items,
        count_line,
    }
}

/// The guide's link of `place`, read as the web view reads it.
fn teleport(ctx: &egui::Context, place: &Place) {
    if let Some(link) = crate::slurl::place_link(&place.teleport_link()) {
        request(ctx, CtxAction::TeleportToPlace(link.location.target(), link.location.pos));
    }
}

fn show_on_map(ctx: &egui::Context, place: &Place) {
    request(ctx, CtxAction::ShowPlace(place.region.clone(), place.pos));
}

fn about_land(ctx: &egui::Context, place: &Place) {
    request(ctx, CtxAction::ShowPlaceInfo(place.region.clone(), place.pos));
}

fn copy_slurl(ctx: &egui::Context, place: &Place) {
    ctx.copy_text(crate::slurl::make(&place.region, place.pos));
}

/// Whether a landmark made now would be of `place`: the agent is there.
fn here(env: &Env, place: &Place) -> bool {
    env.here.is_some_and(|r| r.eq_ignore_ascii_case(&place.region))
}

fn small_label(ui: &mut egui::Ui, text: &str, size: f32, color: Color32) {
    ui.label(RichText::new(text).size(size).color(color));
}

impl AuroraState {
    /// The window's body, in a floater whose body reaches its edges.
    pub fn body(&mut self, ui: &mut egui::Ui, p: &Palette, guide: &mut DestinationGuide, env: &Env) -> Option<Action> {
        let wide = ui.max_rect().width() >= WIDE;
        self.prepare(guide, env, wide);
        let mut action = None;
        let (mut list, foot) = widgets::above_footer(ui);
        if wide {
            self.wide(&mut list, p, guide, env, &mut action);
        } else {
            self.compact(&mut list, p, guide, env, &mut action);
        }
        foot.show(ui, |ui| self.footer(ui, p, guide));
        action
    }

    /// The lists, built again when what they show changed.
    fn prepare(&mut self, guide: &DestinationGuide, env: &Env, wide: bool) {
        let stamp = (env.history.len(), env.history.last().map_or(0, |h| h.date.to_bits()));
        if self.visited.0 != stamp {
            self.visited = (stamp, visited_regions(env.history));
        }
        let categories = &guide.categories;
        // the chosen category, else What's Hot Now, else the first with places
        let chosen = self
            .category
            .as_deref()
            .and_then(|k| categories.iter().position(|c| c.key == k && !c.places.is_empty()))
            .or_else(|| categories.iter().position(|c| !c.places.is_empty()))
            .unwrap_or(0);
        let key = Key {
            generation: guide.generation,
            category: chosen,
            ratings: effective(self.ratings, env.access),
            sort: self.sort,
            hide_visited: self.hide_visited && wide,
            visited: stamp,
            language: aurora_i18n::language(),
        };
        let fresh = self.cache.as_ref().is_some_and(|c| c.key == key && c.filter == self.filter);
        if !fresh {
            self.cache = Some(build(categories, key, &self.filter, &self.visited.1));
        }
        if self.demo_open
            && let Some(cache) = &self.cache
            && let Some(item) = cache.items.iter().find(|i| i.visited).or(cache.items.first())
        {
            self.demo_open = false;
            let slug = categories[item.cat].places[item.idx].slug.clone();
            self.opened = Some(slug.clone());
            self.selected = Some(slug);
        }
    }

    fn select_category(&mut self, categories: &[Category], index: usize) {
        if let Some(c) = categories.get(index) {
            self.category = Some(c.key.clone());
            self.opened = None;
            self.selected = None;
        }
    }

    /// Nothing to list yet: loading, failed, or filtered out.
    fn placeholder(&self, ui: &mut egui::Ui, p: &Palette, guide: &DestinationGuide, empty: bool) -> bool {
        let text = if guide.categories.is_empty() {
            match guide.load {
                Load::Failed => aurora_i18n::tr!("destinations-failed"),
                _ => aurora_i18n::tr!("destinations-loading"),
            }
        } else if empty {
            aurora_i18n::tr!("destinations-empty")
        } else {
            return false;
        };
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            if guide.categories.is_empty() && guide.load != Load::Failed {
                ui.add(egui::Spinner::new().color(p.violet_light).size(18.0));
                ui.add_space(8.0);
            }
            ui.add(egui::Label::new(RichText::new(text).size(12.5).color(p.muted)).wrap());
        });
        true
    }

    fn footer(&mut self, ui: &mut egui::Ui, p: &Palette, guide: &mut DestinationGuide) {
        let minutes = guide.updated.map(|at| at.elapsed().as_secs() / 60);
        let language = aurora_i18n::language();
        if (self.footer.0, &self.footer.1, self.footer.2) != (minutes, &guide.load, language) || self.footer.3.is_empty() {
            let text = match minutes {
                Some(0) => aurora_i18n::tr!("destinations-updated-now").to_owned(),
                Some(m) => aurora_i18n::tr!("destinations-updated", minutes = m as i64),
                None if guide.load == Load::Failed => aurora_i18n::tr!("destinations-failed").to_owned(),
                None => aurora_i18n::tr!("destinations-loading").to_owned(),
            };
            self.footer = (minutes, guide.load.clone(), language, text);
        }
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::hover());
        ui.painter().hline(rect.x_range(), rect.top(), widgets::divider_stroke(p));
        let button = Rect::from_center_size(Pos2::new(rect.right() - 20.0, rect.center().y), Vec2::splat(24.0));
        widgets::paint_elided(
            ui,
            &self.footer.3,
            Pos2::new(rect.left() + 12.0, rect.center().y),
            (button.left() - 8.0 - rect.left() - 12.0).max(20.0),
            FontId::proportional(11.5),
            p.muted_dim,
        );
        let mut b = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(button)
                .layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight)),
        );
        if widgets::icon_button(&mut b, p, "refresh", aurora_i18n::tr!("destinations-refresh"), !guide.busy()) {
            guide.refresh(Instant::now());
        }
    }

    // ------------------------------------------------------------ compact

    fn compact(&mut self, ui: &mut egui::Ui, p: &Palette, guide: &mut DestinationGuide, env: &Env, action: &mut Option<Action>) {
        let Some(cache) = self.cache.take() else { return };
        let full = ui.max_rect();
        let mut ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(full.shrink2(Vec2::new(10.0, 10.0)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        ui.set_clip_rect(full.intersect(ui.clip_rect()));
        let ui = &mut ui;
        let w = ui.available_width();
        ui.horizontal(|ui| widgets::clearable_search_box(ui, p, &mut self.filter, aurora_i18n::tr!("destinations-search"), w));
        ui.add_space(8.0);
        // the categories, in chips
        let mut pick = None;
        egui::ScrollArea::horizontal()
            .id_salt("destinations-chips")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for &ci in &cache.shown {
                        if widgets::filter_pill(ui, p, "", &cache.labels[ci], ci == cache.key.category).clicked() {
                            pick = Some(ci);
                        }
                    }
                });
            });
        ui.add_space(6.0);
        // maturity and order
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            small_label(ui, aurora_i18n::tr!("destinations-maturity"), 12.0, p.muted);
            ui.add_space(2.0);
            self.rating_toggles(ui, p, env.access, false);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| self.sort_button(ui, p));
        });
        ui.add_space(4.0);
        small_label(ui, &cache.count_line, 11.5, p.muted_dim);
        ui.add_space(4.0);
        if !self.placeholder(ui, p, guide, cache.items.is_empty()) {
            egui::ScrollArea::vertical()
                .id_salt("destinations-compact")
                .auto_shrink([false, false])
                .show(ui, |ui| self.rows(ui, p, guide, env, &cache, action));
        }
        if let Some(ci) = pick {
            self.select_category(&guide.categories, ci);
        }
        self.cache = Some(cache);
    }

    fn rating_toggles(&mut self, ui: &mut egui::Ui, p: &Palette, access: AgentAccess, label: bool) {
        // a rating the account may not enter is not offered at all
        let allowed = [true, access.can_access_mature(), access.can_access_adult()];
        let names = [
            aurora_i18n::tr!("common-general"),
            aurora_i18n::tr!("common-moderate"),
            aurora_i18n::tr!("common-adult"),
        ];
        for i in 0..3 {
            if allowed[i] {
                widgets::pills::rating_toggle(ui, p, &mut self.ratings[i], names[i], true, label);
            }
        }
    }

    fn sort_button(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let resp = widgets::action_menu_button(ui, p, "arrows-sort", self.sort.label(), false, 28.0);
        menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            for (by, icon) in [
                (Sort::Population, "users"),
                (Sort::Name, "sort-ascending-2-filled"),
                (Sort::Newest, "calendar-filled"),
            ] {
                if menu::check(ui, p, icon, by.label(), self.sort == by, true) {
                    self.sort = by;
                }
            }
        });
    }

    fn rows(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        guide: &mut DestinationGuide,
        env: &Env,
        cache: &Cache,
        action: &mut Option<Action>,
    ) {
        ui.spacing_mut().item_spacing.y = 2.0;
        let categories = &guide.categories;
        let pictures = &mut guide.pictures;
        let mut toggle = None;
        for item in &cache.items {
            let place = &categories[item.cat].places[item.idx];
            let opened = self.opened.as_deref() == Some(place.slug.as_str());
            // only the rows on screen ask for their picture
            let probe = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), RowSize::DESTINATIONS.height));
            let tex = if ui.is_rect_visible(probe) {
                place.card.as_deref().and_then(|u| pictures.texture(u, guide::CARD_SIDE))
            } else {
                None
            };
            let row = Row {
                thumb: Thumb::Picture(tex.as_ref()),
                title: &place.name,
                subtitle: &cache.labels[item.home],
                rating: None,
                selected: opened,
            };
            let extras = Extras {
                over_picture: Some(("user-filled", &item.people)),
                badge: Some(crate::ui::bars::maturity_name(place.rating)),
                third: item
                    .third
                    .as_ref()
                    .map(|(icon, text, date)| (*icon, text.as_str(), if *date { p.violet_pale } else { p.muted_dim })),
            };
            let mut go = false;
            let resp = place_card::row_with(ui, p, RowSize::DESTINATIONS, &row, &extras, |ui| {
                go = widgets::action_button(ui, p, "location-filled", "", ActionKind::Flat, Vec2::splat(30.0))
                    .on_hover_text(aurora_i18n::tr!("common-teleport-3"))
                    .clicked();
            });
            if go || resp.double_clicked() {
                teleport(ui.ctx(), place);
            } else if resp.clicked() {
                toggle = Some(item);
            }
            if opened {
                details(ui, p, place, item, env, action);
            }
        }
        if let Some(item) = toggle {
            let slug = &categories[item.cat].places[item.idx].slug;
            self.opened = if self.opened.as_ref() == Some(slug) {
                None
            } else {
                Some(slug.clone())
            };
        }
    }

    // ---------------------------------------------------------------- wide

    fn wide(&mut self, ui: &mut egui::Ui, p: &Palette, guide: &mut DestinationGuide, env: &Env, action: &mut Option<Action>) {
        let Some(cache) = self.cache.take() else { return };
        let full = ui.max_rect();
        let stroke = widgets::divider_stroke(p);
        // the toolbar: search, maturity, visited, order
        let bar = Rect::from_min_size(full.min, Vec2::new(full.width(), TOOLBAR_H));
        ui.painter().hline(bar.x_range(), bar.bottom(), stroke);
        let inner = bar.shrink2(Vec2::new(14.0, 10.0));
        let mut right = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        right.spacing_mut().item_spacing.x = 8.0;
        self.sort_button(&mut right, p);
        let hide = aurora_i18n::tr!("destinations-hide-visited");
        if widgets::filter_pill(&mut right, p, "history", hide, self.hide_visited).clicked() {
            self.hide_visited = !self.hide_visited;
        }
        // the toggles, right to left: the last one first
        let mut row = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    inner.min,
                    Pos2::new(right.min_rect().left() - 10.0, inner.bottom()),
                ))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        row.spacing_mut().item_spacing.x = 4.0;
        let allowed = [true, env.access.can_access_mature(), env.access.can_access_adult()];
        let names = [
            aurora_i18n::tr!("common-general"),
            aurora_i18n::tr!("common-moderate"),
            aurora_i18n::tr!("common-adult"),
        ];
        for i in (0..3).rev() {
            if allowed[i] {
                widgets::pills::rating_toggle(&mut row, p, &mut self.ratings[i], names[i], true, true);
            }
        }
        row.add_space(2.0);
        small_label(&mut row, aurora_i18n::tr!("destinations-maturity"), 12.0, p.muted);
        let search_right = (row.min_rect().left() - 14.0).max(inner.left() + 80.0);
        let mut left = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(inner.min, Pos2::new(search_right, inner.bottom())))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let w = left.available_width();
        widgets::clearable_search_box(&mut left, p, &mut self.filter, aurora_i18n::tr!("destinations-search"), w);

        let body = Rect::from_min_max(Pos2::new(full.left(), bar.bottom()), full.max);
        let rail = Rect::from_min_size(body.min, Vec2::new(RAIL_W, body.height()));
        let panel = Rect::from_min_max(Pos2::new(body.right() - PANEL_W, body.top()), body.max);
        let centre = Rect::from_min_max(Pos2::new(rail.right(), body.top()), Pos2::new(panel.left(), body.bottom()));
        ui.painter().rect_filled(rail, 0.0, p.field.gamma_multiply(0.55));
        ui.painter().vline(rail.right(), rail.y_range(), stroke);
        ui.painter().rect_filled(panel, 0.0, p.field.gamma_multiply(0.35));
        ui.painter().vline(panel.left(), panel.y_range(), stroke);

        // the rail: the categories and their counts
        let mut pick = None;
        let mut r = ui.new_child(egui::UiBuilder::new().max_rect(rail.shrink2(Vec2::new(8.0, 10.0))));
        r.set_clip_rect(rail);
        egui::ScrollArea::vertical()
            .id_salt("destinations-rail")
            .auto_shrink([false, false])
            .show(&mut r, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for &ci in &cache.shown {
                    let hot = guide.categories[ci].key == guide::HOT;
                    let (icon, tint) = if hot {
                        ("sparkles-filled", p.amber)
                    } else {
                        ("map-pin-filled", p.muted)
                    };
                    let on = ci == cache.key.category && self.filter.trim().is_empty();
                    if widgets::rail_row(ui, p, icon, tint, &cache.labels[ci], Some(cache.counts[ci]), on, 0).clicked() {
                        pick = Some(ci);
                    }
                }
            });

        // the cards
        let selected = self
            .selected
            .as_deref()
            .and_then(|s| cache.items.iter().position(|i| guide.categories[i.cat].places[i.idx].slug == s))
            .or(if cache.items.is_empty() { None } else { Some(0) });
        let mut chosen = None;
        let mut c = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(centre.shrink2(Vec2::new(12.0, 10.0)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        c.set_clip_rect(centre);
        c.label(
            RichText::new(&cache.count_line)
                .font(FontId::new(14.0, crate::ui::fonts::semibold(c.ctx())))
                .color(p.ink),
        );
        c.add_space(8.0);
        if !self.placeholder(&mut c, p, guide, cache.items.is_empty()) {
            chosen = cards(&mut c, p, guide, &cache, selected);
        }

        // the preview
        let mut v = ui.new_child(egui::UiBuilder::new().max_rect(panel.shrink(14.0)));
        v.set_clip_rect(panel);
        let mut same = None;
        if let Some(i) = selected {
            egui::ScrollArea::vertical()
                .id_salt("destinations-preview")
                .auto_shrink([false, false])
                .show(&mut v, |ui| same = self.preview(ui, p, guide, env, &cache, &cache.items[i], action));
        }
        if let Some(i) = chosen {
            let it = &cache.items[i];
            self.selected = Some(guide.categories[it.cat].places[it.idx].slug.clone());
        }
        if let Some(slug) = same {
            self.selected = Some(slug);
        }
        if let Some(ci) = pick {
            self.filter.clear();
            self.select_category(&guide.categories, ci);
        }
        self.cache = Some(cache);
    }

    /// The previewed place; the slug of a place of the same category clicked.
    #[allow(clippy::too_many_arguments)]
    fn preview(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        guide: &mut DestinationGuide,
        env: &Env,
        cache: &Cache,
        item: &Item,
        action: &mut Option<Action>,
    ) -> Option<String> {
        let categories = &guide.categories;
        let pictures = &mut guide.pictures;
        let place = &categories[item.cat].places[item.idx];
        let width = ui.available_width();
        // the large photo, the card's picture until it came
        let photo = place.photo.as_deref().and_then(|u| pictures.texture(u, guide::PHOTO_SIDE));
        let tex = photo.or_else(|| place.card.as_deref().and_then(|u| pictures.texture(u, guide::CARD_SIDE)));
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 190.0), Sense::hover());
        place_card::picture(ui, p, tex.as_ref(), rect, 40.0);
        let language = aurora_i18n::language();
        if (self.people_line.0, self.people_line.1) != (place.population, language) {
            let line = aurora_i18n::tr!("destinations-people", count = place.population as i64);
            self.people_line = (place.population, language, line);
        }
        place_card::picture_pill(
            ui,
            p,
            Pos2::new(rect.left() + 10.0, rect.bottom() - 10.0),
            "user-filled",
            &self.people_line.2,
            12.0,
        );
        ui.add_space(10.0);
        small_label(ui, &item.kind, 11.5, p.muted);
        ui.add(
            egui::Label::new(
                RichText::new(&place.name)
                    .font(FontId::new(19.0, crate::ui::fonts::semibold(ui.ctx())))
                    .color(p.ink),
            )
            .wrap(),
        );
        if let Some(until) = &item.until {
            small_label(ui, until, 12.0, p.violet_pale);
        }
        ui.add_space(4.0);
        if !place.description.is_empty() {
            ui.add(egui::Label::new(RichText::new(&place.description).size(12.5).color(p.ink.gamma_multiply(0.86))).wrap());
            ui.add_space(4.0);
        }
        small_label(ui, &item.location, 11.5, p.muted);
        ui.add_space(8.0);
        if widgets::action_button(
            ui,
            p,
            "location-filled",
            aurora_i18n::tr!("common-teleport-3"),
            ActionKind::Primary,
            Vec2::new(width, 36.0),
        )
        .clicked()
        {
            teleport(ui.ctx(), place);
        }
        ui.add_space(4.0);
        let half = Vec2::new(((width - 6.0) / 2.0).floor(), 30.0);
        let here_ok = here(env, place);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if widgets::action_button(ui, p, "map", aurora_i18n::tr!("common-show-on-map"), ActionKind::Flat, half).clicked() {
                show_on_map(ui.ctx(), place);
            }
            let lm = widgets::action_button_if(
                ui,
                p,
                "bookmark-filled",
                aurora_i18n::tr!("ui-places-create-landmark"),
                ActionKind::Flat,
                half,
                here_ok,
            )
            .on_disabled_hover_text(aurora_i18n::tr!("destinations-here-only"));
            if lm.clicked() {
                *action = Some(Action::Landmark { favorite: false });
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let fav = widgets::action_button_if(
                ui,
                p,
                "star-filled",
                aurora_i18n::tr!("common-add-to-favorites"),
                ActionKind::Flat,
                half,
                here_ok,
            )
            .on_disabled_hover_text(aurora_i18n::tr!("destinations-here-only"));
            if fav.clicked() {
                *action = Some(Action::Landmark { favorite: true });
            }
            if widgets::action_button(ui, p, "link-filled", aurora_i18n::tr!("common-copy-slurl"), ActionKind::Flat, half).clicked() {
                copy_slurl(ui.ctx(), place);
            }
        });
        ui.add_space(4.0);
        if ui
            .add(egui::Link::new(
                RichText::new(aurora_i18n::tr!("common-about-land"))
                    .size(12.0)
                    .color(p.violet_light),
            ))
            .clicked()
        {
            about_land(ui.ctx(), place);
        }
        ui.add_space(6.0);
        widgets::divider(ui, p);
        ui.add_space(4.0);
        small_label(ui, aurora_i18n::tr!("destinations-same-category"), 11.5, p.muted);
        // the most visited others of its category, the ratings applied
        let on = cache.key.ratings;
        let mut picked = None;
        let mut shown = 0;
        for other in categories[item.home]
            .places
            .iter()
            .filter(|o| o.slug != place.slug && rating_ok(o.rating, on))
            .take(SAME_CATEGORY)
        {
            shown += 1;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::click());
            if resp.hovered() {
                ui.painter()
                    .rect_filled(rect, ui.visuals().window_corner_radius, p.raised.gamma_multiply(0.6));
            }
            let thumb = Rect::from_min_size(Pos2::new(rect.left() + 4.0, rect.center().y - 14.0), Vec2::new(44.0, 28.0));
            let tex = if ui.is_rect_visible(rect) {
                other.card.as_deref().and_then(|u| pictures.texture(u, guide::CARD_SIDE))
            } else {
                None
            };
            place_card::picture(ui, p, tex.as_ref(), thumb, 14.0);
            let count = other.population.to_string();
            let count_w = ui
                .painter()
                .layout_no_wrap(count.clone(), FontId::proportional(11.0), p.muted_dim)
                .size()
                .x;
            widgets::paint_elided(
                ui,
                &count,
                Pos2::new(rect.right() - 6.0 - count_w, rect.center().y),
                count_w + 1.0,
                FontId::proportional(11.0),
                p.muted_dim,
            );
            let name_left = thumb.right() + 8.0;
            let cut = widgets::paint_elided(
                ui,
                &other.name,
                Pos2::new(name_left, rect.center().y),
                (rect.right() - 14.0 - count_w - name_left).max(20.0),
                FontId::proportional(12.0),
                p.ink,
            );
            let resp = if cut { resp.on_hover_text(&other.name) } else { resp };
            if resp.clicked() {
                picked = Some(other.slug.clone());
            }
        }
        if shown == 0 {
            small_label(ui, aurora_i18n::tr!("destinations-no-same"), 12.0, p.muted_dim);
        }
        picked
    }
}

/// The grid of cards, three a row, only the rows on screen drawn; the
/// index of a card clicked.
fn cards(ui: &mut egui::Ui, p: &Palette, guide: &mut DestinationGuide, cache: &Cache, selected: Option<usize>) -> Option<usize> {
    let width = ui.available_width();
    let card_w = ((width - CARD_GAP * (COLUMNS - 1) as f32) / COLUMNS as f32).floor().max(60.0);
    let pic_h = (card_w * 10.0 / 16.0).round();
    let card_h = pic_h + 44.0;
    let rows = cache.items.len().div_ceil(COLUMNS);
    let categories = &guide.categories;
    let pictures = &mut guide.pictures;
    let mut clicked = None;
    ui.spacing_mut().item_spacing.y = 12.0;
    egui::ScrollArea::vertical()
        .id_salt("destinations-cards")
        .auto_shrink([false, false])
        .show_rows(ui, card_h, rows, |ui, range| {
            for row in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = CARD_GAP;
                    for n in row * COLUMNS..((row + 1) * COLUMNS).min(cache.items.len()) {
                        let item = &cache.items[n];
                        let place = &categories[item.cat].places[item.idx];
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(card_w, card_h), Sense::click());
                        let tex = place.card.as_deref().and_then(|u| pictures.texture(u, guide::CARD_SIDE));
                        card(
                            ui,
                            p,
                            rect,
                            &resp,
                            place,
                            item,
                            &cache.labels[item.home],
                            tex.as_ref(),
                            selected == Some(n),
                        );
                        if resp.clicked() {
                            clicked = Some(n);
                        }
                        if resp.double_clicked() {
                            teleport(ui.ctx(), place);
                        }
                    }
                });
            }
        });
    clicked
}

#[allow(clippy::too_many_arguments)]
fn card(
    ui: &egui::Ui,
    p: &Palette,
    rect: Rect,
    resp: &egui::Response,
    place: &Place,
    item: &Item,
    category: &str,
    tex: Option<&egui::TextureHandle>,
    selected: bool,
) {
    let pic = Rect::from_min_size(rect.min, Vec2::new(rect.width(), (rect.width() * 10.0 / 16.0).round()));
    place_card::picture(ui, p, tex, pic, 28.0);
    let radius = ui.visuals().window_corner_radius;
    if selected {
        ui.painter()
            .rect_stroke(pic, radius, egui::Stroke::new(2.0, p.violet), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter()
            .rect_stroke(pic, radius, egui::Stroke::new(1.5, p.violet_light), egui::StrokeKind::Inside);
    }
    let badge = Rect::from_min_size(pic.min + Vec2::splat(6.0), Vec2::splat(18.0));
    widgets::paint_maturity_badge(
        ui.painter(),
        p,
        crate::ui::bars::maturity_name(place.rating),
        badge,
        widgets::inner_radius(ui.visuals()),
    );
    place_card::picture_pill(
        ui,
        p,
        Pos2::new(pic.left() + 6.0, pic.bottom() - 6.0),
        "user-filled",
        &item.people,
        11.0,
    );
    if item.visited {
        let text = aurora_i18n::tr!("destinations-visited");
        let w = place_card::picture_pill_width(ui, "", text, 10.0);
        place_card::picture_pill(ui, p, Pos2::new(pic.right() - 6.0 - w, pic.bottom() - 6.0), "", text, 10.0);
    }
    let name_y = pic.bottom() + 14.0;
    let cut = widgets::paint_elided(
        ui,
        &place.name,
        Pos2::new(rect.left() + 1.0, name_y),
        rect.width() - 2.0,
        FontId::new(13.0, crate::ui::fonts::semibold(ui.ctx())),
        p.ink,
    );
    let (sub, color) = match &item.until {
        Some(u) => (u.as_str(), p.violet_pale),
        None => (category, p.muted),
    };
    widgets::paint_elided(
        ui,
        sub,
        Pos2::new(rect.left() + 1.0, name_y + 18.0),
        rect.width() - 2.0,
        FontId::proportional(11.5),
        color,
    );
    if cut {
        resp.clone().on_hover_text(&place.name);
    }
}

/// A row opened in place: its description, where it is, and its actions.
fn details(ui: &mut egui::Ui, p: &Palette, place: &Place, item: &Item, env: &Env, action: &mut Option<Action>) {
    let ctx = ui.ctx().clone();
    egui::Frame::NONE
        .inner_margin(egui::Margin {
            left: 10,
            right: 10,
            top: 2,
            bottom: 10,
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            if !place.description.is_empty() {
                ui.add(egui::Label::new(RichText::new(&place.description).size(12.0).color(p.ink.gamma_multiply(0.86))).wrap());
            }
            small_label(ui, &item.location, 11.5, p.muted);
            let here_ok = here(env, place);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let side = 30.0;
                let tp_w = (ui.available_width() - 4.0 * (side + 6.0)).max(90.0);
                if widgets::action_button(
                    ui,
                    p,
                    "location-filled",
                    aurora_i18n::tr!("common-teleport-3"),
                    ActionKind::Primary,
                    Vec2::new(tp_w, side),
                )
                .clicked()
                {
                    teleport(&ctx, place);
                }
                if widgets::action_button(ui, p, "map", "", ActionKind::Flat, Vec2::splat(side))
                    .on_hover_text(aurora_i18n::tr!("common-show-on-map"))
                    .clicked()
                {
                    show_on_map(&ctx, place);
                }
                let lm = widgets::action_button_if(ui, p, "bookmark-filled", "", ActionKind::Flat, Vec2::splat(side), here_ok)
                    .on_hover_text(aurora_i18n::tr!("ui-places-create-landmark"))
                    .on_disabled_hover_text(aurora_i18n::tr!("destinations-here-only"));
                if lm.clicked() {
                    *action = Some(Action::Landmark { favorite: false });
                }
                let fav = widgets::action_button_if(ui, p, "star-filled", "", ActionKind::Flat, Vec2::splat(side), here_ok)
                    .on_hover_text(aurora_i18n::tr!("common-add-to-favorites"))
                    .on_disabled_hover_text(aurora_i18n::tr!("destinations-here-only"));
                if fav.clicked() {
                    *action = Some(Action::Landmark { favorite: true });
                }
                let more = widgets::action_button(ui, p, "dots-filled", "", ActionKind::Flat, Vec2::splat(side))
                    .on_hover_text(aurora_i18n::tr!("ui-places-more"));
                menu::dropdown(&more, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    if menu::item(ui, p, "link-filled", aurora_i18n::tr!("common-copy-slurl")) {
                        copy_slurl(&ctx, place);
                    }
                    if menu::item(ui, p, "info-circle-filled", aurora_i18n::tr!("common-about-land")) {
                        about_land(&ctx, place);
                    }
                });
            });
        });
}

#[cfg(test)]
mod tests;
