//! "Places" window, as Firestorm shows it by default (LLFloaterSidePanelContainer
//! "places" with LLPanelPlaces; FSUseStandalonePlaceDetailsFloater off).
//! Port of LLPanelPlaces, LLLandmarksPanel / LLFavoritesPanel and
//! LLTeleportHistoryPanel (indra/newview/llpanelplaces.cpp,
//! llpanellandmarks.cpp, llpanelteleporthistory.cpp, floater_places.xml,
//! panel_places.xml, panel_landmarks.xml, panel_favorites.xml,
//! panel_teleport_history.xml and their menus, originally LGPL 2.1):
//! a filter, the ⚙ / sort / + / trash buttons, three tabs (Favorites: the
//! Favorites folder, Landmarks: the Landmarks folder tree, Teleport
//! history: the saved teleport history in day sections), and at the bottom
//! Teleport / Map / Profile (FIRE-31033). The profile of a place link,
//! a history entry or a landmark replaces the tabs, with a back arrow
//! (`place_details.rs` draws it). Double-click teleports, after the
//! TeleportFromLandmark / TeleportToHistoryEntry confirmations.
//!
//! Aurora's own look of the window (the default) is `places/aurora.rs`, on
//! the data of `places/model.rs`; Preferences › Aurora › Places brings these
//! tabs back.

pub mod aurora;
pub(crate) mod drag;
pub mod model;

use super::place_details::{self, Ctx};
use super::widgets::{self, Floater};
use crate::theme::Palette;
use crate::world::World;
use crate::world::inventory::{FetchState, Inventory};
use crate::world::landmarks::Resolved;
use crate::world::place_details::Source;
use crate::world::tphistory::{self, HistoryItem};
use egui::{RichText, Sense, Vec2};
use glam::DVec3;
use std::collections::HashSet;
use uuid::Uuid;

pub const TAB_FAVORITES: usize = 0;
pub const TAB_LANDMARKS: usize = 1;
pub const TAB_HISTORY: usize = 2;

/// FT_FAVORITE / FT_LANDMARK folder types.
const FOLDER_FAVORITES: i32 = 23;
pub(crate) const FOLDER_LANDMARKS: i32 = 3;
/// AT_LANDMARK.
const ASSET_LANDMARK: i32 = 3;

