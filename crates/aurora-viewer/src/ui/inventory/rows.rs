//! The plain inventory tree (no search, no filter, the Inventory tab and the
//! folder windows) as a flat list of rows, rebuilt only when the inventory,
//! the sort or an open folder changes, and laid out row by row in sight only.
//! LLFolderView::arrange (indra/llui/llfolderview.cpp) likewise skips what is
//! scrolled out of view. Laying out every open row each frame cost 12 ms per
//! frame with a 2,000 item folder open, and a 50 ms frame at each opening.
//!
//! The open folders live here, per window, instead of one egui collapsing
//! state per folder; the main window's are kept with the layout, as egui kept
//! those states.

use super::view::ViewCache;
use super::*;

/// Folders nested deeper are not listed (malformed parent cycles).
pub const MAX_DEPTH: usize = 24;
/// The height of a row at font scale 1, without the spacing between rows.
pub const ROW_HEIGHT: f32 = 20.0;

/// LLInventoryPanel::openAllFolders / closeAllFolders recurse through every
/// known descendant, including branches hidden by a closed ancestor.
pub(super) fn folders(view: &ViewCache, inv: &Inventory, root: Uuid, show_root: bool, show_library: bool) -> Vec<Uuid> {
    let mut todo = vec![(root, 0)];
    let mut seen = HashSet::new();
    let mut folders = Vec::new();
    while let Some((id, depth)) = todo.pop() {
        if depth > MAX_DEPTH || !inv.folders.contains_key(&id) || !seen.insert(id) {
            continue;
        }
        if show_root || id != root {
            folders.push(id);
        }
        todo.extend(
            view.children_iter(id, show_library, inv.lib_root)
                .filter(|id| inv.folders.contains_key(id))
                .map(|id| (id, depth + 1)),
        );
    }
    folders
}

/// The height of the rows of this window: Aurora's are taller. A Legacy-style
/// row is as tall as its tallest label, an item's name with its rights letters
/// (`permissions::Permissions::append_suffix`, in another face), which a font
/// scale above 1 makes taller than `ROW_HEIGHT` (its icons stay 16 points).
/// The lists scroll and draw by this height, so it must be the drawn one:
/// LLFolderViewItem::getItemHeight (indra/llui/llfolderviewitem.cpp) is
/// likewise the height every row is laid out at. Aurora difference: there it
/// is the `FSFolderViewItemHeight` setting, 20 by default, whatever the font
/// (LLInventoryPanel::createFolderRoot, createFolderViewFolder and
/// createFolderViewItem in indra/newview/llinventorypanel.cpp); here a row
/// grows when the font scale makes its label taller.
pub fn row_height(ui: &egui::Ui, p: &Palette, st: &InventoryUi) -> f32 {
    if st.aurora.is_some() {
        return super::aurora::rows::HEIGHT;
    }
    // The name, a worn one's in semi-bold, then the rights: a line is as tall
    // as its tallest face, and egui rounds a laid out line's height up.
    let mut label = tree::name("Ag", p);
    label.append("Ag", 0.0, crate::ui::widgets::bold_format(ui, tree::font(), p.ink));
    permissions::Permissions::default().append_suffix(ui, &mut label, p);
    let text = ui.fonts_mut(|fonts| fonts.layout_job(label).size().y);
    ROW_HEIGHT.max(text + 2.0 * crate::ui::widgets::INVENTORY_LABEL_PADDING.y)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder {
        open: bool,
        /// Whether it has a caret and opens ([`branches`]).
        branches: bool,
    },
    Item,
    /// "Empty" or "Loading folder" under an open folder without children.
    Empty,
    /// The header of a group of the explorer's list (`groups.rs`), the
    /// `index`th of `PlainTree::groups`; its `id` is the folder listed.
    Group {
        index: usize,
        open: bool,
    },
}

/// Whether a folder's row has a caret and opens: while it lists something
/// or its contents are not known yet. LLFolderViewItem::drawOpenFolderArrow
/// (indra/llui/llfolderviewitem.cpp) draws the arrow only when
/// `hasVisibleChildren() || !isFolderComplete()`; a folder fetched and empty
/// has none (issue #4765). Aurora difference: such a folder does not open
/// either, where the Legacy Viewer still toggles it to show nothing.
pub(super) fn branches(view: &ViewCache, inv: &Inventory, show_library: bool, id: Uuid) -> bool {
    inv.folders.get(&id).is_some_and(|folder| folder.state != FetchState::Fetched)
        || view.children_iter(id, show_library, inv.lib_root).next().is_some()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    /// The folder or the item; the folder itself for `Kind::Empty`.
    pub id: Uuid,
    pub level: usize,
    pub kind: Kind,
    /// The row of the folder this one is listed in (`NONE` at the top).
    pub parent: usize,
    /// An open folder's last descendant row (its own index otherwise).
    pub end: usize,
}

pub const NONE: usize = usize::MAX;

