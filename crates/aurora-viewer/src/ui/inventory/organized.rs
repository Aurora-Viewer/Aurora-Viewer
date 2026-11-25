//! Personal, viewer-local organization of the live SL inventory.
#[path = "organized_menu.rs"]
mod menus;
pub(super) use menus::context_menu;
#[path = "organized_commands.rs"]
mod commands;
#[path = "organized_actions.rs"]
mod file_actions;
#[path = "organized_folder.rs"]
pub(super) mod folder;
#[path = "organized_folder_icons.rs"]
mod folder_icons;
#[path = "organized_icons.rs"]
mod item_art;
pub(super) use item_art::{Glyph, RowStyle, paint_tabler};
#[path = "organized_drag_preview.rs"]
pub(super) mod drag_preview;
#[path = "organized_input.rs"]
mod input;
#[path = "organized_navigation.rs"]
mod navigation;
use navigation::search_location;
#[path = "organized_pins.rs"]
mod pins;
use pins::{pin_manager, sidebar};
#[path = "organized_dimensions.rs"]
mod dimensions;
#[path = "organized_views.rs"]
mod views;
use super::organized_model::{self as model, Group, Location, Preferences};
use super::{Facts, FetchState, Icons, InvAction, Inventory, InventoryPreferences, InventoryUi, Palette, Uuid, aurora, context, rules};
use egui::{Align2, Color32, FontId, Key, Rect, Response, Sense, Ui, pos2, vec2};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum Row {
    Place(Location),
    Item(Uuid),
}
impl Row {
    fn id(self) -> Option<Uuid> {
        match self {
            Self::Place(Location::Folder(id)) | Self::Item(id) => Some(id),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct Visit {
    location: Location,
    raw: bool,
    query: String,
    type_filter: Option<i32>,
    selected: HashSet<Row>,
    selected_folders: Option<usize>,
    edit_availability: Option<commands::EditAvailability>,
    // Missing records from a partially loaded visit are never actionable.
    waiting_selection: HashSet<Row>,
    scroll: f32,
}
impl Visit {
    fn new(location: Location, raw: bool) -> Self {
        Self {
            location,
            raw,
            query: String::new(),
            type_filter: None,
            selected: HashSet::new(),
            selected_folders: None,
            edit_availability: None,
            waiting_selection: HashSet::new(),
            scroll: 0.0,
        }
    }
    fn replace_selection(&mut self, rows: HashSet<Row>) {
        self.selected = rows;
        self.selected_folders = None;
        self.edit_availability = None;
    }
    fn selection_folders(&mut self) -> usize {
        *self
            .selected_folders
            .get_or_insert_with(|| self.selected.iter().filter(|row| matches!(row, Row::Place(_))).count())
    }
    fn clear_selection(&mut self) {
        self.selected_folders = None;
        self.edit_availability = None;
        self.selected.clear();
        self.waiting_selection.clear();
    }
}

/// One independent navigation history and selection per Explorer tab.
pub(super) struct ExplorerState {
    here: Visit,
    input: input::State,
    back: Vec<Visit>,
    forward: Vec<Visit>,
    anchor: Option<Row>,
    rows: Vec<Row>,
    // Only grouped views retain their full sorted result. Section counts delimit its slices.
    ordered_rows: Option<Vec<Row>>,
    display_revision: u64,
    total_rows: usize,
    rows_key: Option<(Uuid, u64, u64)>,
    rows_group_date: Option<chrono::NaiveDate>,
    rows_language: &'static str,
    rows_at: Option<Instant>,
    rows_query: String,
    incomplete: bool,
    search_requests: Vec<Uuid>,
    sections: Vec<views::Section>,
    restore_scroll: bool,
    folder_draft: Option<folder::Draft>,
    file_request: Option<file_actions::Request>,
    bar_paste: Option<commands::PasteAvailability>,
    native_feedback: bool,
    paste_destination: Option<file_actions::Destination>,
    new_destination: Option<menus::NewDestination>,
    action_rename: Option<Uuid>,
    rename_original: Option<(Row, String)>,
    pin_editor: Option<pins::Editor>,
    edit_folder_icon: Option<(Location, Rect)>,
    status: String,
    search_focus: bool,
    navigation: navigation::Navigation,
    list_focus: Option<egui::Id>,
    pub(super) focus_contents: bool,
    reveal_row: Option<Row>,
    pub(super) pending_open_tab: Option<(Location, bool)>,
}
impl Default for ExplorerState {
    fn default() -> Self {
        Self::at(Location::Home)
    }
}
impl ExplorerState {
    pub(super) fn at(location: Location) -> Self {
        Self::at_mode(location, location == Location::FullTree)
    }
    pub(super) fn at_mode(location: Location, raw: bool) -> Self {
        Self {
            here: Visit::new(location, raw || location == Location::FullTree),
            input: input::State::default(),
            back: Vec::new(),
            forward: Vec::new(),
            anchor: None,
            rows: Vec::new(),
            ordered_rows: None,
            display_revision: 0,
            total_rows: 0,
            rows_key: None,
            rows_group_date: None,
            rows_language: "",
            rows_at: None,
            rows_query: String::new(),
            incomplete: false,
            search_requests: Vec::new(),
            sections: Vec::new(),
            restore_scroll: true,
            folder_draft: None,
            file_request: None,
            bar_paste: None,
            native_feedback: false,
            paste_destination: None,
            new_destination: None,
            action_rename: None,
            rename_original: None,
            pin_editor: None,
            edit_folder_icon: None,
            status: String::new(),
            search_focus: false,
            navigation: navigation::Navigation::default(),
            list_focus: None,
            focus_contents: false,
            reveal_row: None,
            pending_open_tab: None,
        }
    }
    pub(super) fn clear_filter(&mut self) {
        if self.here.type_filter.take().is_some() {
            self.rows_key = None;
        }
    }
    pub(super) fn title(&self, inv: &Inventory, avatar_name: &str, layout: &Preferences) -> String {
        place_name(inv, layout, self.here.location, self.here.raw, avatar_name)
    }
    pub(super) fn location(&self) -> Location {
        self.here.location
    }
    pub(super) fn navigate_key(&mut self, key: Key, inv: &Inventory, layout: &Preferences) {
        match key {
            Key::ArrowLeft => self.history(false),
            Key::ArrowRight => self.history(true),
            Key::ArrowUp => {
                if let Some(up) = parent(inv, layout, self.here.location, self.here.raw) {
                    self.visit(up, self.here.raw);
                }
            }
            _ => {}
        }
    }
    pub(super) fn on_dashboard(&self) -> bool {
        self.here.location == Location::Home && !self.here.raw && self.here.query.trim().is_empty()
    }
    pub(super) fn dashboard_shortcuts(&self, inv: &Inventory, layout: &Preferences) -> bool {
        self.on_dashboard()
            && self.here.selected.iter().any(|row| {
                !matches!(
                    row, Row::Place(Location::Folder(id)) if layout.is_home_child(inv, *id)
                )
            })
    }
    pub(super) fn suspend(&mut self) {
        self.input.cancel_pointer();
        self.navigation.close();
    }
    pub(super) fn focus_address(&mut self) {
        self.search_focus = false;
        self.navigation.focus = true;
    }
    pub(super) fn focus_search(&mut self) {
        self.focus_contents = false;
        self.navigation.close();
        self.search_focus = true;
    }
    pub(super) fn reveal(&mut self, inv: &Inventory, layout: &Preferences, id: Uuid) -> bool {
        if id == inv.root || id == inv.lib_root {
            self.visit(if id == inv.root { Location::Home } else { Location::Library }, false);
            return !id.is_nil();
        }
        if inv.folders.contains_key(&id) {
            self.visit(Location::Folder(id), false);
            return true;
        }
        if !inv.items.contains_key(&id) {
            return false;
        }
        self.visit(layout.parent(inv, id), false);
        self.clear_filter();
        self.select_and_reveal(Row::Item(id));
        true
    }
    fn select_and_reveal(&mut self, row: Row) {
        self.here.clear_selection();
        self.here.selected.insert(row);
        self.anchor = Some(row);
        self.input.caret = Some(row);
        self.reveal_row = Some(row);
        self.rows_key = None;
    }
    fn changed_location(&mut self) {
        self.focus_contents = true;
        self.navigation.close();
        self.reveal_row = None;
        self.input = input::State::default();
        self.paste_destination = None;
        self.new_destination = None;
        self.action_rename = None;
        self.rename_original = None;
        self.native_feedback = false;
        self.rows_key = None;
        self.rows_query = self.here.query.trim().to_lowercase();
        self.restore_scroll = true;
        // Keep an inline creation while it commits on navigation or awaits the simulator.
        if self.folder_draft.as_ref().is_some_and(|d| d.discard_on_navigation()) {
            self.folder_draft = None;
        }
        self.anchor = None;
        self.status.clear();
    }
    fn visit(&mut self, location: Location, raw: bool) {
        let raw = raw || location == Location::FullTree;
        if (self.here.location, self.here.raw) == (location, raw) && self.here.query.is_empty() {
            return;
        }
        let old = std::mem::replace(&mut self.here, Visit::new(location, raw));
        self.back.push(old);
        if self.back.len() > 96 {
            self.back.remove(0);
        }
        self.forward.clear();
        self.changed_location();
    }
    fn history(&mut self, forward: bool) {
        let visited = if forward { self.forward.pop() } else { self.back.pop() };
        if let Some(visited) = visited {
            let old = std::mem::replace(&mut self.here, visited);
            if forward {
                self.back.push(old);
            } else {
                self.forward.push(old);
            }
            self.changed_location();
        }
    }
    fn select(&mut self, row: Row, mods: egui::Modifiers) {
        self.here.selected_folders = None;
        self.here.edit_availability = None;
        self.here.waiting_selection.clear();
        self.input.caret = Some(row);
        let additive = mods.ctrl || mods.command;
        if mods.shift
            && let Some((first, last)) = self.anchor.and_then(|anchor| {
                Some((
                    self.rows.iter().position(|r| *r == anchor)?,
                    self.rows.iter().position(|r| *r == row)?,
                ))
            })
        {
            if !additive {
                self.here.clear_selection();
            }
            self.here
                .selected
                .extend(self.rows[first.min(last)..=first.max(last)].iter().copied());
        } else if additive {
            if !self.here.selected.insert(row) {
                self.here.selected.remove(&row);
            }
            self.anchor = Some(row);
        } else {
            self.here.clear_selection();
            self.here.selected.insert(row);
            self.anchor = Some(row);
        }
    }
}

pub(super) fn dragging(ctx: &egui::Context) -> bool {
    input::dragging(ctx)
}

pub(super) fn drag_count(ctx: &egui::Context) -> Option<usize> {
    input::drag_count(ctx)
}

fn receive(r: &Response, inv: &Inventory, layout: &mut Preferences, place: Location) -> bool {
    input::receive(r, inv, layout, place)
}

fn place_name(inv: &Inventory, layout: &Preferences, place: Location, raw: bool, avatar: &str) -> String {
    if place == Location::Home {
        return if avatar.trim().is_empty() {
            aurora_i18n::tr!("common-home").into()
        } else {
            avatar.into()
        };
    }
    if raw && let Location::Folder(id) = place {
        return if id == inv.lib_root {
            aurora_i18n::tr!("explorer-panel-stock").into()
        } else {
            inv.folders.get(&id).map_or_else(
                || aurora_i18n::tr!("explorer-panel-unavailable-folder").into(),
                |f| f.info.name.clone(),
            )
        };
    }
    layout.location_label(inv, place).to_owned()
}
fn row_name(inv: &Inventory, layout: &Preferences, row: Row, raw: bool, avatar: &str) -> String {
    match row {
        Row::Place(place) => place_name(inv, layout, place, raw, avatar),
        Row::Item(id) => inv.items.get(&id).map(|item| item.name.clone()).unwrap_or_default(),
    }
}
fn parent(inv: &Inventory, layout: &Preferences, place: Location, raw: bool) -> Option<Location> {
    if raw {
        return match place {
            Location::Folder(id) if id == inv.lib_root => Some(Location::FullTree),
            Location::Folder(id) => rules::parent(inv, id).map(|p| if p == inv.root { Location::FullTree } else { Location::Folder(p) }),
            _ => None,
        };
    }
    layout.location_parent(inv, place)
}

fn path(inv: &Inventory, layout: &Preferences, mut place: Location, raw: bool) -> Vec<Location> {
    let mut path = Vec::new();
    let mut seen = HashSet::new();
    while seen.insert(place) {
        path.push(place);
        let Some(next) = parent(inv, layout, place, raw) else { break };
        place = next;
    }
    path.reverse();
    path
}

fn canonical_children(inv: &Inventory, place: Location) -> Vec<Row> {
    let id = match place {
        Location::FullTree => inv.root,
        Location::Folder(id) => id,
        _ => return Vec::new(),
    };
    let mut children = Vec::new();
    if let Some(folder) = inv.folders.get(&id) {
        children.extend(
            folder
                .children
                .iter()
                .copied()
                .filter(|child| *child != inv.lib_root)
                .map(|id| Row::Place(Location::Folder(id))),
        );
        children.extend(folder.items.iter().copied().map(Row::Item));
    }
    if id == inv.root && !inv.lib_root.is_nil() && inv.folders.contains_key(&inv.lib_root) {
        children.push(Row::Place(Location::Folder(inv.lib_root)));
    }
    children
}

fn shown_children(inv: &Inventory, layout: &Preferences, place: Location) -> Vec<Row> {
    let mut rows: Vec<_> = layout.virtual_children(inv, place).into_iter().map(Row::Place).collect();
    rows.extend(layout.children(inv, place).into_iter().filter_map(|id| {
        if id == inv.lib_root {
            return None;
        }
        if inv.folders.contains_key(&id) {
            Some(Row::Place(Location::Folder(id)))
        } else if inv.items.contains_key(&id) {
            Some(Row::Item(id))
        } else {
            None
        }
    }));
    rows
}

fn search(inv: &Inventory, layout: &Preferences, state: &ExplorerState, avatar: &str, query: &str) -> (Vec<Row>, bool, Vec<Uuid>) {
    // Test titles before walking ancestry. Nearly all entries in a large inventory miss a specific query.
    let terms: Vec<_> = query.split_whitespace().collect();
    let mut rows = Vec::new();
    let mut incomplete = false;
    let mut requests = Vec::new();
    let global = matches!(state.here.location, Location::Home | Location::FullTree);
    let mut scoped = HashMap::new();
    let mut walked = Vec::new();
    let mut within = |mut loc| {
        if global {
            return true;
        }
        walked.clear();
        let result = loop {
            if loc == state.here.location {
                break true;
            }
            if let Some(&result) = scoped.get(&loc) {
                break result;
            }
            if walked.contains(&loc) {
                break false;
            }
            walked.push(loc);
            let Some(next) = parent(inv, layout, loc, state.here.raw) else {
                break false;
            };
            loc = next;
        };
        for &loc in &walked {
            scoped.insert(loc, result);
        }
        result
    };
    let matches = |name: &str| {
        let folded = name.to_lowercase();
        terms.iter().all(|term| folded.contains(term))
    };
    let backing = model::home_folder(inv, layout);
    for (id, folder) in &inv.folders {
        let loc = if *id == inv.lib_root && !state.here.raw {
            Location::Library
        } else {
            Location::Folder(*id)
        };
        let needs_fetch = requests.len() < 16 && folder.state == FetchState::Unknown && !inv.too_big.contains(id);
        if folder.state != FetchState::Fetched && (!incomplete || needs_fetch) && within(loc) {
            incomplete = true;
            if needs_fetch {
                requests.push(*id);
            }
        }
        if *id == inv.root || loc == state.here.location || (!state.here.raw && (*id == inv.lib_root || Some(*id) == backing)) {
            continue;
        }
        let title = place_name(inv, layout, loc, state.here.raw, avatar);
        if (matches(&title)
            || matches(&folder.info.name)
            || (folder.info.parent == inv.root
                && !folder.library
                && folder.info.type_default == 56
                && matches("EEP WindLight Environmental Enhancement Project")))
            && within(loc)
        {
            rows.push(Row::Place(loc));
        }
    }
    for item in inv.items.values() {
        if matches(&item.name) {
            let parent = if !state.here.raw {
                layout.parent(inv, item.id)
            } else if item.parent == inv.root {
                Location::FullTree
            } else {
                Location::Folder(item.parent)
            };
            if within(parent) {
                rows.push(Row::Item(item.id));
            }
        }
    }
    if !state.here.raw {
        let places = Group::ALL
            .iter()
            .copied()
            .map(Location::Group)
            .chain(layout.custom_locations())
            .chain([Location::MyFolders, Location::Library]);
        for loc in places {
            if loc != state.here.location
                && layout.available(inv, loc)
                && matches(&place_name(inv, layout, loc, false, avatar))
                && within(loc)
            {
                rows.push(Row::Place(loc));
            }
        }
    }
    (rows, incomplete, requests)
}

fn refresh_rows(inv: &Inventory, layout: &mut Preferences, state: &mut ExplorerState, avatar: &str) {
    let key = (inv.root, inv.generation, layout.ordering_revision());
    let query = state.here.query.trim().to_lowercase();
    let view = layout.view(state.here.location);
    let group_date = (view.group == model::GroupBy::Date && !state.on_dashboard()).then(|| chrono::Local::now().date_naive());
    let language = aurora_i18n::language();
    let explicit = language != state.rows_language
        || group_date != state.rows_group_date
        || query != state.rows_query
        || state.rows_key.is_none_or(|k| k.0 != key.0 || k.2 != key.2);
    if !explicit && state.rows_key == Some(key) {
        if state.display_revision != layout.revision() {
            // Other tabs share this location's disclosure state. A toggle elsewhere
            // needs no work when all of this tab's groups already match.
            let changed = state.sections.iter().any(|s| {
                s.key
                    .as_ref()
                    .is_some_and(|key| s.entries.is_empty() != layout.group_collapsed(state.here.location, key))
            });
            if changed {
                update_visible_rows(inv, layout, state);
            }
            state.display_revision = layout.revision();
        }
        return;
    }
    if !explicit
        && state.display_revision == layout.revision()
        && !query.is_empty()
        && state.rows_at.is_some_and(|t| t.elapsed() < Duration::from_millis(250))
    {
        return;
    }
    if state.rows_query != query {
        state.status.clear();
        state.native_feedback = false;
        state.here.clear_selection();
        state.anchor = None;
        state.here.scroll = 0.0;
        state.restore_scroll = true;
    }
    let mut home_sections = None;
    let (mut rows, incomplete, requests) = if query.is_empty() {
        let place = state.here.location;
        let rows = if state.on_dashboard() {
            let (rows, sections) = views::home(inv, layout, avatar);
            home_sections = Some(sections);
            rows
        } else if state.here.raw {
            canonical_children(inv, place)
        } else {
            shown_children(inv, layout, place)
        };
        let folder = match place {
            Location::Folder(id) => Some(id),
            Location::Library => Some(inv.lib_root),
            Location::Home | Location::MyFolders | Location::FullTree => Some(inv.root),
            _ => None,
        };
        let backing = (!state.here.raw && matches!(place, Location::Home | Location::MyFolders))
            .then(|| model::home_folder(inv, layout))
            .flatten();
        let incomplete = folder
            .into_iter()
            .chain(backing)
            .any(|id| inv.folders.get(&id).is_some_and(|f| f.state != FetchState::Fetched));
        (rows, incomplete, Vec::new())
    } else {
        search(inv, layout, state, avatar, &query)
    };
    if let Some(kind) = state.here.type_filter
        && !state.on_dashboard()
    {
        rows.retain(|row| commands::matches_filter(inv, *row, kind));
    }
    // The simulator may acknowledge a rename after the editor has already closed.
    // Reveal the renamed selection once its new name places it in another group.
    if let Some((row, original)) = &state.rename_original {
        if view.group != model::GroupBy::Name || !state.here.selected.contains(row) {
            state.rename_original = None;
        } else if row.id().is_some_and(|id| rules::name(inv, id) != *original) {
            state.reveal_row = Some(*row);
            state.rename_original = None;
        }
    }
    let on_home = state.on_dashboard();
    let draft = state.folder_draft.as_ref().and_then(|d| d.row(state));
    let mut sections = home_sections.unwrap_or_else(|| views::order(inv, layout, &mut rows, view, state.here.raw, avatar));
    if let (Some(row), Some(editor)) = (draft, &state.folder_draft) {
        views::insert_draft(&mut rows, &mut sections, row, editor, view, on_home);
    }
    state.total_rows = rows.len();
    state.sections = sections;
    if on_home || view.group != model::GroupBy::None {
        state.ordered_rows = Some(rows);
    } else {
        state.ordered_rows = None;
        state.rows = rows;
    }
    state.incomplete = incomplete;
    state.search_requests = requests;
    update_visible_rows(inv, layout, state);
    state.rows_key = Some(key);
    state.display_revision = layout.revision();
    state.rows_group_date = group_date;
    state.rows_language = language;
    state.rows_at = Some(Instant::now());
    state.rows_query = query;
}

fn update_visible_rows(inv: &Inventory, layout: &mut Preferences, state: &mut ExplorerState) {
    let draft = state.folder_draft.as_ref().and_then(|d| d.row(state));
    if let Some(ordered) = &state.ordered_rows {
        let editing_draft = draft.filter(|_| state.folder_draft.as_ref().is_some_and(|d| d.pending.is_none()));
        let mut offset = 0;
        state.rows.clear();
        for section in &mut state.sections {
            let members = &ordered[offset..offset + section.count];
            offset += section.count;
            let start = state.rows.len();
            let mut collapsed = false;
            if let Some(key) = &section.key
                && layout.group_collapsed(state.here.location, key)
            {
                let reveal = state.reveal_row.is_some_and(|r| members.contains(&r))
                    || editing_draft.is_some_and(|r| members.contains(&r))
                    || state.action_rename.is_some_and(|id| members.iter().any(|r| r.id() == Some(id)));
                if reveal {
                    layout.set_group_collapsed(state.here.location, key, false);
                } else {
                    collapsed = true;
                }
            }
            if !collapsed {
                state.rows.extend_from_slice(members);
            }
            section.entries = start..state.rows.len();
        }
    }
    if let Some(row) = draft {
        state.here.selected.insert(row);
        state.anchor = Some(row);
    }
    // Rows can disappear after another tab moves, trashes or renames a selection.
    // Only visible rows may receive file commands. A history selection whose
    // record has not loaded can reappear when that record arrives in this view.
    if !state.here.selected.is_empty() || !state.here.waiting_selection.is_empty() {
        state.here.selected_folders = None;
        state.here.edit_availability = None;
        let mut candidates = std::mem::take(&mut state.here.selected);
        candidates.extend(state.here.waiting_selection.drain());
        for &row in &state.rows {
            if candidates.remove(&row) {
                state.here.selected.insert(row);
                if candidates.is_empty() {
                    break;
                }
            }
        }
        if state.incomplete {
            state.here.waiting_selection = candidates
                .into_iter()
                .filter(|row| match row {
                    Row::Item(id) => !inv.items.contains_key(id),
                    Row::Place(Location::Folder(id)) => !inv.folders.contains_key(id),
                    _ => false,
                })
                .collect();
        }
    }
    state.anchor = state.anchor.filter(|row| state.rows.contains(row));
    state.input.caret = state.input.caret.filter(|row| state.rows.contains(row));
    input::retain_rows(&mut state.input, &state.rows);
}

/// Inventory highlights use the skin's raised surface and muted ink; the type icons retain their Legacy colors.
fn selection_edge(p: &Palette) -> egui::Stroke {
    if p.high_contrast {
        egui::Stroke::new(2.0, p.ink)
    } else {
        egui::Stroke::new(1.0, crate::interface_colors::readable(p.muted_dim, p.panel, 3.0))
    }
}

fn selected_row_fill(p: &Palette) -> Color32 {
    if p.high_contrast {
        p.raised
    } else {
        p.raised.lerp_to_gamma(p.muted, 0.24)
    }
}

fn detail_ink(p: &Palette, ink: Color32, selected: bool) -> Color32 {
    if selected {
        crate::interface_colors::readable(ink, selected_row_fill(p), 4.5)
    } else {
        ink
    }
}

pub(super) fn paint_row_selection(ui: &Ui, p: &Palette, response: &Response, rect: Rect, selected: bool) -> Color32 {
    if selected {
        let fill = selected_row_fill(p);
        ui.painter().rect(
            rect,
            ui.visuals().widgets.inactive.corner_radius,
            fill,
            selection_edge(p),
            egui::StrokeKind::Inside,
        );
        crate::interface_colors::readable(p.ink, fill, 4.5)
    } else {
        if response.hovered() {
            ui.painter().rect_filled(
                rect,
                ui.visuals().widgets.inactive.corner_radius,
                ui.visuals().widgets.hovered.weak_bg_fill,
            );
        }
        ui.visuals().text_color()
    }
}

/// One interaction surface per row, with clipped text and consistent hit-testing across columns.
fn entry(ui: &mut Ui, style: &RowStyle, text: &str, glyph: Glyph, selected: bool) -> Response {
    let height = (ui.fonts_mut(|f| f.row_height(&FontId::proportional(13.0))).ceil() + 8.0).max(26.0);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width().max(24.0), height), Sense::click_and_drag());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let color = paint_row_selection(ui, style.palette, &response, rect, selected);
    style.glyph(
        ui.painter(),
        glyph,
        Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), vec2(16.0, 16.0)),
    );
    let right = rect.right() - 4.0;
    let painter = ui
        .painter()
        .with_clip_rect(Rect::from_min_max(rect.min, pos2(right.max(rect.left()), rect.max.y)).intersect(ui.clip_rect()));
    let label = painter.layout_no_wrap(text.to_owned(), FontId::proportional(13.0), color);
    let left = rect.left() + 30.0;
    let clipped = left + label.size().x > right;
    crate::ui::widgets::paint_row_text(&painter, label, left, rect.center().y, color);
    if clipped { response.on_hover_text(text) } else { response }
}