/// AURORA_DEMO_PLACES and the like: a tab by its name or index (the former
/// French names are still accepted).
pub fn tab_from_name(name: &str) -> Option<usize> {
    match name.trim().to_lowercase().as_str() {
        "favorites" | "favoris" | "0" => Some(TAB_FAVORITES),
        "landmarks" | "reperes" | "repères" | "1" => Some(TAB_LANDMARKS),
        "history" | "historique" | "2" => Some(TAB_HISTORY),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Sel {
    Item(Uuid),
    Folder(Uuid),
    /// Index in `World::tp_storage.items`.
    History(usize),
}

/// A confirmation in progress (notifications.xml).
enum Confirm {
    /// TeleportFromLandmark.
    Landmark { asset: Uuid, name: String },
    /// TeleportToHistoryEntry.
    History { global: DVec3, title: String },
    /// ConfirmClearTeleportHistory.
    Clear,
}

pub enum PlacesAction {
    /// A place link or a history entry from its profile (no confirmation),
    /// or a confirmed history entry (LLTeleportHistoryStorage::goToItem).
    Teleport(DVec3),
    /// A confirmed landmark (teleport_via_landmark).
    TeleportLandmark(Uuid),
    ShowOnMap(DVec3),
    /// Show a profile (LLPanelPlaces::onOpen with its key).
    OpenProfile(Source),
    /// The back arrow.
    CloseProfile,
    RemoveHistory(usize),
    /// Confirmed "Clear teleport history".
    ClearHistory,
    /// "Sort by date" (LandmarksSortedByDate).
    SortByDate(bool),
    /// "Do not show again" ticked in an accepted teleport confirmation.
    StopConfirmingTeleports,
    /// "Add landmark" (LLLandmarksPanel::onAddAction "add_landmark"): the
    /// Create Landmark window, saving in the selected folder if any.
    CreateLandmark(Option<Uuid>),
    /// Aurora's look: an inventory change (a star, a folder, notes, a name,
    /// a deletion), sent as the inventory window sends its own.
    Edit(aurora_net::inventory::operations::Mutation),
    /// A favourite was starred: its row is marked as just added.
    ExpectFavorite,
    /// A favourite dropped before the one at that rank of the bar.
    MoveFavorite(Uuid, usize),
    /// "Place profile": the standalone window of FSFloaterPlaceDetails.
    OpenWindow(Source),
    /// The order of Aurora's lists.
    Sort(model::PlacesSort),
}

/// One line of a landmarks tab.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub depth: usize,
    pub id: Uuid,
    pub name: String,
    /// None: a landmark; Some(open): a folder.
    pub folder: Option<bool>,
}

/// The Favorites or Landmarks folder of our inventory.
pub(crate) fn system_folder(inv: &Inventory, kind: i32) -> Option<Uuid> {
    inv.folders
        .values()
        .find(|f| !f.library && f.info.type_default == kind)
        .map(|f| f.info.id)
}

fn matches(name: &str, filter: &str) -> bool {
    filter.is_empty() || name.to_lowercase().contains(&filter.to_lowercase())
}

/// Does a folder hold a landmark the filter keeps (FIRE-31051: folders
/// without one are hidden while filtering)?
fn has_match(inv: &Inventory, folder: Uuid, filter: &str, depth: usize) -> bool {
    let Some(f) = inv.folders.get(&folder) else {
        return false;
    };
    depth < 32
        && (f
            .items
            .iter()
            .filter_map(|i| inv.items.get(i))
            .any(|i| i.asset_type == ASSET_LANDMARK && matches(&i.name, filter))
            || f.children.iter().any(|c| has_match(inv, *c, filter, depth + 1)))
}

/// The lines of a landmarks tab (LLPlacesInventoryPanel limited to
/// landmarks): sub-folders by name, then landmarks newest first when sorted
/// by date (SO_DATE) or by name. Without a filter every folder shows
/// (SHOW_ALL_FOLDERS) in the Landmarks tab, only those holding landmarks in
/// Favorites; with one, only folders holding a match, opened.
pub fn landmark_rows(inv: &Inventory, root: Uuid, filter: &str, by_date: bool, open: &HashSet<Uuid>, all_folders: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    add_rows(inv, root, filter, by_date, open, all_folders, 0, &mut rows);
    rows
}

#[allow(clippy::too_many_arguments)]
fn add_rows(
    inv: &Inventory,
    folder: Uuid,
    filter: &str,
    by_date: bool,
    open: &HashSet<Uuid>,
    all: bool,
    depth: usize,
    rows: &mut Vec<Row>,
) {
    let Some(f) = inv.folders.get(&folder) else {
        return;
    };
    if depth > 32 {
        return;
    }
    let mut kids: Vec<_> = f.children.iter().filter_map(|c| inv.folders.get(c)).collect();
    kids.sort_by_key(|k| k.info.name.to_lowercase());
    for k in kids {
        let keep = if filter.is_empty() {
            all || has_match(inv, k.info.id, "", 0)
        } else {
            has_match(inv, k.info.id, filter, 0)
        };
        if !keep {
            continue;
        }
        let is_open = !filter.is_empty() || open.contains(&k.info.id);
        rows.push(Row {
            depth,
            id: k.info.id,
            name: k.info.name.clone(),
            folder: Some(is_open),
        });
        if is_open {
            add_rows(inv, k.info.id, filter, by_date, open, all, depth + 1, rows);
        }
    }
    let mut items: Vec<_> = f
        .items
        .iter()
        .filter_map(|i| inv.items.get(i))
        .filter(|i| i.asset_type == ASSET_LANDMARK && matches(&i.name, filter))
        .collect();
    if by_date {
        items.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
    } else {
        items.sort_by_key(|i| i.name.to_lowercase());
    }
    rows.extend(items.into_iter().map(|i| Row {
        depth,
        id: i.id,
        name: i.name.clone(),
        folder: None,
    }));
}

/// The history in its sections (LLTeleportHistoryPanel::refresh): newest
/// first, filtered by title; empty sections are left out.
pub fn history_sections(items: &[HistoryItem], filter: &str, now: f64) -> Vec<(usize, Vec<usize>)> {
    let mut out: Vec<(usize, Vec<usize>)> = Vec::new();
    for (i, it) in items.iter().enumerate().rev() {
        if !tphistory::history_matches(&it.title, filter) {
            continue;
        }
        let s = tphistory::history_section(it.date, now);
        match out.last_mut() {
            Some((last, v)) if *last == s => v.push(i),
            _ => out.push((s, vec![i])),
        }
    }
    out
}

/// The region of a SLURL for a global position: the map's, else the region
/// part of a history title ("Parcel, Region").
fn slurl_at(world: &World, global: DVec3, title: &str) -> Option<String> {
    if world.rlva.has("showloc") {
        return None;
    }
    let region = world
        .map
        .sim_at_global(global.x, global.y)
        .map(|(_, s)| s.name.clone())
        .or_else(|| title.rsplit(", ").next().map(str::to_owned))
        .filter(|r| !r.is_empty())?;
    let pos = glam::Vec3::new(
        global.x.rem_euclid(256.0) as f32,
        global.y.rem_euclid(256.0) as f32,
        global.z as f32,
    );
    Some(crate::slurl::make(&region, pos))
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

#[derive(Default)]
pub struct PlacesUi {
    pub tab: usize,
    filter: String,
    selected: Option<Sel>,
    /// Folders opened in the landmarks tabs.
    open_folders: HashSet<Uuid>,
    /// History sections closed by the user (isAccordionCollapsedByUser).
    collapsed: HashSet<usize>,
    confirm: Option<Confirm>,
    /// The box "Do not show again" of the teleport confirmation on screen.
    dont_ask: bool,
    /// Landmark assets to load / resolve (the app calls Landmarks::resolve).
    pub want_landmarks: HashSet<Uuid>,
    /// Inventory folders to fetch.
    pub want_folders: HashSet<Uuid>,
    /// Map cells (region grid x, y) whose region names Aurora's rows wait for.
    pub want_cells: Vec<(i64, i64)>,
    /// Parcel snapshots of Aurora's rows on screen without a thumbnail
    /// (spot, global point, region id or nil), for `World::place_snapshots`.
    pub want_snapshots: Vec<(crate::world::place_snapshots::Spot, glam::DVec3, Uuid)>,
    /// Aurora's look.
    pub aurora: aurora::AuroraState,
    /// Inventory changes asked from a menu, sent with the window's actions.
    pending_edits: Vec<aurora_net::inventory::operations::Mutation>,
    raise: bool,
}

impl PlacesUi {
    /// "open_landmark_tab" / "open_teleport_history_tab": the lists on a tab.
    pub fn open_tab(&mut self, tab: usize) {
        self.tab = tab.min(TAB_HISTORY);
        self.selected = None;
        self.aurora.open_tab(self.tab);
        self.raise = true;
    }

    /// Bring the window to the front (a profile was opened).
    pub fn raise(&mut self) {
        self.raise = true;
    }

    /// TeleportFromLandmark from elsewhere (a standalone landmark window).
    pub fn confirm_landmark(&mut self, asset: Uuid, name: String) {
        self.confirm = Some(Confirm::Landmark { asset, name });
    }

    /// `confirm_teleports`: `Settings::confirm_teleports`, off once the box
    /// "Do not show again" of a teleport confirmation was ticked.
    /// `aurora`: Aurora's look and what it needs, None for the Legacy
    /// Viewer's tabs.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        c: &mut Ctx,
        open: &mut bool,
        by_date: bool,
        confirm_teleports: bool,
        aurora: Option<&aurora::Env>,
    ) -> Vec<PlacesAction> {
        let mut actions = Vec::new();
        self.show_confirm(ctx, c.p, confirm_teleports, &mut actions);
        if !*open {
            return actions;
        }
        if std::mem::take(&mut self.raise) {
            widgets::expand_floater(ctx, "places");
        }
        let screen = ctx.content_rect();
        // floater_places.xml: 333 x 588 (200 x 200 at least)
        let size = Vec2::new(400.0, 588.0);
        let pos = screen.center() - size * 0.5 + Vec2::new(120.0, 0.0);
        let mut floater = Floater::new("places", aurora_i18n::tr!("common-places"), pos, size);
        floater.min_size = Vec2::new(260.0, 300.0);
        if let Some(env) = aurora {
            // the mockup's panes reach the window's edges
            floater.flush_body = true;
            floater.min_size = Vec2::new(320.0, 360.0);
            floater.show(ctx, c.p, open, |ui| {
                let body = egui::Rect::from_min_max(ui.cursor().min, ui.max_rect().max);
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(body)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                child.set_clip_rect(body.intersect(ui.clip_rect()));
                self.aurora_body(&mut child, c, env, &mut actions);
                ui.allocate_rect(body, Sense::hover());
            });
            return actions;
        }
        floater.show(ctx, c.p, open, |ui| {
            let body = egui::Rect::from_min_max(ui.cursor().min, ui.max_rect().max);
            ui.allocate_rect(body, Sense::hover());
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(body)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            child.set_clip_rect(body.intersect(ui.clip_rect()));
            let ui = &mut child;
            match c.world.place_details.panel.clone() {
                Some(place) => self.profile_view(ui, c, &place, &mut actions),
                None => self.list_view(ui, c, by_date, &mut actions),
            }
        });
        actions
    }

    fn show_confirm(&mut self, ctx: &egui::Context, p: &Palette, confirm_teleports: bool, actions: &mut Vec<PlacesAction>) {
        let Some(confirm) = &self.confirm else {
            return;
        };
        // the question of a teleport (with its "Do not show again" box)
        let teleport = match confirm {
            Confirm::Landmark { name, .. } => Some(aurora_i18n::tr!(
                "ui-places-do-you-really-want-to-teleport-to",
                name = name.to_string()
            )),
            Confirm::History { title, .. } => Some(aurora_i18n::tr!("ui-places-teleport-to", title = title.to_string())),
            Confirm::Clear => None,
        };
        let answer = match teleport {
            // turned off: the teleport starts without a question
            Some(_) if !confirm_teleports => Some(true),
            Some(text) => super::context::teleport_confirm(ctx, p, "places_confirm", &text, &mut self.dont_ask),
            None => {
                let mut answer = None;
                let modal = crate::ui::widgets::Modal::new(egui::Id::unique("places_confirm")).show(ctx, |ui| {
                    ui.set_width(340.0);
                    ui.label(
                        RichText::new(aurora_i18n::tr!("common-teleport-history"))
                            .size(15.0)
                            .strong()
                            .color(p.ink),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(aurora_i18n::tr!("ui-places-do-you-really-want-to-clear-your-teleport-history"))
                            .size(12.5)
                            .color(p.muted),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if widgets::default_button(widgets::flat_button(ui, p, aurora_i18n::tr!("common-ok"))).clicked() {
                            answer = Some(true);
                        }
                        if widgets::flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                            answer = Some(false);
                        }
                    });
                });
                if modal.should_close() && answer.is_none() {
                    answer = Some(false);
                }
                answer
            }
        };
        if let Some(yes) = answer {
            let dont_ask = std::mem::take(&mut self.dont_ask);
            if yes && let Some(c) = self.confirm.take() {
                if dont_ask && !matches!(c, Confirm::Clear) {
                    actions.push(PlacesAction::StopConfirmingTeleports);
                }
                actions.push(match c {
                    Confirm::Landmark { asset, .. } => PlacesAction::TeleportLandmark(asset),
                    Confirm::History { global, .. } => PlacesAction::Teleport(global),
                    Confirm::Clear => PlacesAction::ClearHistory,
                });
            }
            self.confirm = None;
        }
    }

    // ---------------------------------------------------------------- lists

    fn list_view(&mut self, ui: &mut egui::Ui, c: &mut Ctx, by_date: bool, actions: &mut Vec<PlacesAction>) {
        let p = c.p;
        let world = c.world;
        let history = self.tab == TAB_HISTORY;
        // top_menu_panel: filter, ⚙, sort, + and trash (the last two hidden
        // for the history, which creates and deletes nothing)
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let buttons = if history { 2.0 } else { 4.0 };
            let w = (ui.available_width() - buttons * 25.0).max(60.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text(aurora_i18n::tr!("ui-places-filter-places"))
                    .desired_width(w),
            );
            widgets::icon_menu(ui, p, "settings-filled", aurora_i18n::tr!("ui-places-show-options"), |ui| {
                if history {
                    self.history_item_menu(ui, p, world, actions);
                } else {
                    self.landmark_gear_menu(ui, p, world, by_date, actions);
                }
            });
            widgets::icon_menu(
                ui,
                p,
                "sort-ascending-2-filled",
                aurora_i18n::tr!("ui-places-show-sorting-options"),
                |ui| {
                    if history {
                        self.history_gear_menu(ui, p, world);
                    } else {
                        self.sorting_menu(ui, p, world, by_date, actions);
                    }
                },
            );
            if !history {
                widgets::icon_menu(ui, p, "plus", aurora_i18n::tr!("ui-places-add-a-new-landmark-or-folder"), |ui| {
                    super::menu::todo(ui, p, "folder-filled", aurora_i18n::tr!("ui-places-add-folder"));
                    // "add_landmark_root": the Landmarks folder
                    if super::menu::item(ui, p, "map-pin-filled", aurora_i18n::tr!("ui-places-add-landmark")) {
                        actions.push(PlacesAction::CreateLandmark(None));
                    }
                });
                widgets::icon_button(ui, p, "trash-filled", super::menu::not_yet(), false);
            }
        });
        ui.add_space(4.0);
        let before = self.tab;
        widgets::tabs(
            ui,
            p,
            &mut self.tab,
            &[
                (aurora_i18n::tr!("common-favorites"), true),
                (aurora_i18n::tr!("common-landmarks"), true),
                (aurora_i18n::tr!("ui-places-teleport-history"), true),
            ],
        );
        if before != self.tab {
            self.selected = None;
        }
        ui.add_space(4.0);
        // the list, above the bottom bar
        let list_h = (ui.available_height() - 32.0).max(40.0);
        egui::ScrollArea::vertical()
            .id_salt(("places-list", self.tab))
            .max_height(list_h)
            .min_scrolled_height(list_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if history {
                    self.history_list(ui, p, world, actions);
                } else {
                    self.landmark_list(ui, p, world, by_date, actions);
                }
            });
        ui.add_space(6.0);
        self.list_buttons(ui, p, world, actions);
    }

    /// The landmark selected, with its asset.
    fn selected_landmark<'w>(&self, world: &'w World) -> Option<&'w aurora_net::inventory::InvItem> {
        match self.selected {
            Some(Sel::Item(id)) => world.inventory.items.get(&id).filter(|i| i.asset_type == ASSET_LANDMARK),
            _ => None,
        }
    }

    fn landmark_list(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, by_date: bool, actions: &mut Vec<PlacesAction>) {
        let kind = if self.tab == TAB_FAVORITES {
            FOLDER_FAVORITES
        } else {
            FOLDER_LANDMARKS
        };
        let Some(root) = system_folder(&world.inventory, kind) else {
            ui.label(RichText::new(place_details::loading()).size(12.0).color(p.muted));
            return;
        };
        // fetch what is shown (and everything while filtering)
        self.want_fetch(&world.inventory, root, !self.filter.is_empty(), 0);
        // Favorites keep their own order (EXT-1758): by name here
        let by_date = by_date && self.tab == TAB_LANDMARKS;
        let rows = landmark_rows(
            &world.inventory,
            root,
            self.filter.trim(),
            by_date,
            &self.open_folders,
            self.tab == TAB_LANDMARKS,
        );
        if rows.is_empty() {
            let fetching = world.inventory.folders.get(&root).is_some_and(|f| f.state != FetchState::Fetched);
            let text = if fetching {
                place_details::loading()
            } else if self.tab == TAB_FAVORITES {
                // FavoritesNoMatchingItems
                aurora_i18n::tr!("ui-places-drag-a-landmark-here-to-add-it-to-your")
            } else {
                // PlacesNoMatchingItems (the search is not ported yet)
                aurora_i18n::tr!("ui-places-did-not-find-what-you-were-looking-for-try-2")
            };
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.muted)).wrap());
            return;
        }
        for row in rows {
            let sel = match row.folder {
                Some(_) => Sel::Folder(row.id),
                None => Sel::Item(row.id),
            };
            let icon = match row.folder {
                Some(true) => "folder-open-filled",
                Some(false) => "folder-filled",
                None => "map-pin-filled",
            };
            // a landmark's region, once its asset is read and the region known
            let rating = world.inventory.items.get(&row.id).filter(|_| row.folder.is_none()).and_then(|it| {
                match world.landmarks.peek(it.asset_id) {
                    Resolved::Ready { global, .. } => world.map.sim_at_global(global.x, global.y).map(|(_, s)| s.access),
                    _ => None,
                }
            });
            let r = list_row(
                ui,
                p,
                row.depth,
                row.folder,
                icon,
                rating,
                &row.name,
                self.selected == Some(sel),
                false,
            );
            if row.folder.is_none() {
                drag::source(&r.response, || drag::PlaceDrag {
                    target: drag::Target::Landmark(row.id),
                    favorite: (self.tab == TAB_FAVORITES).then_some(row.id),
                    name: row.name.clone(),
                    thumbnail: None,
                });
            }
            if r.response.clicked() || r.response.secondary_clicked() {
                self.selected = Some(sel);
                if let Some(it) = world.inventory.items.get(&row.id) {
                    self.want_landmarks.insert(it.asset_id);
                }
            }
            if r.toggle
                && let Some(open) = row.folder
            {
                if open {
                    self.open_folders.remove(&row.id);
                } else {
                    self.open_folders.insert(row.id);
                }
            }
            match row.folder {
                Some(open) => {
                    if r.response.double_clicked() {
                        if open {
                            self.open_folders.remove(&row.id);
                        } else {
                            self.open_folders.insert(row.id);
                        }
                    }
                    super::menu::context_menu(&r.response, p, |ui| self.folder_menu(ui, p, world, by_date, actions));
                }
                None => {
                    // LLLandmarkBridgeAction::doIt: TeleportFromLandmark
                    if r.response.double_clicked()
                        && let Some(it) = world.inventory.items.get(&row.id)
                    {
                        self.confirm = Some(Confirm::Landmark {
                            asset: it.asset_id,
                            name: it.name.clone(),
                        });
                    }
                    super::menu::context_menu(&r.response, p, |ui| {
                        self.landmark_menu(ui, p, world, by_date, actions);
                    });
                }
            }
        }
    }

    fn want_fetch(&mut self, inv: &Inventory, folder: Uuid, deep: bool, depth: usize) {
        let Some(f) = inv.folders.get(&folder) else {
            return;
        };
        if matches!(f.state, FetchState::Unknown | FetchState::Failed) {
            self.want_folders.insert(folder);
        }
        if depth < 32 {
            for c in &f.children {
                if deep || self.open_folders.contains(c) {
                    self.want_fetch(inv, *c, deep, depth + 1);
                }
            }
        }
    }

    fn all_folders(world: &World, root: Option<Uuid>) -> Vec<Uuid> {
        let mut out = Vec::new();
        let mut stack: Vec<Uuid> = root.into_iter().collect();
        while let Some(f) = stack.pop() {
            if let Some(folder) = world.inventory.folders.get(&f) {
                for c in &folder.children {
                    out.push(*c);
                    stack.push(*c);
                }
            }
        }
        out
    }

    fn current_root(&self, world: &World) -> Option<Uuid> {
        let kind = if self.tab == TAB_FAVORITES {
            FOLDER_FAVORITES
        } else {
            FOLDER_LANDMARKS
        };
        system_folder(&world.inventory, kind)
    }

    /// "Expand / Collapse all folders" and "Sort by date"
    /// (menu_places_gear_sorting.xml, the end of the gear menus).
    fn expand_collapse_sort(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, by_date: bool, actions: &mut Vec<PlacesAction>) {
        let folders = Self::all_folders(world, self.current_root(world));
        let any_closed = folders.iter().any(|f| !self.open_folders.contains(f));
        let any_open = folders.iter().any(|f| self.open_folders.contains(f));
        if super::menu::item_if(
            ui,
            p,
            "square-rounded-chevrons-down-filled",
            aurora_i18n::tr!("ui-places-expand-all-folders"),
            any_closed,
        ) {
            self.open_folders.extend(folders.iter().copied());
        }
        if super::menu::item_if(
            ui,
            p,
            "square-rounded-chevrons-up-filled",
            aurora_i18n::tr!("ui-places-collapse-all-folders"),
            any_open,
        ) {
            self.open_folders.clear();
        }
        // disabled for Favorites: they keep their own order (EXT-1758)
        if super::menu::check(
            ui,
            p,
            "calendar-filled",
            aurora_i18n::tr!("ui-places-sort-by-date"),
            by_date,
            self.tab == TAB_LANDMARKS,
        ) {
            actions.push(PlacesAction::SortByDate(!by_date));
        }
    }

    fn sorting_menu(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, by_date: bool, actions: &mut Vec<PlacesAction>) {
        self.expand_collapse_sort(ui, p, world, by_date, actions);
    }

    /// The ⚙ menu: menu_places_gear_landmark.xml when a landmark is
    /// selected, menu_places_gear_folder.xml otherwise.
    fn landmark_gear_menu(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, by_date: bool, actions: &mut Vec<PlacesAction>) {
        if self.selected_landmark(world).is_some() {
            self.landmark_menu(ui, p, world, by_date, actions);
        } else {
            self.folder_menu(ui, p, world, by_date, actions);
        }
    }

    /// menu_places_gear_landmark.xml.
    fn landmark_menu(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, by_date: bool, actions: &mut Vec<PlacesAction>) {
        let Some(it) = self.selected_landmark(world).cloned() else {
            return;
        };
        let resolved = match world.landmarks.peek(it.asset_id) {
            Resolved::Ready { global, .. } => Some(global),
            _ => None,
        };
        if super::menu::item(ui, p, "location-filled", aurora_i18n::tr!("common-teleport-3")) {
            self.confirm = Some(Confirm::Landmark {
                asset: it.asset_id,
                name: it.name.clone(),
            });
        }
        if super::menu::item_if(ui, p, "map", aurora_i18n::tr!("common-show-on-map"), resolved.is_some())
            && let Some(g) = resolved
        {
            actions.push(PlacesAction::ShowOnMap(g));
        }
        super::menu::todo(ui, p, "share", aurora_i18n::tr!("common-share"));
        if super::menu::item(ui, p, "info-circle-filled", aurora_i18n::tr!("ui-places-more-information")) {
            actions.push(landmark_profile(&it));
        }
        let in_favorites = system_folder(&world.inventory, FOLDER_FAVORITES).is_some_and(|f| it.parent == f);
        if in_favorites {
            super::menu::todo(ui, p, "map-pin-filled", aurora_i18n::tr!("ui-places-move-to-landmarks"));
        } else {
            super::menu::todo(ui, p, "star-filled", aurora_i18n::tr!("ui-places-move-to-favorites"));
        }
        super::menu::separator(ui, p);
        // "add_landmark" on a landmark: no destination folder
        if super::menu::item(ui, p, "map-pin-filled", aurora_i18n::tr!("ui-places-create-landmark")) {
            actions.push(PlacesAction::CreateLandmark(None));
        }
        super::menu::todo(ui, p, "folder-filled", aurora_i18n::tr!("ui-places-create-folder"));
        super::menu::separator(ui, p);
        super::menu::todo(ui, p, "cut", aurora_i18n::tr!("common-cut"));
        super::menu::todo(ui, p, "copy-filled", aurora_i18n::tr!("ui-places-copy-landmark"));
        let slurl = resolved.and_then(|g| slurl_at(world, g, ""));
        if super::menu::item_if(ui, p, "link-filled", aurora_i18n::tr!("common-copy-slurl"), slurl.is_some())
            && let Some(s) = slurl
        {
            ui.ctx().copy_text(s);
        }
        super::menu::todo(ui, p, "clipboard-text-filled", aurora_i18n::tr!("common-paste"));
        super::menu::todo(ui, p, "pencil-filled", aurora_i18n::tr!("common-rename"));
        super::menu::todo(ui, p, "trash-filled", aurora_i18n::tr!("common-delete"));
        super::menu::separator(ui, p);
        self.expand_collapse_sort(ui, p, world, by_date, actions);
        super::menu::separator(ui, p);
        super::menu::todo(ui, p, "star-filled", aurora_i18n::tr!("common-create-favorite"));
    }

    /// menu_places_gear_folder.xml.
    fn folder_menu(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, by_date: bool, actions: &mut Vec<PlacesAction>) {
        // "add_landmark": saved in the selected folder
        let folder = match self.selected {
            Some(Sel::Folder(f)) => Some(f),
            _ => None,
        };
        if super::menu::item_if(
            ui,
            p,
            "map-pin-filled",
            aurora_i18n::tr!("ui-places-add-landmark"),
            folder.is_some(),
        ) {
            actions.push(PlacesAction::CreateLandmark(folder));
        }
        super::menu::todo(ui, p, "folder-filled", aurora_i18n::tr!("ui-places-add-folder"));
        super::menu::separator(ui, p);
        super::menu::todo(ui, p, "cut", aurora_i18n::tr!("common-cut"));
        super::menu::todo(ui, p, "copy-filled", aurora_i18n::tr!("common-copy-2"));
        super::menu::todo(ui, p, "clipboard-text-filled", aurora_i18n::tr!("common-paste"));
        super::menu::todo(ui, p, "pencil-filled", aurora_i18n::tr!("common-rename"));
        super::menu::todo(ui, p, "trash-filled", aurora_i18n::tr!("common-delete"));
        super::menu::separator(ui, p);
        let folder = match self.selected {
            Some(Sel::Folder(f)) => Some(f),
            _ => None,
        };
        let open = folder.is_some_and(|f| self.open_folders.contains(&f));
        if super::menu::item_if(
            ui,
            p,
            "caret-down-filled",
            aurora_i18n::tr!("common-expand"),
            folder.is_some() && !open,
        ) && let Some(f) = folder
        {
            self.open_folders.insert(f);
        }
        if super::menu::item_if(ui, p, "caret-right-filled", aurora_i18n::tr!("common-collapse"), open)
            && let Some(f) = folder
        {
            self.open_folders.remove(&f);
        }
        self.expand_collapse_sort(ui, p, world, by_date, actions);
    }

    fn history_list(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, actions: &mut Vec<PlacesAction>) {
        let items = &world.tp_storage.items;
        let filter = self.filter.trim().to_owned();
        let sections = history_sections(items, &filter, now_secs());
        if sections.is_empty() {
            // no_teleports_msg / no_matched_teleports_msg (the search is not
            // ported yet)
            let text = if filter.is_empty() {
                aurora_i18n::tr!("ui-places-teleport-history-is-empty-try-search")
            } else {
                aurora_i18n::tr!("ui-places-did-not-find-what-you-were-looking-for-try")
            };
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.muted)).wrap());
            return;
        }
        for (section, indices) in sections {
            // every section opens while filtering
            let open = !filter.is_empty() || !self.collapsed.contains(&section);
            if section_header(ui, p, tphistory::history_sections()[section], open).clicked() && filter.is_empty() {
                if open {
                    self.collapsed.insert(section);
                } else {
                    self.collapsed.remove(&section);
                }
            }
            if !open {
                continue;
            }
            for i in indices {
                let it = &items[i];
                // the region where the entry led, by its position, else by
                // the name at the end of its title (asked once if unknown)
                let rating = world
                    .map
                    .sim_at_global(it.global.x, it.global.y)
                    .map(|(_, s)| s.access)
                    .or_else(|| {
                        let region = it.title.rsplit(", ").next().unwrap_or_default();
                        widgets::region_rating(ui.ctx(), &world.map.ratings, region)
                    });
                let r = list_row(
                    ui,
                    p,
                    0,
                    None,
                    "map-pin-filled",
                    rating,
                    &it.title,
                    self.selected == Some(Sel::History(i)),
                    true,
                );
                drag::source(&r.response, || drag::PlaceDrag {
                    target: drag::Target::History(it.global),
                    favorite: None,
                    name: it.title.clone(),
                    thumbnail: None,
                });
                if r.response.clicked() || r.response.secondary_clicked() {
                    self.selected = Some(Sel::History(i));
                }
                // the "i" button of LLTeleportHistoryFlatItem
                if r.info {
                    actions.push(history_profile(it));
                }
                if r.response.double_clicked() {
                    self.confirm = Some(Confirm::History {
                        global: it.global,
                        title: it.title.clone(),
                    });
                }
                super::menu::context_menu(&r.response, p, |ui| self.history_item_menu(ui, p, world, actions));
            }
        }
    }

    fn selected_history<'w>(&self, world: &'w World) -> Option<(usize, &'w HistoryItem)> {
        match self.selected {
            Some(Sel::History(i)) => world.tp_storage.items.get(i).map(|it| (i, it)),
            _ => None,
        }
    }

    /// menu_teleport_history_item.xml.
    fn history_item_menu(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, actions: &mut Vec<PlacesAction>) {
        let sel = self.selected_history(world).map(|(i, it)| (i, it.clone()));
        let some = sel.is_some();
        if super::menu::item_if(ui, p, "location-filled", aurora_i18n::tr!("common-teleport-3"), some)
            && let Some((_, it)) = &sel
        {
            self.confirm = Some(Confirm::History {
                global: it.global,
                title: it.title.clone(),
            });
        }
        if super::menu::item_if(ui, p, "info-circle-filled", aurora_i18n::tr!("ui-places-more-information"), some)
            && let Some((_, it)) = &sel
        {
            actions.push(history_profile(it));
        }
        if super::menu::item_if(ui, p, "map", aurora_i18n::tr!("common-show-on-map"), some)
            && let Some((_, it)) = &sel
        {
            actions.push(PlacesAction::ShowOnMap(it.global));
        }
        let slurl = sel.as_ref().and_then(|(_, it)| slurl_at(world, it.global, &it.title));
        if super::menu::item_if(ui, p, "link-filled", aurora_i18n::tr!("common-copy-slurl"), slurl.is_some())
            && let Some(s) = slurl
        {
            ui.ctx().copy_text(s);
        }
        if super::menu::item_if(ui, p, "trash-filled", aurora_i18n::tr!("ui-places-remove-from-history"), some)
            && let Some((i, _)) = sel
        {
            actions.push(PlacesAction::RemoveHistory(i));
            self.selected = None;
        }
    }

    /// menu_teleport_history_gear.xml.
    fn history_gear_menu(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World) {
        let present: Vec<usize> = history_sections(&world.tp_storage.items, "", now_secs())
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        let any_closed = present.iter().any(|s| self.collapsed.contains(s));
        let any_open = present.iter().any(|s| !self.collapsed.contains(s));
        if super::menu::item_if(
            ui,
            p,
            "square-rounded-chevrons-down-filled",
            aurora_i18n::tr!("ui-places-expand-all-folders"),
            any_closed,
        ) {
            self.collapsed.clear();
        }
        if super::menu::item_if(
            ui,
            p,
            "square-rounded-chevrons-up-filled",
            aurora_i18n::tr!("ui-places-collapse-all-folders"),
            any_open,
        ) {
            self.collapsed.extend(0..tphistory::history_sections().len());
            self.selected = None;
        }
        // FIRE-31025
        if super::menu::item_if(
            ui,
            p,
            "trash-filled",
            aurora_i18n::tr!("ui-places-clear-teleport-history"),
            !world.tp_storage.items.is_empty(),
        ) {
            self.confirm = Some(Confirm::Clear);
        }
    }

    /// Teleport / Map / Profile (FIRE-31033), for the selection.
    fn list_buttons(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, actions: &mut Vec<PlacesAction>) {
        let landmark = self.selected_landmark(world).cloned();
        let history = self.selected_history(world).map(|(_, it)| it.clone());
        let map_target = match (&landmark, &history) {
            (Some(it), _) => match world.landmarks.peek(it.asset_id) {
                Resolved::Ready { global, .. } => Some(global),
                _ => None,
            },
            (_, Some(h)) => Some(h.global),
            _ => None,
        };
        let any = landmark.is_some() || history.is_some();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let w = ((ui.available_width() - 6.0) / 3.0).max(60.0);
            if place_details::button(
                ui,
                p,
                aurora_i18n::tr!("common-teleport"),
                aurora_i18n::tr!("ui-places-teleport-to-a-location-of-your-choice"),
                any,
                w,
            ) {
                self.confirm = match (&landmark, &history) {
                    (Some(it), _) => Some(Confirm::Landmark {
                        asset: it.asset_id,
                        name: it.name.clone(),
                    }),
                    (_, Some(h)) => Some(Confirm::History {
                        global: h.global,
                        title: h.title.clone(),
                    }),
                    _ => None,
                };
            }
            if place_details::button(
                ui,
                p,
                aurora_i18n::tr!("common-map"),
                aurora_i18n::tr!("ui-places-show-the-corresponding-location-on-the-map"),
                map_target.is_some(),
                w,
            ) && let Some(g) = map_target
            {
                actions.push(PlacesAction::ShowOnMap(g));
            }
            if place_details::button(
                ui,
                p,
                aurora_i18n::tr!("common-profile"),
                aurora_i18n::tr!("ui-places-show-place-profile"),
                any,
                w,
            ) {
                match (&landmark, &history) {
                    (Some(it), _) => actions.push(landmark_profile(it)),
                    (_, Some(h)) => actions.push(history_profile(h)),
                    _ => {}
                }
            }
        });
    }

    // -------------------------------------------------------------- profile

    fn profile_view(
        &mut self,
        ui: &mut egui::Ui,
        c: &mut Ctx,
        place: &crate::world::place_details::Place,
        actions: &mut Vec<PlacesAction>,
    ) {
        let p = c.p;
        let world = c.world;
        // header_container: back arrow and title
        ui.horizontal(|ui| {
            if widgets::icon_button(ui, p, "caret-left-filled", aurora_i18n::tr!("common-back"), true) {
                actions.push(PlacesAction::CloseProfile);
            }
            ui.label(RichText::new(place_details::header_title(place)).size(16.0).strong().color(p.ink));
        });
        ui.add_space(4.0);
        // the buttons are laid out first, from the bottom, and the profile
        // takes what is left above them: a fixed share for the buttons cut
        // them off when the theme's spacing differed from it
        let rest = ui.available_rect_before_wrap();
        ui.allocate_rect(rest, Sense::hover());
        let mut bottom_up = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rest)
                .layout(egui::Layout::bottom_up(egui::Align::Min)),
        );
        let ui = &mut bottom_up;
        // updateVerbs with a profile shown: Teleport, Map and the
        // overflow menu (Profile hidden)
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let w = ((ui.available_width() - 30.0) / 2.0).max(60.0);
            if place_details::button(
                ui,
                p,
                aurora_i18n::tr!("common-teleport"),
                aurora_i18n::tr!("ui-places-teleport-to-a-location-of-your-choice"),
                place.global.is_some(),
                w,
            ) && let Some(a) = place_details::teleport_action(place, world)
            {
                match a {
                    place_details::PlaceAction::TeleportLandmark { asset, name } => {
                        self.confirm = Some(Confirm::Landmark { asset, name });
                    }
                    place_details::PlaceAction::Teleport(g) => actions.push(PlacesAction::Teleport(g)),
                    _ => {}
                }
            }
            if place_details::button(
                ui,
                p,
                aurora_i18n::tr!("common-map"),
                aurora_i18n::tr!("ui-places-show-the-corresponding-location-on-the-map"),
                place.global.is_some(),
                w,
            ) && let Some(g) = place.global
            {
                actions.push(PlacesAction::ShowOnMap(g));
            }
            widgets::icon_menu(ui, p, "caret-up-filled", aurora_i18n::tr!("ui-places-show-more-options"), |ui| {
                place_details::overflow_menu(ui, p, world, place);
            });
        });
        ui.add_space(6.0);
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("places-profile", place.serial))
                .auto_shrink([false, false])
                .show(ui, |ui| place_details::profile_body(ui, c, place));
        });
    }
}

