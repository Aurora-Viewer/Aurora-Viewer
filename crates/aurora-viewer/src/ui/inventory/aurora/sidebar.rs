//! The column at the left of Aurora's medium and wide window: the
//! shortcuts (the Legacy Viewer's tabs: everything, recent, worn,
//! favourites, received items), the tree of the folders alone, and the
//! Trash at its foot. A folder clicked is the one the explorer lists; cargo
//! dropped on one goes into it, held over one opens it, as in the tree.

use super::super::view::ViewCache;
use super::super::*;
use crate::ui::{menu, widgets};
use egui::{Pos2, Rect, Sense, Vec2};

const ROW: f32 = super::rows::HEIGHT;
const INDENT: f32 = 14.0;
const CARET: f32 = 13.0;
const ICON: f32 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Row {
    pub(super) id: Uuid,
    depth: usize,
    open: bool,
    /// It holds folders, or is not fetched yet.
    branches: bool,
}

/// The folders open in the column, and its rows.
#[derive(Default)]
pub struct Sidebar {
    open: HashSet<Uuid>,
    revision: u64,
    key: Option<(u64, u64, bool, Uuid)>,
    pub(super) rows: Vec<Row>,
    /// The explorer's folder last shown: a new one opens its ancestors and
    /// scrolls into sight.
    shown: Uuid,
    #[cfg(test)]
    pub(super) drawn: std::collections::HashMap<Uuid, Rect>,
    #[cfg(test)]
    pub(super) scroll: Option<(f32, Rect)>,
}

impl Sidebar {
    /// Open the ancestors of `folder` when it is new; whether it is.
    fn follow(&mut self, inv: &Inventory, folder: Uuid, base: Uuid) -> bool {
        if self.shown == folder {
            return false;
        }
        self.shown = folder;
        let mut seen = HashSet::new();
        let mut id = rules::parent(inv, folder);
        while let Some(parent) = id.filter(|id| *id != inv.root && *id != base && seen.insert(*id)) {
            self.set_open(parent, true);
            id = rules::parent(inv, parent);
        }
        true
    }

    fn set_open(&mut self, id: Uuid, open: bool) {
        let changed = if open { self.open.insert(id) } else { self.open.remove(&id) };
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// The folders under the root, in the inventory's order, the Trash
    /// left for the foot of the column.
    fn prepare(&mut self, view: &ViewCache, inv: &Inventory, base: Uuid, show_library: bool) {
        let key = (view.built(), self.revision, show_library, base);
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        self.rows.clear();
        let mut stack: Vec<(Uuid, usize)> = view
            .children_iter(base, show_library, inv.lib_root)
            .filter(|id| inv.folders.get(id).is_some_and(|f| f.info.type_default != 14))
            .map(|id| (id, 0))
            .collect();
        stack.reverse();
        while let Some((id, depth)) = stack.pop() {
            let Some(folder) = inv.folders.get(&id) else { continue };
            if depth > super::super::rows::MAX_DEPTH {
                continue;
            }
            let open = self.open.contains(&id);
            let branches = folder.state != FetchState::Fetched || !folder.children.is_empty();
            self.rows.push(Row { id, depth, open, branches });
            if open {
                let children: Vec<Uuid> = view
                    .children_iter(id, show_library, inv.lib_root)
                    .filter(|id| inv.folders.contains_key(id))
                    .collect();
                stack.extend(children.into_iter().rev().map(|child| (child, depth + 1)));
            }
        }
    }
}

/// A row of the column: highlighted while `selected` (where the explorer
/// is), its `count` (or a badge of what waits) at its end.
fn entry(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    label: &str,
    count: Option<usize>,
    waiting: usize,
    selected: bool,
    left: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click());
    let hot = response.hovered() || response.dnd_hover_payload::<InvDrag>().is_some();
    if selected || hot {
        let fill = if selected { p.violet.gamma_multiply(0.28) } else { p.raised };
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
    }
    let icon_center = Pos2::new(rect.left() + left + ICON / 2.0, rect.center().y);
    let tint = if selected { p.violet_pale } else { p.muted };
    widgets::paint_glyph(ui.painter(), icon, icon_center, ICON, tint);
    let mut end = rect.right() - 8.0;
    if waiting > 0 {
        let badge = widgets::unread_count(
            ui.painter(),
            p,
            Pos2::new(end, rect.center().y - 7.5),
            waiting,
            widgets::CountSize::List,
        );
        end = badge.left() - 6.0;
    } else if let Some(count) = count {
        let galley = ui
            .painter()
            .layout_no_wrap(aurora_i18n::integer(count as i64), egui::FontId::proportional(11.5), p.muted);
        let left = end - galley.size().x;
        widgets::paint_row_text(ui.painter(), galley, left, rect.center().y, p.muted);
        end = left - 6.0;
    }
    let name_left = icon_center.x + ICON / 2.0 + 9.0;
    let mut job =
        egui::text::LayoutJob::single_section(label.to_owned(), egui::TextFormat::simple(egui::FontId::proportional(13.0), p.ink));
    job.wrap = egui::text::TextWrapping::truncate_at_width((end - name_left).max(20.0));
    let galley = ui.painter().layout_job(job);
    widgets::paint_row_text(ui.painter(), galley, name_left, rect.center().y, p.ink);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    response.on_hover_text(label)
}