fn nav_button(ui: &mut Ui, direction: egui::Vec2, tip: &str, enabled: bool) -> bool {
    let side = ui.available_height();
    let response = ui.add_enabled(enabled, egui::Button::new("").min_size(vec2(side, side)));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, tip));
    let color = if enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let name = if direction.x < 0.0 {
        "arrow-left"
    } else if direction.x > 0.0 {
        "arrow-right"
    } else {
        "arrow-up"
    };
    item_art::paint_tabler(
        ui.painter(),
        name,
        Rect::from_center_size(response.rect.center(), vec2(16.0, 16.0)),
        color,
    );
    response.on_hover_text(tip).on_disabled_hover_text(tip).clicked()
}

fn toolbar(ui: &mut Ui, inv: &mut Inventory, layout: &mut Preferences, state: &mut ExplorerState, avatar: &str) {
    let height = (ui.text_style_height(&egui::TextStyle::Body).ceil() + 10.0)
        .max(ui.spacing().interact_size.y)
        .max(26.0);
    let (row_rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    {
        let mut row = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("explorer-navigation-row")
                .max_rect(row_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let ui = &mut row;
        ui.spacing_mut().item_spacing.x = 5.0;
        if nav_button(ui, vec2(-1.0, 0.0), aurora_i18n::tr!("explorer-panel-back"), !state.back.is_empty()) {
            state.history(false);
        }
        if nav_button(
            ui,
            vec2(1.0, 0.0),
            aurora_i18n::tr!("explorer-panel-forward"),
            !state.forward.is_empty(),
        ) {
            state.history(true);
        }
        let up = parent(inv, layout, state.here.location, state.here.raw);
        if nav_button(ui, vec2(0.0, -1.0), aurora_i18n::tr!("explorer-panel-up"), up.is_some())
            && let Some(up) = up
        {
            state.visit(up, state.here.raw);
        }
        let gap = ui.spacing().item_spacing.x;
        let available = (ui.available_width() - gap).max(2.0);
        let search_width = (available * 0.32).clamp(100.0, 300.0).min(available * 0.48);
        let (address_rect, _) = ui.allocate_exact_size(vec2(available - search_width, height), Sense::hover());
        let (search_rect, _) = ui.allocate_exact_size(vec2(search_width, height), Sense::hover());
        let field_layout = egui::Layout::left_to_right(egui::Align::Center);
        let mut address = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("explorer-path")
                .max_rect(address_rect)
                .layout(field_layout),
        );
        address.set_clip_rect(address_rect.intersect(ui.clip_rect()));
        if !navigation::address_field(&mut address, inv, layout, state, avatar) {
            ui.painter().rect(
                address_rect,
                ui.visuals().widgets.inactive.corner_radius,
                ui.visuals().text_edit_bg_color(),
                ui.visuals().widgets.inactive.bg_stroke,
                egui::StrokeKind::Inside,
            );
            let contents = address_rect.shrink2(vec2(6.0, 0.0));
            let mut crumbs = address.new_child(egui::UiBuilder::new().max_rect(contents).layout(field_layout));
            crumbs.set_clip_rect(contents.intersect(ui.clip_rect()));
            breadcrumbs(&mut crumbs, inv, layout, state, avatar);
        }
        let mut search = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("explorer-search")
                .max_rect(search_rect)
                .layout(field_layout),
        );
        search.set_clip_rect(search_rect.intersect(ui.clip_rect()));
        search_field(&mut search, inv, layout, state, avatar);
    }
    navigation::feedback(ui, state);
}