/// What the explorer's folder shows as tiles (`aurora::gallery`): nothing
/// (its list), every entry (the gallery view), or the entries that have a
/// picture, listed before the others (the combination view).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tiles {
    #[default]
    None,
    All,
    Pictured,
    /// Every entry, in the list's own order (the Compact and Tiles views).
    Listed,
}

#[derive(Default)]
pub struct PlainTree {
    open: HashSet<Uuid>,
    restored: bool,
    revision: u64,
    saved: u64,
    key: Option<(u64, Uuid, bool, bool, u64, Tiles)>,
    tiles_mode: Tiles,
    /// The first rows, drawn as tiles; the list draws the rows after them.
    pub tiles: usize,
    pub rows: Vec<Row>,
    /// The listed folders and items, in order (Shift selection).
    pub order: Vec<Uuid>,
    /// The open folders listed: fetched while their contents are unknown.
    pub listed_open: Vec<Uuid>,
    /// The widest row measured since the last rebuild (horizontal range).
    pub width: f32,
    /// The words the explorer's folder filter keeps (Ctrl+Alt+F, lowercase):
    /// only its own entries, not a whole tree's.
    filter: Vec<String>,
    /// The entries the filter left out of the folder listed.
    pub filtered_out: usize,
    /// What the explorer's folder is grouped by, and what changes its groups
    /// (the creators' names known).
    group: (super::groups::GroupBy, u64),
    /// The groups listed: their names and how many entries each holds.
    pub groups: Vec<(String, usize)>,
    /// The groups closed, by name.
    closed_groups: HashSet<String>,
    /// The filter or a closed group hid entries since the list was last
    /// drawn: the selection drops what is no longer shown (`take_hidden`).
    hid: bool,
}

