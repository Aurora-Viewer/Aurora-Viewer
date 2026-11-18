//! The menu entries of a desktop file manager, as customizable as the rest:
//! "Copy path" and "Show in folder" on an entry, and the menu of the
//! explorer's empty space (Windows Explorer's and Dolphin's background
//! menu): View, Sort by, Select all, Invert selection, with the folder's
//! own New, Paste, Paste as link, Reload and Properties. Aurora difference:
//! the Legacy Viewer's single folder mode has no menu on its empty space
//! and no path to copy.

use super::super::view::ExplorerView;
use super::*;

pub(in super::super) enum ExplorerCommand {
    View(ExplorerView),
    ByDate(bool),
    FoldersByName,
    SystemFirst,
    SelectAll,
    Invert,
    CopyPath(Uuid),
    ShowInFolder(Uuid),
    /// A folder in or out of the column's Quick access.
    Pin(Uuid, bool),
}

/// The entries of the folder's menu that its empty space keeps (with the
/// whole New group); everything else acts on the folder itself.
const BACKGROUND_KEEPS: &[&str] = &["new", "paste", "paste-link", "reload", "properties", "show-window"];

/// Whether the empty space's menu keeps `entry`.
pub(super) fn kept_in_background(entry: &Entry) -> bool {
    entry.is_separator()
        || entry.kind == Kind::Lock
        || entry.zone == children("new")
        || entry.zone == children("explorer-view")
        || entry.zone == children("explorer-sort")
        || BACKGROUND_KEEPS.contains(&entry.id.as_str())
        || BACKGROUND_ADDS.contains(&entry.id.as_str())
}

/// The entries only the empty space's menu shows (and the editor).
const BACKGROUND_ADDS: &[&str] = &["explorer-view", "explorer-sort", "select-all", "invert-selection"];

fn check(c: &mut Catalog, id: &str, on: bool) {
    if !c.editing
        && let Some(entry) = c.entries.iter_mut().rev().find(|e| e.id == id)
    {
        entry.checked = Some(on);
    }
}