/// What a folder of the column does: cargo dropped in, held over it to
/// open it, its right-click menu.
#[allow(clippy::too_many_arguments)]
fn folder_target(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    id: Uuid,
    response: &egui::Response,
    open: &mut bool,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    task_drop::target(ui, p, response, inv, id, prefs, actions);
    drag::target(ui, p, response, inv, id, st, prefs, facts, actions);
    drag::navigation::folder(ui, response, st.ui_id, id, open);
    menu::context_menu(response, p, |ui| context::show(ui, p, inv, id, st, prefs, facts, actions));
}

/// Show the explorer in `folder`.
fn browse(st: &mut InventoryUi, prefs: &InventoryPreferences, folder: Uuid) {
    set_tab(st, prefs, 0);
    st.search.clear();
    st.browse = folder;
    // A pointer selection is already in sight, even if its row is partly clipped.
    st.side.shown = folder;
}

/// The account's saved smart searches: a click shows one over the whole
/// inventory, the right-click menu renames or deletes it. The name being
/// edited lives in egui's memory: it belongs to this column alone.
fn smart_searches(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    saved: &[super::super::smart_store::Saved],
) {
    use super::super::smart_store;
    let editing = ui.make_persistent_id("aurora_smart_rename");
    // (search, name, first frame)
    let mut edit: Option<(u64, String, bool)> = ui.data_mut(|d| d.get_temp(editing));
    if edit.as_ref().is_some_and(|(id, ..)| !saved.iter().any(|s| s.id == *id)) {
        edit = None;
    }
    for search in saved {
        let label = if search.name.is_empty() {
            aurora_i18n::tr!("inventory-smart-untitled")
        } else {
            search.name.as_str()
        };
        if let Some((id, name, first)) = edit.as_mut().filter(|(id, ..)| *id == search.id) {
            let r = ui.add_sized(
                [ui.available_width(), ROW],
                egui::TextEdit::singleline(name)
                    .id(ui.make_persistent_id(("aurora_smart_rename_field", *id)))
                    .font(egui::FontId::proportional(13.0))
                    .text_color(p.ink)
                    .char_limit(63),
            );
            if std::mem::take(first) {
                r.request_focus();
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(name.chars().count()),
                    )));
                    state.store(ui.ctx(), r.id);
                }
            } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                ui.memory_mut(|m| m.surrender_focus(r.id));
                edit = None;
            } else if !r.has_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                smart_store::rename(*id, name.trim());
                edit = None;
            }
            continue;
        }
        let response = entry(ui, p, "search-filled", label, None, 0, st.active_smart == Some(search.id), 8.0);
        if response.clicked() {
            browse(st, prefs, inv.root);
            st.open_smart(Some(search.id), search.query.clone());
        }
        menu::context_menu(&response, p, |ui| {
            if menu::item(ui, p, "pencil-filled", aurora_i18n::tr!("inventory-smart-rename")) {
                edit = Some((search.id, search.name.clone(), true));
            }
            if menu::item(ui, p, "trash-filled", aurora_i18n::tr!("inventory-smart-delete")) {
                smart_store::delete(search.id);
                if st.active_smart == Some(search.id) {
                    st.leave_advanced();
                }
            }
        });
    }
    match edit {
        Some(edit) => ui.data_mut(|d| {
            d.insert_temp(editing, edit);
        }),
        None => ui.data_mut(|d| {
            d.remove_temp::<(u64, String, bool)>(editing);
        }),
    }

    // Find similar: an unsaved search, shown until its chip is removed.
    if st.finding_similar() {
        entry(
            ui,
            p,
            "search-filled",
            aurora_i18n::tr!("inventory-smart-similar"),
            None,
            0,
            st.active_smart.is_none(),
            8.0,
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    _icons: &Icons,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let rect = ui.max_rect();
    ui.painter()
        .rect_filled(rect, ui.visuals().window_corner_radius, p.field.gamma_multiply(0.55));
    let mut column = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink(6.0))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let ui = &mut column;
    ui.spacing_mut().item_spacing.y = 1.0;
    let base = super::base(inv, st);
    let scoped = base != inv.root;
    if scoped {
        // A folder window: its folder, where "All inventory" would be.
        let response = entry(ui, p, "folder-filled", &folder_name(inv, base), None, 0, st.browse == base, 8.0);
        if response.clicked() {
            browse(st, prefs, base);
        }
        let mut open = false;
        folder_target(ui, p, inv, base, &response, &mut open, st, prefs, facts, actions);
    }
    // The shortcuts.
    let everything = st.tab == 0 && st.browse == inv.root;
    if !scoped
        && entry(
            ui,
            p,
            "briefcase-filled",
            aurora_i18n::tr!("inventory-all"),
            None,
            0,
            everything,
            8.0,
        )
        .clicked()
    {
        browse(st, prefs, inv.root);
    }
    let unseen = super::toolbar::unseen_received(inv, st);
    for (tab, icon, label, count, shown) in if scoped {
        Vec::new()
    } else {
        vec![
            (
                2,
                "clock-filled",
                aurora_i18n::tr!("inventory-tab-recent"),
                st.counts.recent,
                prefs.show_recent,
            ),
            (
                3,
                "hanger-2-filled",
                aurora_i18n::tr!("inventory-tab-worn"),
                facts.worn.len(),
                prefs.show_worn,
            ),
            (
                4,
                "star-filled",
                aurora_i18n::tr!("inventory-tab-favorites"),
                st.counts.favorites,
                prefs.show_favorites,
            ),
            (
                RECEIVED_TAB,
                "inbox",
                aurora_i18n::tr!("inventory-tab-received"),
                st.counts.received,
                // the Received Items in the main window only
                st.is_main(),
            ),
        ]
    } {
        if !shown {
            continue;
        }
        let waiting = if tab == RECEIVED_TAB { unseen } else { 0 };
        if entry(ui, p, icon, label, Some(count), waiting, st.tab == tab, 8.0).clicked() {
            set_tab(st, prefs, tab);
        }
    }
    let saved = if scoped { Vec::new() } else { super::super::smart_store::list() };
    if !saved.is_empty() || (st.finding_similar() && !scoped) {
        ui.add_space(10.0);
        menu::section(ui, p, aurora_i18n::tr!("inventory-smart-section"));
        ui.add_space(2.0);
        smart_searches(ui, p, inv, st, prefs, &saved);
    }
    ui.add_space(10.0);
    crate::ui::menu::section(ui, p, aurora_i18n::tr!("inventory-folders"));
    ui.add_space(2.0);
    // The folders.
    st.view.prepare(inv, prefs.sort_for(st.tab));
    let follow = st.side.follow(inv, st.browse, base);
    st.side.prepare(&st.view, inv, base, prefs.show_library);
    let trash = rules::system(inv, 14).filter(|_| !scoped);
    let foot = if trash.is_some() { ROW + 8.0 } else { 0.0 };
    let height = (ui.available_height() - foot).max(ROW);
    let rows = st.side.rows.clone();
    // Keep show_rows' virtual pitch identical to the rows laid out below.
    let spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut area = egui::ScrollArea::vertical()
        .id_salt("aurora_inventory_side_folders")
        .max_height(height)
        .auto_shrink([false, false]);
    if follow && let Some(index) = rows.iter().position(|row| row.id == st.browse) {
        // LLScrollContainer::scrollToShowRect (indra/llui/llscrollcontainer.cpp):
        // leave visible folders in place; reveal offscreen ones at the nearest edge.
        let id = ui.make_persistent_id(egui::IdSalt::new("aurora_inventory_side_folders"));
        let offset = egui::scroll_area::State::load(ui.ctx(), id).map_or(0.0, |state| state.unclamped_offset().y);
        let top = index as f32 * ROW;
        area = area.vertical_scroll_offset(offset.min(top).max(top + ROW - height).max(0.0));
    }
    #[cfg(test)]
    st.side.drawn.clear();
    let _out = area.show_rows(ui, ROW, rows.len(), |ui, range| {
        for row in &rows[range] {
            let Some(folder) = inv.folders.get(&row.id) else { continue };
            let left = 4.0 + row.depth as f32 * INDENT + CARET + 4.0;
            let selected = st.tab == 0 && st.browse == row.id;
            let name = folder_name(inv, row.id);
            let icon = match tree::folder_icon(folder.info.type_default, row.open) {
                "Inv_TrashClosed" => "trash-filled",
                "Inv_FolderOpen" => "folder-open-filled",
                _ => "folder-filled",
            };
            let response = entry(ui, p, icon, &name, None, 0, selected, left);
            #[cfg(test)]
            st.side.drawn.insert(row.id, response.rect);
            let caret_center = Pos2::new(
                response.rect.left() + 4.0 + row.depth as f32 * INDENT + CARET / 2.0,
                response.rect.center().y,
            );
            let mut open = row.open;
            if row.branches {
                let caret = ui.interact(
                    Rect::from_center_size(caret_center, Vec2::splat(CARET + 6.0)),
                    ui.scope_id().with(("aurora_side_caret", row.id)),
                    Sense::click(),
                );
                widgets::paint_glyph(
                    ui.painter(),
                    widgets::disclosure_caret(row.open),
                    caret_center,
                    CARET,
                    if caret.hovered() { p.ink } else { p.muted_dim },
                );
                if caret.clicked() {
                    open = !open;
                }
            }
            if response.clicked() {
                browse(st, prefs, row.id);
            }
            if response.double_clicked() && row.branches {
                open = !open;
            }
            folder_target(ui, p, inv, row.id, &response, &mut open, st, prefs, facts, actions);
            if open != row.open {
                st.side.set_open(row.id, open);
                if open
                    && inv
                        .folders
                        .get(&row.id)
                        .is_some_and(|f| matches!(f.state, FetchState::Unknown | FetchState::Failed))
                {
                    inv.request(row.id);
                }
            }
        }
    });
    ui.spacing_mut().item_spacing.y = spacing;
    #[cfg(test)]
    {
        st.side.scroll = Some((_out.state.unclamped_offset().y, _out.inner_rect));
    }
    // The Trash, at the foot.
    if let Some(trash) = trash {
        let (line, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 8.0), Sense::hover());
        ui.painter().hline(line.x_range(), line.center().y, widgets::divider_stroke(p));
        let response = entry(
            ui,
            p,
            "trash-filled",
            &folder_name(inv, trash),
            Some(st.counts.trash),
            0,
            st.tab == 0 && st.browse == trash,
            8.0,
        );
        if response.clicked() {
            browse(st, prefs, trash);
        }
        let mut open = false;
        folder_target(ui, p, inv, trash, &response, &mut open, st, prefs, facts, actions);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::Folder;
    use aurora_net::inventory::InvFolder;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn the_column_lists_folders_only_and_leaves_the_trash_for_its_foot() {
        let mut inv = Inventory {
            root: id(1),
            ..Default::default()
        };
        for (n, parent, kind, name) in [
            (1, 0, 8, "root"),
            (10, 1, -1, "Clothing"),
            (11, 10, -1, "Evening"),
            (14, 1, 14, "Trash"),
            (20, 1, -1, "Objects"),
        ] {
            inv.folders.insert(
                id(n),
                Folder {
                    info: InvFolder {
                        id: id(n),
                        parent: id(parent),
                        name: name.into(),
                        type_default: kind,
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: Vec::new(),
                    state: FetchState::Fetched,
                    library: false,
                },
            );
        }
        inv.add_items(vec![aurora_net::inventory::InvItem {
            id: id(100),
            parent: id(10),
            name: "dress".into(),
            ..Default::default()
        }]);
        for (folder, child) in [(1, 10), (10, 11), (1, 14), (1, 20)] {
            inv.folders.get_mut(&id(folder)).expect("folder").children.push(id(child));
        }
        let mut view = ViewCache::default();
        view.prepare(
            &inv,
            view::Sort {
                by_date: false,
                ..Default::default()
            },
        );
        let mut side = Sidebar::default();
        side.prepare(&view, &inv, inv.root, true);
        let listed: Vec<_> = side.rows.iter().map(|r| (r.id.as_u128(), r.depth)).collect();
        assert_eq!(listed, [(10, 0), (20, 0)], "no item, no trash, closed folders");
        assert!(side.rows[0].branches && !side.rows[1].branches);
        side.set_open(id(10), true);
        side.prepare(&view, &inv, inv.root, true);
        let listed: Vec<_> = side.rows.iter().map(|r| (r.id.as_u128(), r.depth)).collect();
        assert_eq!(listed, [(10, 0), (11, 1), (20, 0)]);
    }
}