fn landmark_profile(it: &aurora_net::inventory::InvItem) -> PlacesAction {
    PlacesAction::OpenProfile(Source::Landmark {
        item: it.id,
        asset: it.asset_id,
    })
}

fn history_profile(it: &HistoryItem) -> PlacesAction {
    PlacesAction::OpenProfile(Source::History {
        title: it.title.clone(),
        global: it.global,
    })
}

/// What happened on a list line.
struct RowResponse {
    response: egui::Response,
    /// The folder's caret was clicked.
    toggle: bool,
    /// The "i" button was clicked.
    info: bool,
}

/// One line of a list: indentation, caret for a folder, icon, the rating
/// badge of the place's region when known, name; the history lines show an
/// "i" button while hovered.
#[allow(clippy::too_many_arguments)]
fn list_row(
    ui: &mut egui::Ui,
    p: &Palette,
    depth: usize,
    folder: Option<bool>,
    icon: &str,
    rating: Option<u8>,
    name: &str,
    selected: bool,
    info_button: bool,
) -> RowResponse {
    let w = ui.available_width();
    let sense = if folder.is_none() {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(w, 20.0), sense);
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, ui.visuals().window_corner_radius, p.violet.gamma_multiply(0.28));
    } else if response.hovered() {
        painter.rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
    }
    let mut x = rect.left() + 4.0 + depth as f32 * 14.0;
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    let mut toggle = false;
    if let Some(open) = folder {
        let caret = egui::Rect::from_center_size(egui::pos2(x + 6.0, rect.center().y), Vec2::splat(12.0));
        if let Some(t) = super::icons::global(if open { "caret-down-filled" } else { "caret-right-filled" }) {
            painter.image(t.id(), caret, uv, p.muted);
        }
        toggle = response.clicked() && response.interact_pointer_pos().is_some_and(|pos| caret.expand(3.0).contains(pos));
        x += 14.0;
    }
    if let Some(t) = super::icons::global(icon) {
        let tint = if folder.is_some() { p.violet_light } else { p.ink };
        painter.image(
            t.id(),
            egui::Rect::from_center_size(egui::pos2(x + 8.0, rect.center().y), Vec2::splat(15.0)),
            uv,
            tint,
        );
    }
    x += 21.0;
    if let Some(access) = rating {
        let side = 13.0;
        let badge = egui::Rect::from_min_size(egui::pos2(x, rect.center().y - side * 0.5), Vec2::splat(side));
        if widgets::paint_maturity_badge(
            painter,
            p,
            super::bars::maturity_name(access),
            badge,
            widgets::inner_radius(ui.visuals()),
        ) {
            x += side + 5.0;
        }
    }
    let mut right = rect.right() - 4.0;
    let mut info = false;
    if info_button && (response.hovered() || selected) {
        let r = egui::Rect::from_center_size(egui::pos2(right - 8.0, rect.center().y), Vec2::splat(16.0));
        let hovered = response.hover_pos().is_some_and(|pos| r.contains(pos));
        if let Some(t) = super::icons::global("info-circle-filled") {
            painter.image(t.id(), r, uv, if hovered { p.ink } else { p.muted });
        }
        info = response.clicked() && response.interact_pointer_pos().is_some_and(|pos| r.contains(pos));
        right -= 20.0;
    }
    let galley = painter.layout(name.to_owned(), egui::FontId::proportional(12.5), p.ink, f32::INFINITY);
    let clip = egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(right, rect.bottom()));
    painter
        .with_clip_rect(clip.intersect(ui.clip_rect()))
        .galley(egui::pos2(x, rect.center().y - galley.size().y * 0.5), galley, p.ink);
    let response = if ui.fonts_mut(|f| f.layout_no_wrap(name.to_owned(), egui::FontId::proportional(12.5), p.ink).size().x) > right - x {
        response.on_hover_text(name)
    } else {
        response
    };
    RowResponse { response, toggle, info }
}

