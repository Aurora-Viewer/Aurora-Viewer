//! Aurora's "Places" (Preferences › Aurora › Places), the mockup chosen by
//! the human on 2026-10-19 (variant A, "list and preview"). Narrow, as it
//! opens: a search, the Favorites / Landmarks / History tabs, the landmark
//! folders in chips and rows that open in place. Widened: a rail of sources
//! and landmark folders, the list, and the preview of the selected place.
//!
//! Same data and rules as the Legacy Viewer's tabs (`places.rs`, LLPanelPlaces,
//! LLLandmarksPanel, LLTeleportHistoryPanel, LLPanelPlaceProfile,
//! LLPanelLandmarkInfo, indra/newview, originally LGPL 2.1): the Favorites and
//! Landmarks folders, the saved teleport history, the parcel of a place asked
//! with RemoteParcelRequest / ParcelInfoRequest for the one previewed (and,
//! for its snapshot only, each row on screen without a thumbnail,
//! `world/place_snapshots.rs`), the TeleportFromLandmark / TeleportToHistoryEntry confirmations, the
//! favourites bar's order. What Aurora adds is drawn from those: a landmark
//! and its copy in Favorites are one place with a star (the star copies the
//! landmark into Favorites as "add_to_favbar" does, and takes the copy away),
//! its last visit comes from the history, and the notes and the folder of a
//! landmark are edited where they are shown.

use super::model::{self, Catalog, Key, Place, PlacesSort};
use super::{Confirm, PlacesAction, PlacesUi, history_sections, now_secs, slurl_at, system_folder};
use crate::theme::Palette;
use crate::ui::place_card::{self, RowSize, Thumb};
use crate::ui::place_details::{self, Ctx};
use crate::ui::{menu, widgets};
use crate::world::World;
use crate::world::inventory::Inventory;
use crate::world::inventory::actions;
use crate::world::landmarks::{Landmarks, Resolved};
use crate::world::place_details::{Source as Profile, Stage};
use crate::world::place_snapshots::Spot;
use crate::world::tphistory::{self, HistoryItem};
use aurora_llsd::llsd_map;
use aurora_net::inventory::operations::{CopyItem, Mutation, Operation};
use egui::{Color32, FontId, Pos2, Rect, Sense, Vec2};
use glam::DVec3;
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

/// From this width the window shows the rail and the preview.
pub const WIDE: f32 = 760.0;
const RAIL_W: f32 = 210.0;
const PREVIEW_W: f32 = 316.0;
const TOOLBAR_H: f32 = 52.0;
const FOOTER_H: f32 = 38.0;
/// The last visit's column of a wide row.
const VISIT_W: f32 = 112.0;
/// FT_TRASH.
const FOLDER_TRASH: i32 = 14;
/// DB_INV_ITEM_DESC_STR_LEN - 1: the longest notes a landmark keeps.
const NOTE_MAX: usize = 127;

/// Inventory generation, landmark assets version, history length and last
/// date, regions known to the map.
type CatalogKey = (u64, u64, usize, u64, usize);

/// What the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    /// Every landmark, then the places only the history knows (narrow:
    /// the Landmarks tab, landmarks only).
    #[default]
    All,
    Favorites,
    History,
    /// One folder of the Landmarks tree (narrow: its chip).
    Folder(Uuid),
}

/// Aurora's state of the window.
#[derive(Default)]
pub struct AuroraState {
    pub source: Source,
    selected: Option<Key>,
    /// The place profile asked for the preview (OpenProfile sent).
    previewed: Option<Profile>,
    /// The notes being typed, for that place.
    note: Option<(Key, String)>,
    /// The new name being typed.
    rename: Option<(Key, String)>,
    /// "New folder": the name typed.
    new_folder: Option<String>,
    /// A folder of the rail being renamed.
    rename_folder: Option<(Uuid, String)>,
    /// The places, and what they were built from.
    catalog: Option<(CatalogKey, Arc<Catalog>)>,
    /// AURORA_DEMO_PLACES=…:open: the first place selected (a narrow row
    /// opened) once the list has one.
    pub demo_open: bool,
}

impl AuroraState {
    /// `PlacesUi::open_tab` in Aurora's terms.
    pub fn open_tab(&mut self, tab: usize) {
        self.source = match tab {
            super::TAB_FAVORITES => Source::Favorites,
            super::TAB_HISTORY => Source::History,
            _ => Source::All,
        };
        self.selected = None;
    }
}

/// What the app gives the window besides the world.
pub struct Env<'a> {
    pub sort: PlacesSort,
    /// The Favorites folder's items in the bar's order (`world/favorites.rs`).
    pub favorites: &'a [Uuid],
}

/// The profile the preview asks for a place.
fn profile_of(place: &Place, inv: &Inventory, history: &[HistoryItem]) -> Option<Profile> {
    if let Some(item) = place.item {
        let asset = inv.items.get(&item).map_or(Uuid::nil(), |it| it.asset_id);
        return Some(Profile::Landmark { item, asset });
    }
    let entry = history.get(place.history?)?;
    Some(Profile::History {
        title: entry.title.clone(),
        global: entry.global,
    })
}

/// Where the parcel snapshot of a row without a thumbnail comes from: its
/// landmark's point (shared with the favourites bar), or the spot a history
/// entry arrived at; with the region id, nil when unknown.
fn snapshot_spot(place: &Place, landmarks: &Landmarks) -> Option<(Spot, DVec3, Uuid)> {
    match place.asset {
        Some(asset) => match landmarks.peek(asset) {
            Resolved::Ready { global, region_id, .. } => Some((Spot::Landmark(asset), global, region_id)),
            _ => None,
        },
        None => place.global.map(|g| (Spot::at(g), g, Uuid::nil())),
    }
}

/// A one-line text cut with "…" at `width`.
fn line(ui: &egui::Ui, s: &str, size: f32, color: Color32, strong: bool, width: f32) -> Arc<egui::Galley> {
    let family = if strong {
        crate::ui::fonts::semibold(ui.ctx())
    } else {
        egui::FontFamily::Proportional
    };
    let mut job = egui::text::LayoutJob::single_section(s.to_owned(), egui::TextFormat::simple(FontId::new(size, family), color));
    job.wrap = egui::text::TextWrapping {
        max_width: width.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.painter().layout_job(job)
}

fn paint_icon(painter: &egui::Painter, name: &str, rect: Rect, tint: Color32) {
    if let Some(t) = crate::ui::icons::global(name) {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        painter.image(t.id(), rect, uv, tint);
    }
}

/// A small caps section title ("LANDMARK FOLDERS").
fn section(ui: &egui::Ui, p: &Palette, text: &str, at: Pos2) {
    let g = line(ui, &text.to_uppercase(), 11.0, p.muted, true, 400.0);
    ui.painter().galley(at, g, p.muted);
}

/// The texture of a place's picture (its item's thumbnail, else the snapshot
/// of its parcel once the preview read it), asked for while not loaded;
/// `place_card::picture` draws the mountain without one.
fn texture<'x>(c: &mut Ctx<'x>, tex: Uuid) -> Option<&'x egui::TextureHandle> {
    if tex.is_nil() {
        return None;
    }
    c.wanted_images.insert(tex);
    let images = c.images;
    images.get(&tex)
}

/// The G / M / A badge of a region, when known.
fn badge(ui: &egui::Ui, p: &Palette, rating: Option<u8>, rect: Rect) {
    if let Some(r) = rating {
        widgets::paint_maturity_badge(
            ui.painter(),
            p,
            crate::ui::bars::maturity_name(r),
            rect,
            widgets::inner_radius(ui.visuals()),
        );
    }
}

/// "Region (x, y, z)".
fn region_line(place: &Place) -> String {
    let region = if place.region.is_empty() {
        place_details::loading().to_owned()
    } else {
        place.region.clone()
    };
    match place.region_pos() {
        Some(pos) => format!("{region} ({}, {}, {})", pos.x.round(), pos.y.round(), pos.z.round()),
        None => region,
    }
}

/// The item and its copy in Favorites, once each.
fn items_of(place: &Place) -> Vec<Uuid> {
    let mut v: Vec<Uuid> = place.item.into_iter().chain(place.favorite).collect();
    v.dedup();
    v
}

/// Patch every copy of a landmark the same way (its notes, its name).
fn patch_all(inv: &Inventory, place: &Place, body: aurora_llsd::Llsd) -> Option<Mutation> {
    let mut change = Mutation::default();
    for id in items_of(place) {
        let parent = inv.items.get(&id)?.parent;
        change.operations.push(Operation::Patch {
            id,
            folder: false,
            body: body.clone(),
        });
        change.refresh.push(parent);
    }
    change.refresh.dedup();
    (!change.operations.is_empty()).then_some(change)
}

fn no_protection() -> HashSet<Uuid> {
    HashSet::new()
}