impl PlainTree {
    /// Lists only the folder's entries whose names hold every word of
    /// `text` (Dolphin's filter bar, any case); empty lists them all.
    pub fn set_filter(&mut self, text: &str) {
        let words: Vec<String> = text.to_lowercase().split_whitespace().map(str::to_owned).collect();
        if words != self.filter {
            self.hid |= !words.is_empty();
            self.filter = words;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Groups the folder's entries (`groups.rs`), `context` being what
    /// changes their groups.
    pub fn set_group(&mut self, by: super::groups::GroupBy, context: u64) {
        let context = if by == super::groups::GroupBy::Creator { context } else { 0 };
        if self.group != (by, context) {
            // another grouping opens every group (their names may repeat)
            if self.group.0 != by {
                self.closed_groups.clear();
            }
            self.group = (by, context);
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Opens or closes the group `name`.
    pub fn toggle_group(&mut self, name: &str) {
        if !self.closed_groups.remove(name) {
            self.closed_groups.insert(name.to_owned());
            self.hid = true;
        }
        self.revision = self.revision.wrapping_add(1);
    }

    /// Whether entries were hidden since the last call.
    pub fn take_hidden(&mut self) -> bool {
        std::mem::take(&mut self.hid)
    }

    /// Whether the folder filter keeps `id`.
    fn kept(&self, inv: &Inventory, id: Uuid) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        let name = match inv.items.get(&id) {
            Some(it) => it.name.to_lowercase(),
            None => folder_name(inv, id).to_lowercase(),
        };
        self.filter.iter().all(|word| name.contains(word.as_str()))
    }

    pub fn is_open(&self, id: Uuid) -> bool {
        self.open.contains(&id)
    }

    /// Whether the folder's row is listed with a caret ([`branches`]).
    pub fn folds(&self, id: Uuid) -> bool {
        self.rows
            .iter()
            .any(|row| row.id == id && matches!(row.kind, Kind::Folder { branches: true, .. }))
    }

    pub fn set_open(&mut self, id: Uuid, open: bool) {
        let changed = if open { self.open.insert(id) } else { self.open.remove(&id) };
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn open_all(&mut self, ids: impl IntoIterator<Item = Uuid>) {
        for id in ids {
            self.set_open(id, true);
        }
    }

    /// Expand every known folder; ordinary lazy loading fetches their contents.
    /// Collapse also closes descendants, keeping only a displayed root open.
    pub fn expand(&mut self, view: &ViewCache, inv: &Inventory, root: Uuid, show_root: bool, show_library: bool, open: bool) {
        for id in folders(view, inv, root, show_root, show_library) {
            self.set_open(id, open || (show_root && id == root));
        }
    }

    /// Which entries of the listed folder are tiles; only the explorer's
    /// folder, listed without its root, has any.
    pub fn set_tiles(&mut self, tiles: Tiles) {
        self.tiles_mode = tiles;
    }

    /// The first row the list draws: the tiles come before it.
    pub fn list_start(&self) -> usize {
        if self.tiles_mode == Tiles::Pictured { self.tiles } else { 0 }
    }

    /// List the rows again if the inventory, the sort or an open folder
    /// changed since the last call. Returns whether it did.
    pub fn prepare(&mut self, view: &ViewCache, inv: &Inventory, root: Uuid, show_root: bool, show_library: bool) -> bool {
        let tiles = if show_root { Tiles::None } else { self.tiles_mode };
        let key = (view.built(), root, show_root, show_library, self.revision, tiles);
        if self.key == Some(key) {
            return false;
        }
        self.rows.clear();
        self.order.clear();
        self.listed_open.clear();
        self.width = 0.0;
        self.tiles = 0;
        self.filtered_out = 0;
        self.groups.clear();
        if show_root {
            self.folder(view, inv, show_library, root, 0, 0, NONE);
        } else {
            self.listed_open.push(root);
            if tiles == Tiles::None {
                self.children(view, inv, show_library, root, 1, 0, NONE);
            } else {
                self.tiled(view, inv, show_library, root, tiles);
            }
        }
        self.key = Some(key);
        true
    }

    /// The explorer's folder with tiles, in the gallery's order
    /// (LLInventoryGallery::compareGalleryItem): the folders before the
    /// items, and in each group the entries with a picture first, in the
    /// list's order otherwise. The combination view's gallery takes the
    /// entries with a picture and leaves the others to its list, in the
    /// list's order (LLPanelMainInventory::setViewMode with
    /// FILTER_ONLY_THUMBNAILS and FILTER_EXCLUDE_THUMBNAILS,
    /// indra/newview/llpanelmaininventory.cpp).
    fn tiled(&mut self, view: &ViewCache, inv: &Inventory, show_library: bool, folder: Uuid, tiles: Tiles) {
        let mut entries: Vec<(Uuid, bool, bool)> = view
            .children_iter(folder, show_library, inv.lib_root)
            .filter(|id| {
                let kept = self.kept(inv, *id);
                self.filtered_out += usize::from(!kept);
                kept
            })
            .filter_map(|id| {
                let is_folder = inv.folders.contains_key(&id);
                (is_folder || inv.items.contains_key(&id)).then(|| (id, is_folder, thumbnail::tile_picture(inv, id).is_some()))
            })
            .collect();
        match tiles {
            Tiles::All => entries.sort_by_key(|&(_, is_folder, pictured)| (!is_folder, !pictured)),
            Tiles::Listed => {}
            _ => entries.sort_by_key(|&(_, _, pictured)| !pictured),
        }
        self.tiles = match tiles {
            Tiles::All | Tiles::Listed => entries.len(),
            _ => entries.iter().filter(|(_, _, pictured)| *pictured).count(),
        };
        for (id, is_folder, _) in entries {
            self.rows.push(Row {
                id,
                level: 1,
                // the explorer goes into a tile's folder, empty or not
                kind: if is_folder {
                    Kind::Folder {
                        open: false,
                        branches: true,
                    }
                } else {
                    Kind::Item
                },
                parent: NONE,
                end: self.rows.len(),
            });
            self.order.push(id);
        }
        if self.rows.is_empty() {
            self.rows.push(Row {
                id: folder,
                level: 1,
                kind: Kind::Empty,
                parent: NONE,
                end: 0,
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn folder(&mut self, view: &ViewCache, inv: &Inventory, show_library: bool, id: Uuid, depth: usize, level: usize, parent: usize) {
        if depth > MAX_DEPTH || !inv.folders.contains_key(&id) {
            return;
        }
        // A shown root stays open, with "Empty" under it when it is.
        let branches = depth == 0 || branches(view, inv, show_library, id);
        let open = branches && self.open.contains(&id);
        let index = self.rows.len();
        self.rows.push(Row {
            id,
            level,
            kind: Kind::Folder { open, branches },
            parent,
            end: index,
        });
        self.order.push(id);
        if open {
            self.listed_open.push(id);
            self.children(view, inv, show_library, id, depth + 1, level + 1, index);
            self.rows[index].end = self.rows.len() - 1;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn children(&mut self, view: &ViewCache, inv: &Inventory, show_library: bool, folder: Uuid, depth: usize, level: usize, parent: usize) {
        if level == 0 && self.group.0 != super::groups::GroupBy::None {
            self.grouped(view, inv, show_library, folder);
            return;
        }
        let mut any = false;
        for child in view.children_iter(folder, show_library, inv.lib_root) {
            // the filter keeps the folder's own entries (the explorer lists
            // them at the first level), not its subfolders' in a tree
            if level == 0 && !self.kept(inv, child) {
                self.filtered_out += 1;
                continue;
            }
            any = true;
            if inv.folders.contains_key(&child) {
                self.folder(view, inv, show_library, child, depth, level, parent);
            } else if inv.items.contains_key(&child) {
                self.rows.push(Row {
                    id: child,
                    level,
                    kind: Kind::Item,
                    parent,
                    end: self.rows.len(),
                });
                self.order.push(child);
            }
        }
        if !any {
            self.rows.push(Row {
                id: folder,
                level,
                kind: Kind::Empty,
                parent,
                end: self.rows.len(),
            });
        }
    }

    /// The explorer's folder in its groups: a header row each, then its
    /// entries while it is open.
    fn grouped(&mut self, view: &ViewCache, inv: &Inventory, show_library: bool, folder: Uuid) {
        let mut kept = Vec::new();
        for child in view.children_iter(folder, show_library, inv.lib_root) {
            if !self.kept(inv, child) {
                self.filtered_out += 1;
            } else if inv.folders.contains_key(&child) || inv.items.contains_key(&child) {
                kept.push(child);
            }
        }
        if kept.is_empty() {
            self.rows.push(Row {
                id: folder,
                level: 0,
                kind: Kind::Empty,
                parent: NONE,
                end: self.rows.len(),
            });
            return;
        }
        for (group, members) in super::groups::split(inv, view, &kept, self.group.0, super::view::now()) {
            let index = self.groups.len();
            let open = !self.closed_groups.contains(&group.label);
            self.groups.push((group.label, members.len()));
            self.rows.push(Row {
                id: folder,
                level: 0,
                kind: Kind::Group { index, open },
                parent: NONE,
                end: self.rows.len(),
            });
            if !open {
                continue;
            }
            for id in members {
                self.rows.push(Row {
                    id,
                    level: 0,
                    // the explorer goes into a folder, empty or not
                    kind: if inv.folders.contains_key(&id) {
                        Kind::Folder {
                            open: false,
                            branches: true,
                        }
                    } else {
                        Kind::Item
                    },
                    parent: NONE,
                    end: self.rows.len(),
                });
                self.order.push(id);
            }
        }
    }

    /// The row of a folder or an item (not of a placeholder).
    pub fn position(&self, id: Uuid) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| row.id == id && matches!(row.kind, Kind::Folder { .. } | Kind::Item))
    }

    /// The open folders whose guide crosses `range`: the ancestors of its
    /// first row and the open folders inside it.
    pub fn guides(&self, range: std::ops::Range<usize>) -> Vec<usize> {
        let mut folders = Vec::new();
        if let Some(first) = self.rows.get(range.start) {
            let mut parent = first.parent;
            while parent != NONE {
                folders.push(parent);
                parent = self.rows[parent].parent;
            }
        }
        folders.extend(range.filter(|i| self.rows.get(*i).is_some_and(|row| row.end > *i)));
        folders
    }

    /// The main window's open folders, kept with the layout like egui's
    /// collapsing states were: read once, written when they change.
    pub fn persist(&mut self, ctx: &egui::Context, id: egui::Id) {
        if !self.restored {
            self.restored = true;
            if let Some(open) = ctx.data_mut(|d| d.get_persisted::<HashSet<Uuid>>(id)) {
                self.open.extend(open);
                self.revision = self.revision.wrapping_add(1);
            }
            self.saved = self.revision;
        }
        if self.saved != self.revision {
            self.saved = self.revision;
            let open = self.open.clone();
            ctx.data_mut(|d| d.insert_persisted(id, open));
        }
    }
}

/// Where Collapse leaves the list: back at its top, with the top folders in
/// sight, rather than at the end of a long list of them (the Legacy
/// Viewer's `LLFolderView::closeAllFolders` keeps the offset). An
/// inventory update held at the old place no longer brings it back.
pub(super) fn top_after_collapse(st: &mut InventoryUi) -> f32 {
    st.scroll_hold = None;
    0.0
}

/// The plain tree in its scroll area, `root` shown as the first row or not
/// (folder windows list its contents only).
#[allow(clippy::too_many_arguments)]
pub(super) fn scroll(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
    height: f32,
    root: Uuid,
    show_root: bool,
) -> egui::scroll_area::ScrollAreaOutput<()> {
    tree_spacing(ui);
    // The explorer's list opens no folder: nothing of it to keep.
    if st.ui_id.is_nil() && !st.aurora.is_some_and(|frame| frame.explorer) {
        st.plain.persist(ui.ctx(), egui::Id::unique("inventory_open_folders"));
    }
    // the Legacy style sorts as the Legacy Viewer: by name or date, the
    // explorer's columns and reversed orders staying in the Aurora style
    let sort = prefs.sort_for(st.tab);
    st.view.prepare(inv, if st.aurora.is_some() { sort } else { sort.legacy() });
    let collapsed = st.expand_all.take().is_some_and(|open| {
        st.plain.expand(&st.view, inv, root, show_root, prefs.show_library, open);
        !open
    });
    // Back from a search: where the list was; else the place `list` gives it
    // (a revealed row below wins).
    let restore = st.scroll_restore.take();
    let mut offset = st.plain_scroll_restore.take().or(restore);
    if collapsed {
        offset = Some(top_after_collapse(st));
    }
    // The root opens the first time it is shown, scrolled to the top.
    if show_root && st.roots_seen.insert(root) {
        st.plain.set_open(root, true);
        offset = Some(0.0);
    }
    // The ancestors of a revealed row open until it is shown.
    st.plain.open_all(st.reveal_path.iter().copied());
    st.plain.prepare(&st.view, inv, root, show_root, prefs.show_library);
    // what the filter or a closed group hides leaves the selection: a
    // Delete after Ctrl+A acts only on what is shown
    if st.plain.take_hidden() {
        let shown: HashSet<Uuid> = st.plain.order.iter().copied().collect();
        st.selection.retain(|id| shown.contains(id));
        if st.selected.is_some_and(|id| !shown.contains(&id)) {
            st.selected = None;
        }
    }
    let navigated = keyboard::navigate(ui, inv, st, false);
    let now = std::time::Instant::now();
    for id in &st.plain.listed_open {
        inv.request_shown(*id, now);
    }
    let height_of_row = row_height(ui, p, st);
    let pitch = height_of_row + ui.spacing().item_spacing.y;
    // the combination view's tiles come first, drawn above the list
    let start = st.plain.list_start();
    let listed = |st: &InventoryUi, id: Option<Uuid>| id.and_then(|id| st.plain.position(id)).and_then(|index| index.checked_sub(start));
    if navigated {
        st.scroll_hold = None;
    }
    if navigated && let Some(index) = listed(st, st.selected) {
        offset = Some(keyboard::scroll_offset(st, index, height_of_row, pitch, height));
    }
    if let Some(index) = listed(st, st.reveal) {
        // Bring it in sight; the row centres itself once drawn.
        offset = Some((index as f32 * pitch - height / 2.0).max(0.0));
    }
    let mut area = list_scroll(ui, height);
    if let Some(offset) = offset {
        area = area.vertical_scroll_offset(offset);
    }
    let total = st.plain.rows.len().saturating_sub(start);
    let out = area.show_viewport(ui, |ui, viewport| {
        ui.set_height((pitch * total as f32 - ui.spacing().item_spacing.y).max(0.0));
        ui.set_min_width(st.plain.width);
        let origin = ui.max_rect().min;
        // As ScrollArea::show_rows, with the renamed row added when out of
        // sight so that its field keeps the keyboard.
        let first = ((viewport.min.y / pitch).floor().max(0.0) as usize).min(total);
        let last = ((viewport.max.y / pitch).ceil().max(0.0) as usize + 1).min(total);
        let x_range = ui.max_rect().x_range();
        let rect = |range: &std::ops::Range<usize>| {
            egui::Rect::from_x_y_ranges(x_range, origin.y + range.start as f32 * pitch..=origin.y + range.end as f32 * pitch)
        };
        let range = first..last;
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect(&range)), |ui| {
            ui.skip_ahead_auto_ids(first);
            rows(ui, p, icons, inv, st, prefs, facts, actions, start + first..start + last);
        });
        // F2 on a selected row scrolled out of sight starts its renaming too.
        let renamed = st.rename.as_ref().map(|edit| edit.id).or_else(|| {
            st.selected
                .filter(|_| st.rename.is_none() && ui.input(|i| i.key_pressed(egui::Key::F2)))
        });
        if let Some(index) = listed(st, renamed)
            && !range.contains(&index)
        {
            let one = index..index + 1;
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect(&one)), |ui| {
                rows(ui, p, icons, inv, st, prefs, facts, actions, start + index..start + index + 1);
            });
        }
        let origin = origin - egui::vec2(0.0, start as f32 * pitch);
        guides(ui, &st.plain, start + first..start + last, origin, pitch, height_of_row);
        ui.set_min_width(st.plain.width);
    });
    st.plain_scroll = out.state.unclamped_offset().y;
    st.list_viewport = Some((out.state.unclamped_offset().y, out.inner_rect.height()));
    super::record_list_shown(ui, st, &out);
    drag::navigation::scroll(ui, &out, st.ui_id);
    out
}

#[allow(clippy::too_many_arguments)]
fn rows(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
    range: std::ops::Range<usize>,
) {
    tree_spacing(ui);
    let height = row_height(ui, p, st);
    for index in range {
        let Some(row) = st.plain.rows.get(index).copied() else { break };
        // Aurora's rows fit the list's width: they cut long names short.
        if row.kind != Kind::Empty && st.aurora.is_none() {
            let width = row_width(
                ui,
                p,
                inv,
                facts,
                row.id,
                matches!(row.kind, Kind::Folder { .. }),
                prefs.protected.contains(&row.id),
                row.level,
                st.show_next_owner,
            );
            st.plain.width = st.plain.width.max(width);
        }
        // A Legacy row's own row centres its contents; this one places it at
        // the top, so the row is as tall as it. Centred here, a row that starts
        // at the theme's interact height sits lower and grows past `height`.
        let legacy = st.aurora.is_none();
        let draw = |ui: &mut egui::Ui| {
            ui.set_min_height(height);
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(row.level as f32 * ui.spacing().indent);
            match row.kind {
                Kind::Folder { open, branches } => {
                    let mut now = open;
                    folder_row(ui, p, icons, inv, row.id, &mut now, branches, st, prefs, facts, actions);
                    if now && !open {
                        inv.open_folder(row.id);
                    }
                    if now != open && st.aurora.is_none() && st.lists_flat() {
                        // the Legacy-style single folder window goes in
                        // (Aurora's explorer goes in from its own row),
                        // empty or not
                        st.browse = row.id;
                    } else if now != open && branches {
                        // cargo held over an empty folder does not open it
                        st.plain.set_open(row.id, now);
                    }
                }
                Kind::Item => item_row(ui, p, icons, inv, &row.id, st, prefs, facts, actions),
                Kind::Group { index, open } => {
                    if let Some((name, count)) = st.plain.groups.get(index).cloned()
                        && super::aurora::rows::group_header(ui, p, &name, count, open)
                    {
                        st.plain.toggle_group(&name);
                    }
                }
                Kind::Empty => {
                    let text = match inv.folders.get(&row.id).map(|f| f.state) {
                        _ if st.plain.filtered_out > 0 => aurora_i18n::tr!("inventory-filter-nothing"),
                        Some(FetchState::Fetched) => aurora_i18n::tr!("status-empty"),
                        // asked again after `SHOWN_RETRY`, not loading meanwhile
                        Some(FetchState::Failed) => aurora_i18n::tr!("ui-inventory-folder-not-loaded"),
                        _ => aurora_i18n::tr!("ui-inventory-loading-folder"),
                    };
                    if st.aurora.is_some() {
                        ui.add_space(super::aurora::rows::LEAD);
                    }
                    let text = RichText::new(text).size(12.0).color(p.muted_dim);
                    if legacy {
                        // Centred in the row, as before it was placed at the top.
                        let centred = egui::Layout::left_to_right(egui::Align::Center);
                        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), height), centred, |ui| ui.label(text));
                    } else {
                        ui.label(text);
                    }
                }
            }
        };
        if legacy {
            ui.horizontal_top(draw);
        } else {
            ui.horizontal(draw);
        }
    }
}

/// The faint line along an open folder's contents (`egui::Ui::indent` draws
/// it, the tree used to be built with it).
fn guides(ui: &egui::Ui, tree: &PlainTree, range: std::ops::Range<usize>, origin: egui::Pos2, pitch: f32, row_height: f32) {
    if !ui.visuals().indent_has_left_vline {
        return;
    }
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    let indent = ui.spacing().indent;
    for index in tree.guides(range) {
        let row = tree.rows[index];
        let x = origin.x + (row.level as f32 + 0.5) * indent;
        let top = origin.y + (index + 1) as f32 * pitch;
        let bottom = origin.y + row.end as f32 * pitch + row_height - 2.0;
        ui.painter().line_segment([egui::pos2(x, top), egui::pos2(x, bottom)], stroke);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::Folder;
    use aurora_net::inventory::{InvFolder, InvItem};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// root 1: folders 10 (items 100, 101; folder 11 with item 110), 20
    /// (empty), library 3.
    fn fixture() -> (Inventory, ViewCache, InventoryPreferences) {
        let mut inv = Inventory {
            root: id(1),
            lib_root: id(3),
            ..Default::default()
        };
        for (n, parent, name) in [(1, 0, "root"), (3, 0, "library"), (10, 1, "A"), (11, 10, "A nested"), (20, 1, "B")] {
            inv.folders.insert(
                id(n),
                Folder {
                    info: InvFolder {
                        id: id(n),
                        parent: id(parent),
                        name: name.into(),
                        type_default: if n == 1 { 8 } else { -1 },
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: Vec::new(),
                    state: FetchState::Fetched,
                    library: n == 3,
                },
            );
        }
        inv.add_items(
            [(100, 10, "a item"), (101, 10, "b item"), (110, 11, "nested item")]
                .map(|(n, parent, name)| InvItem {
                    id: id(n),
                    parent: id(parent),
                    name: name.into(),
                    created_at: n as i64,
                    ..Default::default()
                })
                .to_vec(),
        );
        let prefs = InventoryPreferences {
            sort: view::Sort {
                by_date: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut view = ViewCache::default();
        view.prepare(&inv, prefs.sort);
        (inv, view, prefs)
    }

    fn listed(tree: &PlainTree) -> Vec<(u128, usize, Kind)> {
        tree.rows.iter().map(|r| (r.id.as_u128(), r.level, r.kind)).collect()
    }

    const OPEN: Kind = Kind::Folder {
        open: true,
        branches: true,
    };
    const SHUT: Kind = Kind::Folder {
        open: false,
        branches: true,
    };
    const LEAF: Kind = Kind::Folder {
        open: false,
        branches: false,
    };

    #[test]
    fn only_open_folders_list_their_contents_and_an_empty_one_has_no_caret() {
        let (inv, view, prefs) = fixture();
        let mut tree = PlainTree::default();
        tree.prepare(&view, &inv, inv.root, true, true);
        assert_eq!(listed(&tree), [(1, 0, SHUT)]);
        tree.open_all([id(1), id(10), id(20)]);
        assert!(tree.prepare(&view, &inv, inv.root, true, prefs.show_library));
        // 20 and the library are fetched and empty: no caret, and opening
        // them lists nothing (issue #4765).
        assert_eq!(
            listed(&tree),
            [
                (1, 0, OPEN),
                (10, 1, OPEN),
                (11, 2, SHUT),
                (100, 2, Kind::Item),
                (101, 2, Kind::Item),
                (20, 1, LEAF),
                (3, 1, LEAF),
            ]
        );
        assert!(tree.folds(id(10)) && !tree.folds(id(20)));
        assert_eq!(tree.order, [1, 10, 11, 100, 101, 20, 3].map(id));
        assert_eq!(tree.rows[0].end, 6);
        assert_eq!(tree.rows[1].end, 4);
        assert_eq!(tree.rows[3].parent, 1);
        assert_eq!(tree.listed_open, [1, 10].map(id));
        assert!(!tree.prepare(&view, &inv, inv.root, true, true), "nothing changed");
        // Hiding the library, closing a folder and folder windows.
        assert!(tree.prepare(&view, &inv, inv.root, true, false));
        assert!(!tree.order.contains(&id(3)));
        tree.set_open(id(10), false);
        tree.prepare(&view, &inv, inv.root, true, true);
        assert_eq!(tree.order, [1, 10, 20, 3].map(id));
        tree.prepare(&view, &inv, id(10), false, true);
        assert_eq!(listed(&tree), [(11, 0, SHUT), (100, 0, Kind::Item), (101, 0, Kind::Item)]);
        assert_eq!(tree.listed_open, [id(10)]);
    }

    #[test]
    fn a_folder_keeps_its_caret_until_its_contents_are_known() {
        let (mut inv, mut view, prefs) = fixture();
        let mut tree = PlainTree::default();
        tree.open_all([id(1), id(20)]);
        for state in [FetchState::Unknown, FetchState::Fetching, FetchState::Failed] {
            if let Some(folder) = inv.folders.get_mut(&id(20)) {
                folder.state = state;
            }
            view.prepare(&inv, prefs.sort);
            tree.key = None;
            tree.prepare(&view, &inv, inv.root, true, true);
            let at = tree.position(id(20)).expect("folder 20 is listed");
            assert_eq!(tree.rows[at].kind, OPEN, "{state:?}: the caret stays");
            assert_eq!(tree.rows[at + 1].kind, Kind::Empty, "{state:?}: loading or not loaded");
            assert!(tree.listed_open.contains(&id(20)), "{state:?}: it is fetched");
        }
        // Fetched and empty, it shuts though it was left open; an item put
        // in it brings the caret back, and the folder opens as it was left.
        if let Some(folder) = inv.folders.get_mut(&id(20)) {
            folder.state = FetchState::Fetched;
        }
        view.prepare(&inv, prefs.sort);
        tree.key = None;
        tree.prepare(&view, &inv, inv.root, true, true);
        assert_eq!(tree.rows[tree.position(id(20)).expect("listed")].kind, LEAF);
        inv.add_items(vec![InvItem {
            id: id(200),
            parent: id(20),
            name: "new item".into(),
            ..Default::default()
        }]);
        view.prepare(&inv, prefs.sort);
        tree.prepare(&view, &inv, inv.root, true, true);
        let at = tree.position(id(20)).expect("listed");
        assert_eq!(tree.rows[at].kind, OPEN);
        assert_eq!(tree.rows[at + 1].id, id(200));
    }

    #[test]
    fn an_empty_shown_root_and_an_empty_folder_window_say_so() {
        let (inv, view, _) = fixture();
        let mut tree = PlainTree::default();
        tree.set_open(id(20), true);
        tree.prepare(&view, &inv, id(20), true, true);
        assert_eq!(listed(&tree), [(20, 0, OPEN), (20, 1, Kind::Empty)], "a shown root keeps its caret");
        tree.prepare(&view, &inv, id(20), false, true);
        assert_eq!(listed(&tree), [(20, 0, Kind::Empty)]);
    }

    #[test]
    fn inventory_changes_relist_and_guides_cover_the_ancestors_of_the_first_row() {
        let (mut inv, mut view, prefs) = fixture();
        let mut tree = PlainTree::default();
        tree.open_all([id(1), id(10), id(11)]);
        tree.prepare(&view, &inv, inv.root, true, true);
        assert_eq!(tree.order, [1, 10, 11, 110, 100, 101, 20, 3].map(id));
        // Rows 3..5 lie in folder 10 (row 1) inside the root (row 0); the
        // nested folder (row 2) ends above them.
        let mut guides = tree.guides(3..5);
        guides.sort();
        assert_eq!(guides, [0, 1, 2]);
        assert_eq!(tree.guides(4..6), [1, 0]);
        inv.add_items(vec![InvItem {
            id: id(102),
            parent: id(10),
            name: "c item".into(),
            ..Default::default()
        }]);
        view.prepare(&inv, prefs.sort);
        assert!(tree.prepare(&view, &inv, inv.root, true, true));
        assert_eq!(tree.position(id(102)), Some(6));
    }

    #[test]
    fn expand_opens_unknown_folders_and_collapse_closes_every_descendant() {
        let (mut inv, view, _) = fixture();
        if let Some(folder) = inv.folders.get_mut(&id(11)) {
            folder.state = FetchState::Unknown;
        }
        let mut tree = PlainTree::default();
        tree.expand(&view, &inv, inv.root, true, true, true);
        assert!([1, 10, 11, 20, 3].into_iter().all(|n| tree.is_open(id(n))));
        tree.prepare(&view, &inv, inv.root, true, true);
        assert!(tree.listed_open.contains(&id(11)), "unknown contents use ordinary lazy loading");
        tree.set_open(id(10), false);
        tree.expand(&view, &inv, inv.root, true, true, false);
        assert!(tree.is_open(id(1)));
        assert!(![10, 11, 20, 3].into_iter().any(|n| tree.is_open(id(n))));
        // A folder window has no root row to keep open.
        tree.expand(&view, &inv, id(10), false, true, true);
        assert!(!tree.is_open(id(10)) && tree.is_open(id(11)));
        assert!(!tree.is_open(id(20)), "another window's scope stays unchanged");
        tree.expand(&view, &inv, inv.root, true, false, true);
        assert!(!tree.is_open(id(3)), "the hidden Library stays closed");
    }

    #[test]
    fn a_parent_cycle_stops_at_the_depth_limit() {
        let (mut inv, _, prefs) = fixture();
        for (n, parent) in [(30, 31), (31, 30)] {
            inv.folders.insert(
                id(n),
                Folder {
                    info: InvFolder {
                        id: id(n),
                        parent: id(parent),
                        type_default: -1,
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: Vec::new(),
                    state: FetchState::Fetched,
                    library: false,
                },
            );
        }
        inv.generation += 1;
        let mut view = ViewCache::default();
        view.prepare(&inv, prefs.sort);
        let mut tree = PlainTree::default();
        tree.open_all([id(30), id(31)]);
        tree.prepare(&view, &inv, id(30), false, true);
        assert_eq!(tree.rows.len(), MAX_DEPTH);
    }

    #[test]
    fn the_open_folders_survive_a_new_session_through_the_layout() {
        let ctx = egui::Context::default();
        let key = egui::Id::unique("test_open_folders");
        let mut tree = PlainTree::default();
        tree.persist(&ctx, key);
        tree.set_open(id(10), true);
        tree.persist(&ctx, key);
        let saved = ron::to_string(&ctx.memory(|m| m.clone())).expect("layout");
        let next = egui::Context::default();
        next.memory_mut(|m| *m = ron::from_str(&saved).expect("restored layout"));
        let mut tree = PlainTree::default();
        tree.persist(&next, key);
        assert!(tree.is_open(id(10)));
    }

    /// The list scrolls by a pitch of `row_height`: drawn rows taller than it,
    /// as a font scale above 1 made them, leave the keyboard's and type-ahead's
    /// target short of sight, further down the list.
    #[test]
    fn legacy_rows_are_drawn_at_the_pitch_the_list_scrolls_by_at_every_font_scale() {
        let fonts = [crate::ui::fonts::INTER, crate::ui::fonts::NOTO, crate::ui::fonts::ROBOTO];
        for (font, scale) in fonts.into_iter().flat_map(|font| [1.0, 1.46, 2.0].map(|scale| (font, scale))) {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            crate::ui::fonts::apply(&ctx, font);
            crate::ui::fonts::set_scale(&ctx, scale);
            let p = theme.palette();
            let mut inv = Inventory::default();
            crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
            let mut prefs = InventoryPreferences::default();
            let mut st = InventoryUi::default();
            st.view.prepare(&inv, prefs.sort_for(st.tab));
            st.plain.expand(&st.view, &inv, inv.root, false, false, true);
            st.plain.prepare(&st.view, &inv, inv.root, false, false);
            let count = st.plain.rows.len();
            assert!(
                st.plain.rows.iter().any(|row| row.kind == Kind::Item) && count > 10,
                "folders and items to draw: {count} rows"
            );
            let (mut pitch, mut drawn) = (0.0, 0.0);
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                tree_spacing(ui);
                pitch = row_height(ui, &p, &st) + ui.spacing().item_spacing.y;
                let top = ui.cursor().top();
                let icons = Icons::default();
                rows(
                    ui,
                    &p,
                    &icons,
                    &mut inv,
                    &mut st,
                    &mut prefs,
                    &Facts::default(),
                    &mut Vec::new(),
                    0..count,
                );
                drawn = ui.cursor().top() - top;
            });
            out.textures_delta.clear();
            let expected = count as f32 * pitch;
            assert!(
                (drawn - expected).abs() < 0.01,
                "{font} at {scale}: {count} rows drawn {drawn}, scrolled by {expected}"
            );
            assert!(pitch > ROW_HEIGHT);
            if scale > 1.2 {
                assert!(pitch > ROW_HEIGHT + 2.0, "scale {scale} grows the row");
            }
        }
    }
}