/// An accordion tab header of the history.
fn section_header(ui: &mut egui::Ui, p: &Palette, title: &str, open: bool) -> egui::Response {
    let w = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(w, 22.0), Sense::click());
    let fill = if response.hovered() { p.raised } else { p.field };
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    if let Some(t) = super::icons::global(if open { "caret-down-filled" } else { "caret-right-filled" }) {
        ui.painter().image(
            t.id(),
            egui::Rect::from_center_size(egui::pos2(rect.left() + 10.0, rect.center().y), Vec2::splat(12.0)),
            uv,
            p.muted,
        );
    }
    ui.painter().text(
        egui::pos2(rect.left() + 20.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(12.5),
        p.ink,
    );
    ui.add_space(1.0);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::inventory::{FolderContents, InvFolder, InvItem};

    const DAY: f64 = 86_400.0;

    /// Runs the confirmation of `ui`, clicking in turn the texts of `clicks`,
    /// and gives the actions it asked for.
    fn answer_confirm(ui: &mut PlacesUi, confirm_teleports: bool, clicks: &[&str]) -> Vec<PlacesAction> {
        // The last text drawn: in English the title and the button of the
        // question are both "Teleport", and the button comes after.
        fn text_pos(shape: &egui::epaint::Shape, text: &str) -> Option<egui::Pos2> {
            match shape {
                egui::epaint::Shape::Text(t) if t.galley.text() == text => Some(t.pos + egui::vec2(4.0, 4.0)),
                egui::epaint::Shape::Vec(v) => v.iter().rev().find_map(|s| text_pos(s, text)),
                _ => None,
            }
        }
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let mut actions = Vec::new();
        let mut frame = |events: Vec<egui::Event>| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                    events,
                    ..Default::default()
                },
                |root| ui.show_confirm(root.ctx(), &theme.palette(), confirm_teleports, &mut actions),
            );
            output.textures_delta.clear();
            output
        };
        for _ in 0..3 {
            frame(Vec::new());
        }
        for text in clicks {
            let output = frame(Vec::new());
            let pos = output
                .shapes
                .iter()
                .rev()
                .find_map(|s| text_pos(&s.shape, text))
                .unwrap_or_else(|| panic!("« {text} » in the confirmation"));
            for pressed in [true, false] {
                frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
        }
        actions
    }

    #[test]
    fn teleport_confirmation_has_a_do_not_show_again_box() {
        let asset = Uuid::from_u128(7);
        let teleport = aurora_i18n::tr!("common-teleport-3");
        let dont_ask = aurora_i18n::tr!("ui-context-do-not-show-again");
        let pending = || {
            let mut ui = PlacesUi::default();
            ui.confirm_landmark(asset, "Plage".into());
            ui
        };

        // asked, box left alone: the teleport only
        let mut ui = pending();
        let actions = answer_confirm(&mut ui, true, &[teleport]);
        assert!(matches!(&actions[..], [PlacesAction::TeleportLandmark(a)] if *a == asset));
        assert!(ui.confirm.is_none());

        // box ticked, then "Teleport": the setting goes off first
        let mut ui = pending();
        let actions = answer_confirm(&mut ui, true, &[dont_ask, teleport]);
        assert!(matches!(
            &actions[..],
            [PlacesAction::StopConfirmingTeleports, PlacesAction::TeleportLandmark(a)] if *a == asset
        ));
        assert!(!ui.dont_ask, "the box starts empty for the next question");

        // box ticked, then "Cancel": nothing happens, the question stays on
        let mut ui = pending();
        let actions = answer_confirm(&mut ui, true, &[dont_ask, aurora_i18n::tr!("common-cancel")]);
        assert!(actions.is_empty());
        assert!(ui.confirm.is_none() && !ui.dont_ask);

        // turned off: no question, a history entry too
        let mut ui = pending();
        let actions = answer_confirm(&mut ui, false, &[]);
        assert!(matches!(&actions[..], [PlacesAction::TeleportLandmark(a)] if *a == asset));
        let mut ui = PlacesUi {
            confirm: Some(Confirm::History {
                global: DVec3::new(1.0, 2.0, 3.0),
                title: "Lagune".into(),
            }),
            ..Default::default()
        };
        let actions = answer_confirm(&mut ui, false, &[]);
        assert!(matches!(&actions[..], [PlacesAction::Teleport(g)] if *g == DVec3::new(1.0, 2.0, 3.0)));

        // clearing the history is always asked, and has no such box
        let mut ui = PlacesUi {
            confirm: Some(Confirm::Clear),
            ..Default::default()
        };
        assert!(answer_confirm(&mut ui, false, &[]).is_empty());
        assert!(ui.confirm.is_some());
        let actions = answer_confirm(&mut ui, false, &[aurora_i18n::tr!("common-ok")]);
        assert!(matches!(&actions[..], [PlacesAction::ClearHistory]));
    }

    fn folder(id: u128, parent: u128, name: &str, t: i32) -> InvFolder {
        InvFolder {
            id: Uuid::from_u128(id),
            parent: Uuid::from_u128(parent),
            name: name.into(),
            type_default: t,
            version: 1,
            favorite: false,
            favorite_known: false,
            thumbnail: Uuid::nil(),
        }
    }

    fn landmark(id: u128, parent: u128, name: &str, created: i64) -> InvItem {
        InvItem {
            id: Uuid::from_u128(id),
            parent: Uuid::from_u128(parent),
            name: name.into(),
            desc: String::new(),
            asset_type: 3,
            inv_type: 3,
            asset_id: Uuid::from_u128(id + 1000),
            flags: 0,
            favorite: false,
            favorite_known: false,
            creator: Uuid::nil(),
            created_at: created,
            owner: Uuid::nil(),
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            ..Default::default()
        }
    }

    fn inventory() -> Inventory {
        let mut inv = Inventory::default();
        let contents = |f: InvFolder, folders: Vec<InvFolder>, items: Vec<InvItem>| FolderContents {
            folder_id: f.id,
            owner_id: Uuid::nil(),
            version: 2,
            folders,
            items,
        };
        let root = folder(1, 0, aurora_i18n::tr!("common-my-inventory"), 8);
        let lms = folder(10, 1, aurora_i18n::tr!("common-landmarks"), 3);
        let shops = folder(20, 10, aurora_i18n::tr!("common-shopping"), -1);
        let empty = folder(21, 10, "Vide", -1);
        inv.apply(vec![contents(root, vec![lms.clone()], Vec::new())]);
        inv.apply(vec![contents(
            lms.clone(),
            vec![shops.clone(), empty.clone()],
            vec![
                landmark(100, 10, "Plage", 300),
                landmark(101, 10, "Arbre", 100),
                landmark(102, 10, "Musée", 200),
            ],
        )]);
        inv.apply(vec![contents(shops, Vec::new(), vec![landmark(200, 20, "Boutique du Loup", 50)])]);
        inv.apply(vec![contents(empty, Vec::new(), Vec::new())]);
        inv
    }

    fn names(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn landmark_tree_sorting_and_folders() {
        let inv = inventory();
        let root = system_folder(&inv, FOLDER_LANDMARKS).expect("landmarks folder");
        let none = HashSet::new();
        // folders first by name, then the newest landmark first
        let rows = landmark_rows(&inv, root, "", true, &none, true);
        assert_eq!(
            names(&rows),
            [aurora_i18n::tr!("common-shopping"), "Vide", "Plage", "Musée", "Arbre"]
        );
        assert_eq!(rows[0].folder, Some(false));
        // by name, and an opened folder shows its landmarks one level down
        let open: HashSet<Uuid> = [Uuid::from_u128(20)].into();
        let rows = landmark_rows(&inv, root, "", false, &open, true);
        assert_eq!(
            names(&rows),
            [
                aurora_i18n::tr!("common-shopping"),
                "Boutique du Loup",
                "Vide",
                "Arbre",
                "Musée",
                "Plage"
            ]
        );
        assert_eq!(rows[1].depth, 1);
        // Favorites-style: empty folders are not shown
        let rows = landmark_rows(&inv, root, "", true, &none, false);
        assert_eq!(names(&rows), [aurora_i18n::tr!("common-shopping"), "Plage", "Musée", "Arbre"]);
    }

    #[test]
    fn filter_keeps_matching_landmarks_and_their_folders() {
        let inv = inventory();
        let root = system_folder(&inv, FOLDER_LANDMARKS).expect("landmarks folder");
        let rows = landmark_rows(&inv, root, "LOUP", true, &HashSet::new(), true);
        assert_eq!(names(&rows), [aurora_i18n::tr!("common-shopping"), "Boutique du Loup"]);
        assert_eq!(rows[0].folder, Some(true), "opened while filtering");
        assert!(landmark_rows(&inv, root, "nulle part", true, &HashSet::new(), true).is_empty());
    }

    #[test]
    fn history_grouped_newest_first() {
        let now = 20_000.0 * DAY + 12.0 * 3600.0;
        let item = |title: &str, date: f64| HistoryItem {
            title: title.into(),
            global: DVec3::ZERO,
            date,
            slurl: String::new(),
        };
        let items = vec![
            item("Ancien", now - 400.0 * DAY),
            item("Avant-hier", now - 2.0 * DAY),
            item(aurora_i18n::tr!("world-tphistory-yesterday"), now - DAY),
            item("Matin", now - 2.0 * 3600.0),
            item(aurora_i18n::tr!("common-midday"), now - 60.0),
        ];
        let s = history_sections(&items, "", now);
        assert_eq!(s, vec![(0, vec![4, 3]), (1, vec![2]), (2, vec![1]), (8, vec![0])]);
        let s = history_sections(&items, "MAT", now);
        assert_eq!(s, vec![(0, vec![3])]);
    }

    #[test]
    fn tab_names() {
        assert_eq!(tab_from_name("favorites"), Some(TAB_FAVORITES));
        assert_eq!(tab_from_name("landmarks"), Some(TAB_LANDMARKS));
        assert_eq!(tab_from_name("History"), Some(TAB_HISTORY));
        // the former French names
        assert_eq!(tab_from_name("favoris"), Some(TAB_FAVORITES));
        assert_eq!(tab_from_name("reperes"), Some(TAB_LANDMARKS));
        assert_eq!(tab_from_name("historique"), Some(TAB_HISTORY));
        assert_eq!(tab_from_name("autre"), None);
    }
}