/// The star: copy the landmark into Favorites (a no-copy one is moved
/// there), or take its favourite away: the copy goes to the Trash, a
/// landmark that only lives in Favorites goes back to Landmarks.
pub fn toggle_favorite(inv: &Inventory, place: &Place) -> Option<Mutation> {
    let none = no_protection();
    match place.favorite {
        None => {
            let favorites = system_folder(inv, model::FOLDER_FAVORITES)?;
            let item = place.item?;
            let it = inv.items.get(&item)?;
            if it.owner_mask & actions::COPY != 0 {
                Some(Mutation {
                    copies: vec![CopyItem {
                        item,
                        owner: it.owner,
                        parent: favorites,
                        name: String::new(),
                    }],
                    refresh: vec![favorites],
                    ..Default::default()
                })
            } else {
                actions::move_selection(inv, &[item], favorites, &none, &none).ok()
            }
        }
        Some(fav) if place.item.is_some_and(|i| i != fav) => {
            let trash = system_folder(inv, FOLDER_TRASH)?;
            actions::move_selection(inv, &[fav], trash, &none, &none).ok()
        }
        Some(fav) => {
            let landmarks = system_folder(inv, model::FOLDER_LANDMARKS)?;
            actions::move_selection(inv, &[fav], landmarks, &none, &none).ok()
        }
    }
}

/// "Your landmark" › folder: the landmark moves there; a favourite with no
/// other copy is copied there and stays a favourite (moved when no-copy).
pub fn move_to_folder(inv: &Inventory, place: &Place, folder: Uuid) -> Option<Mutation> {
    let none = no_protection();
    if let Some(item) = place.item.filter(|i| Some(*i) != place.favorite) {
        return actions::move_selection(inv, &[item], folder, &none, &none).ok();
    }
    let fav = place.favorite?;
    let it = inv.items.get(&fav)?;
    if it.owner_mask & actions::COPY != 0 {
        Some(Mutation {
            copies: vec![CopyItem {
                item: fav,
                owner: it.owner,
                parent: folder,
                name: String::new(),
            }],
            refresh: vec![folder],
            ..Default::default()
        })
    } else {
        actions::move_selection(inv, &[fav], folder, &none, &none).ok()
    }
}

/// "Delete landmark": every copy goes to the Trash (LLInventoryAction
/// "delete", which the Legacy Viewer's landmark menu runs).
pub fn delete_place(inv: &Inventory, place: &Place) -> Option<Mutation> {
    let trash = system_folder(inv, FOLDER_TRASH)?;
    let none = no_protection();
    actions::move_selection(inv, &items_of(place), trash, &none, &none).ok()
}

/// The lines of the list: a place, or a section title of the history.
/// A frame's list is short-lived: its places stay inline.
#[allow(clippy::large_enum_variant)]
enum Line {
    Title(&'static str),
    Place(Key, Place),
}

/// What a row answered: the row's own response, and its star's click.
struct RowOut {
    resp: egui::Response,
    star: bool,
}

impl PlacesUi {
    /// The body of Aurora's window.
    pub(super) fn aurora_body(&mut self, ui: &mut egui::Ui, c: &mut Ctx, env: &Env, actions: &mut Vec<PlacesAction>) {
        let world = c.world;
        let history = &world.tp_storage.items;
        // rebuilt only when the inventory, a landmark asset, the history or
        // the map's regions change: a long history against many landmarks
        // is not matched again every frame
        let key = (
            world.inventory.generation,
            world.landmarks.version,
            history.len(),
            history.last().map_or(0, |h| h.date.to_bits()),
            world.map.sims.len(),
        );
        let cat = match &self.aurora.catalog {
            Some((k, cat)) if *k == key => Arc::clone(cat),
            _ => {
                let cat = Arc::new(model::catalog(&world.inventory, &world.landmarks, &world.map, history, true));
                self.aurora.catalog = Some((key, Arc::clone(&cat)));
                cat
            }
        };
        let cat = &*cat;
        self.want_landmarks.extend(cat.wanted_assets.iter().copied());
        self.want_cells.extend(cat.wanted_cells.iter().copied());
        // both folders are read whole, as the Legacy Viewer has them after
        // its background fetch
        for kind in [model::FOLDER_LANDMARKS, model::FOLDER_FAVORITES] {
            if let Some(root) = system_folder(&world.inventory, kind) {
                self.want_fetch(&world.inventory, root, true, 0);
            }
        }
        // a profile opened from elsewhere (a place or parcel link): the
        // preview shows it
        let link = world
            .place_details
            .panel
            .as_ref()
            .filter(|p| matches!(p.source, Profile::Link { .. } | Profile::Parcel(_)))
            .map(|p| p.source.clone());
        if let Some(l) = &link
            && self.aurora.previewed.as_ref() != Some(l)
        {
            self.aurora.selected = None;
            self.aurora.previewed = Some(l.clone());
        }
        // a selection whose place went away (deleted, moved out)
        if let Some(k) = self.aurora.selected
            && cat.place(k, history).is_none()
        {
            self.aurora.selected = None;
        }
        // a folder of the rail that went away
        if let Source::Folder(f) = self.aurora.source
            && !world.inventory.folders.contains_key(&f)
        {
            self.aurora.source = Source::All;
        }
        let lines = self.lines(cat, history, env);
        let wide = ui.max_rect().width() >= WIDE;
        // (a landmark: the history's places come before the folders are read)
        if self.aurora.demo_open
            && let Some(Line::Place(k, place)) = lines.iter().find(|l| match l {
                Line::Place(Key::Landmark(_), _) => true,
                Line::Place(Key::Visit(_), _) => self.aurora.source == Source::History,
                Line::Title(_) => false,
            })
        {
            self.aurora.demo_open = false;
            self.select(*k, place, &world.inventory, history, actions);
        }
        if wide && self.aurora.selected.is_none() && link.is_none() {
            // the preview always shows something: the first place
            if let Some(Line::Place(k, place)) = lines.iter().find(|l| matches!(l, Line::Place(..))) {
                self.select(*k, place, &world.inventory, history, actions);
            }
        }
        if wide {
            self.wide(ui, c, env, cat, &lines, link.is_some(), actions);
        } else {
            self.narrow(ui, c, env, cat, &lines, link.is_some(), actions);
        }
        self.folder_dialogs(ui.ctx(), c.p, &world.inventory, actions);
    }

    /// Select a line and ask for its profile.
    fn select(&mut self, key: Key, place: &Place, inv: &Inventory, history: &[HistoryItem], actions: &mut Vec<PlacesAction>) {
        self.aurora.selected = Some(key);
        if self.aurora.note.as_ref().is_some_and(|(k, _)| *k != key) {
            self.aurora.note = None;
        }
        if self.aurora.rename.as_ref().is_some_and(|(k, _)| *k != key) {
            self.aurora.rename = None;
        }
        let profile = profile_of(place, inv, history);
        if profile != self.aurora.previewed {
            match &profile {
                Some(s) => actions.push(PlacesAction::OpenProfile(s.clone())),
                None => actions.push(PlacesAction::CloseProfile),
            }
            self.aurora.previewed = profile;
        }
    }

    /// The lines of the source shown, filtered and in order.
    fn lines(&self, cat: &Catalog, history: &[HistoryItem], env: &Env) -> Vec<Line> {
        let filter = self.filter.trim();
        let landmark = |p: &Place| Line::Place(Key::Landmark(p.asset.unwrap_or_default()), p.clone());
        match self.aurora.source {
            Source::Favorites => model::favorites(cat, env.favorites)
                .into_iter()
                .filter(|p| p.matches(filter))
                .map(|p| landmark(&p))
                .collect(),
            Source::History => {
                let mut out = Vec::new();
                for (section, indices) in history_sections(history, "", now_secs()) {
                    let mut first = true;
                    for i in indices {
                        let Some(place) = cat.place(Key::Visit(i), history) else { continue };
                        if !place.matches(filter) && !tphistory::history_matches(&history[i].title, filter) {
                            continue;
                        }
                        if std::mem::take(&mut first) {
                            out.push(Line::Title(tphistory::history_sections()[section]));
                        }
                        out.push(Line::Place(Key::Visit(i), place));
                    }
                }
                out
            }
            Source::All | Source::Folder(_) => {
                let folder = match self.aurora.source {
                    Source::Folder(f) => Some(f),
                    _ => None,
                };
                let mut places: Vec<Place> = cat
                    .landmarks
                    .iter()
                    .filter(|p| folder.is_none_or(|f| p.folder == Some(f)) && p.matches(filter))
                    .cloned()
                    .collect();
                model::sort(&mut places, env.sort);
                let mut out: Vec<Line> = places.iter().map(landmark).collect();
                if folder.is_none() {
                    out.extend(
                        cat.visits
                            .iter()
                            .filter(|(_, p)| p.matches(filter))
                            .map(|(i, p)| Line::Place(Key::Visit(*i), p.clone())),
                    );
                }
                out
            }
        }
    }

    // ---------------------------------------------------------------- wide

