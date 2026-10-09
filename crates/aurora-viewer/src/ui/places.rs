//! "Lieux" window, as Firestorm shows it by default (LLFloaterSidePanelContainer
//! "places" with LLPanelPlaces; FSUseStandalonePlaceDetailsFloater off).
//! Port of LLPanelPlaces, LLLandmarksPanel / LLFavoritesPanel and
//! LLTeleportHistoryPanel (indra/newview/llpanelplaces.cpp,
//! llpanellandmarks.cpp, llpanelteleporthistory.cpp, floater_places.xml,
//! panel_places.xml, panel_landmarks.xml, panel_favorites.xml,
//! panel_teleport_history.xml and their menus, originally LGPL 2.1):
//! a filter, the ⚙ / sort / + / trash buttons, three tabs (Favoris: the
//! Favorites folder, Repères: the Landmarks folder tree, Historique de
//! teleport.: the saved teleport history in day sections), and at the bottom
//! Se téléporter / Carte / Profil (FIRE-31033). The profile of a place link,
//! a history entry or a landmark replaces the tabs, with a back arrow
//! (`place_details.rs` draws it). Double-click teleports, after the
//! TeleportFromLandmark / TeleportToHistoryEntry confirmations.

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

/// AURORA_DEMO_PLACES and the like: a tab by its French name or index.
pub fn tab_from_name(name: &str) -> Option<usize> {
    match name.trim().to_lowercase().as_str() {
        "favoris" | "0" => Some(TAB_FAVORITES),
        "reperes" | "repères" | "1" => Some(TAB_LANDMARKS),
        "historique" | "2" => Some(TAB_HISTORY),
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
    /// Confirmed « Effacer l'historique des téléportations ».
    ClearHistory,
    /// « Trier par date » (LandmarksSortedByDate).
    SortByDate(bool),
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
    /// Landmark assets to load / resolve (the app calls Landmarks::resolve).
    pub want_landmarks: HashSet<Uuid>,
    /// Inventory folders to fetch.
    pub want_folders: HashSet<Uuid>,
    raise: bool,
}

impl PlacesUi {
    /// "open_landmark_tab" / "open_teleport_history_tab": the lists on a tab.
    pub fn open_tab(&mut self, tab: usize) {
        self.tab = tab.min(TAB_HISTORY);
        self.selected = None;
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

    pub fn show(&mut self, ctx: &egui::Context, c: &mut Ctx, open: &mut bool, by_date: bool) -> Vec<PlacesAction> {
        let mut actions = Vec::new();
        self.show_confirm(ctx, c.p, &mut actions);
        if !*open {
            return actions;
        }
        if std::mem::take(&mut self.raise) {
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::new("places")));
        }
        let screen = ctx.content_rect();
        // floater_places.xml: 333 x 588 (200 x 200 at least)
        let size = Vec2::new(333.0, 588.0);
        let pos = screen.center() - size * 0.5 + Vec2::new(120.0, 0.0);
        let mut floater = Floater::new("places", "Lieux", pos, size);
        floater.min_size = Vec2::new(260.0, 300.0);
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

    fn show_confirm(&mut self, ctx: &egui::Context, p: &Palette, actions: &mut Vec<PlacesAction>) {
        let Some(confirm) = &self.confirm else {
            return;
        };
        let (title, text, yes) = match confirm {
            Confirm::Landmark { name, .. } => (
                "Téléportation",
                format!("Souhaitez-vous vraiment vous téléporter vers {name} ?"),
                "Téléporter",
            ),
            Confirm::History { title, .. } => ("Téléportation", format!("Vous téléporter vers {title} ?"), "Téléporter"),
            Confirm::Clear => (
                "Historique de téléportation",
                "Voulez-vous vraiment supprimer votre historique des téléportations ?".to_owned(),
                "OK",
            ),
        };
        let mut answer = None;
        let modal = egui::Modal::new(egui::Id::new("places_confirm")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.label(RichText::new(title).size(15.0).strong().color(p.ink));
            ui.add_space(6.0);
            ui.label(RichText::new(text).size(12.5).color(p.muted));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if widgets::flat_button(ui, p, yes).clicked() {
                    answer = Some(true);
                }
                if widgets::flat_button(ui, p, "Annuler").clicked() {
                    answer = Some(false);
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(false);
        }
        if let Some(yes) = answer {
            if yes && let Some(c) = self.confirm.take() {
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
                    .hint_text("Filtrer les lieux")
                    .desired_width(w),
            );
            widgets::icon_menu(ui, p, "gear-six", "Afficher les options", |ui| {
                if history {
                    self.history_item_menu(ui, p, world, actions);
                } else {
                    self.landmark_gear_menu(ui, p, world, by_date, actions);
                }
            });
            widgets::icon_menu(ui, p, "sort-ascending", "Afficher les options de tri", |ui| {
                if history {
                    self.history_gear_menu(ui, p, world);
                } else {
                    self.sorting_menu(ui, p, world, by_date, actions);
                }
            });
            if !history {
                widgets::icon_menu(ui, p, "plus", "Ajouter un nouveau point de repère ou un nouveau dossier", |ui| {
                    super::menu::todo(ui, p, "folder", "Ajouter un dossier");
                    super::menu::todo(ui, p, "map-pin", "Ajouter un repère");
                });
                widgets::icon_button(ui, p, "trash", super::menu::NOT_YET, false);
            }
        });
        ui.add_space(4.0);
        let before = self.tab;
        widgets::tabs(
            ui,
            p,
            &mut self.tab,
            &[("Favoris", true), ("Repères", true), ("Historique de teleport.", true)],
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
            ui.label(RichText::new(place_details::LOADING).size(12.0).color(p.muted));
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
                place_details::LOADING
            } else if self.tab == TAB_FAVORITES {
                // FavoritesNoMatchingItems
                "Faites glisser un repère ici pour l'ajouter à vos Favoris."
            } else {
                // PlacesNoMatchingItems (the search is not ported yet)
                "Vous n'avez pas trouvé ce que vous cherchiez ? Essayez Rechercher."
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
                Some(true) => "folder-open",
                Some(false) => "folder",
                None => "map-pin",
            };
            let r = list_row(ui, p, row.depth, row.folder, icon, &row.name, self.selected == Some(sel), false);
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

    /// « Développer / Réduire tous les dossiers » and « Trier par date »
    /// (menu_places_gear_sorting.xml, the end of the gear menus).
    fn expand_collapse_sort(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, by_date: bool, actions: &mut Vec<PlacesAction>) {
        let folders = Self::all_folders(world, self.current_root(world));
        let any_closed = folders.iter().any(|f| !self.open_folders.contains(f));
        let any_open = folders.iter().any(|f| self.open_folders.contains(f));
        if super::menu::item_if(ui, p, "arrows-out-simple", "Développer tous les dossiers", any_closed) {
            self.open_folders.extend(folders.iter().copied());
        }
        if super::menu::item_if(ui, p, "arrows-in-simple", "Réduire tous les dossiers", any_open) {
            self.open_folders.clear();
        }
        // disabled for Favorites: they keep their own order (EXT-1758)
        if super::menu::check(ui, p, "calendar-dots", "Trier par date", by_date, self.tab == TAB_LANDMARKS) {
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
        if super::menu::item(ui, p, "navigation-arrow", "Téléporter") {
            self.confirm = Some(Confirm::Landmark {
                asset: it.asset_id,
                name: it.name.clone(),
            });
        }
        if super::menu::item_if(ui, p, "map-trifold", "Afficher sur la carte", resolved.is_some())
            && let Some(g) = resolved
        {
            actions.push(PlacesAction::ShowOnMap(g));
        }
        super::menu::todo(ui, p, "share-network", "Partager");
        if super::menu::item(ui, p, "info", "Plus d'informations") {
            actions.push(landmark_profile(&it));
        }
        let in_favorites = system_folder(&world.inventory, FOLDER_FAVORITES).is_some_and(|f| it.parent == f);
        if in_favorites {
            super::menu::todo(ui, p, "map-pin", "Déplacer vers Repères");
        } else {
            super::menu::todo(ui, p, "star", "Déplacer vers Favoris");
        }
        super::menu::separator(ui, p);
        super::menu::todo(ui, p, "map-pin", "Créer un repère");
        super::menu::todo(ui, p, "folder", "Créer un dossier");
        super::menu::separator(ui, p);
        super::menu::todo(ui, p, "scissors", "Couper");
        super::menu::todo(ui, p, "copy", "Copier le repère");
        let slurl = resolved.and_then(|g| slurl_at(world, g, ""));
        if super::menu::item_if(ui, p, "link", "Copier la SLurl", slurl.is_some())
            && let Some(s) = slurl
        {
            ui.ctx().copy_text(s);
        }
        super::menu::todo(ui, p, "clipboard-text", "Coller");
        super::menu::todo(ui, p, "pencil-simple", "Renommer");
        super::menu::todo(ui, p, "trash", "Supprimer");
        super::menu::separator(ui, p);
        self.expand_collapse_sort(ui, p, world, by_date, actions);
        super::menu::separator(ui, p);
        super::menu::todo(ui, p, "star", "Créer un favori");
    }

    /// menu_places_gear_folder.xml.
    fn folder_menu(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, by_date: bool, actions: &mut Vec<PlacesAction>) {
        super::menu::todo(ui, p, "map-pin", "Ajouter un repère");
        super::menu::todo(ui, p, "folder", "Ajouter un dossier");
        super::menu::separator(ui, p);
        super::menu::todo(ui, p, "scissors", "Couper");
        super::menu::todo(ui, p, "copy", "Copier");
        super::menu::todo(ui, p, "clipboard-text", "Coller");
        super::menu::todo(ui, p, "pencil-simple", "Renommer");
        super::menu::todo(ui, p, "trash", "Supprimer");
        super::menu::separator(ui, p);
        let folder = match self.selected {
            Some(Sel::Folder(f)) => Some(f),
            _ => None,
        };
        let open = folder.is_some_and(|f| self.open_folders.contains(&f));
        if super::menu::item_if(ui, p, "caret-down", "Développer", folder.is_some() && !open)
            && let Some(f) = folder
        {
            self.open_folders.insert(f);
        }
        if super::menu::item_if(ui, p, "caret-right", "Réduire", open)
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
                "L'historique de téléportation est vide. Essayez La recherche."
            } else {
                "Vous n'avez pas trouvé ce que vous cherchiez ? Essayez La recherche."
            };
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.muted)).wrap());
            return;
        }
        for (section, indices) in sections {
            // every section opens while filtering
            let open = !filter.is_empty() || !self.collapsed.contains(&section);
            if section_header(ui, p, tphistory::HISTORY_SECTIONS[section], open).clicked() && filter.is_empty() {
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
                let r = list_row(ui, p, 0, None, "map-pin", &it.title, self.selected == Some(Sel::History(i)), true);
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
        if super::menu::item_if(ui, p, "navigation-arrow", "Téléporter", some)
            && let Some((_, it)) = &sel
        {
            self.confirm = Some(Confirm::History {
                global: it.global,
                title: it.title.clone(),
            });
        }
        if super::menu::item_if(ui, p, "info", "Plus d'informations", some)
            && let Some((_, it)) = &sel
        {
            actions.push(history_profile(it));
        }
        if super::menu::item_if(ui, p, "map-trifold", "Afficher sur la carte", some)
            && let Some((_, it)) = &sel
        {
            actions.push(PlacesAction::ShowOnMap(it.global));
        }
        let slurl = sel.as_ref().and_then(|(_, it)| slurl_at(world, it.global, &it.title));
        if super::menu::item_if(ui, p, "link", "Copier la SLurl", slurl.is_some())
            && let Some(s) = slurl
        {
            ui.ctx().copy_text(s);
        }
        if super::menu::item_if(ui, p, "trash", "Supprimer de l'historique", some)
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
        if super::menu::item_if(ui, p, "arrows-out-simple", "Développer tous les dossiers", any_closed) {
            self.collapsed.clear();
        }
        if super::menu::item_if(ui, p, "arrows-in-simple", "Réduire tous les dossiers", any_open) {
            self.collapsed.extend(0..tphistory::HISTORY_SECTIONS.len());
            self.selected = None;
        }
        // FIRE-31025
        if super::menu::item_if(
            ui,
            p,
            "trash",
            "Effacer l'historique des téléportations",
            !world.tp_storage.items.is_empty(),
        ) {
            self.confirm = Some(Confirm::Clear);
        }
    }

    /// Se téléporter / Carte / Profil (FIRE-31033), for the selection.
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
            if place_details::button(ui, p, "Se téléporter", "Se téléporter à l'endroit de son choix", any, w) {
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
                "Carte",
                "Afficher l'emplacement correspondant sur la carte",
                map_target.is_some(),
                w,
            ) && let Some(g) = map_target
            {
                actions.push(PlacesAction::ShowOnMap(g));
            }
            if place_details::button(ui, p, "Profil", "Afficher le profil du lieu", any, w) {
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
            if widgets::icon_button(ui, p, "caret-left", "Retour", true) {
                actions.push(PlacesAction::CloseProfile);
            }
            ui.label(RichText::new(place_details::header_title(place)).size(16.0).strong().color(p.ink));
        });
        ui.add_space(4.0);
        let h = (ui.available_height() - 32.0).max(40.0);
        egui::ScrollArea::vertical()
            .id_salt(("places-profile", place.serial))
            .max_height(h)
            .min_scrolled_height(h)
            .auto_shrink([false, false])
            .show(ui, |ui| place_details::profile_body(ui, c, place));
        ui.add_space(6.0);
        // updateVerbs with a profile shown: Se téléporter, Carte and the
        // overflow menu (Profil hidden)
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let w = ((ui.available_width() - 30.0) / 2.0).max(60.0);
            if place_details::button(
                ui,
                p,
                "Se téléporter",
                "Se téléporter à l'endroit de son choix",
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
                "Carte",
                "Afficher l'emplacement correspondant sur la carte",
                place.global.is_some(),
                w,
            ) && let Some(g) = place.global
            {
                actions.push(PlacesAction::ShowOnMap(g));
            }
            widgets::icon_menu(ui, p, "caret-up", "Afficher les options supplémentaires", |ui| {
                place_details::overflow_menu(ui, p, world, place);
            });
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