pub(super) fn build(
    c: &mut Catalog,
    inv: &Inventory,
    id: Uuid,
    st: &InventoryUi,
    prefs: &InventoryPreferences,
    all: bool,
    background: bool,
) {
    let single = st.selected_ids(id).len() == 1;
    if all || !background {
        c.add(
            "copy-path",
            TEXT,
            "copy",
            aurora_i18n::tr!("inventory-copy-path"),
            single,
            Command::Explorer(ExplorerCommand::CopyPath(id)),
        );
        // from a search, a filter or a shortcut such as Recent: its folder
        let elsewhere = st.result_view() || st.tab != 0;
        if all || (elsewhere && single && id != inv.root) {
            c.add(
                "show-in-folder",
                TEXT,
                "folder-open-filled",
                aurora_i18n::tr!("inventory-show-in-folder"),
                single && rules::parent(inv, id).is_some_and(|p| inv.folders.contains_key(&p)),
                Command::Explorer(ExplorerCommand::ShowInFolder(id)),
            );
        }
        // pinning: a folder in the column's Quick access
        if all || (single && inv.folders.contains_key(&id)) {
            let pinned = prefs.pinned.contains(&id);
            c.add(
                "pin",
                TEXT,
                if pinned { "pinned" } else { "pin" },
                if pinned {
                    aurora_i18n::tr!("inventory-unpin")
                } else {
                    aurora_i18n::tr!("inventory-pin")
                },
                single && id != inv.root,
                Command::Explorer(ExplorerCommand::Pin(id, !pinned)),
            );
        }
    }
    if !(all || background) {
        return;
    }
    c.divider("explorer-view", TEXT);
    let view = c.group(
        "explorer-view",
        TEXT,
        "layout-grid",
        aurora_i18n::tr!("inventory-view-menu"),
        true,
        Direction::Horizontal,
        false,
    );
    for (key, icon, label, mode) in [
        ("view-list", "list", aurora_i18n::tr!("inventory-view-list"), ExplorerView::List),
        (
            "view-gallery",
            "layout-grid",
            aurora_i18n::tr!("inventory-view-gallery"),
            ExplorerView::Gallery,
        ),
        (
            "view-combination",
            "layout-board-split",
            aurora_i18n::tr!("inventory-view-combination"),
            ExplorerView::Combination,
        ),
    ] {
        c.add(key, &view, icon, label, true, Command::Explorer(ExplorerCommand::View(mode)));
        check(c, key, prefs.explorer.view == mode);
    }
    let sort = c.group(
        "explorer-sort",
        TEXT,
        "arrows-sort",
        aurora_i18n::tr!("inventory-sort-menu"),
        true,
        Direction::Horizontal,
        false,
    );
    c.add(
        "sort-name",
        &sort,
        "sort-ascending-letters",
        aurora_i18n::tr!("common-by-name"),
        true,
        Command::Explorer(ExplorerCommand::ByDate(false)),
    );
    check(c, "sort-name", !prefs.sort.by_date);
    c.add(
        "sort-date",
        &sort,
        "calendar",
        aurora_i18n::tr!("ui-inventory-controls-newest-first"),
        true,
        Command::Explorer(ExplorerCommand::ByDate(true)),
    );
    check(c, "sort-date", prefs.sort.by_date);
    c.divider("sort-folders", &sort);
    c.add(
        "sort-folders-by-name",
        &sort,
        "folder-filled",
        aurora_i18n::tr!("ui-inventory-controls-folders-always-by-name"),
        true,
        Command::Explorer(ExplorerCommand::FoldersByName),
    );
    check(c, "sort-folders-by-name", prefs.sort.folders_by_name);
    c.add(
        "sort-system-first",
        &sort,
        "folder-filled",
        aurora_i18n::tr!("ui-inventory-controls-system-folders-first"),
        true,
        Command::Explorer(ExplorerCommand::SystemFirst),
    );
    check(c, "sort-system-first", prefs.sort.system_first);
    c.divider("explorer-select", TEXT);
    c.add(
        "select-all",
        TEXT,
        "select-all",
        aurora_i18n::tr!("common-select-all"),
        true,
        Command::Explorer(ExplorerCommand::SelectAll),
    );
    c.add(
        "invert-selection",
        TEXT,
        "square-check",
        aurora_i18n::tr!("inventory-invert-selection"),
        true,
        Command::Explorer(ExplorerCommand::Invert),
    );
}

pub(super) fn execute(command: ExplorerCommand, ui: &egui::Ui, inv: &Inventory, st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    match command {
        ExplorerCommand::View(view) => prefs.explorer.view = view,
        ExplorerCommand::ByDate(by_date) => prefs.sort.by_date = by_date,
        ExplorerCommand::FoldersByName => prefs.sort.folders_by_name = !prefs.sort.folders_by_name,
        ExplorerCommand::SystemFirst => prefs.sort.system_first = !prefs.sort.system_first,
        ExplorerCommand::SelectAll => {
            let order = super::super::keyboard::shown_order(st).to_vec();
            st.selection.extend(order.iter().copied());
            if st.selected.is_none() {
                st.selected = order.first().copied();
            }
        }
        ExplorerCommand::Invert => {
            let order = super::super::keyboard::shown_order(st).to_vec();
            let before = std::mem::take(&mut st.selection);
            st.selection.extend(order.iter().filter(|id| !before.contains(id)));
            st.selected = order.iter().find(|id| st.selection.contains(id)).copied();
        }
        ExplorerCommand::CopyPath(id) => {
            let base = if st.scope.is_nil() { inv.root } else { st.scope };
            let folder = if inv.folders.contains_key(&id) {
                Some(id)
            } else {
                rules::parent(inv, id)
            };
            if let Some(folder) = folder {
                let mut path = super::super::aurora::path_text(inv, base, folder);
                if !inv.folders.contains_key(&id) {
                    path.push(super::super::aurora::PATH_SEPARATOR);
                    path.push_str(&rules::name(inv, id));
                }
                ui.ctx().copy_text(path);
            }
        }
        ExplorerCommand::ShowInFolder(id) => st.show_original(inv, id),
        ExplorerCommand::Pin(id, pin) => {
            prefs.pinned.retain(|pinned| *pinned != id);
            if pin {
                prefs.pinned.push(id);
            }
        }
    }
}