    #[allow(clippy::too_many_arguments)]
    fn wide(
        &mut self,
        ui: &mut egui::Ui,
        c: &mut Ctx,
        env: &Env,
        cat: &Catalog,
        lines: &[Line],
        link: bool,
        actions: &mut Vec<PlacesAction>,
    ) {
        let p = c.p;
        let full = ui.max_rect();
        ui.allocate_rect(full, Sense::hover());
        let stroke = widgets::divider_stroke(p);
        // the toolbar: search and order
        let bar = Rect::from_min_size(full.min, Vec2::new(full.width(), TOOLBAR_H));
        ui.painter().hline(bar.x_range(), bar.bottom(), stroke);
        let mut tb = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bar.shrink2(Vec2::new(14.0, 10.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        tb.spacing_mut().item_spacing.x = 10.0;
        let sort_w = widgets::action_button_fit(&tb, "arrows-sort", self.sort_label(env.sort)) + 18.0;
        let w = (tb.available_width() - sort_w - 10.0).max(80.0);
        widgets::pills::search_box(&mut tb, p, &mut self.filter, aurora_i18n::tr!("ui-places-search"), w);
        self.sort_button(&mut tb, p, c.world, env.sort, actions);

        let body = Rect::from_min_max(Pos2::new(full.left(), bar.bottom()), full.max);
        let rail = Rect::from_min_size(body.min, Vec2::new(RAIL_W, body.height()));
        let preview = Rect::from_min_max(Pos2::new(body.right() - PREVIEW_W, body.top()), body.max);
        let list = Rect::from_min_max(Pos2::new(rail.right(), body.top()), Pos2::new(preview.left(), body.bottom()));
        ui.painter().rect_filled(rail, 0.0, p.field.gamma_multiply(0.55));
        ui.painter().vline(rail.right(), rail.y_range(), stroke);
        ui.painter().rect_filled(preview, 0.0, p.field.gamma_multiply(0.35));
        ui.painter().vline(preview.left(), preview.y_range(), stroke);

        let mut r = ui.new_child(egui::UiBuilder::new().max_rect(rail.shrink2(Vec2::new(8.0, 10.0))));
        r.set_clip_rect(rail);
        self.rail(&mut r, p, c.world, cat, env);

        let mut l = ui.new_child(egui::UiBuilder::new().max_rect(list));
        l.set_clip_rect(list);
        self.wide_list(&mut l, c, lines, actions);

        let mut v = ui.new_child(egui::UiBuilder::new().max_rect(preview.shrink(14.0)));
        v.set_clip_rect(preview);
        egui::ScrollArea::vertical()
            .id_salt("places-preview")
            .auto_shrink([false, false])
            .show(&mut v, |ui| self.preview(ui, c, cat, link, actions));
    }

    fn sort_label(&self, sort: PlacesSort) -> &'static str {
        match self.aurora.source {
            Source::Favorites => aurora_i18n::tr!("ui-places-sort-bar"),
            Source::History => aurora_i18n::tr!("ui-places-sort-newest"),
            _ => match sort {
                PlacesSort::Visit => aurora_i18n::tr!("ui-places-sort-visit"),
                PlacesSort::Name => aurora_i18n::tr!("ui-places-sort-name"),
                PlacesSort::Created => aurora_i18n::tr!("ui-places-sort-created"),
            },
        }
    }

    /// The order button and its menu; the history's offers to clear it.
    fn sort_button(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, sort: PlacesSort, actions: &mut Vec<PlacesAction>) {
        let resp = widgets::action_menu_button(ui, p, "arrows-sort", self.sort_label(sort), false, 28.0);
        menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| match self.aurora.source {
            Source::Favorites => {
                menu::check(ui, p, "star-filled", aurora_i18n::tr!("ui-places-sort-bar"), true, false);
            }
            Source::History => {
                menu::check(ui, p, "history", aurora_i18n::tr!("ui-places-sort-newest"), true, false);
                menu::separator(ui, p);
                if menu::item_if(
                    ui,
                    p,
                    "trash-filled",
                    aurora_i18n::tr!("ui-places-clear-teleport-history"),
                    !world.tp_storage.items.is_empty(),
                ) {
                    self.confirm = Some(Confirm::Clear);
                }
            }
            _ => {
                for (by, icon, label) in [
                    (PlacesSort::Visit, "clock-filled", aurora_i18n::tr!("ui-places-sort-visit")),
                    (PlacesSort::Name, "sort-ascending-2-filled", aurora_i18n::tr!("ui-places-sort-name")),
                    (PlacesSort::Created, "calendar-filled", aurora_i18n::tr!("ui-places-sort-created")),
                ] {
                    if menu::check(ui, p, icon, label, sort == by, true) && sort != by {
                        actions.push(PlacesAction::Sort(by));
                    }
                }
            }
        });
    }

    /// The rail: the three sources, then the landmark folders.
    fn rail(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, cat: &Catalog, env: &Env) {
        ui.spacing_mut().item_spacing.y = 1.0;
        let favorites = model::favorites(cat, env.favorites).len();
        let sources = [
            (
                Source::All,
                "map-pin-filled",
                p.muted,
                aurora_i18n::tr!("ui-places-all"),
                cat.landmarks.len() + cat.visits.len(),
            ),
            (
                Source::Favorites,
                "star-filled",
                p.amber,
                aurora_i18n::tr!("common-favorites"),
                favorites,
            ),
            (
                Source::History,
                "history",
                p.muted,
                aurora_i18n::tr!("ui-places-history"),
                world.tp_storage.items.len(),
            ),
        ];
        for (source, icon, tint, label, n) in sources {
            if rail_row(ui, p, icon, tint, label, Some(n), self.aurora.source == source, 0).clicked() {
                self.set_source(source);
            }
        }
        ui.add_space(14.0);
        let at = ui.cursor().min + Vec2::new(10.0, 0.0);
        section(ui, p, aurora_i18n::tr!("ui-places-folders"), at);
        ui.add_space(22.0);
        for f in model::folders(&world.inventory) {
            let n = cat.landmarks.iter().filter(|pl| pl.folder == Some(f.id)).count();
            let resp = rail_row(
                ui,
                p,
                "folder-filled",
                p.muted,
                &f.name,
                Some(n),
                self.aurora.source == Source::Folder(f.id),
                f.depth,
            );
            if resp.clicked() {
                self.set_source(Source::Folder(f.id));
            }
            menu::context_menu(&resp, p, |ui| {
                let inv = &world.inventory;
                if menu::item_if(
                    ui,
                    p,
                    "pencil-filled",
                    aurora_i18n::tr!("common-rename"),
                    actions::renameable(inv, f.id, &no_protection()),
                ) {
                    self.aurora.rename_folder = Some((f.id, f.name.clone()));
                }
                if menu::item_if(
                    ui,
                    p,
                    "trash-filled",
                    aurora_i18n::tr!("common-delete"),
                    actions::movable(inv, f.id, &no_protection()),
                ) && let Some(trash) = system_folder(inv, FOLDER_TRASH)
                    && let Ok(change) = actions::move_selection(inv, &[f.id], trash, &no_protection(), &no_protection())
                {
                    self.pending_edits.push(change);
                }
            });
        }
        if rail_row(
            ui,
            p,
            "plus",
            p.violet_light,
            aurora_i18n::tr!("ui-places-new-folder"),
            None,
            false,
            0,
        )
        .clicked()
        {
            self.aurora.new_folder = Some(String::new());
        }
    }

    fn set_source(&mut self, source: Source) {
        if self.aurora.source != source {
            self.aurora.source = source;
            self.aurora.selected = None;
        }
    }

