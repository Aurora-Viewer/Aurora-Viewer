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
/// The height of a row, without the spacing between rows.
pub const ROW_HEIGHT: f32 = 20.0;

/// The height of the rows of this window: Aurora's are taller.
pub fn row_height(st: &InventoryUi) -> f32 {
    if st.aurora.is_some() {
        super::aurora::rows::HEIGHT
    } else {
        ROW_HEIGHT
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder {
        open: bool,
    },
    Item,
    /// "Empty" or "Loading folder" under an open folder without children.
    Empty,
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

#[derive(Default)]
pub struct PlainTree {
    open: HashSet<Uuid>,
    restored: bool,
    revision: u64,
    saved: u64,
    key: Option<(u64, Uuid, bool, bool, u64)>,
    pub rows: Vec<Row>,
    /// The listed folders and items, in order (Shift selection).
    pub order: Vec<Uuid>,
    /// The open folders listed: fetched while their contents are unknown.
    pub listed_open: Vec<Uuid>,
    /// The widest row measured since the last rebuild (horizontal range).
    pub width: f32,
}

impl PlainTree {
    pub fn is_open(&self, id: Uuid) -> bool {
        self.open.contains(&id)
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

    /// "Expand" opens only the folders already fetched, to avoid fetching
    /// everything; "Collapse" closes all but the root.
    pub fn expand(&mut self, view: &ViewCache, inv: &Inventory, root: Uuid, show_root: bool, show_library: bool, open: bool) {
        fn walk(tree: &mut PlainTree, view: &ViewCache, inv: &Inventory, id: Uuid, depth: usize, show_library: bool, open: bool) {
            let Some(folder) = inv.folders.get(&id) else { return };
            if depth > MAX_DEPTH {
                return;
            }
            if !open || folder.state == FetchState::Fetched || depth == 0 {
                tree.set_open(id, open || depth == 0);
            }
            if tree.is_open(id) {
                for child in view.children_iter(id, show_library, inv.lib_root).collect::<Vec<_>>() {
                    walk(tree, view, inv, child, depth + 1, show_library, open);
                }
            }
        }
        if show_root {
            walk(self, view, inv, root, 0, show_library, open);
        } else {
            for child in view.children_iter(root, show_library, inv.lib_root).collect::<Vec<_>>() {
                walk(self, view, inv, child, 1, show_library, open);
            }
        }
    }

    /// List the rows again if the inventory, the sort or an open folder
    /// changed since the last call. Returns whether it did.
    pub fn prepare(&mut self, view: &ViewCache, inv: &Inventory, root: Uuid, show_root: bool, show_library: bool) -> bool {
        let key = (view.built(), root, show_root, show_library, self.revision);
        if self.key == Some(key) {
            return false;
        }
        self.rows.clear();
        self.order.clear();
        self.listed_open.clear();
        self.width = 0.0;
        if show_root {
            self.folder(view, inv, show_library, root, 0, 0, NONE);
        } else {
            self.listed_open.push(root);
            self.children(view, inv, show_library, root, 1, 0, NONE);
        }
        self.key = Some(key);
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn folder(&mut self, view: &ViewCache, inv: &Inventory, show_library: bool, id: Uuid, depth: usize, level: usize, parent: usize) {
        if depth > MAX_DEPTH || !inv.folders.contains_key(&id) {
            return;
        }
        let open = self.open.contains(&id);
        let index = self.rows.len();
        self.rows.push(Row {
            id,
            level,
            kind: Kind::Folder { open },
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
        let mut any = false;
        for child in view.children_iter(folder, show_library, inv.lib_root) {
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

    /// The row of a folder or an item (not of a placeholder).
    pub fn position(&self, id: Uuid) -> Option<usize> {
        self.rows.iter().position(|row| row.id == id && row.kind != Kind::Empty)
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
    st.view.prepare(inv, prefs.sort_for(st.tab));
    if let Some(open) = st.expand_all.take() {
        st.plain.expand(&st.view, inv, root, show_root, prefs.show_library, open);
    }
    // Back from a search: where the list was (a revealed row below wins).
    let mut offset = st.plain_scroll_restore.take();
    // The root opens the first time it is shown, scrolled to the top.
    if show_root && st.roots_seen.insert(root) {
        st.plain.set_open(root, true);
        offset = Some(0.0);
    }
    // The ancestors of a revealed row open until it is shown.
    st.plain.open_all(st.reveal_path.iter().copied());
    st.plain.prepare(&st.view, inv, root, show_root, prefs.show_library);
    let navigated = keyboard::navigate(ui, inv, st, false);
    for id in &st.plain.listed_open {
        if inv
            .folders
            .get(id)
            .is_some_and(|f| matches!(f.state, FetchState::Unknown | FetchState::Failed))
        {
            inv.request(*id);
        }
    }
    let height_of_row = row_height(st);
    let pitch = height_of_row + ui.spacing().item_spacing.y;
    if navigated && let Some(index) = st.selected.and_then(|id| st.plain.position(id)) {
        offset = Some(keyboard::scroll_offset(st, index, height_of_row, pitch, height));
    }
    if let Some(index) = st.reveal.and_then(|id| st.plain.position(id)) {
        // Bring it in sight; the row centres itself once drawn.
        offset = Some((index as f32 * pitch - height / 2.0).max(0.0));
    }
    let mut area = list_scroll(ui, height);
    if let Some(offset) = offset {
        area = area.vertical_scroll_offset(offset);
    }
    let total = st.plain.rows.len();
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
            rows(ui, p, icons, inv, st, prefs, facts, actions, range.clone());
        });
        // F2 on a selected row scrolled out of sight starts its renaming too.
        let renamed = st.rename.as_ref().map(|edit| edit.id).or_else(|| {
            st.selected
                .filter(|_| st.rename.is_none() && ui.input(|i| i.key_pressed(egui::Key::F2)))
        });
        if let Some(index) = renamed.and_then(|id| st.plain.position(id))
            && !range.contains(&index)
        {
            let one = index..index + 1;
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect(&one)), |ui| {
                rows(ui, p, icons, inv, st, prefs, facts, actions, one);
            });
        }
        guides(ui, &st.plain, range, origin, pitch, height_of_row);
        ui.set_min_width(st.plain.width);
    });
    st.plain_scroll = out.state.unclamped_offset().y;
    st.list_viewport = Some((out.state.unclamped_offset().y, out.inner_rect.height()));
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
    let height = row_height(st);
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
                row.level,
                st.show_next_owner,
            );
            st.plain.width = st.plain.width.max(width);
        }
        ui.horizontal(|ui| {
            ui.set_min_height(height);
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(row.level as f32 * ui.spacing().indent);
            match row.kind {
                Kind::Folder { open } => {
                    let mut now = open;
                    folder_row(ui, p, icons, inv, row.id, &mut now, st, prefs, facts, actions);
                    if now != open {
                        st.plain.set_open(row.id, now);
                    }
                }
                Kind::Item => item_row(ui, p, icons, inv, &row.id, st, prefs, facts, actions),
                Kind::Empty => {
                    let text = if inv.folders.get(&row.id).is_some_and(|f| f.state == FetchState::Fetched) {
                        aurora_i18n::tr!("status-empty")
                    } else {
                        aurora_i18n::tr!("ui-inventory-loading-folder")
                    };
                    if st.aurora.is_some() {
                        ui.add_space(super::aurora::rows::LEAD);
                    }
                    ui.label(RichText::new(text).size(12.0).color(p.muted_dim));
                }
            }
        });
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

    #[test]
    fn only_open_folders_list_their_contents_and_an_empty_one_gets_a_placeholder() {
        let (inv, view, prefs) = fixture();
        let mut tree = PlainTree::default();
        tree.prepare(&view, &inv, inv.root, true, true);
        assert_eq!(listed(&tree), [(1, 0, Kind::Folder { open: false })]);
        tree.open_all([id(1), id(10), id(20)]);
        assert!(tree.prepare(&view, &inv, inv.root, true, prefs.show_library));
        assert_eq!(
            listed(&tree),
            [
                (1, 0, Kind::Folder { open: true }),
                (10, 1, Kind::Folder { open: true }),
                (11, 2, Kind::Folder { open: false }),
                (100, 2, Kind::Item),
                (101, 2, Kind::Item),
                (20, 1, Kind::Folder { open: true }),
                (20, 2, Kind::Empty),
                (3, 1, Kind::Folder { open: false }),
            ]
        );
        assert_eq!(tree.order, [1, 10, 11, 100, 101, 20, 3].map(id));
        assert_eq!(tree.rows[0].end, 7);
        assert_eq!(tree.rows[1].end, 4);
        assert_eq!(tree.rows[3].parent, 1);
        assert_eq!(tree.listed_open, [1, 10, 20].map(id));
        assert!(!tree.prepare(&view, &inv, inv.root, true, true), "nothing changed");
        // Hiding the library, closing a folder and folder windows.
        assert!(tree.prepare(&view, &inv, inv.root, true, false));
        assert!(!tree.order.contains(&id(3)));
        tree.set_open(id(10), false);
        tree.prepare(&view, &inv, inv.root, true, true);
        assert_eq!(tree.order, [1, 10, 20, 3].map(id));
        tree.prepare(&view, &inv, id(10), false, true);
        assert_eq!(
            listed(&tree),
            [(11, 0, Kind::Folder { open: false }), (100, 0, Kind::Item), (101, 0, Kind::Item)]
        );
        assert_eq!(tree.listed_open, [id(10)]);
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
    fn expand_opens_fetched_folders_only_and_collapse_keeps_the_root() {
        let (mut inv, view, _) = fixture();
        if let Some(folder) = inv.folders.get_mut(&id(11)) {
            folder.state = FetchState::Unknown;
        }
        let mut tree = PlainTree::default();
        tree.expand(&view, &inv, inv.root, true, true, true);
        assert!([1, 10, 20, 3].into_iter().all(|n| tree.is_open(id(n))));
        assert!(!tree.is_open(id(11)), "not fetched yet");
        tree.expand(&view, &inv, inv.root, true, true, false);
        assert!(tree.is_open(id(1)));
        assert!(![10, 20, 3].into_iter().any(|n| tree.is_open(id(n))));
        // A folder window has no root row to keep open.
        tree.expand(&view, &inv, id(10), false, true, true);
        assert!(!tree.is_open(id(10)) && !tree.is_open(id(11)));
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
}
