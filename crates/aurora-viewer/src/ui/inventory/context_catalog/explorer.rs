//! The menu entries of a desktop file manager, as customizable as the rest:
//! "Copy path" and "Show in folder" on an entry, and the menu of the
//! explorer's empty space (Windows Explorer's and Dolphin's background
//! menu): View, Sort by, Select all, Invert selection, Undo and Redo
//! (`world::inventory::undo`), with the folder's own New, Paste, Paste as
//! link, Reload and Properties. On a folder, "Open in new tab"
//! (`aurora::tabs`); in the split view, Copy and Move to the other pane, as
//! Dolphin's "Copy to Other View" and "Move to Other View". Aurora
//! difference:
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
    /// A folder in a new tab, shown.
    OpenTab(Uuid),
    /// The selection to the other pane's folder: copied (true) or moved.
    ToOtherPane(Vec<Uuid>, bool),
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

/// The empty space's New: a labelled "New ▸" at the head of the text,
/// opening at its side as View and Sort do, as Explorer's and Dolphin's
/// "New" (the "+" of the icon bar was easy to miss there). A layout the
/// user arranged still places it.
pub(super) fn new_first(entries: &mut Vec<Entry>) {
    let Some(at) = entries.iter().position(|e| e.id == "new" && e.zone == "top") else {
        return;
    };
    let mut new = entries.remove(at);
    new.zone = TEXT.into();
    new.slot = None;
    let first_text = entries.iter().position(|e| e.zone == TEXT).unwrap_or(entries.len());
    entries.insert(first_text, new);
}

/// The entries only the empty space's menu shows (and the editor).
const BACKGROUND_ADDS: &[&str] = &["explorer-view", "explorer-sort", "select-all", "invert-selection", "undo", "redo"];

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
            "copy-filled",
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
        // only where the column shows Quick access (Aurora's window)
        if all || (single && inv.folders.contains_key(&id) && st.aurora.is_some()) {
            let pinned = prefs.pinned.contains(&id);
            c.add(
                "pin",
                TEXT,
                if pinned { "pinned-filled" } else { "pin-filled" },
                if pinned {
                    aurora_i18n::tr!("inventory-unpin")
                } else {
                    aurora_i18n::tr!("inventory-pin")
                },
                single && id != inv.root,
                Command::Explorer(ExplorerCommand::Pin(id, !pinned)),
            );
        }
        // the explorer's tabs, where it shows them
        if all
            || (single
                && inv.folders.contains_key(&id)
                && st
                    .aurora
                    .is_some_and(|f| f.layout != super::super::aurora::Layout::Compact || st.other_pane.is_some()))
        {
            c.add(
                "open-tab",
                TEXT,
                "app-window-filled",
                aurora_i18n::tr!("inventory-open-in-new-tab"),
                single && st.tabs.len() < super::super::aurora::tabs::LIMIT,
                Command::Explorer(ExplorerCommand::OpenTab(id)),
            );
        }
        // the split view: the other pane's folder, as a drop there would
        let other = st.other_pane.filter(|folder| inv.folders.contains_key(folder));
        if all || other.is_some() {
            let ids = st.selected_ids(id);
            let to = other.unwrap_or_default();
            let allowed = other.is_some() && !ids.contains(&to) && ids.iter().all(|id| rules::parent(inv, *id) != Some(to));
            c.add(
                "copy-to-pane",
                TEXT,
                "copy-filled",
                aurora_i18n::tr!("inventory-copy-to-other-pane"),
                allowed,
                Command::Explorer(ExplorerCommand::ToOtherPane(ids.clone(), true)),
            );
            c.add(
                "move-to-pane",
                TEXT,
                "arrow-right",
                aurora_i18n::tr!("inventory-move-to-other-pane"),
                allowed,
                Command::Explorer(ExplorerCommand::ToOtherPane(ids, false)),
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
        "layout-grid-filled",
        aurora_i18n::tr!("inventory-view-menu"),
        true,
        Direction::Horizontal,
        false,
    );
    for (key, icon, label, mode) in [
        (
            "view-list",
            "list-filled",
            aurora_i18n::tr!("inventory-view-list"),
            ExplorerView::List,
        ),
        (
            "view-gallery",
            "layout-grid-filled",
            aurora_i18n::tr!("inventory-view-gallery"),
            ExplorerView::Gallery,
        ),
        (
            "view-combination",
            "layout-board-split-filled",
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
        "calendar-filled",
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
        "square-check-filled",
        aurora_i18n::tr!("inventory-invert-selection"),
        true,
        Command::Explorer(ExplorerCommand::Invert),
    );
    // Explorer's Undo / Redo, the step named ("Undo move of 3 items");
    // the history is the session's (`world::inventory::undo`)
    c.divider("explorer-history", TEXT);
    let busy = inv.undo.busy() || st.pending.is_some();
    for (key, icon, undo, step) in [
        ("undo", "arrow-back-up", true, inv.undo.next_undo()),
        ("redo", "arrow-forward-up", false, inv.undo.next_redo()),
    ] {
        let label = match (step, c.editing) {
            (Some(step), false) if undo => aurora_i18n::tr!("inventory-undo-step", what = step.what()),
            (Some(step), false) => aurora_i18n::tr!("inventory-redo-step", what = step.what()),
            _ if undo => aurora_i18n::tr!("inventory-undo").to_owned(),
            _ => aurora_i18n::tr!("inventory-redo").to_owned(),
        };
        c.add(
            key,
            TEXT,
            icon,
            &label,
            step.is_some() && !busy,
            Command::Invoke(InvAction::History { undo }),
        );
    }
}

pub(super) fn execute(
    command: ExplorerCommand,
    ui: &egui::Ui,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    match command {
        ExplorerCommand::View(view) => prefs.explorer.view = view,
        ExplorerCommand::ByDate(by_date) => prefs.sort.by_date = by_date,
        ExplorerCommand::FoldersByName => prefs.sort.folders_by_name = !prefs.sort.folders_by_name,
        ExplorerCommand::SystemFirst => prefs.sort.system_first = !prefs.sort.system_first,
        ExplorerCommand::SelectAll => super::super::select_keys::select_all(st),
        ExplorerCommand::Invert => super::super::select_keys::invert(st),
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
        ExplorerCommand::OpenTab(id) => {
            super::super::aurora::tabs::open(st, prefs, id, true);
        }
        ExplorerCommand::ToOtherPane(ids, copy) => {
            if let Some(to) = st.other_pane.filter(|folder| inv.folders.contains_key(folder)) {
                let mode = if copy {
                    super::super::drag::DropMode::Copy
                } else {
                    super::super::drag::DropMode::Move
                };
                super::super::drag::drop_cargo(inv, &ids, to, mode, st, prefs, facts, actions);
            }
        }
    }
}