    /// The list of the wide window: title, rows, then "Here".
    fn wide_list(&mut self, ui: &mut egui::Ui, c: &mut Ctx, lines: &[Line], actions: &mut Vec<PlacesAction>) {
        let p = c.p;
        let world = c.world;
        let full = ui.max_rect();
        let (title, hint) = match self.aurora.source {
            Source::All => (aurora_i18n::tr!("ui-places-all").to_owned(), aurora_i18n::tr!("ui-places-all-hint")),
            Source::Favorites => (
                aurora_i18n::tr!("common-favorites").to_owned(),
                aurora_i18n::tr!("ui-places-favorites-hint"),
            ),
            Source::History => (
                aurora_i18n::tr!("ui-places-history").to_owned(),
                aurora_i18n::tr!("ui-places-history-hint"),
            ),
            Source::Folder(f) => (actions::name(&world.inventory, f), aurora_i18n::tr!("ui-places-folder-hint")),
        };
        let head = Rect::from_min_size(full.min + Vec2::new(16.0, 12.0), Vec2::new(full.width() - 32.0, 20.0));
        let t = line(ui, &title, 14.0, p.ink, true, head.width());
        let tw = t.size().x;
        widgets::paint_row_text(ui.painter(), t, head.left(), head.center().y, p.ink);
        let h = line(ui, hint, 12.0, p.muted, false, (head.width() - tw - 10.0).max(10.0));
        widgets::paint_row_text(ui.painter(), h, head.left() + tw + 10.0, head.center().y, p.muted);

        let footer = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - FOOTER_H), full.max);
        let rows = Rect::from_min_max(
            Pos2::new(full.left() + 8.0, head.bottom() + 6.0),
            Pos2::new(full.right() - 8.0, footer.top()),
        );
        let mut list = ui.new_child(egui::UiBuilder::new().max_rect(rows));
        list.set_clip_rect(rows);
        egui::ScrollArea::vertical()
            .id_salt(("places-aurora-list", self.aurora.source == Source::History))
            .auto_shrink([false, false])
            .show(&mut list, |ui| self.rows(ui, c, lines, true, actions));
        self.here_bar(ui, p, world, footer, true, actions);
    }

    /// Place rows share scene cargo; only Favorites reserve a reorder grip.
    fn rows(&mut self, ui: &mut egui::Ui, c: &mut Ctx, lines: &[Line], wide: bool, actions: &mut Vec<PlacesAction>) {
        let p = c.p;
        let world = c.world;
        let history = &world.tp_storage.items;
        ui.spacing_mut().item_spacing.y = 1.0;
        if lines.is_empty() {
            self.empty(ui, p, world);
            return;
        }
        let reorder = self.aurora.source == Source::Favorites && self.filter.trim().is_empty();
        let mut rects: Vec<(Rect, Uuid)> = Vec::new();
        for l in lines {
            let (key, place) = match l {
                Line::Title(t) => {
                    ui.add_space(10.0);
                    let at = ui.cursor().min + Vec2::new(10.0, 0.0);
                    section(ui, p, t, at);
                    ui.add_space(20.0);
                    continue;
                }
                Line::Place(k, place) => (*k, place),
            };
            let selected = self.aurora.selected == Some(key);
            let spot = if place.thumbnail.is_nil() {
                snapshot_spot(place, &world.landmarks)
            } else {
                None
            };
            let snapshot = match spot {
                Some((s, ..)) => world.place_snapshots.snapshot(s, &world.profiles.parcels).unwrap_or_default(),
                None => place.thumbnail,
            };
            let out = place_row(ui, p, c, place, selected, snapshot, self.aurora.source == Source::Favorites, wide);
            let target = match key {
                Key::Visit(_) => place.global.map(super::drag::Target::History),
                Key::Landmark(_) => place.item.map(super::drag::Target::Landmark),
            };
            if let Some(target) = target {
                super::drag::source(&out.resp, || super::drag::PlaceDrag {
                    target,
                    favorite: target.item().and(place.favorite),
                    name: place.name.clone(),
                    thumbnail: texture(c, snapshot).map(egui::TextureHandle::id),
                });
            }
            // every row on screen asks for its picture, not only the one
            // previewed (#798)
            if let Some(s) = spot
                && ui.is_rect_visible(out.resp.rect)
            {
                self.want_snapshots.push(s);
            }
            if let Some(fav) = place.favorite.filter(|_| reorder) {
                rects.push((out.resp.rect, fav));
            }
            if out.resp.clicked() {
                if !wide && selected {
                    // a second click folds the row back
                    self.aurora.selected = None;
                } else {
                    self.select(key, place, &world.inventory, history, actions);
                }
            }
            if out.resp.double_clicked() {
                self.teleport(key, place, &world.inventory, history);
            }
            if out.star
                && let Some(change) = toggle_favorite(&world.inventory, place)
            {
                if place.favorite.is_none() {
                    actions.push(PlacesAction::ExpectFavorite);
                }
                actions.push(PlacesAction::Edit(change));
            }
            // asked every frame: the menu shows as long as it is open
            menu::context_menu(&out.resp, p, |ui| {
                self.select(key, place, &world.inventory, history, actions);
                self.place_menu(ui, p, world, key, place, actions);
            });
            if !wide && self.aurora.selected == Some(key) {
                self.narrow_details(ui, c, place, actions);
            }
        }
        // a favourite dropped between two others (LLFavoritesBarCtrl's drop)
        if let Some(dragged) = egui::DragAndDrop::payload::<super::drag::PlaceDrag>(ui.ctx()).and_then(|cargo| cargo.favorite)
            && reorder
        {
            let pointer = ui
                .input(|i| i.pointer.interact_pos())
                .filter(|pos| ui.rect_contains_pointer(ui.clip_rect()) && ui.min_rect().contains(*pos));
            let released = ui.input(|i| i.pointer.any_released());
            if let Some(pos) = pointer {
                let rank = rects.iter().take_while(|(r, _)| pos.y > r.center().y).count();
                let y = match rects.get(rank) {
                    Some((r, _)) => r.top(),
                    None => rects.last().map_or(ui.cursor().top(), |(r, _)| r.bottom() + 1.0),
                };
                ui.painter().hline(ui.max_rect().x_range(), y, egui::Stroke::new(2.0, p.violet));
                if released {
                    actions.push(PlacesAction::MoveFavorite(dragged, rank));
                    egui::DragAndDrop::clear_payload(ui.ctx());
                }
            }
        }
    }

    fn empty(&self, ui: &mut egui::Ui, p: &Palette, world: &World) {
        let filter = self.filter.trim();
        let loading = [model::FOLDER_LANDMARKS, model::FOLDER_FAVORITES].into_iter().any(|k| {
            system_folder(&world.inventory, k)
                .and_then(|f| world.inventory.folders.get(&f))
                .is_none_or(|f| f.state != crate::world::inventory::FetchState::Fetched)
        });
        let text = if !filter.is_empty() {
            aurora_i18n::tr!("ui-places-no-match", text = filter.to_string()).to_owned()
        } else if loading && self.aurora.source != Source::History {
            place_details::loading().to_owned()
        } else {
            match self.aurora.source {
                Source::Favorites => aurora_i18n::tr!("ui-places-no-favorites").to_owned(),
                Source::History => aurora_i18n::tr!("ui-places-teleport-history-is-empty-try-search").to_owned(),
                _ => aurora_i18n::tr!("ui-places-no-landmarks").to_owned(),
            }
        };
        ui.add_space(36.0);
        ui.vertical_centered(|ui| {
            ui.add(egui::Label::new(egui::RichText::new(text).size(12.5).color(p.muted)).wrap());
        });
    }

    /// TeleportFromLandmark for a landmark, TeleportToHistoryEntry for a
    /// place of the history. A line of the history goes back to where that
    /// visit arrived, even when it shows the landmark it was a visit of
    /// (LLTeleportHistoryStorage::goToItem, #759).
    fn teleport(&mut self, key: Key, place: &Place, inv: &Inventory, history: &[HistoryItem]) {
        let visit = match key {
            Key::Visit(i) => history.get(i),
            Key::Landmark(_) => None,
        };
        if let Some(entry) = visit {
            self.confirm = Some(Confirm::History {
                global: entry.global,
                title: entry.title.clone(),
            });
        } else if let Some(item) = place.item.and_then(|i| inv.items.get(&i)) {
            self.confirm = Some(Confirm::Landmark {
                asset: item.asset_id,
                name: item.name.clone(),
            });
        }
    }

    /// The menu of a place (its row's right click, the "…" of the preview).
    fn place_menu(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, key: Key, place: &Place, actions: &mut Vec<PlacesAction>) {
        let inv = &world.inventory;
        let history = &world.tp_storage.items;
        if menu::item(ui, p, "location-filled", aurora_i18n::tr!("common-teleport-3")) {
            self.teleport(key, place, inv, history);
        }
        if menu::item_if(ui, p, "map", aurora_i18n::tr!("common-show-on-map"), place.global.is_some())
            && let Some(g) = place.global
        {
            actions.push(PlacesAction::ShowOnMap(g));
        }
        let title = place.history.and_then(|i| history.get(i)).map_or("", |h| h.title.as_str());
        let slurl = place.global.and_then(|g| slurl_at(world, g, title));
        if menu::item_if(ui, p, "link-filled", aurora_i18n::tr!("common-copy-slurl"), slurl.is_some())
            && let Some(s) = slurl
        {
            ui.ctx().copy_text(s);
        }
        if let Some(profile) = profile_of(place, inv, history)
            && menu::item(ui, p, "info-circle-filled", aurora_i18n::tr!("common-place-profile"))
        {
            actions.push(PlacesAction::OpenWindow(profile));
        }
        if place.is_landmark() {
            menu::separator(ui, p);
            let label = if place.favorite.is_some() {
                aurora_i18n::tr!("ui-places-remove-favorite")
            } else {
                aurora_i18n::tr!("ui-places-add-favorite")
            };
            if menu::item(ui, p, "star-filled", label)
                && let Some(change) = toggle_favorite(inv, place)
            {
                if place.favorite.is_none() {
                    actions.push(PlacesAction::ExpectFavorite);
                }
                actions.push(PlacesAction::Edit(change));
            }
            let renameable = place.item.is_some_and(|i| actions::renameable(inv, i, &no_protection()));
            if menu::item_if(ui, p, "pencil-filled", aurora_i18n::tr!("common-rename"), renameable) {
                self.aurora.rename = Some((key, place.name.clone()));
            }
            menu::separator(ui, p);
            if menu::item(ui, p, "trash-filled", aurora_i18n::tr!("ui-places-delete-landmark"))
                && let Some(change) = delete_place(inv, place)
            {
                actions.push(PlacesAction::Edit(change));
                self.aurora.selected = None;
            }
        }
        if let Key::Visit(i) = key {
            if !place.is_landmark() {
                menu::separator(ui, p);
            }
            if menu::item(ui, p, "trash-filled", aurora_i18n::tr!("ui-places-remove-from-history")) {
                actions.push(PlacesAction::RemoveHistory(i));
                // the entries after it move up one index: nothing stays
                // keyed by one (#798)
                self.aurora.selected = None;
                let by_index = |k: &Key| matches!(k, Key::Visit(_));
                if self.aurora.note.as_ref().is_some_and(|(k, _)| by_index(k)) {
                    self.aurora.note = None;
                }
                if self.aurora.rename.as_ref().is_some_and(|(k, _)| by_index(k)) {
                    self.aurora.rename = None;
                }
            }
        }
    }

    /// "Here: Parcel, Region (x, y, z)" and "Create a landmark here".
    fn here_bar(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, rect: Rect, wide: bool, actions: &mut Vec<PlacesAction>) {
        ui.painter().hline(rect.x_range(), rect.top(), widgets::divider_stroke(p));
        let label = if wide {
            aurora_i18n::tr!("ui-places-create-here")
        } else {
            aurora_i18n::tr!("ui-places-create-here-short")
        };
        let bw = widgets::action_button_fit(ui, "plus", label);
        let button = Rect::from_min_size(Pos2::new(rect.right() - 12.0 - bw, rect.center().y - 13.0), Vec2::new(bw, 26.0));
        let mut b = ui.new_child(egui::UiBuilder::new().max_rect(button));
        let resp = b.add_enabled_ui(world.parcel.is_some(), |ui| {
            widgets::action_button(ui, p, "plus", label, widgets::ActionKind::Flat, Vec2::new(bw, 26.0))
        });
        if resp.inner.clicked() {
            let dest = match self.aurora.source {
                Source::Folder(f) => Some(f),
                _ => None,
            };
            actions.push(PlacesAction::CreateLandmark(dest));
        }
        // the parcel we stand on
        let mut x = rect.left() + if wide { 16.0 } else { 12.0 };
        let cy = rect.center().y;
        paint_icon(
            ui.painter(),
            "map-pin-filled",
            Rect::from_center_size(Pos2::new(x + 7.0, cy), Vec2::splat(14.0)),
            p.teal,
        );
        x += 22.0;
        let room = (button.left() - 10.0 - x).max(10.0);
        let here = aurora_i18n::tr!("ui-places-here");
        let mut job = egui::text::LayoutJob::default();
        let fmt = |color: Color32| egui::TextFormat::simple(FontId::proportional(12.0), color);
        job.append(&format!("{here} "), 0.0, fmt(p.muted));
        let parcel = if world.parcel_name.is_empty() {
            world.rlva_region_label()
        } else {
            world.rlva_parcel_label()
        };
        job.append(&parcel, 0.0, fmt(p.ink));
        let pos = world.agent.position;
        let rest = if world.rlva.has("showloc") {
            String::new()
        } else if wide {
            format!(
                ", {} ({}, {}, {})",
                world.region_name(),
                pos.x.round(),
                pos.y.round(),
                pos.z.round()
            )
        } else {
            format!(", {}", world.region_name())
        };
        job.append(&rest, 0.0, fmt(p.muted));
        job.wrap = egui::text::TextWrapping {
            max_width: room,
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let g = ui.painter().layout_job(job);
        widgets::paint_row_text(ui.painter(), g, x, cy, p.ink);
    }

    /// The preview of the wide window: the selected place, or a place link
    /// opened from elsewhere.
    fn preview(&mut self, ui: &mut egui::Ui, c: &mut Ctx, cat: &Catalog, link: bool, actions: &mut Vec<PlacesAction>) {
        let p = c.p;
        let world = c.world;
        let history = &world.tp_storage.items;
        let width = ui.available_width();
        let place = self.aurora.selected.and_then(|k| cat.place(k, history));
        let panel = world
            .place_details
            .panel
            .as_ref()
            .filter(|pl| self.aurora.previewed.as_ref() == Some(&pl.source));
        let parcel = panel.and_then(|pl| pl.parcel()).and_then(|id| world.profiles.parcels.get(&id));
        let failed = panel.and_then(|pl| match pl.stage {
            Stage::Failed(e) => Some(e),
            _ => None,
        });
        let (key, place) = match (self.aurora.selected, place) {
            (Some(k), Some(pl)) => (k, pl),
            _ if link => {
                // a place link: what the parcel says
                let Some(pl) = panel else { return };
                let place = Place {
                    name: parcel.map_or_else(|| place_details::loading().to_owned(), |d| d.name.clone()),
                    region: parcel.map(|d| d.sim_name.clone()).unwrap_or_default(),
                    global: pl.global,
                    rating: parcel.map(|d| match d.flags & 0x3 {
                        0 => 13,
                        1 => 21,
                        _ => 42,
                    }),
                    ..Default::default()
                };
                (Key::Visit(usize::MAX), place)
            }
            _ => {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(aurora_i18n::tr!("ui-places-choose")).size(12.5).color(p.muted));
                });
                return;
            }
        };
        // the picture
        let tex = if !place.thumbnail.is_nil() {
            place.thumbnail
        } else {
            parcel.map(|d| d.snapshot).unwrap_or_default()
        };
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 158.0), Sense::hover());
        place_card::picture(ui, p, texture(c, tex), rect, 40.0);
        ui.add_space(12.0);
        // name (or its edit) and region
        self.title(ui, p, world, key, &place, width, actions);
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            if let Some(r) = place.rating {
                widgets::maturity_badge_of(ui, p, r, 16.0);
            }
            ui.label(egui::RichText::new(region_line(&place)).size(12.5).color(p.muted));
        });
        ui.add_space(12.0);
        // Teleport, Map, Copy SLurl, more
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let small = 28.0;
            let map_w = widgets::action_button_fit(ui, "map", aurora_i18n::tr!("common-map"));
            let tp_w = (width - map_w - small * 2.0 - 18.0).max(90.0);
            if widgets::action_button(
                ui,
                p,
                "location-filled",
                aurora_i18n::tr!("common-teleport-3"),
                widgets::ActionKind::Primary,
                Vec2::new(tp_w, small),
            )
            .clicked()
            {
                if link {
                    if let Some(g) = place.global {
                        actions.push(PlacesAction::Teleport(g));
                    }
                } else {
                    self.teleport(key, &place, &world.inventory, history);
                }
            }
            if ui
                .add_enabled_ui(place.global.is_some(), |ui| {
                    widgets::action_button(
                        ui,
                        p,
                        "map",
                        aurora_i18n::tr!("common-map"),
                        widgets::ActionKind::Flat,
                        Vec2::new(map_w, small),
                    )
                })
                .inner
                .clicked()
                && let Some(g) = place.global
            {
                actions.push(PlacesAction::ShowOnMap(g));
            }
            let title = place.history.and_then(|i| history.get(i)).map_or("", |h| h.title.as_str());
            let slurl = place.global.and_then(|g| slurl_at(world, g, title));
            let copy = ui
                .add_enabled_ui(slurl.is_some(), |ui| {
                    widgets::action_button(ui, p, "link-filled", "", widgets::ActionKind::Flat, Vec2::splat(small))
                })
                .inner
                .on_hover_text(aurora_i18n::tr!("common-copy-slurl"));
            if copy.clicked()
                && let Some(s) = slurl
            {
                ui.ctx().copy_text(s);
            }
            if !link {
                let more = widgets::action_button(ui, p, "dots-filled", "", widgets::ActionKind::Flat, Vec2::splat(small))
                    .on_hover_text(aurora_i18n::tr!("ui-places-more"));
                menu::dropdown(&more, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    self.place_menu(ui, p, world, key, &place, actions);
                });
            }
        });
        ui.add_space(12.0);
        // the parcel's description
        let desc = match (parcel, failed) {
            (Some(d), _) if !d.desc.trim().is_empty() => Some(d.desc.clone()),
            (Some(_), _) => None,
            (None, Some(e)) => Some(place_details::error_text(e, place.is_landmark()).to_owned()),
            (None, None) if panel.is_some() => Some(place_details::loading().to_owned()),
            _ => None,
        };
        if let Some(text) = desc {
            ui.add(egui::Label::new(egui::RichText::new(text).size(12.5).color(p.ink.gamma_multiply(0.86))).wrap());
            ui.add_space(10.0);
        }
        widgets::divider(ui, p);
        ui.add_space(4.0);
        let none = || place_details::loading().to_owned();
        kv(
            ui,
            p,
            aurora_i18n::tr!("ui-places-traffic"),
            &parcel.map_or_else(none, |d| format!("{}", d.dwell as i32)),
        );
        kv(
            ui,
            p,
            aurora_i18n::tr!("ui-places-area"),
            &parcel.map_or_else(none, |d| {
                aurora_i18n::tr!("ui-places-area-value", area = d.actual_area.to_string()).to_owned()
            }),
        );
        if !link {
            let visit = place.last_visit.map_or_else(
                || aurora_i18n::tr!("ui-places-never-visited").to_owned(),
                |d| model::visit_label(d, now_secs()),
            );
            kv(ui, p, aurora_i18n::tr!("ui-places-last-visit"), &visit);
            ui.add_space(6.0);
            if place.is_landmark() {
                widgets::divider(ui, p);
                ui.add_space(8.0);
                self.landmark_section(ui, p, world, key, &place, width, actions);
            } else {
                widgets::divider(ui, p);
                ui.add_space(10.0);
                self.visit_section(ui, p, world, &place, actions);
            }
        }
    }

    /// The name of the preview, or its edit after "Rename".
    #[allow(clippy::too_many_arguments)]
    fn title(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &World,
        key: Key,
        place: &Place,
        width: f32,
        actions: &mut Vec<PlacesAction>,
    ) {
        match self.aurora.rename.as_mut().filter(|(k, _)| *k == key) {
            Some((_, name)) => {
                let r = ui.add(egui::TextEdit::singleline(name).char_limit(63).desired_width(width));
                // the field takes the keyboard when it appears, and a click
                // elsewhere then ends the edit
                let fresh = egui::Id::unique(("places-rename", key));
                if ui.data_mut(|d| d.get_temp::<bool>(fresh).is_none()) {
                    ui.data_mut(|d| d.insert_temp(fresh, true));
                    r.request_focus();
                }
                let (enter, escape) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
                if escape {
                    self.aurora.rename = None;
                } else if enter || r.lost_focus() {
                    if let Ok(name) = actions::clean_name(name)
                        && name != place.name
                        && let Some(change) = patch_all(&world.inventory, place, llsd_map! { "name" => name })
                    {
                        actions.push(PlacesAction::Edit(change));
                    }
                    self.aurora.rename = None;
                }
                if self.aurora.rename.is_none() {
                    ui.data_mut(|d| d.remove::<bool>(fresh));
                }
            }
            None => {
                let g = line(ui, &place.name, 15.0, p.ink, true, width);
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, g.size().y), Sense::click());
                ui.painter().galley(rect.min, g, p.ink);
                let renameable = place
                    .item
                    .is_some_and(|i| actions::renameable(&world.inventory, i, &no_protection()));
                if renameable && resp.double_clicked() {
                    self.aurora.rename = Some((key, place.name.clone()));
                }
            }
        }
    }

    /// "Your landmark": its folder, its notes and its date.
    #[allow(clippy::too_many_arguments)]
    fn landmark_section(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &World,
        key: Key,
        place: &Place,
        width: f32,
        actions: &mut Vec<PlacesAction>,
    ) {
        let inv = &world.inventory;
        ui.horizontal(|ui| {
            let at = ui.cursor().min + Vec2::new(0.0, 6.0);
            section(ui, p, aurora_i18n::tr!("ui-places-your-landmark"), at);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let folder_name = match place.folder {
                    Some(f) => actions::name(inv, f),
                    None => aurora_i18n::tr!("landmark-create-favorites-bar").to_owned(),
                };
                let resp = widgets::action_menu_button(ui, p, "folder-filled", &folder_name, false, 24.0);
                menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    let mut targets: Vec<(Uuid, String)> = Vec::new();
                    if let Some(root) = system_folder(inv, model::FOLDER_LANDMARKS) {
                        targets.push((root, actions::name(inv, root)));
                    }
                    targets.extend(
                        model::folders(inv)
                            .into_iter()
                            .map(|f| (f.id, format!("{}{}", "   ".repeat(f.depth + 1), f.name))),
                    );
                    for (id, name) in targets {
                        let here = place.folder == Some(id);
                        if menu::check(ui, p, "folder-filled", &name, here, true)
                            && !here
                            && let Some(change) = move_to_folder(inv, place, id)
                        {
                            actions.push(PlacesAction::Edit(change));
                        }
                    }
                });
            });
        });
        ui.add_space(8.0);
        ui.label(egui::RichText::new(aurora_i18n::tr!("ui-places-note")).size(12.0).color(p.muted));
        ui.add_space(2.0);
        if self.aurora.note.as_ref().is_none_or(|(k, _)| *k != key) {
            self.aurora.note = Some((key, place.note.clone()));
        }
        let editable = place.item.is_some_and(|i| actions::renameable(inv, i, &no_protection()));
        if let Some((_, text)) = self.aurora.note.as_mut() {
            let r = crate::ui::widgets::text_field(ui, editable, text, |b, _| {
                egui::TextEdit::multiline(b)
                    .hint_text(aurora_i18n::tr!("ui-places-note-hint"))
                    .char_limit(NOTE_MAX)
                    .desired_rows(2)
                    .desired_width(width)
            });
            // the notes are saved when the field is left
            if r.lost_focus()
                && text.trim() != place.note.trim()
                && let Some(change) = patch_all(inv, place, llsd_map! { "desc" => text.trim().to_owned() })
            {
                actions.push(PlacesAction::Edit(change));
            }
        }
        ui.add_space(6.0);
        ui.label(egui::RichText::new(model::created_label(place.created)).size(11.5).color(p.muted));
    }

    /// A place of the history without a landmark: its landmark can only be
    /// made there (createLandmarkHere).
    fn visit_section(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, place: &Place, actions: &mut Vec<PlacesAction>) {
        let there = place.global.is_some_and(|g| {
            world
                .parcel
                .as_ref()
                .zip(world.main_origin())
                .is_some_and(|(parcel, origin)| crate::world::landmarks::in_parcel(&parcel.bitmap, origin, g))
        });
        let label = aurora_i18n::tr!("ui-places-create-landmark");
        let w = widgets::action_button_fit(ui, "plus", label);
        let button = |ui: &mut egui::Ui| {
            ui.add_enabled_ui(there, |ui| {
                widgets::action_button(ui, p, "plus", label, widgets::ActionKind::Flat, Vec2::new(w, 26.0))
            })
            .inner
            .on_disabled_hover_text(aurora_i18n::tr!("ui-places-create-only-there"))
            .clicked()
        };
        let text = egui::RichText::new(aurora_i18n::tr!("ui-places-visited-no-landmark"))
            .size(12.5)
            .color(p.muted);
        let text_w = egui::WidgetText::from(text.clone())
            .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body)
            .size()
            .x;
        // side by side when both fit, else the button under the text (the
        // narrow preview column of the wide window)
        let clicked = if text_w + ui.spacing().item_spacing.x + w <= ui.available_width() {
            ui.horizontal(|ui| {
                ui.label(text);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), button).inner
            })
            .inner
        } else {
            ui.label(text);
            ui.add_space(4.0);
            button(ui)
        };
        if clicked {
            actions.push(PlacesAction::CreateLandmark(None));
        }
    }

    // -------------------------------------------------------------- narrow

    #[allow(clippy::too_many_arguments)]
    fn narrow(
        &mut self,
        ui: &mut egui::Ui,
        c: &mut Ctx,
        env: &Env,
        cat: &Catalog,
        lines: &[Line],
        link: bool,
        actions: &mut Vec<PlacesAction>,
    ) {
        let p = c.p;
        let world = c.world;
        let full = ui.max_rect();
        ui.allocate_rect(full, Sense::hover());
        let footer = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - FOOTER_H), full.max);
        let top = Rect::from_min_max(full.min, Pos2::new(full.right(), footer.top()));
        // "Here" first, on the window's own Ui: the list's is cut above it
        self.here_bar(ui, p, world, footer, false, actions);
        let mut ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(top.shrink2(Vec2::new(12.0, 10.0)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        ui.set_clip_rect(top);
        let ui = &mut ui;
        if link {
            // a place link opened from elsewhere, with a way back
            ui.horizontal(|ui| {
                if widgets::icon_button(ui, p, "caret-left-filled", aurora_i18n::tr!("common-back"), true) {
                    actions.push(PlacesAction::CloseProfile);
                    self.aurora.previewed = None;
                }
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("common-place-profile"))
                        .size(14.0)
                        .strong()
                        .color(p.ink),
                );
            });
            ui.add_space(8.0);
            egui::ScrollArea::vertical()
                .id_salt("places-link")
                .auto_shrink([false, false])
                .show(ui, |ui| self.preview(ui, c, cat, true, actions));
            return;
        }
        let w = ui.available_width();
        // in a row: the field takes the height of its line only
        ui.horizontal(|ui| widgets::pills::search_box(ui, p, &mut self.filter, aurora_i18n::tr!("ui-places-search-short"), w));
        ui.add_space(8.0);
        // the three tabs
        let favorites = model::favorites(cat, env.favorites).len();
        let tabs = [
            (Source::Favorites, aurora_i18n::tr!("common-favorites"), favorites),
            (Source::All, aurora_i18n::tr!("common-landmarks"), cat.landmarks.len()),
            (Source::History, aurora_i18n::tr!("ui-places-history"), world.tp_storage.items.len()),
        ];
        let current = match self.aurora.source {
            Source::Folder(_) => Source::All,
            s => s,
        };
        let mut chosen = tabs.iter().position(|t| t.0 == current).unwrap_or(0);
        let labels: Vec<(&str, usize)> = tabs.iter().map(|(_, label, count)| (&**label, *count)).collect();
        if widgets::count_tabs(ui, p, "places-tab", &mut chosen, &labels) {
            self.set_source(tabs[chosen].0);
        }
        ui.add_space(8.0);
        // the folders of the Landmarks tab, in chips
        if current == Source::All {
            egui::ScrollArea::horizontal()
                .id_salt("places-chips")
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if widgets::pills::filter_pill(ui, p, "", aurora_i18n::tr!("ui-places-chip-all"), self.aurora.source == Source::All)
                            .clicked()
                        {
                            self.set_source(Source::All);
                        }
                        for f in model::folders(&world.inventory) {
                            let on = self.aurora.source == Source::Folder(f.id);
                            if widgets::pills::filter_pill(ui, p, "", &f.name, on).clicked() {
                                self.set_source(Source::Folder(f.id));
                            }
                        }
                    });
                });
            ui.add_space(8.0);
        }
        // the narrow list keeps the landmarks of the Landmarks tab only
        let shown: Vec<Line> = lines
            .iter()
            .filter(|l| !matches!(l, Line::Place(Key::Visit(_), _)) || self.aurora.source == Source::History)
            .map(|l| match l {
                Line::Title(t) => Line::Title(t),
                Line::Place(k, pl) => Line::Place(*k, pl.clone()),
            })
            .collect();
        egui::ScrollArea::vertical()
            .id_salt(("places-narrow", self.aurora.source == Source::History))
            .auto_shrink([false, false])
            .show(ui, |ui| self.rows(ui, c, &shown, false, actions));
    }

    /// A row opened in place: the description, the notes and the actions.
    fn narrow_details(&mut self, ui: &mut egui::Ui, c: &mut Ctx, place: &Place, actions: &mut Vec<PlacesAction>) {
        let p = c.p;
        let world = c.world;
        let history = &world.tp_storage.items;
        let panel = world
            .place_details
            .panel
            .as_ref()
            .filter(|pl| self.aurora.previewed.as_ref() == Some(&pl.source));
        let parcel = panel.and_then(|pl| pl.parcel()).and_then(|id| world.profiles.parcels.get(&id));
        let failed = panel.and_then(|pl| match pl.stage {
            Stage::Failed(e) => Some(e),
            _ => None,
        });
        let key = self.aurora.selected.unwrap_or(Key::Visit(usize::MAX));
        let left = ui.max_rect().left() + 68.0;
        let width = (ui.max_rect().right() - 10.0 - left).max(60.0);
        let rect = Rect::from_min_size(Pos2::new(left, ui.cursor().top() + 2.0), Vec2::new(width, 400.0));
        let mut d = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        d.spacing_mut().item_spacing.y = 8.0;
        let text = match (parcel, failed) {
            (Some(d), _) if !d.desc.trim().is_empty() => d.desc.clone(),
            (Some(_), _) => String::new(),
            (None, Some(e)) => place_details::error_text(e, place.is_landmark()).to_owned(),
            (None, None) => place_details::loading().to_owned(),
        };
        if !text.is_empty() {
            d.add(egui::Label::new(egui::RichText::new(text).size(12.0).color(p.ink.gamma_multiply(0.86))).wrap());
        }
        if !place.note.trim().is_empty() {
            d.label(
                egui::RichText::new(aurora_i18n::tr!("ui-places-note-line", note = place.note.trim().to_string()))
                    .size(12.0)
                    .color(p.muted),
            );
        }
        d.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let tp = aurora_i18n::tr!("common-teleport-3");
            let tw = widgets::action_button_fit(ui, "location-filled", tp);
            if widgets::action_button(ui, p, "location-filled", tp, widgets::ActionKind::Primary, Vec2::new(tw, 26.0)).clicked() {
                self.teleport(key, place, &world.inventory, history);
            }
            let mw = widgets::action_button_fit(ui, "map", aurora_i18n::tr!("common-map"));
            if ui
                .add_enabled_ui(place.global.is_some(), |ui| {
                    widgets::action_button(
                        ui,
                        p,
                        "map",
                        aurora_i18n::tr!("common-map"),
                        widgets::ActionKind::Flat,
                        Vec2::new(mw, 26.0),
                    )
                })
                .inner
                .clicked()
                && let Some(g) = place.global
            {
                actions.push(PlacesAction::ShowOnMap(g));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let more = widgets::action_button(ui, p, "dots-filled", "", widgets::ActionKind::Flat, Vec2::splat(26.0))
                    .on_hover_text(aurora_i18n::tr!("ui-places-more"));
                menu::dropdown(&more, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    self.place_menu(ui, p, world, key, place, actions);
                });
            });
        });
        // the rename asked from the menu, under the actions
        if self.aurora.rename.as_ref().is_some_and(|(k, _)| *k == key) {
            self.title(&mut d, p, world, key, place, width, actions);
        }
        let used = d.min_rect().height() + 12.0;
        ui.add_space(used);
    }

    /// "New folder" and "Rename" of a folder of the rail.
    fn folder_dialogs(&mut self, ctx: &egui::Context, p: &Palette, inv: &Inventory, actions: &mut Vec<PlacesAction>) {
        for change in self.pending_edits.drain(..) {
            actions.push(PlacesAction::Edit(change));
        }
        if let Some(name) = self.aurora.new_folder.as_mut() {
            match name_dialog(ctx, p, "places_new_folder", aurora_i18n::tr!("ui-places-new-folder-prompt"), name) {
                Some(true) => {
                    if let Some(root) = system_folder(inv, model::FOLDER_LANDMARKS)
                        && let Ok(change) = actions::create_folder(root, name)
                    {
                        actions.push(PlacesAction::Edit(change));
                    }
                    self.aurora.new_folder = None;
                }
                Some(false) => self.aurora.new_folder = None,
                None => {}
            }
        }
        if let Some((id, name)) = self.aurora.rename_folder.as_mut() {
            match name_dialog(ctx, p, "places_rename_folder", aurora_i18n::tr!("common-rename"), name) {
                Some(true) => {
                    if let Ok(clean) = actions::clean_name(name)
                        && let Ok(change) = actions::patch(inv, *id, llsd_map! { "name" => clean })
                    {
                        actions.push(PlacesAction::Edit(change));
                    }
                    self.aurora.rename_folder = None;
                }
                Some(false) => self.aurora.rename_folder = None,
                None => {}
            }
        }
    }
}