fn breadcrumbs(ui: &mut Ui, inv: &mut Inventory, layout: &mut Preferences, state: &mut ExplorerState, avatar: &str) {
    let crumbs = path(inv, layout, state.here.location, state.here.raw);
    let skip = crumbs.len().saturating_sub(if ui.available_width() < 320.0 { 2 } else { 4 });
    if skip > 0 {
        let (response, _) = egui::containers::menu::MenuButton::from_button(
            egui::Button::new(item_art::glyph_image(
                ui,
                "…",
                "dots-filled",
                false,
                crate::ui::widgets::PICTURE_TINT,
            ))
            .image_tint_follows_text_color(true),
        )
        .ui(ui, |ui| {
            for &loc in &crumbs[..skip] {
                if ui.button(place_name(inv, layout, loc, state.here.raw, avatar)).clicked() {
                    state.visit(loc, state.here.raw);
                    ui.close();
                }
            }
        });
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "…"));
    }
    for (index, &loc) in crumbs[skip..].iter().enumerate() {
        if index > 0 {
            ui.add(item_art::glyph_image(
                ui,
                "›",
                "chevron-right",
                false,
                ui.visuals().weak_text_color(),
            ));
        }
        let name = place_name(inv, layout, loc, state.here.raw, avatar);
        let left = crumbs.len() - skip - index;
        let width = ((ui.available_width() - 12.0) / left as f32).max(1.0);
        let color = ui.visuals().text_color();
        let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), egui::TextStyle::Body.resolve(ui.style()), color);
        job.wrap.max_width = width;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let label = ui.painter().layout_job(job);
        let (rect, response) = ui.allocate_exact_size(vec2(label.size().x.min(width), ui.available_height()), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        crate::ui::widgets::paint_row_text(&painter, label, rect.left(), rect.center().y, color);
        let response = response.on_hover_text(&name);
        if !state.here.raw {
            receive(&response, inv, layout, loc);
        }
        if response.clicked() {
            if loc == state.here.location {
                state.focus_address();
            } else {
                state.visit(loc, state.here.raw);
            }
        }
    }
    let edit = ui
        .allocate_response(ui.available_size().max(egui::Vec2::ZERO), Sense::click())
        .on_hover_cursor(egui::CursorIcon::Text)
        .on_hover_text(aurora_i18n::tr!("explorer-address-edit"));
    if edit.clicked() {
        state.focus_address();
    }
}

fn search_field(ui: &mut Ui, inv: &Inventory, layout: &Preferences, state: &mut ExplorerState, avatar: &str) {
    let location = place_name(inv, layout, state.here.location, state.here.raw, avatar);
    let hint = if matches!(state.here.location, Location::Home | Location::FullTree) {
        aurora_i18n::tr!("explorer-panel-search").to_owned()
    } else {
        aurora_i18n::tr!("explorer-search-in-folder", name = location)
    };
    let clear_width = if state.here.query.is_empty() {
        0.0
    } else {
        20.0 + ui.spacing().item_spacing.x
    };
    let field = ui.add_sized(
        [(ui.available_width() - clear_width).max(1.0), ui.available_height()],
        egui::TextEdit::singleline(&mut state.here.query)
            .id_salt("query")
            .align(Align2::LEFT_CENTER)
            .min_size(vec2(1.0, ui.available_height()))
            .margin(egui::Margin::symmetric(6, 2))
            .hint_text(hint),
    );
    let fields = [field.id, state.list_focus.unwrap_or(field.id)];
    let find =
        !egui::Popup::is_any_open(ui.ctx()) && ui.memory(|m| m.top_modal_layer()).is_none() && crate::ui::find_shortcut::take(ui, &fields);
    if std::mem::take(&mut state.search_focus) || find {
        field.request_focus();
        if let Some(mut edit) = egui::TextEdit::load_state(ui.ctx(), field.id) {
            edit.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(state.here.query.chars().count()),
            )));
            edit.store(ui.ctx(), field.id);
        }
    }
    if !state.here.query.is_empty() {
        let clear = ui.add(
            egui::Button::new(item_art::glyph_image(ui, "×", "x", false, crate::ui::widgets::PICTURE_TINT))
                .small()
                .image_tint_follows_text_color(true),
        );
        clear.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "×"));
        if clear.on_hover_text(aurora_i18n::tr!("explorer-panel-clear")).clicked() {
            state.here.query.clear();
            field.request_focus();
        }
    }
}