/// One line of a list: indentation, caret for a folder, icon, name; the
/// history lines show an "i" button while hovered.
#[allow(clippy::too_many_arguments)]
fn list_row(
    ui: &mut egui::Ui,
    p: &Palette,
    depth: usize,
    folder: Option<bool>,
    icon: &str,
    name: &str,
    selected: bool,
    info_button: bool,
) -> RowResponse {
    let w = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(w, 20.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 2.0, p.violet.gamma_multiply(0.28));
    } else if response.hovered() {
        painter.rect_filled(rect, 2.0, p.raised);
    }
    let mut x = rect.left() + 4.0 + depth as f32 * 14.0;
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    let mut toggle = false;
    if let Some(open) = folder {
        let caret = egui::Rect::from_center_size(egui::pos2(x + 6.0, rect.center().y), Vec2::splat(12.0));
        if let Some(t) = super::icons::global(if open { "caret-down" } else { "caret-right" }) {
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
    let mut right = rect.right() - 4.0;
    let mut info = false;
    if info_button && (response.hovered() || selected) {
        let r = egui::Rect::from_center_size(egui::pos2(right - 8.0, rect.center().y), Vec2::splat(16.0));
        let hovered = response.hover_pos().is_some_and(|pos| r.contains(pos));
        if let Some(t) = super::icons::global("info") {
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
    ui.painter().rect_filled(rect, 2.0, fill);
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    if let Some(t) = super::icons::global(if open { "caret-down" } else { "caret-right" }) {
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

    fn folder(id: u128, parent: u128, name: &str, t: i32) -> InvFolder {
        InvFolder {
            id: Uuid::from_u128(id),
            parent: Uuid::from_u128(parent),
            name: name.into(),
            type_default: t,
            version: 1,
            favorite: false,
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
            creator: Uuid::nil(),
            created_at: created,
            owner: Uuid::nil(),
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
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
        let root = folder(1, 0, "Mon inventaire", 8);
        let lms = folder(10, 1, "Repères", 3);
        let shops = folder(20, 10, "Boutiques", -1);
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
        assert_eq!(names(&rows), ["Boutiques", "Vide", "Plage", "Musée", "Arbre"]);
        assert_eq!(rows[0].folder, Some(false));
        // by name, and an opened folder shows its landmarks one level down
        let open: HashSet<Uuid> = [Uuid::from_u128(20)].into();
        let rows = landmark_rows(&inv, root, "", false, &open, true);
        assert_eq!(names(&rows), ["Boutiques", "Boutique du Loup", "Vide", "Arbre", "Musée", "Plage"]);
        assert_eq!(rows[1].depth, 1);
        // Favorites-style: empty folders are not shown
        let rows = landmark_rows(&inv, root, "", true, &none, false);
        assert_eq!(names(&rows), ["Boutiques", "Plage", "Musée", "Arbre"]);
    }

    #[test]
    fn filter_keeps_matching_landmarks_and_their_folders() {
        let inv = inventory();
        let root = system_folder(&inv, FOLDER_LANDMARKS).expect("landmarks folder");
        let rows = landmark_rows(&inv, root, "LOUP", true, &HashSet::new(), true);
        assert_eq!(names(&rows), ["Boutiques", "Boutique du Loup"]);
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
            item("Hier", now - DAY),
            item("Matin", now - 2.0 * 3600.0),
            item("Midi", now - 60.0),
        ];
        let s = history_sections(&items, "", now);
        assert_eq!(s, vec![(0, vec![4, 3]), (1, vec![2]), (2, vec![1]), (8, vec![0])]);
        let s = history_sections(&items, "MAT", now);
        assert_eq!(s, vec![(0, vec![3])]);
    }

    #[test]
    fn tab_names() {
        assert_eq!(tab_from_name("Favoris"), Some(TAB_FAVORITES));
        assert_eq!(tab_from_name("reperes"), Some(TAB_LANDMARKS));
        assert_eq!(tab_from_name("historique"), Some(TAB_HISTORY));
        assert_eq!(tab_from_name("autre"), None);
    }
}