/// A modal asking for a name: Some(true) on OK / Enter, Some(false) on
/// Cancel / Escape.
fn name_dialog(ctx: &egui::Context, p: &Palette, id: &str, prompt: &str, name: &mut String) -> Option<bool> {
    let mut done = None;
    let modal = widgets::Modal::new(egui::Id::unique(id)).show(ctx, |ui| {
        ui.set_width(320.0);
        ui.label(egui::RichText::new(prompt).size(12.5).color(p.ink));
        ui.add_space(6.0);
        let r = ui.add(egui::TextEdit::singleline(name).char_limit(63).desired_width(f32::INFINITY));
        r.request_focus();
        if r.lost_focus() && widgets::enter_pressed(ui) {
            done = Some(!name.trim().is_empty());
        }
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                done = Some(false);
            }
            let ok = ui.add_enabled_ui(!name.trim().is_empty(), |ui| {
                widgets::primary_button_sized(ui, p, aurora_i18n::tr!("common-ok"), Vec2::new(80.0, 24.0))
            });
            if ok.inner.clicked() {
                done = Some(true);
            }
        });
    });
    if done.is_none() && (modal.should_close() || ctx.input(|i| i.key_pressed(egui::Key::Escape))) {
        done = Some(false);
    }
    done
}

/// A label and its value, on one line.
fn kv(ui: &mut egui::Ui, p: &Palette, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).size(12.5).color(p.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(value).size(12.5).color(p.ink));
        });
    });
}