fn shortcuts(ui: &Ui, state: &mut ExplorerState) -> Option<Row> {
    if state.navigation.active() || state.action_rename.is_some() || state.folder_draft.is_some() || input::header_focused(ui, state) {
        return None;
    }
    let list_focused = state.list_focus.is_some_and(|id| ui.memory(|m| m.has_focus(id)));
    let top = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|pos| ui.max_rect().contains(pos) && ui.ctx().layer_id_at(pos) == Some(ui.layer_id()));
    if egui::Popup::is_any_open(ui.ctx()) || (!list_focused && (!top || ui.ctx().egui_wants_keyboard_input())) {
        return None;
    }
    if list_focused {
        input::keyboard(ui, state);
    }
    if top && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1)) {
        state.history(false);
    }
    if top && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra2)) {
        state.history(true);
    }
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) && state.here.selected.len() == 1 {
        return state.here.selected.iter().next().copied();
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut Ui,
    palette: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    avatar_name: &str,
    facts: &Facts,
    prefs: &mut InventoryPreferences,
    layout: &mut Preferences,
    state: &mut ExplorerState,
    native: &mut InventoryUi,
    hover_images: &HashMap<Uuid, egui::TextureHandle>,
    actions: &mut Vec<InvAction>,
) {
    if inv.root.is_nil() {
        ui.spinner();
        ui.label(aurora_i18n::tr!("explorer-panel-loading"));
        return;
    }
    if state.folder_draft.as_ref().is_some_and(|d| d.left_origin(state)) {
        folder::leave(state, inv, native.pending.is_some(), actions);
    }
    let folder = match state.here.location {
        Location::Library => Some(inv.lib_root),
        Location::Home | Location::MyFolders | Location::FullTree | Location::Group(_) => Some(inv.root),
        Location::Folder(id) => Some(id),
        Location::Custom(_) => None,
    };
    let backing = (!state.here.raw && matches!(state.here.location, Location::Home | Location::MyFolders))
        .then(|| model::home_folder(inv, layout))
        .flatten();
    for id in folder.into_iter().chain(backing) {
        if inv.folders.get(&id).is_some_and(|f| f.state == FetchState::Unknown) {
            inv.request(id);
        }
    }
    // Another tab can collapse a shared group before this tab receives a shortcut.
    if layout.available(inv, state.here.location) {
        refresh_rows(inv, layout, state, avatar_name);
    }
    let mut open = shortcuts(ui, state);
    toolbar(ui, inv, layout, state, avatar_name);
    commands::bar(
        ui,
        palette,
        inv,
        layout,
        state,
        avatar_name,
        &file_actions::Menu { native, prefs, facts },
        &mut open,
    );
    ui.add_space(4.0);
    let surface = ui.available_rect_before_wrap().intersect(ui.clip_rect());
    let (left, right, footer) = dimensions::panes(ui, surface, layout);
    let mut side = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("organized_sidebar")
            .max_rect(left)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    side.set_clip_rect(left.intersect(ui.clip_rect()));
    let style = RowStyle {
        palette,
        icons,
        prefs,
        facts,
    };
    sidebar(&mut side, &style, inv, layout, state, avatar_name);
    let mut main = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("organized_main")
            .max_rect(right)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    main.set_clip_rect(right.intersect(ui.clip_rect()));
    if !layout.available(inv, state.here.location) {
        main.label(aurora_i18n::tr!("explorer-panel-unavailable"));
        if main.button(aurora_i18n::tr!("explorer-panel-home")).clicked() {
            state.visit(Location::Home, false);
        }
    } else {
        folder::tools(&mut main, inv, layout, state);
        menus::destination(&mut main, inv, layout, state, &file_actions::Menu { native, prefs, facts });
        file_actions::destination(&mut main, inv, layout, state, native, prefs, facts, actions);
        if let Some(id) = folder
            && inv.folders.get(&id).is_some_and(|f| f.state == FetchState::Failed)
            && main.button(aurora_i18n::tr!("explorer-panel-retry")).clicked()
        {
            inv.request(id);
        }
        refresh_rows(inv, layout, state, avatar_name);
        // Each result scan gathers one bounded batch in its displayed scope, including
        // special folders and Library. A server reply invalidates the cached results
        // and discovers the next batch. Idle completed searches do no extra tree walks.
        let count = 16usize.saturating_sub(inv.queue.len()).min(state.search_requests.len());
        for id in state.search_requests.drain(..count) {
            inv.request(id);
        }

        let mut wanted = std::mem::take(&mut native.wanted_explorer_images);
        let mut materials = std::mem::take(&mut native.explorer_materials);
        let files = file_actions::Menu { native, prefs, facts };
        let rendered_revision = layout.revision();
        let editor = views::contents(
            &mut main,
            &style,
            inv,
            layout,
            state,
            avatar_name,
            &mut open,
            hover_images,
            &mut wanted,
            &mut materials,
            &files,
            actions,
        );
        if layout.revision() != rendered_revision && state.rows_key.is_some() {
            // A disclosure or header menu can change membership in this same input frame.
            refresh_rows(inv, layout, state, avatar_name);
        }
        file_actions::keyboard(&main, inv, layout, state, &files);
        if let Some((row, rect, clip)) = editor {
            let mut cell = main.new_child(
                egui::UiBuilder::new()
                    .id_salt(("name-in-entry", row))
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            cell.set_clip_rect(clip.intersect(main.clip_rect()));
            if state.folder_draft.as_ref().and_then(|d| d.row(state)) == Some(row) {
                folder::inline_create(&mut cell, inv, state, native, actions);
            } else if let Some(id) = row.id()
                && state.action_rename == Some(id)
                && native.rename.as_ref().is_some_and(|edit| edit.id == id)
                && !model::client_managed(inv, id)
            {
                native.visible.push(id);
                let finish_with_key = cell.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Escape));
                super::rename_row(&mut cell, palette, inv, id, native, actions);
                if native.rename.is_none() {
                    // The tree reveals its renamed node. Explorer must reveal the row in its
                    // current folder; passing this to the host would open a renamed folder.
                    if native.reveal == Some(id) {
                        if layout.view(state.here.location).group == model::GroupBy::Name && !state.on_dashboard() {
                            state.rename_original = Some((row, rules::name(inv, id)));
                        }
                        native.reveal = None;
                        native.reveal_path.clear();
                    }
                    cell.input_mut(|i| {
                        i.consume_key(egui::Modifiers::NONE, Key::Enter);
                        i.consume_key(egui::Modifiers::NONE, Key::Escape);
                    });
                    if finish_with_key {
                        state.select(row, egui::Modifiers::NONE);
                    }
                    state.action_rename = None;
                    state.reveal_row = Some(row);
                    state.focus_contents = true;
                }
            }
        }
        native.wanted_explorer_images = wanted;
        native.explorer_materials = materials;
        file_actions::execute(&main, inv, layout, state, native, prefs, facts, actions);
    }
    folder_icons::picker(
        ui,
        &RowStyle {
            palette,
            icons,
            prefs,
            facts,
        },
        inv,
        layout,
        state,
    );
    dimensions::status(ui, footer, inv, layout, state, avatar_name, facts, &native.message);
    ui.advance_cursor_after_rect(surface);
    match open {
        Some(Row::Place(loc)) => state.visit(loc, state.here.raw && matches!(loc, Location::Folder(_) | Location::FullTree)),
        Some(Row::Item(id)) => {
            if let Some(original) = rules::original(inv, id)
                && inv.folders.contains_key(&original)
            {
                state.visit(Location::Folder(original), state.here.raw);
            } else {
                context::double_click(inv, id, native, prefs, facts, actions);
            }
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::Folder;
    use aurora_net::inventory::{InvFolder, InvItem};

    pub(super) fn id(v: u128) -> Uuid {
        Uuid::from_u128(v)
    }
    pub(super) fn fixture() -> Inventory {
        let mut inv = Inventory {
            root: id(1),
            lib_root: id(2),
            ..Default::default()
        };
        for (v, p, name, kind, lib) in [
            (1, 0, "Root", 8, false),
            (2, 0, "Linden", 8, true),
            (3, 1, "Workshop", -1, false),
            (4, 1, "Archive", -1, false),
            (5, 1, "Settings", 56, false),
            (6, 2, "Stock clothing", -1, true),
            (7, 1, "Snapshots", 15, false),
            (8, 1, "Unknown kind", 178, false),
        ] {
            let node = id(v);
            let parent = id(p);
            inv.folders.insert(
                node,
                Folder {
                    info: InvFolder {
                        id: node,
                        parent,
                        name: name.into(),
                        type_default: kind,
                        ..Default::default()
                    },
                    children: vec![],
                    items: vec![],
                    state: FetchState::Fetched,
                    library: lib,
                },
            );
            if let Some(f) = inv.folders.get_mut(&parent) {
                f.children.push(node);
            }
        }
        let item = InvItem {
            id: id(20),
            parent: id(3),
            name: "Bench notes".into(),
            asset_type: 7,
            ..Default::default()
        };
        inv.folders.get_mut(&id(3)).unwrap().items.push(item.id);
        inv.items.insert(item.id, item);
        inv
    }

    #[test]
    fn organized_home_keeps_personal_rows_and_recovers_system_destinations_through_search() {
        let inv = fixture();
        let layout = Preferences::default();
        let rows = shown_children(&inv, &layout, Location::MyFolders);
        assert!(rows.contains(&Row::Place(Location::Folder(id(3)))));
        assert!(rows.contains(&Row::Place(Location::Folder(id(8)))));
        for v in [2, 5, 7] {
            assert!(!rows.contains(&Row::Place(Location::Folder(id(v)))));
        }
        assert!(
            rows.iter()
                .all(|r| !matches!(r, Row::Place(Location::Library | Location::Group(_))))
        );
        let state = ExplorerState::default();
        for term in ["windlight", "eep", "settings"] {
            assert!(
                search(&inv, &layout, &state, "Tester", term)
                    .0
                    .contains(&Row::Place(Location::Folder(id(5))))
            );
        }
        assert!(
            search(&inv, &layout, &state, "Tester", "stock clothing")
                .0
                .contains(&Row::Place(Location::Folder(id(6))))
        );
    }

    #[test]
    fn organized_search_caches_complete_results_and_requeues_new_unknowns_in_every_scope() {
        let mut layout = Preferences::default();
        for location in [Location::Home, Location::FullTree, Location::Folder(id(3))] {
            let mut inv = fixture();
            let mut state = ExplorerState::at(location);
            state.here.query = "bench".into();
            refresh_rows(&inv, &mut layout, &mut state, "Tester");
            assert!(!state.incomplete && state.search_requests.is_empty());
            let scanned = Instant::now() - Duration::from_secs(2);
            state.rows_at = Some(scanned);
            refresh_rows(&inv, &mut layout, &mut state, "Tester");
            assert_eq!(state.rows_at, Some(scanned));
            inv.folders.get_mut(&id(5)).unwrap().state = FetchState::Unknown;
            inv.folders.get_mut(&id(6)).unwrap().state = FetchState::Unknown;
            inv.generation += 1;
            refresh_rows(&inv, &mut layout, &mut state, "Tester");
            if matches!(location, Location::Folder(_)) {
                assert!(!state.incomplete && state.search_requests.is_empty());
            } else {
                assert!(state.incomplete);
                state.search_requests.sort_unstable();
                assert_eq!(state.search_requests, [id(5), id(6)]);
            }
        }
    }

    #[test]
    fn organized_changed_search_clears_old_action_feedback_and_keeps_it_on_cache_updates() {
        let mut inv = fixture();
        let mut layout = Preferences::default();
        let mut state = ExplorerState {
            status: "local action".into(),
            native_feedback: true,
            ..Default::default()
        };
        state.here.query = "bench".into();
        refresh_rows(&inv, &mut layout, &mut state, "Tester");
        assert!(state.status.is_empty() && !state.native_feedback);
        state.status = "new action".into();
        state.native_feedback = true;
        state.rows_at = None;
        inv.generation += 1;
        refresh_rows(&inv, &mut layout, &mut state, "Tester");
        assert_eq!(state.status, "new action");
        assert!(state.native_feedback);
        state.here.query.clear();
        refresh_rows(&inv, &mut layout, &mut state, "Tester");
        assert!(state.status.is_empty() && !state.native_feedback);
    }

    #[test]
    fn organized_external_move_deselects_hidden_file_and_partial_history_restores_only_visible_rows() {
        let mut inv = fixture();
        let mut layout = Preferences::default();
        let item = Row::Item(id(20));
        let mut state = ExplorerState::at(Location::Folder(id(3)));
        refresh_rows(&inv, &mut layout, &mut state, "Tester");
        state.select(item, egui::Modifiers::NONE);
        assert!(layout.place(&inv, id(20), Location::Folder(id(4))));
        refresh_rows(&inv, &mut layout, &mut state, "Tester");
        assert!(!state.rows.contains(&item));
        assert!(state.here.selected.is_empty() && state.here.waiting_selection.is_empty());
        assert!(state.anchor.is_none() && state.input.caret.is_none());

        state.visit(Location::Folder(id(4)), false);
        refresh_rows(&inv, &mut layout, &mut state, "Tester");
        state.select(item, egui::Modifiers::NONE);
        state.visit(Location::Home, false);
        let entry = inv.items.remove(&id(20)).unwrap();
        inv.folders.get_mut(&id(4)).unwrap().state = FetchState::Fetching;
        inv.generation += 1;
        state.history(false);
        refresh_rows(&inv, &mut layout, &mut state, "Tester");
        assert!(state.here.selected.is_empty());
        assert!(state.here.waiting_selection.contains(&item));
        inv.items.insert(entry.id, entry);
        inv.generation += 1;
        refresh_rows(&inv, &mut layout, &mut state, "Tester");
        assert!(state.here.selected.contains(&item));
        assert!(state.here.waiting_selection.is_empty());
    }

    #[test]
    fn organized_search_respects_virtual_item_placements_and_history() {
        let inv = fixture();
        let mut layout = Preferences::default();
        let virtual_place = layout.create_custom(&inv, Location::MyFolders, "Shaders").unwrap();
        assert!(layout.place(&inv, id(20), virtual_place));
        let mut state = ExplorerState::at(virtual_place);
        assert_eq!(search(&inv, &layout, &state, "Tester", "bench").0, vec![Row::Item(id(20))]);
        state.here.selected.insert(Row::Item(id(20)));
        state.here.scroll = 65.0;
        state.visit(Location::Folder(id(3)), false);
        assert!(search(&inv, &layout, &state, "Tester", "bench").0.is_empty());
        state.history(false);
        refresh_rows(&inv, &mut layout, &mut state, "Tester");
        assert_eq!(state.here.location, virtual_place);
        assert_eq!(state.here.scroll, 65.0);
        assert!(state.here.selected.contains(&Row::Item(id(20))));
        assert!(state.reveal(&inv, &layout, id(20)));
        assert_eq!(state.here.location, virtual_place);
    }

    struct Panel {
        ctx: egui::Context,
        palette: Palette,
        facts: Facts,
        inv: Inventory,
        prefs: InventoryPreferences,
        layout: Preferences,
        tab: ExplorerState,
        native: InventoryUi,
        actions: Vec<InvAction>,
        frame: u32,
        width: f32,
        icons: Icons,
        images: HashMap<Uuid, egui::TextureHandle>,
        output: egui::FullOutput,
    }
    impl Panel {
        fn new() -> Self {
            Self {
                ctx: Default::default(),
                palette: crate::theme::Theme::default().palette(),
                facts: Facts::default(),
                inv: fixture(),
                prefs: Default::default(),
                layout: Default::default(),
                tab: Default::default(),
                native: Default::default(),
                actions: Vec::new(),
                frame: 0,
                width: 960.0,
                icons: Icons::default(),
                images: HashMap::new(),
                output: Default::default(),
            }
        }
        fn tick(&mut self, events: Vec<egui::Event>) {
            self.frame += 1;
            self.output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(self.width, 640.0))),
                    time: Some(self.frame as f64 / 60.0),
                    events,
                    ..Default::default()
                },
                |root| {
                    egui::CentralPanel::default().show(root, |ui| {
                        show(
                            ui,
                            &self.palette,
                            &self.icons,
                            &mut self.inv,
                            "Tester",
                            &self.facts,
                            &mut self.prefs,
                            &mut self.layout,
                            &mut self.tab,
                            &mut self.native,
                            &self.images,
                            &mut self.actions,
                        );
                    });
                },
            );
            self.output.textures_delta.clear();
        }
        fn idle(&mut self, n: u32) {
            for _ in 0..n {
                self.tick(vec![]);
            }
        }
        fn text_pos(&self, name: &str) -> egui::Pos2 {
            if name == "…" {
                let texture = Icons::tabler(&self.ctx, "dots-filled");
                return self
                    .output
                    .shapes
                    .iter()
                    .rev()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Rect(r) if r.brush.as_ref().is_some_and(|b| b.fill_texture_id == texture.id()) => {
                            Some(r.rect.center())
                        }
                        egui::Shape::Mesh(m) if m.texture_id == texture.id() => Some(m.calc_bounds().center()),
                        _ => None,
                    })
                    .expect("missing Tabler overflow");
            }
            fn locate(shape: &egui::Shape, name: &str) -> Option<egui::Pos2> {
                match shape {
                    egui::Shape::Text(t) if t.galley.text() == name => Some(t.pos + t.galley.size() * 0.5),
                    egui::Shape::Vec(group) => group.iter().find_map(|shape| locate(shape, name)),
                    _ => None,
                }
            }
            self.output
                .shapes
                .iter()
                .rev()
                .find_map(|s| locate(&s.shape, name))
                .unwrap_or_else(|| panic!("missing label {name}"))
        }
        fn pointer(&mut self, pos: egui::Pos2, btn: egui::PointerButton, pressed: bool) {
            self.tick(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: btn,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        fn click_text(&mut self, name: &str, btn: egui::PointerButton) {
            let p = self.text_pos(name);
            self.tick(vec![egui::Event::PointerMoved(p)]);
            self.pointer(p, btn, true);
            self.pointer(p, btn, false);
            self.idle(2);
        }
    }

    #[test]
    fn organized_address_and_search_share_a_centerline_at_normal_and_large_text_sizes() {
        for (width, scale) in [(420.0, 1.0), (960.0, 1.5)] {
            let mut p = Panel::new();
            p.width = width;
            crate::ui::fonts::apply(&p.ctx, crate::ui::fonts::INTER);
            crate::theme::Theme::default().apply(&p.ctx, scale);
            p.idle(4);
            for editing in [false, true] {
                if editing {
                    p.tab.focus_address();
                    p.idle(3);
                }
                let search = p.text_pos(aurora_i18n::tr!("explorer-panel-search"));
                let field = |x: f32| {
                    p.output
                        .shapes
                        .iter()
                        .filter_map(|s| match &s.shape {
                            egui::Shape::Rect(face)
                                if face.rect.contains(pos2(x, search.y)) && face.rect.width() > 40.0 && face.rect.height() < 60.0 =>
                            {
                                Some(face.rect)
                            }
                            _ => None,
                        })
                        .min_by(|a, b| a.width().total_cmp(&b.width()))
                        .expect("painted field surface")
                };
                let path_box = field(width * 0.5);
                let query_box = field(search.x);
                assert!(path_box.right() < query_box.left(), "independent field surfaces");
                assert!((path_box.top() - query_box.top()).abs() <= 1.0, "{path_box:?} {query_box:?}");
                assert!((path_box.bottom() - query_box.bottom()).abs() <= 1.0, "{path_box:?} {query_box:?}");
                let text = p
                    .output
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Text(t) if !t.galley.text().is_empty() && path_box.contains(t.pos + t.galley.size() * 0.5) => Some(t),
                        _ => None,
                    })
                    .expect("breadcrumb or editable path");
                let center = crate::ui::widgets::ink_rect(&text.galley).translate(text.pos.to_vec2()).center().y;
                assert!((center - path_box.center().y).abs() <= 3.0, "ink at {center}, field {path_box:?}");
            }
        }
    }

    #[test]
    fn organized_footer_shows_current_selection_fully_inside_the_window() {
        use egui::PointerButton;
        let mut p = Panel::new();
        p.tab.visit(Location::Folder(id(3)), false);
        p.idle(3);
        let row = Row::Item(id(20));
        let name = p.inv.items[&id(20)].name.clone();
        p.click_text(&name, PointerButton::Primary);
        let expected_count = aurora_i18n::tr!("explorer-panel-selection-count", selected = 1_i64, count = 1_i64);
        let footer = p
            .output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.text().contains(&expected_count)
                    && text.galley.text().contains(&name)
                {
                    Some((text, shape.clip_rect))
                } else {
                    None
                }
            })
            .expect("selection status contains the selected name and both counts");
        let (text, clip) = footer;
        assert!(text.galley.text().contains(&views::kind(&p.inv, row)));
        let painted = Rect::from_min_size(text.pos, text.galley.size());
        assert!(clip.contains_rect(painted));
        assert!(
            painted.top() > 585.0 && painted.left() < 30.0,
            "footer spans the sidebar and content pane"
        );
        // Re-evaluate only on selection changes, including replacing a same-sized marquee.
        let dir = Row::Place(Location::Folder(id(3)));
        let other = Row::Item(id(122));
        p.tab.here.replace_selection(HashSet::from([row, dir]));
        assert_eq!(p.tab.here.selection_folders(), 1);
        p.tab.here.replace_selection(HashSet::from([row, other]));
        assert_eq!(p.tab.here.selection_folders(), 0);
    }

    #[test]
    fn organized_home_displays_pinned_sections_and_personal_root_opens_without_moving_inventory() {
        use egui::PointerButton;
        let mut p = Panel::new();
        let pin = Location::Folder(id(3));
        let custom_group = p.layout.create_pin_group("Projects").unwrap();
        assert!(p.layout.pin_location(&p.inv, pin));
        assert!(p.layout.move_pin_to_group(&p.inv, pin, Some(custom_group)));
        p.layout.toggle_pin_group(Some(custom_group));
        let root = p.inv.root;
        let mut loose = p.inv.items[&id(20)].clone();
        loose.id = id(21);
        loose.parent = root;
        loose.name = "Loose note".into();
        p.inv.items.insert(loose.id, loose);
        p.inv.folders.get_mut(&root).unwrap().items.push(id(21));
        p.idle(3);
        p.text_pos("Projects");
        assert!(p.tab.rows.contains(&Row::Place(Location::MyFolders)));
        assert!(p.tab.rows.contains(&Row::Place(pin)));
        assert!(!p.tab.rows.contains(&Row::Place(Location::Folder(id(4)))));
        assert!(!p.tab.rows.contains(&Row::Item(id(21))));
        p.click_text("General Inventory", PointerButton::Primary);
        p.click_text("General Inventory", PointerButton::Primary);
        assert_eq!(p.tab.location(), Location::MyFolders);
        assert!(p.tab.rows.contains(&Row::Place(pin)));
        assert!(p.tab.rows.contains(&Row::Place(Location::Folder(id(4)))));
        assert!(p.tab.rows.contains(&Row::Item(id(21))));
        assert_eq!(p.inv.items[&id(21)].parent, root);
        assert_eq!(p.inv.folders[&id(3)].info.parent, root);
        p.tab.history(false);
        p.tab.here.query = "stock clothing".into();
        p.idle(2);
        assert!(!p.tab.on_dashboard());
        assert_eq!(p.tab.rows, [Row::Place(Location::Folder(id(6)))]);
        assert!(p.layout.unpin_location(&p.inv, pin));
        p.tab.here.query.clear();
        p.idle(2);
        assert!(!p.tab.rows.contains(&Row::Place(pin)));
        assert!(p.actions.is_empty());
    }

    #[test]
    fn organized_choose_and_restore_folder_icon_by_pointer() {
        let mut panel = Panel::new();
        let place = Location::Folder(id(4));
        panel.tab.visit(place, false);
        panel.icons = Icons::load(&panel.ctx, None);
        let anchor = Rect::from_min_size(pos2(220.0, 155.0), vec2(150.0, 24.0));
        panel.tab.edit_folder_icon = Some((place, anchor));
        panel.idle(4);
        let texture = panel.icons.get("Inv_Snapshot").unwrap().id();
        let tile = panel
            .output
            .shapes
            .iter()
            .rev()
            .find_map(|s| match &s.shape {
                egui::Shape::Mesh(m) if m.texture_id == texture && m.calc_bounds().width() == 20.0 => Some(m.calc_bounds().center()),
                _ => None,
            })
            .expect("Photos symbol in compact picker");
        panel.pointer(tile, egui::PointerButton::Primary, true);
        panel.pointer(tile, egui::PointerButton::Primary, false);
        panel.idle(2);
        assert_eq!(folder_icons::for_row(&panel.layout, Row::Place(place)), Some("Inv_Snapshot"));
        assert!(panel.tab.edit_folder_icon.is_none());
        panel.tab.edit_folder_icon = Some((place, anchor));
        panel.idle(4);
        panel.click_text(aurora_i18n::tr!("explorer-restore-folder-icon"), egui::PointerButton::Primary);
        assert_eq!(folder_icons::for_row(&panel.layout, Row::Place(place)), None);
        assert!(panel.tab.edit_folder_icon.is_none());
        assert!(panel.actions.is_empty());
    }

    #[test]
    fn organized_ctrl_wheel_and_pinch_change_only_the_pointed_folder_view() {
        use egui::{Event, Modifiers, MouseWheelSource, MouseWheelUnit, TouchPhase};
        use model::{DirectoryView, GroupBy, IconSize, SortKey, ViewLayout};
        let mut p = Panel::new();
        p.tab.visit(Location::MyFolders, false);
        let start = DirectoryView {
            sort: SortKey::Date,
            descending: true,
            group: GroupBy::Type,
            ..Default::default()
        };
        p.layout.set_view(Location::MyFolders, start);
        p.idle(3);
        let here = p.text_pos("Workshop");
        let wheel = |unit, y, modifiers| Event::MouseWheel {
            unit,
            delta: vec2(0.0, y),
            phase: TouchPhase::Move,
            source: MouseWheelSource::Unknown,
            modifiers,
        };
        let draw = |p: &mut Panel, pos, event| p.tick(vec![Event::PointerMoved(pos), event]);
        draw(&mut p, here, wheel(MouseWheelUnit::Line, 1.0, Modifiers::NONE));
        let sidebar = p.text_pos(p.layout.location_label(&p.inv, Location::Library));
        draw(&mut p, sidebar, wheel(MouseWheelUnit::Line, 1.0, Modifiers::CTRL));
        assert_eq!(p.layout.view(Location::MyFolders), start);
        draw(&mut p, here, wheel(MouseWheelUnit::Line, 1.0, Modifiers::CTRL));
        assert_eq!(
            p.layout.view(Location::MyFolders),
            DirectoryView {
                layout: ViewLayout::List,
                ..start
            }
        );
        draw(&mut p, here, wheel(MouseWheelUnit::Point, 24.0, Modifiers::CTRL));
        assert_eq!(p.layout.view(Location::MyFolders).layout, ViewLayout::List);
        draw(&mut p, here, wheel(MouseWheelUnit::Point, 24.0, Modifiers::CTRL));
        assert_eq!(p.layout.view(Location::MyFolders).layout, ViewLayout::Icons(IconSize::Small));
        draw(&mut p, here, Event::Zoom(1.11));
        assert_eq!(p.layout.view(Location::MyFolders).layout, ViewLayout::Icons(IconSize::Medium));
        draw(&mut p, here, wheel(MouseWheelUnit::Line, -3.0, Modifiers::CTRL));
        assert_eq!(p.layout.view(Location::MyFolders), start);
        assert_eq!(p.layout.view(Location::Folder(id(3))), DirectoryView::default());
    }

    #[test]
    fn organized_environment_menu_dispatches_to_self_and_requests_parcel_confirmation() {
        use aurora_i18n::tr;
        use egui::PointerButton::{Primary, Secondary};
        let mut p = Panel::new();
        let target = id(20);
        let asset = id(87);
        let name = "Linked sky";
        let original = p.inv.items.get_mut(&target).unwrap();
        original.asset_type = 56;
        original.inv_type = 25;
        original.asset_id = asset;
        original.flags = 0x100;
        let link = InvItem {
            id: id(21),
            parent: id(3),
            name: name.into(),
            asset_type: 24,
            inv_type: 25,
            asset_id: target,
            ..Default::default()
        };
        p.inv.folders.get_mut(&link.parent).unwrap().items.push(link.id);
        p.inv.items.insert(link.id, link);
        p.tab.visit(Location::Folder(id(3)), false);
        p.idle(3);

        p.click_text(name, Secondary);
        p.click_text(tr!("explorer-set-parcel-environment"), Primary);
        assert!(p.actions.is_empty());
        p.click_text("Bench notes", Primary);
        p.click_text(name, Secondary);
        p.click_text(tr!("common-apply-to-myself-only"), Primary);
        assert!(matches!(&p.actions[..], [InvAction::Environment(a, 0x100, i)] if *a == asset && *i == target));
        p.actions.clear();

        p.facts.parcel_environment = true;
        p.click_text(name, Secondary);
        p.click_text(tr!("explorer-set-parcel-environment"), Primary);
        assert!(matches!(&p.actions[..], [InvAction::EnvironmentParcel(i)] if *i == target));
    }

    #[test]
    fn organized_context_frames_survive_menu_glass_for_items_and_their_flyouts() {
        let dark = crate::theme::Theme::default().skin_palette();
        let light = Palette {
            panel: Color32::from_gray(235),
            raised: Color32::WHITE,
            ink: Color32::BLACK,
            muted_dim: Color32::from_gray(215),
            ..dark
        };
        for palette in [dark, light, crate::high_contrast::palette()] {
            let mut p = Panel::new();
            p.palette = palette;
            p.ctx.on_end_pass(
                "viewer menu glass",
                std::sync::Arc::new(move |ui| crate::ui::menu::glaze(ui.ctx(), &palette)),
            );
            p.tab.visit(Location::Folder(id(3)), false);
            p.idle(3);
            p.click_text("Bench notes", egui::PointerButton::Secondary);
            let frames = |p: &Panel| {
                // Explorer uses the same panel face/border/radius as native menus,
                // including native glass's nested shapes rather than its old opaque shadow.
                let border = crate::ui::widgets::border_stroke(&palette, egui::Stroke::new(1.0, palette.raised)).color;
                let radius = p.ctx.global_style().visuals.window_corner_radius;
                fn count(shape: &egui::Shape, fill: Color32, border: Color32, radius: egui::CornerRadius) -> usize {
                    match shape {
                        egui::Shape::Rect(face) => usize::from(
                            face.fill == fill && face.stroke.color == border && face.stroke.width == 1.0 && face.corner_radius == radius,
                        ),
                        egui::Shape::Vec(parts) => parts.iter().map(|part| count(part, fill, border, radius)).sum(),
                        _ => 0,
                    }
                }
                p.output
                    .shapes
                    .iter()
                    .map(|shape| count(&shape.shape, palette.panel, border, radius))
                    .sum::<usize>()
            };
            p.idle(20);
            assert_eq!(frames(&p), 1, "item menu in {palette:?}");
            let organize = p.text_pos(aurora_i18n::tr!("inventory-menu-organize"));
            p.tick(vec![egui::Event::PointerMoved(organize)]);
            p.idle(20);
            assert_eq!(frames(&p), 2, "item menu and Organize flyout in {palette:?}");
        }
    }

    fn edit_key(key: Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn organized_committing_an_inline_home_folder_rename_keeps_its_parent_and_selection() {
        use aurora_net::inventory::operations::Operation;
        let mut p = Panel::new();
        let backing = id(3);
        let child = id(4);
        p.layout.set_home_folder(backing);
        p.inv.folders.get_mut(&id(1)).unwrap().children.retain(|id| *id != child);
        p.inv.folders.get_mut(&backing).unwrap().children.push(child);
        p.inv.folders.get_mut(&child).unwrap().info.parent = backing;
        let row = Row::Place(Location::Folder(child));
        p.tab.reveal_row = Some(row);
        p.idle(3);
        let hit = p
            .output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "Archive" => {
                    let ink = crate::ui::widgets::ink_rect(&t.galley).translate(t.pos.to_vec2());
                    let visible = ink.intersect(s.clip_rect);
                    visible.is_positive().then(|| visible.center())
                }
                _ => None,
            })
            .expect("Home folder caption must be visible");
        p.pointer(hit, egui::PointerButton::Primary, true);
        p.pointer(hit, egui::PointerButton::Primary, false);
        assert_eq!(p.tab.here.selected, HashSet::from([row]));
        p.tab.file_request = Some(file_actions::Edit::Rename.request(vec![child]));
        p.idle(3);
        assert_eq!(p.tab.action_rename, Some(child));
        p.tick(vec![egui::Event::Text("Builds".into())]);
        p.tick(vec![edit_key(Key::Enter)]);
        p.idle(2);
        assert!(matches!(
            &p.actions[..], [InvAction::Edit(edit)] if matches!(
                &edit.operations[..], [Operation::Patch { id, folder: true, body }]
                    if *id == child && body["name"].to_string_value() == "Builds"
            )
        ));
        assert!(p.tab.action_rename.is_none());
        assert!(p.native.reveal.is_none(), "a host reveal would open this folder on the next frame");
        assert_eq!(p.tab.location(), Location::Home);
        assert_eq!(p.tab.here.selected, HashSet::from([Row::Place(Location::Folder(child))]));
    }

    #[test]
    fn organized_blank_space_new_submenus_create_real_items_in_the_current_folder() {
        use aurora_i18n::tr;
        use aurora_net::inventory::operations::NewItem;
        let primary = egui::PointerButton::Primary;
        let secondary = egui::PointerButton::Secondary;
        for (submenu, command, expected) in [
            (None, tr!("common-new-notecard"), NewItem::Note),
            (
                Some(tr!("ui-inventory-context-new-clothes")),
                tr!("common-new-shirt"),
                NewItem::Wearable(4),
            ),
            (
                Some(tr!("ui-inventory-context-new-body-parts")),
                tr!("common-new-shape"),
                NewItem::Wearable(0),
            ),
        ] {
            let mut p = Panel::new();
            p.tab.visit(Location::Folder(id(3)), false);
            p.idle(4);
            p.click_text("Bench notes", primary);
            assert!(p.tab.here.selected.contains(&Row::Item(id(20))));
            let pane = p.tab.input.viewport.unwrap();
            let blank = pos2(pane.center().x, pane.bottom() - 16.0);
            p.tick(vec![egui::Event::PointerMoved(blank)]);
            p.pointer(blank, secondary, true);
            p.pointer(blank, secondary, false);
            p.idle(3);
            assert!(p.tab.here.selected.is_empty());
            p.click_text(tr!("inventory-menu-new"), primary);
            if let Some(submenu) = submenu {
                p.click_text(submenu, primary);
            }
            p.click_text(command, primary);
            assert!(p.actions.is_empty());
            assert!(p.tab.folder_draft.is_some());
            p.idle(2);
            let name = if expected == NewItem::Wearable(0) {
                p.tick(vec![egui::Event::Text("Custom shape".into())]);
                p.tick(vec![edit_key(Key::Enter)]);
                "Custom shape"
            } else {
                p.tick(vec![edit_key(Key::Escape)]);
                command
            };
            p.idle(2);
            assert!(
                matches!(&p.actions[..], [InvAction::CreateExplorerFolder(r)] if r.parent == id(3) && r.item == Some(expected) && r.name == name)
            );
            assert!(p.native.reveal.is_none());
            assert_eq!(p.tab.location(), Location::Folder(id(3)));
        }
    }

    #[test]
    fn organized_blank_space_creation_rejects_protected_folders_and_rechecks_a_pending_target() {
        let mut inv = fixture();
        let layout = Preferences::default();
        let native = InventoryUi::default();
        let prefs = InventoryPreferences::default();
        let facts = Facts::default();
        let menu = file_actions::Menu {
            native: &native,
            prefs: &prefs,
            facts: &facts,
        };
        let mut state = ExplorerState::at(Location::Group(Group::Appearance));
        let mut actions = Vec::new();
        menus::create(
            &inv,
            &layout,
            &mut state,
            &menu,
            Location::Group(Group::Appearance),
            "create-wearable:0",
            &mut actions,
        );
        assert!(state.new_destination.is_some() && actions.is_empty());
        inv.folders.get_mut(&id(3)).unwrap().info.name = "#RLV".into();
        for place in [
            Location::Folder(id(3)),
            Location::Folder(id(6)),
            Location::Library,
            Location::Group(Group::System),
        ] {
            menus::create(&inv, &layout, &mut state, &menu, place, "create-wearable:0", &mut actions);
            assert!(actions.is_empty());
        }
        menus::create(
            &inv,
            &layout,
            &mut state,
            &menu,
            Location::Folder(id(4)),
            "create-wearable:0",
            &mut actions,
        );
        assert!(actions.is_empty());
        assert!(state.folder_draft.is_some());
        assert_eq!(state.here.location, Location::Folder(id(4)));
    }

    #[test]
    fn organized_address_edit_navigates_waits_for_cache_and_escapes_without_navigation() {
        let mut p = Panel::new();
        p.idle(3);
        let press = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let release = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        p.tab.focus_address();
        p.idle(2);
        p.tick(vec![egui::Event::Text("Tester/Archive".into())]);
        p.tick(vec![press(Key::Enter)]);
        p.tick(vec![release(Key::Enter)]);
        assert_eq!(p.tab.location(), Location::Folder(id(4)));
        p.inv.folders.get_mut(&id(4)).unwrap().state = FetchState::Fetching;
        p.tab.focus_address();
        p.idle(2);
        p.tick(vec![egui::Event::Text("missing".into())]);
        p.tick(vec![press(Key::Enter)]);
        p.tick(vec![release(Key::Enter)]);
        assert!(p.tab.navigation.active());
        assert_eq!(p.tab.location(), Location::Folder(id(4)));
        p.inv.folders.get_mut(&id(4)).unwrap().state = FetchState::Fetched;
        p.idle(2);
        p.tick(vec![press(Key::Escape)]);
        p.tick(vec![release(Key::Escape)]);
        assert!(!p.tab.navigation.active());
        assert_eq!(p.tab.location(), Location::Folder(id(4)));
    }

    #[test]
    fn organized_address_copy_paste_and_search_share_one_row_and_are_independent() {
        use egui::{Event, Key, Modifiers, OutputCommand};
        let mut p = Panel::new();
        p.tab.visit(Location::Folder(id(3)), false);
        p.tab.focus_address();
        p.idle(3);
        let pathname = p.text_pos("Tester\\General Inventory\\Workshop");
        let query = p.text_pos(&aurora_i18n::tr!("explorer-search-in-folder", name = "Workshop".to_string()));
        assert!(query.x > pathname.x && (query.y - pathname.y).abs() < 4.0);
        p.tick(vec![Event::Copy]);
        assert!(
            p.output
                .platform_output
                .commands
                .iter()
                .any(|c| matches!(c, OutputCommand::CopyText(text) if text == "Tester\\General Inventory\\Workshop"))
        );
        p.tick(vec![Event::Paste("Tester\\Archive".into())]);
        p.tick(vec![Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        p.idle(2);
        assert_eq!(p.tab.location(), Location::Folder(id(4)));
        p.tab.focus_search();
        p.idle(2);
        p.tick(vec![Event::Paste("bench".into())]);
        p.idle(2);
        assert_eq!(p.tab.here.query, "bench");
        assert!(p.tab.rows.is_empty());
        p.tab.visit(Location::Home, false);
        p.tab.here.query = "bench".into();
        p.idle(2);
        assert_eq!(p.tab.rows, [Row::Item(id(20))]);
    }

    #[test]
    fn organized_panel_renders_and_search_accepts_keyboard_input() {
        let mut p = Panel::new();
        p.tab.visit(Location::MyFolders, false);
        p.idle(3);
        p.text_pos("Workshop");
        p.tab.focus_search();
        p.idle(2);
        p.tick(vec![egui::Event::Text("Archive".into())]);
        p.idle(2);
        assert_eq!(p.tab.here.query, "Archive");
        assert_eq!(p.tab.rows, vec![Row::Place(Location::Folder(id(4)))]);
        p.text_pos("Archive");
    }

    #[test]
    fn organized_item_drag_scrolls_faster_outside_the_pane_and_stops_on_release() {
        let mut p = Panel::new();
        let item = p.inv.items[&id(20)].clone();
        p.inv.add_items(
            (0..160)
                .map(|n| InvItem {
                    id: id(200 + n),
                    name: format!("Spare {n:03}"),
                    ..item.clone()
                })
                .collect(),
        );
        p.tab.visit(Location::Folder(id(3)), false);
        p.idle(3);
        let start = p.text_pos("Bench notes");
        let viewport = p.tab.input.viewport.unwrap();
        let pointer = |y| egui::Event::PointerMoved(pos2(viewport.center().x, y));
        p.pointer(start, egui::PointerButton::Primary, true);
        p.tick(vec![egui::Event::PointerMoved(start + vec2(12.0, 0.0))]);
        assert!(dragging(&p.ctx));
        p.tick(vec![pointer(viewport.bottom() - 15.0)]);
        p.idle(15);
        let inside = p.tab.here.scroll;
        assert!(inside > 5.0);
        p.tick(vec![pointer(viewport.bottom() + 12.0)]);
        p.idle(15);
        let outside = p.tab.here.scroll;
        assert!(outside - inside > inside * 2.0, "inside {inside}, outside {outside}");
        p.tick(vec![pointer(viewport.top() - 55.0)]);
        p.idle(4);
        assert!(p.tab.here.scroll < outside - 25.0);
        p.pointer(
            pos2(viewport.center().x, viewport.top() - 55.0),
            egui::PointerButton::Primary,
            false,
        );
        p.idle(2);
        let stopped = p.tab.here.scroll;
        p.idle(10);
        assert_eq!(p.tab.here.scroll, stopped);
        assert!(p.actions.is_empty());
    }

    #[test]
    fn organized_pointer_unpins_library_and_moves_folder_without_server_mutation() {
        let mut p = Panel::new();
        p.tab.visit(Location::MyFolders, false);
        p.idle(3);
        let library = p.layout.location_label(&p.inv, Location::Library).to_owned();
        p.click_text(&library, egui::PointerButton::Secondary);
        p.click_text(aurora_i18n::tr!("explorer-panel-unpin"), egui::PointerButton::Primary);
        assert!(!p.layout.pins(&p.inv).contains(&Location::Library));
        let start = p.text_pos("Workshop");
        let end = p.text_pos("Archive");
        p.pointer(start, egui::PointerButton::Primary, true);
        p.tick(vec![egui::Event::PointerMoved(start + vec2(12.0, 0.0))]);
        p.tick(vec![egui::Event::PointerMoved(end)]);
        p.idle(2);
        assert!(dragging(&p.ctx));
        p.pointer(end, egui::PointerButton::Primary, false);
        p.idle(2);
        assert_eq!(p.layout.parent(&p.inv, id(3)), Location::Folder(id(4)));
        assert_eq!(p.inv.folders[&id(3)].info.parent, p.inv.root);
    }

    #[test]
    fn organized_new_folder_button_sends_one_named_operation_and_handles_rejection() {
        let mut p = Panel::new();
        p.tab.visit(Location::Folder(id(3)), false);
        p.idle(3);
        p.click_text(aurora_i18n::tr!("inventory-new"), egui::PointerButton::Primary);
        p.click_text(aurora_i18n::tr!("common-new-folder"), egui::PointerButton::Primary);
        p.idle(2);
        p.tick(vec![egui::Event::Text("Bench tools".into())]);
        p.tick(vec![edit_key(Key::Enter)]);
        let [InvAction::CreateExplorerFolder(origin)] = &p.actions[..] else {
            panic!("expected exactly one folder create")
        };
        let origin = origin.clone();
        assert_eq!(origin.parent, id(3));
        assert_eq!(origin.name, "Bench tools");
        let plan = origin.change(&p.inv).unwrap();
        let [
            aurora_net::inventory::operations::Operation::Folder {
                id: asked,
                parent,
                name,
                type_default,
            },
        ] = &plan.operations[..]
        else {
            panic!("a single native category operation");
        };
        assert!(!asked.is_nil());
        assert_eq!((*parent, name.as_str(), *type_default), (id(3), "Bench tools", -1));
        assert!(!p.inv.folders.contains_key(asked));
        p.idle(3);
        assert_eq!(p.actions.len(), 1);
        folder::finish(
            &mut p.tab,
            &origin,
            Location::Folder(id(3)),
            &Err("Fixture server rejection"),
            false,
        );
        p.idle(2);
        assert_eq!(p.tab.status, "Fixture server rejection");
        p.tick(vec![edit_key(Key::Enter)]);
        assert_eq!(p.actions.len(), 2);
        let InvAction::CreateExplorerFolder(retry) = &p.actions[1] else {
            panic!("expected a folder retry")
        };
        assert_ne!(origin.token, retry.token);
        assert_eq!(retry.name, origin.name);
        assert_eq!(p.inv.folders.len(), 8);
    }

    #[test]
    fn organized_view_menu_changes_only_the_open_folder() {
        let mut p = notes_panel(model::ViewLayout::Details);
        let here = p.tab.location();
        p.click_text(aurora_i18n::tr!("explorer-mode-view"), egui::PointerButton::Primary);
        p.click_text(aurora_i18n::tr!("explorer-mode-large"), egui::PointerButton::Primary);
        assert_eq!(p.layout.view(here).layout, model::ViewLayout::Icons(model::IconSize::Large));
        assert_eq!(p.layout.view(Location::MyFolders).layout, model::ViewLayout::Details);
        p.tab.visit(Location::MyFolders, false);
        p.tick(Vec::new());
        p.click_text(aurora_i18n::tr!("common-name"), egui::PointerButton::Primary);
        assert!(p.layout.view(Location::MyFolders).descending);
        assert!(!p.layout.view(here).descending);
    }

    #[test]
    fn organized_icon_view_uses_native_thumbnail_and_respects_texture_restrictions() {
        let mut p = notes_panel(model::ViewLayout::Icons(model::IconSize::Large));
        let asset = id(13_000);
        p.inv.items.get_mut(&id(20)).unwrap().thumbnail = asset;
        p.tick(Vec::new());
        assert!(p.native.wanted_explorer_images.contains(&(asset, 76)));
        let image = egui::ColorImage::filled([80, 40], Color32::LIGHT_BLUE);
        let handle = p.ctx.load_texture("inventory-test-thumbnail", image, egui::TextureOptions::LINEAR);
        let texture = handle.id();
        p.images.insert(asset, handle);
        p.tick(Vec::new());
        let drawn_size = |output: &egui::FullOutput| {
            output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Mesh(mesh) if mesh.texture_id == texture => Some(mesh.calc_bounds().size()),
                _ => None,
            })
        };
        let size = drawn_size(&p.output).expect("native image rendered");
        assert!((size.x / size.y - 2.0).abs() < 0.01);
        p.native.wanted_explorer_images.clear();
        p.inv.rlva_preview_mask = 1;
        p.tick(Vec::new());
        assert!(drawn_size(&p.output).is_none());
        assert!(!p.native.wanted_explorer_images.iter().any(|(image, _)| *image == asset));
    }

    #[test]
    fn organized_sort_direction_and_real_type_grouping_are_folder_specific() {
        let mut p = notes_panel(model::ViewLayout::Details);
        let here = p.tab.location();
        for (n, date) in [(20, 30), (21, 10), (22, 20)] {
            p.inv.items.get_mut(&id(n)).unwrap().created_at = date;
        }
        p.inv.items.get_mut(&id(21)).unwrap().asset_type = 6;
        p.inv.generation += 1;
        let mut view = p.layout.view(here);
        view.sort = model::SortKey::Date;
        view.descending = true;
        p.layout.set_view(here, view);
        p.tick(Vec::new());
        assert_eq!(p.tab.rows, vec![Row::Item(id(20)), Row::Item(id(22)), Row::Item(id(21))]);
        assert_eq!(p.layout.view(Location::MyFolders), model::DirectoryView::default());
        view.group = model::GroupBy::Type;
        p.layout.set_view(here, view);
        p.tick(Vec::new());
        assert_eq!(p.tab.sections.len(), 2);
        let first = p.tab.rows.iter().position(|r| *r == Row::Item(id(20))).unwrap();
        let second = p.tab.rows.iter().position(|r| *r == Row::Item(id(22))).unwrap();
        assert_eq!(second, first + 1);
    }

    #[test]
    fn organized_drag_over_ordinary_item_shows_denied_destination_and_leaves_source_in_place() {
        let mut p = notes_panel(model::ViewLayout::Details);
        let start = p.text_pos("Bench notes");
        let end = p.text_pos("Check list");
        p.pointer(start, egui::PointerButton::Primary, true);
        p.tick(vec![egui::Event::PointerMoved(start + vec2(12.0, 0.0))]);
        p.tick(vec![egui::Event::PointerMoved(end)]);
        p.idle(2);
        assert!(dragging(&p.ctx));
        assert_eq!(p.output.platform_output.cursor_icon, egui::CursorIcon::NotAllowed);
        p.text_pos(&aurora_i18n::tr!("explorer-denied-drop", destination = "Check list"));
        p.pointer(end, egui::PointerButton::Primary, false);
        assert!(!p.layout.is_placed(id(20)));
        assert_eq!(p.inv.items[&id(20)].parent, id(3));
        assert!(p.actions.is_empty());
    }

    #[test]
    fn organized_rows_publish_native_cargo_and_give_the_entire_selection_once() {
        use super::super::{FolderDrag, InvDrag, drag};
        use egui::PointerButton::Primary;
        let start_drag = |p: &mut Panel, label: &str| {
            let start = p.text_pos(label);
            p.pointer(start, Primary, true);
            let pointer = start + vec2(15.0, 0.0);
            p.tick(vec![egui::Event::PointerMoved(pointer)]);
            pointer
        };
        for (raw, kind, name) in [(false, 0, "Wall texture"), (true, 6, "Chair")] {
            let mut p = notes_panel(model::ViewLayout::Details);
            let item = p.inv.items.get_mut(&id(20)).unwrap();
            item.asset_type = kind;
            item.inv_type = kind;
            item.name = name.into();
            p.inv.generation += 1;
            p.tab.visit(Location::Folder(id(3)), raw);
            p.idle(3);
            start_drag(&mut p, name);
            assert_eq!(egui::DragAndDrop::payload::<InvDrag>(&p.ctx).unwrap().0, id(20));
        }
        let mut p = notes_panel(model::ViewLayout::Details);
        p.tab.visit(Location::MyFolders, false);
        p.idle(3);
        start_drag(&mut p, "Workshop");
        assert_eq!(egui::DragAndDrop::payload::<FolderDrag>(&p.ctx).unwrap().0, id(3));
        assert!(egui::DragAndDrop::payload::<InvDrag>(&p.ctx).is_none());

        let mut p = notes_panel(model::ViewLayout::Details);
        p.tab
            .here
            .replace_selection([Row::Item(id(22)), Row::Item(id(20))].into_iter().collect());
        let pointer = start_drag(&mut p, "Bench notes");
        assert_eq!(drag::carried_ids(&p.ctx).unwrap(), [id(20), id(22)]);
        assert!(egui::DragAndDrop::payload::<InvDrag>(&p.ctx).is_none());
        let mut output = p.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(960.0, 640.0))),
                events: vec![egui::Event::PointerButton {
                    pos: pointer,
                    button: Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    let zone = ui.max_rect();
                    // Private IMs and profiles both use this stock receiver.
                    assert_eq!(drag::give_target(ui, zone), Some((vec![id(20), id(22)], true)));
                    assert!(drag::give_target(ui, zone).is_none());
                    assert!(super::super::world_drag(ui).is_none());
                });
            },
        );
        output.textures_delta.clear();
        assert!(!p.layout.is_placed(id(20)));
        assert!(p.actions.is_empty());
    }

    #[test]
    fn organized_cursor_image_follows_the_grabbed_row_outside_the_source_and_stops_with_the_drag() {
        use egui::PointerButton::Primary;
        for escape in [false, true] {
            let mut p = notes_panel(model::ViewLayout::Details);
            let image = p.ctx.load_texture(
                "held texture",
                egui::ColorImage::filled([40, 20], Color32::WHITE),
                egui::TextureOptions::LINEAR,
            );
            let texture = image.id();
            p.inv.items.get_mut(&id(22)).unwrap().thumbnail = id(90);
            p.images.insert(id(90), image);
            p.tab.here.replace_selection(HashSet::from([Row::Item(id(20)), Row::Item(id(22))]));
            // Grab the second entry: sorting the cargo IDs puts id(20) first.
            let start = p.text_pos("Drawings");
            p.pointer(start, Primary, true);
            p.tick(vec![egui::Event::PointerMoved(start + vec2(15.0, 0.0))]);
            let frame = |p: &Panel, pos: egui::Pos2, events: Vec<egui::Event>| {
                let mut raw = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(960.0, 640.0))),
                    ..Default::default()
                };
                raw.events.push(egui::Event::PointerMoved(pos));
                raw.events.extend(events);
                let mut output = p.ctx.run_ui(raw, |ui| {
                    // The source list no longer renders (eg. after a hover tab switch).
                    let hit = ui.ctx().layer_id_at(pos);
                    super::super::drag::badge_with_inventory(ui.ctx(), &p.palette, &p.inv);
                    assert_eq!(ui.ctx().layer_id_at(pos), hit, "the image is never a drop target");
                });
                output.textures_delta.clear();
                output
            };
            let pos = pos2(645.0, 260.0);
            let a = frame(&p, pos, vec![]);
            let text = |out: &egui::FullOutput, name: &str| {
                out.shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Text(t) if t.galley.text() == name => Some(t.pos),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            let image_size = |out: &egui::FullOutput| {
                out.shapes.iter().find_map(|s| match &s.shape {
                    egui::Shape::Mesh(m) if m.texture_id == texture => Some(m.calc_bounds().size()),
                    _ => None,
                })
            };
            assert_eq!(text(&a, "Drawings").len(), 1);
            assert!(text(&a, "Bench notes").is_empty());
            assert_eq!(text(&a, "2").len(), 1);
            assert!(text(&a, &aurora_i18n::tr!("inventory-move-selection-count", count = 2)).is_empty());
            let size = image_size(&a).expect("loaded inventory thumbnail");
            assert!((size.x / size.y - 2.0).abs() < 0.01);
            p.inv.rlva_preview_mask = 1;
            let delta = vec2(-52.0, 38.0);
            let b = frame(&p, pos + delta, vec![]);
            assert!((text(&b, "Drawings")[0] - text(&a, "Drawings")[0] - delta).length() < 1.1);
            assert!(image_size(&b).is_none(), "a newly restricted texture falls back to the item glyph");
            let stop = if escape {
                edit_key(Key::Escape)
            } else {
                egui::Event::PointerButton {
                    pos: pos + delta,
                    button: Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }
            };
            let c = frame(&p, pos + delta, vec![stop]);
            assert!(text(&c, "Drawings").is_empty());
            assert!(text(&c, "2").is_empty());
        }
    }

    #[test]
    fn organized_mixed_virtual_and_real_selection_never_offers_partial_inventory() {
        let mut p = notes_panel(model::ViewLayout::Details);
        let custom = p.layout.create_custom(&p.inv, Location::Folder(id(3)), "Local folder").unwrap();
        p.idle(2);
        p.tab
            .here
            .replace_selection([Row::Place(custom), Row::Item(id(20))].into_iter().collect());
        let start = p.text_pos("Bench notes");
        p.pointer(start, egui::PointerButton::Primary, true);
        p.tick(vec![egui::Event::PointerMoved(start + vec2(15.0, 0.0))]);
        assert_eq!(drag_count(&p.ctx), Some(2));
        assert!(super::super::drag::carried_ids(&p.ctx).is_none());
    }

    #[test]
    fn organized_folder_takes_object_contents_through_the_native_transfer_action() {
        use super::super::task_drop::TaskDrag;
        use crate::build::task_inventory::{TransferKind, TransferRequest};
        let mut p = Panel::new();
        p.tab.visit(Location::MyFolders, false);
        p.idle(4);
        let folder = p.text_pos("Workshop");
        p.pointer(folder, egui::PointerButton::Primary, true);
        egui::DragAndDrop::set_payload(
            &p.ctx,
            TaskDrag {
                kind: TransferKind::Move { scripted: false },
                requests: vec![TransferRequest {
                    object: id(90),
                    item: aurora_net::build::TaskItem {
                        item_id: id(91),
                        ..Default::default()
                    },
                    folder: Uuid::nil(),
                    approved: None,
                    ignore_warning: false,
                }],
            },
        );
        p.idle(2);
        p.pointer(folder, egui::PointerButton::Primary, false);
        assert!(matches!(&p.actions[..], [InvAction::TakeTask(req)] if
            req.object == id(90) && req.item.item_id == id(91) && req.folder == id(3)
            && req.approved.is_none() && !req.ignore_warning));
    }

    #[test]
    fn organized_pinned_folder_checkboxes_stay_open_for_multiple_changes() {
        use egui::PointerButton::{Primary, Secondary};
        for (width, home_context) in [(960.0, false), (328.0, false), (960.0, true)] {
            let mut p = Panel::new();
            p.width = width;
            p.tab.visit(Location::Folder(id(3)), false);
            p.idle(3);
            if home_context {
                p.click_text("Tester", Secondary);
            } else {
                if width < 400.0 {
                    p.click_text("…", Primary);
                    let view = p.text_pos(aurora_i18n::tr!("explorer-mode-view"));
                    p.tick(vec![egui::Event::PointerMoved(view)]);
                    p.idle(12);
                } else {
                    p.click_text(aurora_i18n::tr!("explorer-mode-view"), Primary);
                }
                let pins = p.text_pos(aurora_i18n::tr!("explorer-mode-pins"));
                p.tick(vec![egui::Event::PointerMoved(pins)]);
                p.idle(12);
            }
            for (group, expected) in [
                (Group::Appearance, false),
                (Group::ObjectsScripts, false),
                (Group::Appearance, true),
            ] {
                p.click_text(group.label(), Primary);
                assert_eq!(p.layout.pins(&p.inv).contains(&Location::Group(group)), expected);
                assert_eq!(
                    p.tab.location(),
                    Location::Folder(id(3)),
                    "toggling a label must keep the current folder"
                );
                p.text_pos(aurora_i18n::tr!("explorer-pins-new-group"));
            }
            let restored: Preferences = serde_json::from_slice(&serde_json::to_vec(&p.layout).unwrap()).unwrap();
            assert_eq!(restored.pins(&p.inv), p.layout.pins(&p.inv));
            if home_context {
                // text_pos chooses the last painted occurrence, the last catalog row.
                let destination = *p.layout.catalog(&p.inv).last().unwrap();
                p.click_text(aurora_i18n::tr!("common-open"), Primary);
                assert_eq!(p.tab.location(), destination);
            } else {
                let outside = pos2(15.0, 622.0);
                p.pointer(outside, Primary, true);
                p.pointer(outside, Primary, false);
                p.idle(2);
            }
            assert!(!egui::Popup::is_any_open(&p.ctx));
        }
    }

    #[test]
    fn organized_command_bar_filters_the_current_visit_and_keeps_checked_actions_in_overflow() {
        use egui::PointerButton::Primary;
        let mut panel = Panel::new();
        let place = Location::Folder(id(3));
        panel.tab.visit(place, false);
        panel.inv.items.get_mut(&id(20)).unwrap().inv_type = 7;
        panel.idle(3);
        panel.click_text(aurora_i18n::tr!("ui-people-filter"), Primary);
        panel.click_text(aurora_i18n::tr!("common-sounds"), Primary);
        assert!(panel.tab.rows.is_empty());
        panel.click_text(aurora_i18n::tr!("ui-people-filter"), Primary);
        panel.click_text(aurora_i18n::tr!("common-notes"), Primary);
        assert_eq!(panel.tab.rows, [Row::Item(id(20))]);

        panel.click_text(aurora_i18n::tr!("inventory-sort-menu"), Primary);
        panel.click_text(aurora_i18n::tr!("explorer-mode-descending"), Primary);
        assert!(panel.layout.view(place).descending);
        panel.click_text("Bench notes", Primary);
        panel.width = 328.0;
        panel.idle(3);
        panel.click_text("…", Primary);
        panel.click_text(aurora_i18n::tr!("common-copy"), Primary);
        assert_eq!(panel.native.clipboard, [id(20)]);

        panel.tab.visit(Location::Home, false);
        assert_eq!(panel.tab.here.type_filter, None);
        panel.tab.history(false);
        assert_eq!(panel.tab.here.type_filter, Some(7));
    }

    #[test]
    fn organized_context_copy_and_toolbar_paste_as_link_share_stock_clipboard_across_tabs() {
        let mut p = Panel::new();
        p.tab = ExplorerState::at(Location::Folder(id(3)));
        p.idle(4);
        p.click_text("Bench notes", egui::PointerButton::Secondary);
        p.click_text(aurora_i18n::tr!("common-copy-2"), egui::PointerButton::Primary);
        assert_eq!(p.native.clipboard, vec![id(20)]);
        assert!(!p.native.cut);

        // Independent tab state, the same real inventory/clipboard. Archive is initially empty.
        p.tab = ExplorerState::at(Location::Folder(id(4)));
        p.idle(3);
        p.click_text("…", egui::PointerButton::Primary);
        p.click_text(aurora_i18n::tr!("ui-inventory-context-paste-as-link"), egui::PointerButton::Primary);
        assert!(p.actions.iter().any(|action| matches!(action, InvAction::Edit(change) if
            matches!(&change.operations[..],[aurora_net::inventory::operations::Operation::Links {parent,links}]
                if *parent == id(4) && links[0].target == id(20)))));
        assert_eq!(p.inv.items[&id(20)].parent, id(3));
    }

    #[test]
    fn organized_sidebar_creates_and_collapses_a_group_and_receives_a_pin_drag() {
        let mut p = Panel::new();
        p.tab.visit(Location::MyFolders, false);
        p.idle(4);
        let left = egui::PointerButton::Primary;
        let right = egui::PointerButton::Secondary;
        p.click_text("Tester", right);
        p.click_text(aurora_i18n::tr!("explorer-pins-new-group"), left);
        assert!(p.tab.pin_editor.is_some());
        p.tick(vec![egui::Event::Text("Workshop shortcuts".into())]);
        p.click_text(aurora_i18n::tr!("common-save"), left);
        assert!(p.tab.pin_editor.is_none());
        let id = p.layout.pin_groups()[0].id;
        let source = p.text_pos(Group::Appearance.label());
        let target = p.text_pos("Workshop shortcuts");
        p.pointer(source, left, true);
        p.tick(vec![egui::Event::PointerMoved(source + vec2(12.0, 0.0))]);
        p.tick(vec![egui::Event::PointerMoved(target)]);
        p.pointer(target, left, false);
        p.idle(3);
        assert_eq!(p.layout.pin_sections(&p.inv)[0].1, [Location::Group(Group::Appearance)]);
        p.click_text("Workshop shortcuts", left);
        assert!(p.layout.pin_groups()[0].collapsed);
        p.click_text("Workshop shortcuts", left);
        assert!(!p.layout.pin_groups()[0].collapsed);
        p.click_text("Workshop shortcuts", right);
        p.click_text(aurora_i18n::tr!("explorer-pins-rename-group"), left);
        p.tick(vec![egui::Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }]);
        p.tick(vec![egui::Event::Text("Favorites".into())]);
        p.click_text(aurora_i18n::tr!("common-save"), left);
        assert_eq!(p.layout.pin_groups()[0].name, "Favorites");
        assert_eq!(p.layout.pin_sections(&p.inv)[0].1, [Location::Group(Group::Appearance)]);
        p.click_text(Group::Appearance.label(), right);
        p.click_text(aurora_i18n::tr!("explorer-pins-move-group"), left);
        p.click_text(aurora_i18n::tr!("explorer-pins-ungrouped"), left);
        assert!(p.layout.pin_sections(&p.inv)[0].1.is_empty());
        p.click_text("Favorites", right);
        p.click_text(aurora_i18n::tr!("explorer-pins-remove-group"), left);
        assert!(!p.layout.pin_groups().iter().any(|g| g.id == id));
        p.text_pos(Group::Appearance.label());
        assert_eq!(p.inv.folders.len(), fixture().folders.len());
    }

    fn notes_panel(view: model::ViewLayout) -> Panel {
        let mut p = Panel::new();
        let folder = Location::Folder(id(3));
        p.tab = ExplorerState::at(folder);
        p.layout.set_view(
            folder,
            model::DirectoryView {
                layout: view,
                ..Default::default()
            },
        );
        for (number, name) in [(21, "Check list"), (22, "Drawings")] {
            p.inv.items.insert(
                id(number),
                InvItem {
                    id: id(number),
                    parent: id(3),
                    name: name.into(),
                    asset_type: 7,
                    ..Default::default()
                },
            );
            p.inv.folders.get_mut(&id(3)).unwrap().items.push(id(number));
        }
        p.inv.generation += 1;
        p.idle(4);
        p
    }

    #[test]
    fn organized_section_headers_collapse_and_expand_in_every_layout() {
        use egui::PointerButton::Primary;
        use model::{GroupBy, IconSize, ViewLayout};
        for layout in [
            ViewLayout::Details,
            ViewLayout::List,
            ViewLayout::Icons(IconSize::Small),
            ViewLayout::Icons(IconSize::Medium),
            ViewLayout::Icons(IconSize::Large),
            ViewLayout::Icons(IconSize::ExtraLarge),
        ] {
            let mut panel = notes_panel(layout);
            let place = panel.tab.location();
            let mut view = panel.layout.view(place);
            view.group = GroupBy::Type;
            panel.layout.set_view(place, view);
            panel.idle(3);
            let original = panel.tab.rows.clone();
            assert_eq!(original.len(), 3);
            let count = format!("({})", aurora_i18n::integer(3));
            let header = panel.text_pos(&count);
            assert!(input::over_header(&panel.tab, header));
            panel.click_text(&count, Primary);
            assert!(panel.tab.rows.is_empty(), "{layout:?}");
            assert!(input::over_header(&panel.tab, panel.text_pos(&count)));
            panel.tick(vec![edit_key(Key::ArrowRight)]);
            panel.idle(2);
            assert_eq!(panel.tab.rows, original, "{layout:?}");
            panel.tick(vec![edit_key(Key::ArrowLeft)]);
            panel.idle(2);
            assert!(panel.tab.rows.is_empty(), "{layout:?}");
            panel.click_text(&count, Primary);
            assert_eq!(panel.tab.rows, original, "{layout:?}");
        }
    }

    #[test]
    fn organized_list_keeps_a_collapsed_wide_group_in_view() {
        use egui::PointerButton::Primary;
        use model::{GroupBy, ViewLayout};
        let mut panel = notes_panel(ViewLayout::List);
        let place = panel.tab.location();
        let mut view = panel.layout.view(place);
        view.group = GroupBy::Type;
        panel.layout.set_view(place, view);
        for n in 100..300 {
            let mut item = panel.inv.items[&id(20)].clone();
            item.id = id(n);
            item.name = format!("Note {n}");
            panel.inv.folders.get_mut(&id(3)).unwrap().items.push(item.id);
            panel.inv.items.insert(item.id, item);
        }
        // Leave a later group to anchor the list's new extent after the wide group closes.
        panel.inv.items.get_mut(&id(22)).unwrap().asset_type = 6;
        panel.inv.generation += 1;
        panel.tab.here.scroll = 500.0;
        panel.tab.restore_scroll = true;
        panel.idle(3);
        let count = format!("({})", aurora_i18n::integer(202));
        assert!(panel.tab.here.scroll >= 490.0);
        let sticky = panel.text_pos(&count);
        assert!(input::over_header(&panel.tab, sticky));
        panel.click_text(&count, Primary);
        assert_eq!(panel.tab.here.scroll, 0.0);
        assert_eq!(panel.tab.rows, [Row::Item(id(22))]);
        panel.click_text(&count, Primary);
        assert_eq!(panel.tab.rows.len(), 203);
    }

    #[test]
    fn organized_home_headers_offer_bulk_collapse_and_expand_without_hiding_the_headers() {
        use aurora_i18n::tr;
        use egui::PointerButton::{Primary, Secondary};
        let mut panel = Panel::new();
        panel.idle(3);
        let original = panel.tab.rows.clone();
        assert!(!original.is_empty());
        panel.click_text(tr!("explorer-home-categories"), Secondary);
        panel.click_text(tr!("explorer-collapse-all-groups"), Primary);
        assert!(panel.tab.rows.is_empty());
        panel.text_pos(tr!("explorer-home-utilities"));
        panel.click_text(tr!("explorer-mode-view"), Primary);
        panel.click_text(tr!("explorer-mode-group"), Primary);
        panel.click_text(tr!("explorer-expand-all-groups"), Primary);
        assert_eq!(panel.tab.rows, original);
    }

    #[test]
    fn organized_menu_clicks_change_visible_order_layout_filter_and_column_sort() {
        use aurora_i18n::tr;
        use egui::PointerButton::Primary;
        use model::{GroupBy, IconSize, SortKey, ViewLayout};
        let mut p = notes_panel(ViewLayout::List);
        let place = p.tab.location();
        for (n, asset_type, inv_type, date) in [(20, 6, 17, 20), (21, 7, 7, 10), (22, 24, 7, 30)] {
            let item = p.inv.items.get_mut(&id(n)).unwrap();
            item.asset_type = asset_type;
            item.inv_type = inv_type;
            item.created_at = date;
        }
        p.inv.items.get_mut(&id(22)).unwrap().asset_id = id(20);
        p.inv.generation += 1;
        p.idle(3);
        let choose = |p: &mut Panel, menu: &str, option: &str| {
            p.click_text(menu, Primary);
            p.click_text(option, Primary);
        };
        let rows = |ids: [u128; 3]| ids.map(|n| Row::Item(id(n)));
        choose(&mut p, tr!("inventory-sort-menu"), tr!("common-type"));
        assert_eq!(p.layout.view(place).sort, SortKey::Type);
        assert_eq!(p.tab.rows, rows([22, 21, 20])); // Link, Notecard, Object
        choose(&mut p, tr!("inventory-sort-menu"), tr!("explorer-mode-descending"));
        assert_eq!(p.tab.rows, rows([20, 21, 22]));
        p.click_text(tr!("explorer-mode-view"), Primary);
        p.click_text(tr!("explorer-mode-group"), Primary);
        p.click_text(tr!("common-type"), Primary);
        assert_eq!(p.layout.view(place).group, GroupBy::Type);
        assert_eq!(p.tab.sections.len(), 3);
        assert_eq!(p.tab.rows, rows([22, 21, 20])); // Group order defaults to ascending, independently of item order.
        let group_from_sort = |p: &mut Panel, option: &str| {
            p.click_text(tr!("inventory-sort-menu"), Primary);
            p.click_text(tr!("explorer-mode-group"), Primary);
            p.click_text(option, Primary);
        };
        group_from_sort(&mut p, tr!("explorer-mode-descending"));
        assert!(p.layout.view(place).group_descending);
        assert_eq!(p.tab.rows, rows([20, 21, 22]));
        group_from_sort(&mut p, tr!("common-type"));
        assert!(p.layout.view(place).group_descending); // Re-selecting the active field keeps the chosen order.
        group_from_sort(&mut p, tr!("common-name"));
        assert_eq!(p.layout.view(place).group, GroupBy::Name);
        assert!(!p.layout.view(place).group_descending);
        group_from_sort(&mut p, tr!("explorer-mode-date"));
        assert_eq!(p.layout.view(place).group, GroupBy::Date);
        assert!(p.layout.view(place).group_descending);
        assert_eq!(p.layout.view(place).sort, SortKey::Type);
        group_from_sort(&mut p, tr!("common-none"));
        assert_eq!(p.layout.view(place).group, GroupBy::None);

        choose(&mut p, tr!("ui-people-filter"), tr!("common-objects"));
        assert_eq!(p.tab.rows, [Row::Item(id(20)), Row::Item(id(22))]); // attachment and link to it
        choose(&mut p, tr!("ui-people-filter"), tr!("common-all"));
        assert_eq!(p.tab.rows, rows([20, 21, 22]));
        for (label, layout) in [
            (tr!("explorer-mode-small"), ViewLayout::Icons(IconSize::Small)),
            (tr!("explorer-mode-list"), ViewLayout::List),
            (tr!("explorer-mode-details"), ViewLayout::Details),
        ] {
            choose(&mut p, tr!("explorer-mode-view"), label);
            assert_eq!(p.layout.view(place).layout, layout);
            for name in ["Bench notes", "Check list", "Drawings"] {
                p.text_pos(name);
            }
        }
        p.click_text(tr!("explorer-mode-date"), Primary);
        assert_eq!(p.layout.view(place).sort, SortKey::Date);
        assert!(p.layout.view(place).descending);
        assert_eq!(p.tab.rows, rows([22, 20, 21]));
        p.click_text(tr!("explorer-mode-date"), Primary);
        assert!(!p.layout.view(place).descending);
        assert_eq!(p.tab.rows, rows([21, 20, 22]));
        assert_eq!(p.layout.view(Location::MyFolders), model::DirectoryView::default());
    }

    #[test]
    fn organized_folder_filters_include_an_unfetched_folder_link_and_exclude_it_from_assets() {
        let mut inv = fixture();
        let item = inv.items.get_mut(&id(20)).unwrap();
        item.asset_type = 25;
        item.inv_type = -1;
        item.asset_id = id(999);
        let row = Row::Item(id(20));
        assert!(commands::matches_filter(&inv, row, 8));
        for &(kind, _) in super::super::view::types() {
            assert!(!commands::matches_filter(&inv, row, kind));
        }
    }

    fn select_with(p: &mut Panel, name: &str, mods: egui::Modifiers) {
        let pos = p.text_pos(name);
        for pressed in [true, false] {
            p.tick(vec![
                egui::Event::ModifiersChanged(mods),
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: mods,
                },
            ]);
        }
        p.tick(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
    }

    #[test]
    fn collapsed_groups_exclude_members_from_range_keyboard_drag_and_command_selection() {
        use egui::{Event, Modifiers, PointerButton};
        use model::{GroupBy, GroupKey, NameBucket, ViewLayout};
        let mut p = notes_panel(ViewLayout::Details);
        let place = p.tab.location();
        p.inv.items.get_mut(&id(21)).unwrap().name = "Middle inventory".into();
        p.inv.items.get_mut(&id(22)).unwrap().name = "Zebra supplies".into();
        p.inv.generation += 1;
        p.layout.set_view(
            place,
            model::DirectoryView {
                group: GroupBy::Name,
                ..Default::default()
            },
        );
        p.idle(2);

        select_with(&mut p, "Middle inventory", Modifiers::NONE);
        p.layout.set_group_collapsed(place, &GroupKey::Name(NameBucket::ItoP), true);
        p.idle(2);
        let expected = HashSet::from([Row::Item(id(20)), Row::Item(id(22))]);
        assert_eq!(p.tab.total_rows, 3);
        assert_eq!(p.tab.rows.iter().copied().collect::<HashSet<_>>(), expected);
        assert!(p.tab.here.selected.is_empty() && p.tab.anchor.is_none() && p.tab.input.caret.is_none());
        assert!(file_actions::ids(&p.tab).is_none());
        p.text_pos(&aurora_i18n::tr!("explorer-panel-entry-count", count = 3));

        select_with(&mut p, "Bench notes", Modifiers::NONE);
        p.tick(vec![edit_key(Key::ArrowDown)]);
        assert_eq!(p.tab.here.selected, HashSet::from([Row::Item(id(22))]));
        let ctrl = Modifiers::CTRL;
        p.tick(vec![
            Event::ModifiersChanged(ctrl),
            Event::Key {
                key: Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: ctrl,
            },
        ]);
        assert_eq!(p.tab.here.selected, expected);
        select_with(&mut p, "Bench notes", Modifiers::NONE);
        select_with(&mut p, "Zebra supplies", Modifiers::SHIFT);
        assert_eq!(file_actions::ids(&p.tab).unwrap(), vec![id(20), id(22)]);

        let start = p.text_pos("Zebra supplies");
        p.pointer(start, PointerButton::Primary, true);
        p.tick(vec![Event::PointerMoved(start + vec2(14.0, 0.0))]);
        assert_eq!(super::super::drag::Cargo::carried(&p.ctx).unwrap().ids(), &[id(20), id(22)]);
        p.pointer(start, PointerButton::Primary, false);

        // Collapse everything, then reveal a selected search result in its real parent.
        for bucket in [NameBucket::AtoH, NameBucket::QtoZ] {
            p.layout.set_group_collapsed(place, &GroupKey::Name(bucket), true);
        }
        p.idle(2);
        assert!(p.tab.rows.is_empty());
        p.text_pos(&aurora_i18n::tr!("explorer-panel-entry-count", count = 3));
        file_actions::show_parent(&p.inv, &mut p.tab, Row::Item(id(21)), id(3));
        p.idle(2);
        assert_eq!(p.tab.rows, [Row::Item(id(21))]);
        assert_eq!(file_actions::ids(&p.tab).unwrap(), vec![id(21)]);
        p.text_pos("Middle inventory");
    }

    #[test]
    fn group_disclosures_reuse_ordered_search_results_across_tabs_until_inventory_or_sort_changes() {
        let mut inv = fixture();
        let mut layout = Preferences::default();
        let place = Location::Home;
        let mut view = model::DirectoryView {
            group: model::GroupBy::Type,
            ..Default::default()
        };
        layout.set_view(place, view);
        let mut first = ExplorerState::at(place);
        let mut second = ExplorerState::at(place);
        for tab in [&mut first, &mut second] {
            tab.here.query = "bench".into();
            refresh_rows(&inv, &mut layout, tab, "Tester");
        }
        let key = first.sections[0].key.unwrap();
        let buffer = second.ordered_rows.as_ref().unwrap().as_ptr();
        let scanned = Instant::now() - Duration::from_secs(1);
        second.rows_at = Some(scanned);
        for collapsed in [true, false, true] {
            layout.set_group_collapsed(place, &key, collapsed);
            for tab in [&mut first, &mut second] {
                refresh_rows(&inv, &mut layout, tab, "Tester");
                assert_eq!(tab.rows.is_empty(), collapsed);
                assert_eq!(tab.total_rows, 1);
            }
            assert_eq!(second.rows_at, Some(scanned));
            assert_eq!(second.ordered_rows.as_ref().unwrap().as_ptr(), buffer);
        }

        view.descending = true;
        layout.set_view(place, view);
        refresh_rows(&inv, &mut layout, &mut second, "Tester");
        assert_ne!(second.rows_at, Some(scanned));
        assert!(second.rows.is_empty() && second.total_rows == 1);
        second.rows_at = Some(scanned);
        inv.items.get_mut(&id(20)).unwrap().name = "Moved design".into();
        inv.generation += 1;
        refresh_rows(&inv, &mut layout, &mut second, "Tester");
        assert_ne!(second.rows_at, Some(scanned));
        assert_eq!(second.total_rows, 0);
        view.group = model::GroupBy::None;
        layout.set_view(place, view);
        refresh_rows(&inv, &mut layout, &mut second, "Tester");
        assert!(second.ordered_rows.is_none());
    }

    #[test]
    fn grouped_cache_rebuilds_after_a_local_day_or_language_change() {
        let inv = fixture();
        let place = Location::Folder(id(3));
        let mut layout = Preferences::default();
        let mut state = ExplorerState::at(place);
        layout.set_view(
            place,
            model::DirectoryView {
                group: model::GroupBy::Date,
                ..Default::default()
            },
        );
        refresh_rows(&inv, &mut layout, &mut state, "Tester");
        let key = state.sections[0].key.unwrap();
        layout.set_group_collapsed(place, &key, true);
        refresh_rows(&inv, &mut layout, &mut state, "Tester");
        let today = state.rows_group_date.unwrap();
        let label = state.sections[0].label.clone();

        state.rows_group_date = today.pred_opt();
        state.sections[0].label.clear();
        refresh_rows(&inv, &mut layout, &mut state, "Tester");
        assert_eq!(state.rows_group_date, Some(today));
        assert_eq!(state.sections[0].label, label);
        state.rows_language = "previous-locale";
        state.sections[0].label.clear();
        refresh_rows(&inv, &mut layout, &mut state, "Tester");
        assert_eq!(state.sections[0].label, label);
        assert_eq!(state.rows_language, aurora_i18n::language());
        assert!(state.rows.is_empty() && layout.group_collapsed(place, &key));
    }

    #[test]
    fn collapsed_names_reveal_new_draft_and_server_acknowledged_rename() {
        use model::{GroupBy, GroupKey, NameBucket, ViewLayout};
        let mut p = notes_panel(ViewLayout::Details);
        let place = p.tab.location();
        let early = GroupKey::Name(NameBucket::AtoH);
        let late = GroupKey::Name(NameBucket::QtoZ);
        p.inv.items.get_mut(&id(22)).unwrap().name = "Zebra supplies".into();
        p.inv.generation += 1;
        p.layout.set_view(
            place,
            model::DirectoryView {
                group: GroupBy::Name,
                ..Default::default()
            },
        );
        for key in [&early, &late] {
            p.layout.set_group_collapsed(place, key, true);
        }
        folder::start(&mut p.tab, Some(id(3)), None, "Zero folder", false);
        p.idle(2);
        let draft = p.tab.folder_draft.as_ref().unwrap().row(&p.tab).unwrap();
        let section = p.tab.sections.iter().find(|s| s.key.as_ref() == Some(&late)).unwrap();
        assert_eq!(section.count, 2);
        assert!(p.tab.rows[section.entries.clone()].contains(&draft));
        assert!(p.layout.group_collapsed(place, &early));
        assert!(!p.layout.group_collapsed(place, &late));

        p.tab.folder_draft = None;
        p.tab.rows_key = None;
        p.layout.set_group_collapsed(place, &late, true);
        assert!(p.tab.reveal(&p.inv, &p.layout, id(20)));
        p.idle(2);
        assert!(p.tab.rows.contains(&Row::Item(id(20))));
        assert!(p.layout.group_collapsed(place, &late));
        let entry = p.inv.items.get_mut(&id(20)).unwrap();
        entry.owner_mask = u32::MAX;
        entry.everyone_mask = u32::MAX;
        p.tab.file_request = Some(file_actions::Edit::Rename.request(vec![id(20)]));
        p.idle(3);
        p.tick(vec![egui::Event::Text("Zulu notes".into())]);
        p.tick(vec![edit_key(Key::Enter)]);
        p.idle(2);
        assert!(p.tab.action_rename.is_none());
        assert!(p.layout.group_collapsed(place, &late));
        p.inv.items.get_mut(&id(20)).unwrap().name = "Zulu notes".into();
        p.inv.generation += 1;
        p.idle(2);
        assert_eq!(file_actions::ids(&p.tab), Some(vec![id(20)]));
        assert!(!p.layout.group_collapsed(place, &late));
        p.text_pos("Zulu notes");
    }

    #[test]
    fn organized_ctrl_shift_and_group_drop_use_the_entire_selection() {
        for view in [
            model::ViewLayout::Details,
            model::ViewLayout::List,
            model::ViewLayout::Icons(model::IconSize::Medium),
        ] {
            let mut p = notes_panel(view);
            let ctrl = egui::Modifiers {
                ctrl: true,
                ..Default::default()
            };
            select_with(&mut p, "Bench notes", egui::Modifiers::NONE);
            select_with(&mut p, "Drawings", ctrl);
            assert_eq!(
                p.tab.here.selected,
                HashSet::from([Row::Item(id(20)), Row::Item(id(22))]),
                "{view:?}"
            );
            select_with(&mut p, "Drawings", ctrl);
            assert_eq!(p.tab.here.selected, HashSet::from([Row::Item(id(20))]));
            // Reset anchor, then select the inclusive range and drag it from its middle.
            select_with(&mut p, "Bench notes", egui::Modifiers::NONE);
            select_with(&mut p, "Drawings", egui::Modifiers::SHIFT);
            assert_eq!(p.tab.here.selected.len(), 3, "shift range in {view:?}");
            let source = p.text_pos("Check list");
            let target = p.text_pos(p.layout.location_label(&p.inv, Location::Folder(id(7))));
            p.pointer(source, egui::PointerButton::Primary, true);
            p.tick(vec![egui::Event::PointerMoved(source + vec2(14.0, 1.0))]);
            p.tick(vec![egui::Event::PointerMoved(target)]);
            assert!(dragging(&p.ctx));
            p.pointer(target, egui::PointerButton::Primary, false);
            p.idle(2);
            for n in 20..=22 {
                assert_eq!(p.layout.parent(&p.inv, id(n)), Location::Folder(id(7)), "{view:?}");
                assert_eq!(p.inv.items[&id(n)].parent, id(3));
            }
        }
    }

    #[test]
    fn organized_marquee_selects_and_contracts_in_details_and_icons() {
        for view in [model::ViewLayout::Details, model::ViewLayout::Icons(model::IconSize::Medium)] {
            let mut p = notes_panel(view);
            let first = p.text_pos("Bench notes");
            let second = p.text_pos("Check list");
            let third = p.text_pos("Drawings");
            let start = third + vec2(0.0, 76.0);
            p.pointer(start, egui::PointerButton::Primary, true);
            p.tick(vec![egui::Event::PointerMoved(first)]);
            p.idle(1);
            assert_eq!(p.tab.here.selected.len(), 3, "expand {view:?}");
            p.tick(vec![egui::Event::PointerMoved(second)]);
            p.pointer(second, egui::PointerButton::Primary, false);
            assert_eq!(
                p.tab.here.selected,
                HashSet::from([Row::Item(id(21)), Row::Item(id(22))]),
                "contract {view:?}"
            );
        }
    }
}