/// A row of the rail: icon, name, count; `depth` indents a sub-folder.
#[allow(clippy::too_many_arguments)]
fn rail_row(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    tint: Color32,
    label: &str,
    count: Option<usize>,
    on: bool,
    depth: usize,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
    let radius = ui.visuals().window_corner_radius;
    if on {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.28));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised);
    }
    let x = rect.left() + 10.0 + depth as f32 * 14.0;
    paint_icon(
        ui.painter(),
        icon,
        Rect::from_center_size(Pos2::new(x + 8.0, rect.center().y), Vec2::splat(16.0)),
        if on && tint == p.muted { p.violet_pale } else { tint },
    );
    let num = count.map(|n| line(ui, &n.to_string(), 11.5, p.muted, false, 40.0));
    let num_w = num.as_ref().map_or(0.0, |g| g.size().x + 10.0);
    let ink = if tint == p.violet_light { tint } else { p.ink };
    let g = line(ui, label, 13.0, ink, false, rect.right() - num_w - x - 25.0);
    widgets::paint_row_text(ui.painter(), g, x + 25.0, rect.center().y, ink);
    if let Some(g) = num {
        let w = g.size().x;
        widgets::paint_row_text(ui.painter(), g, rect.right() - 10.0 - w, rect.center().y, p.muted);
    }
    resp
}

/// The favourite star at a row's right end: true when clicked.
fn star(ui: &mut egui::Ui, p: &Palette, place: &Place) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let fav = place.favorite.is_some();
    let hot = resp.hovered();
    if hot {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised);
    }
    let (icon, tint) = if fav {
        ("star-filled", p.amber)
    } else {
        ("star", if hot { p.ink } else { p.muted_dim })
    };
    paint_icon(ui.painter(), icon, Rect::from_center_size(rect.center(), Vec2::splat(16.0)), tint);
    let tip = if fav {
        aurora_i18n::tr!("ui-places-remove-favorite")
    } else {
        aurora_i18n::tr!("ui-places-add-favorite")
    };
    resp.on_hover_text(tip).clicked()
}

/// The G / M / A badge as a widget of a row's right end (room kept while the
/// rating is not known, so the badges stay in one column).
fn badge_slot(ui: &mut egui::Ui, p: &Palette, rating: Option<u8>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
    badge(ui, p, rating, rect);
}

/// A row of the list on the shared place card: picture, name and second
/// line, then from the right the mockup's star, (wide) last visit, badge.
#[allow(clippy::too_many_arguments)]
fn place_row(
    ui: &mut egui::Ui,
    p: &Palette,
    c: &mut Ctx,
    place: &Place,
    selected: bool,
    picture_id: Uuid,
    grip: bool,
    wide: bool,
) -> RowOut {
    let subtitle = if wide {
        let region = region_line(place);
        if place.is_landmark() {
            region
        } else {
            format!("{region} · {}", aurora_i18n::tr!("ui-places-no-landmark"))
        }
    } else {
        let region = if place.region.is_empty() {
            place_details::loading().to_owned()
        } else {
            place.region.clone()
        };
        match place.last_visit {
            Some(d) => format!("{region} · {}", model::visit_label(d, now_secs())),
            None => region,
        }
    };
    let card = place_card::Row {
        thumb: Thumb::Picture(texture(c, picture_id)),
        title: &place.name,
        subtitle: &subtitle,
        // drawn in `trailing`, between the names and the star
        rating: None,
        selected,
    };
    let size = if wide { RowSize::PLACES_WIDE } else { RowSize::PLACES_NARROW };
    let mut starred = false;
    let trailing = |ui: &mut egui::Ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if place.is_landmark() {
            starred = star(ui, p, place);
            ui.add_space(4.0);
        } else if wide {
            // the wide list keeps the column of the stars
            ui.add_space(28.0 + 4.0);
        }
        if wide {
            ui.allocate_ui_with_layout(
                Vec2::new(VISIT_W, size.height),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if let Some(d) = place.last_visit {
                        ui.add(
                            egui::Label::new(egui::RichText::new(model::visit_label(d, now_secs())).size(12.0).color(p.muted)).truncate(),
                        );
                    }
                },
            );
            ui.add_space(12.0);
        }
        badge_slot(ui, p, place.rating);
    };
    let resp = if grip {
        place_card::draggable_row(ui, p, size, &card, trailing)
    } else {
        place_card::row(ui, p, size, &card, trailing).interact(egui::Sense::click_and_drag())
    };
    RowOut { resp, star: starred }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::inventory::{FolderContents, InvFolder, InvItem};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn folder(n: u128, parent: u128, t: i32) -> InvFolder {
        InvFolder {
            id: id(n),
            parent: id(parent),
            name: format!("f{n}"),
            type_default: t,
            version: 1,
            ..Default::default()
        }
    }

    fn landmark(n: u128, parent: u128, asset: u128, copy: bool) -> InvItem {
        InvItem {
            id: id(n),
            parent: id(parent),
            name: format!("lm{n}"),
            asset_type: 3,
            inv_type: 3,
            asset_id: id(asset),
            owner: id(7),
            owner_mask: if copy { 0x7fff_ffff } else { 0x7fff_ffff & !actions::COPY },
            base_mask: 0x7fff_ffff,
            ..Default::default()
        }
    }

    /// Landmarks 10 (sub-folder 11), Favorites 20, Trash 30.
    fn inventory() -> Inventory {
        let mut inv = Inventory::default();
        let contents = |f: InvFolder, folders: Vec<InvFolder>, items: Vec<InvItem>| FolderContents {
            folder_id: f.id,
            owner_id: Uuid::nil(),
            version: 2,
            folders,
            items,
        };
        let root = folder(1, 0, 8);
        inv.root = root.id;
        let (lms, sub, fav, trash) = (folder(10, 1, 3), folder(11, 10, -1), folder(20, 1, 23), folder(30, 1, 14));
        inv.apply(vec![contents(root, vec![lms.clone(), fav.clone(), trash.clone()], Vec::new())]);
        inv.apply(vec![contents(
            lms,
            vec![sub.clone()],
            vec![landmark(100, 10, 1000, true), landmark(101, 10, 1001, false)],
        )]);
        inv.apply(vec![contents(sub, Vec::new(), Vec::new())]);
        inv.apply(vec![contents(
            fav,
            Vec::new(),
            vec![landmark(200, 20, 1000, true), landmark(201, 20, 1002, true)],
        )]);
        inv.apply(vec![contents(trash, Vec::new(), Vec::new())]);
        inv
    }

    fn place(inv: &Inventory, asset: u128) -> Place {
        let cat = model::catalog(inv, &crate::world::landmarks::Landmarks::default(), &Default::default(), &[], false);
        cat.landmark(id(asset)).cloned().expect("place")
    }

    fn parent_of(change: &Mutation, item: u128) -> Option<Uuid> {
        change.operations.iter().find_map(|op| match op {
            Operation::Patch { id: i, body, .. } if *i == id(item) => Some(body["parent_id"].as_uuid()),
            _ => None,
        })
    }

    #[test]
    fn the_star_copies_into_favorites_and_takes_the_copy_away() {
        let inv = inventory();
        // starred: a copy in Favorites
        let change = toggle_favorite(&inv, &place(&inv, 1001)).expect("no-copy: moved");
        assert_eq!(parent_of(&change, 101), Some(id(20)), "a no-copy landmark moves to Favorites");
        let lm = Landmarks::default();
        let _ = lm;
        let mut plain = place(&inv, 1001);
        plain.item = Some(id(100));
        plain.favorite = None;
        let change = toggle_favorite(&inv, &plain).expect("copied");
        assert!(matches!(&change.copies[..], [c] if c.item == id(100) && c.parent == id(20)));
        // unstarred: the copy goes to the Trash, the landmark stays
        let change = toggle_favorite(&inv, &place(&inv, 1000)).expect("unstarred");
        assert_eq!(parent_of(&change, 200), Some(id(30)));
        assert_eq!(parent_of(&change, 100), None);
        // a favourite only: back to Landmarks
        let change = toggle_favorite(&inv, &place(&inv, 1002)).expect("back");
        assert_eq!(parent_of(&change, 201), Some(id(10)));
    }

    #[test]
    fn folders_delete_and_notes() {
        let inv = inventory();
        let change = move_to_folder(&inv, &place(&inv, 1000), id(11)).expect("moved");
        assert_eq!(parent_of(&change, 100), Some(id(11)), "the landmark moves, its favourite stays");
        assert_eq!(parent_of(&change, 200), None);
        let change = move_to_folder(&inv, &place(&inv, 1002), id(11)).expect("copied");
        assert!(matches!(&change.copies[..], [c] if c.item == id(201) && c.parent == id(11)));
        let change = delete_place(&inv, &place(&inv, 1000)).expect("deleted");
        assert_eq!((parent_of(&change, 100), parent_of(&change, 200)), (Some(id(30)), Some(id(30))));
        let change = patch_all(&inv, &place(&inv, 1000), llsd_map! { "desc" => "jazz" }).expect("notes");
        assert_eq!(change.operations.len(), 2, "both copies keep the same notes");
    }

    #[test]
    fn a_history_picture_follows_its_entry_not_its_index() {
        let entry = |title: &str, x: f64, date: f64| HistoryItem {
            title: title.into(),
            global: DVec3::new(256_000.0 + x, 256_300.0, 22.0),
            date,
            slurl: String::new(),
        };
        let mut history = vec![entry("Lagune", 40.0, 1.0), entry("Nordheim", 140.0, 2.0)];
        let lm = Landmarks::default();
        let spot = |history: &[HistoryItem], i: usize| {
            let cat = model::catalog(&inventory(), &lm, &Default::default(), history, false);
            let place = cat.place(Key::Visit(i), history).expect("line");
            snapshot_spot(&place, &lm).map(|(s, ..)| s)
        };
        let nordheim = Some(Spot::at(history[1].global));
        assert_eq!(spot(&history, 1), nordheim);
        assert_ne!(spot(&history, 0), nordheim);
        // "Remove from history" of the first: Nordheim is now index 0 and
        // keeps its own picture (#798)
        history.remove(0);
        assert_eq!(spot(&history, 0), nordheim);
        // a landmark whose asset is not read yet waits for it
        assert_eq!(snapshot_spot(&place(&inventory(), 1000), &lm), None);
    }
}
