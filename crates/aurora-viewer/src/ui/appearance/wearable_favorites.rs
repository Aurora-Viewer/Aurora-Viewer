//! The Favorite Wearables window. Port of FSFloaterWearableFavorites
//! (indra/newview/fsfloaterwearablefavorites.cpp,
//! floater_fs_wearable_favorites.xml, menu_fs_wearable_favorites.xml) and
//! FSWearableFavoritesItemsList, originally LGPL 2.1: a folder picker ("All
//! Favorites" and the favorites' subfolders) with New, Rename and Delete
//! folder, a filter, the sort menu and Remove, then the list of the folder's
//! wearables with their worn state. Wearables dropped on the list are
//! linked in the folder shown; a double click wears or takes off; the
//! menu of a row is the outfit rows' (LLWearableItemsList::ContextMenu)
//! with Assign to Folder. The rules are in `world/wearable_favorites.rs`.

use super::{Action, AppearanceUi, items, row, row_gap, selection};
use crate::{
    theme::Palette,
    ui::{inventory, menu, widgets},
    world::{World, inventory::Inventory, wearable_favorites as model},
};
use aurora_net::inventory::operations::Mutation;
use egui::{RichText, Vec2};
use model::SortOrder;
use std::collections::HashSet;
use uuid::Uuid;

/// The window's id (Floater.Toggle "fs_wearable_favorites").
pub const WINDOW: &str = "wearable_favorites";

#[derive(Default)]
pub(crate) struct State {
    /// The folder picked (mSelectedFolderID); `None` for All Favorites.
    shown: Option<Uuid>,
    filter: String,
    selection: selection::Selection,
    dialog: Option<Dialog>,
    focus: bool,
    /// initCategory was sent this session: never twice.
    created: bool,
    /// A folder asked for, with the subfolders there before it: picked
    /// when it appears (onNewFolderCallback).
    select_new: Option<(String, HashSet<Uuid>)>,
    /// The import was accepted for sending and the inventory has not
    /// answered yet: the button stays hidden, so that a second click cannot
    /// link twice. Set by `import_accepted` only.
    import_sent: bool,
    /// The import clicked, for the app to submit (`take_import`).
    import_request: Option<Mutation>,
    /// The import plan with the inventory generation and favorites folder it
    /// was built for: rebuilt only when the inventory changes.
    plan: Option<(u64, Option<Uuid>, Option<model::Import>)>,
}

impl State {
    /// The app submitted the import and the inventory accepted it.
    pub(crate) fn import_accepted(&mut self) {
        self.import_sent = true;
    }

    /// The accepted change is over (done or failed): while the plan still
    /// exists the button comes back, so a failed import can be retried.
    pub(crate) fn import_settled(&mut self, change_pending: bool) {
        if !change_pending {
            self.import_sent = false;
        }
    }

    #[cfg(test)]
    pub(crate) fn import_hidden(&self) -> bool {
        self.import_sent
    }
}

/// The import the window asked for since the last call, if any.
pub fn take_import(st: &mut AppearanceUi) -> Option<Mutation> {
    st.wearable_favorites.import_request.take()
}

enum Dialog {
    New(String),
    Rename {
        id: Uuid,
        name: String,
    },
    Delete(Uuid),
    /// A folder holding an original is not deleted (the name shown).
    DeleteRefused(String),
    /// A folder whose subtree is not fully fetched is not deleted (the name shown).
    DeleteUnloaded(String),
    /// FSWearableFavoritesFolderInvalidName.
    Invalid(String),
}

/// While the window is open: the folders of the favorites are fetched, and
/// the folder is created once when it does not exist (onOpen, initCategory).
/// `writable` when an inventory change may be sent now. Deliberate
/// difference: the Legacy Viewer also calls initCategory at every login
/// (llstartup.cpp), so the folder appears in every account's inventory;
/// Aurora creates it only once the window is opened.
pub fn prepare(inv: &mut Inventory, st: &mut AppearanceUi, writable: bool) -> Option<Mutation> {
    let st = &mut st.wearable_favorites;
    if let Some(root) = model::folder(inv) {
        crate::world::inventory::actions::request_tree(inv, root);
        // the Legacy Viewer's favorites, to offer their import
        if let Some(legacy) = model::legacy_folder(inv) {
            crate::world::inventory::actions::request_tree(inv, legacy);
        }
        return None;
    }
    if st.created || !writable {
        return None;
    }
    let change = model::create_plan(inv)?;
    log::info!(
        "wearable favorites: creating the folder ({} folders to create)",
        change.operations.len()
    );
    st.created = true;
    Some(change)
}

/// Draw the window while `open`; `sort` is FSWearableFavoritesSortOrder.
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    open: &mut bool,
    sort: &mut SortOrder,
    writable: bool,
) -> Vec<Action> {
    let mut actions = Vec::new();
    if !*open {
        return actions;
    }
    let inv = &world.inventory;
    let root = model::folder(inv);
    let folders = root.map(|root| model::subfolders(inv, root)).unwrap_or_default();
    {
        let fav = &mut st.wearable_favorites;
        // refreshFolderCombo: a folder gone shows All Favorites again
        if fav.shown.is_some_and(|id| !folders.iter().any(|(f, _)| *f == id)) {
            fav.shown = None;
        }
        if let Some((name, before)) = &fav.select_new
            && let Some((id, _)) = folders.iter().find(|(id, n)| n == name && !before.contains(id))
        {
            fav.shown = Some(*id);
            fav.select_new = None;
        }
    }
    let mut floater = widgets::Floater::new(
        WINDOW,
        aurora_i18n::tr!("toolbar-command-wearable-favorites"),
        egui::pos2(320.0, 120.0),
        Vec2::new(360.0, 430.0),
    );
    floater.min_size = Vec2::new(260.0, 220.0);
    floater.show(ctx, p, open, |ui| {
        let shown = root.map(|root| st.wearable_favorites.shown.unwrap_or(root));
        folder_bar(ui, p, inv, st, root, &folders);
        ui.add_space(4.0);
        import_bar(ui, p, inv, st, root, writable);
        let labels = items::worn_labels(world);
        let mut rows = match (root, shown) {
            (Some(root), Some(shown)) => model::entries(inv, root, shown),
            _ => Vec::new(),
        };
        model::sort(inv, &mut rows, *sort);
        let total = rows.len();
        let filter = st.wearable_favorites.filter.clone();
        rows.retain(|e| {
            inv.items
                .get(&e.target)
                .is_some_and(|it| model::matches(&filter, &it.name, labels.get(&e.target).map(String::as_str)))
        });
        let order: Vec<Uuid> = rows.iter().map(|e| e.link).collect();
        st.wearable_favorites.selection.retain(&order);
        filter_bar(ui, p, inv, st, sort, &order, &mut actions);
        ui.add_space(4.0);
        let list = egui::ScrollArea::vertical()
            .id_salt("wearable_favorites_rows")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for e in &rows {
                    let Some(it) = inv.items.get(&e.target) else { continue };
                    ui.push_id(("wearable_favorite", e.link), |ui| {
                        let worn = labels.get(&e.target).map(String::as_str);
                        let selected = st.wearable_favorites.selection.contains(e.link);
                        let response = row(ui, p, it, "", selected, worn);
                        if response.clicked() || response.secondary_clicked() {
                            let modifiers = ui.input(|i| i.modifiers);
                            st.wearable_favorites
                                .selection
                                .click(e.link, &order, modifiers, response.secondary_clicked());
                        }
                        if it.asset_type == 21 {
                            // a gesture is activated or deactivated (the inventory's double click)
                            if items::double_clicked(&response) {
                                actions.push(Action::Gesture(vec![it.id], !world.active_gestures.contains(&it.id)));
                            }
                        } else {
                            items::double_click(&response, world, it, worn.is_some(), &mut actions);
                        }
                        // the menu acts on the originals of the rows picked
                        let picked: Vec<Uuid> = if st.wearable_favorites.selection.contains(e.link) {
                            rows.iter()
                                .filter(|r| st.wearable_favorites.selection.contains(r.link))
                                .map(|r| r.target)
                                .collect()
                        } else {
                            vec![e.target]
                        };
                        items::context_menu(&response, p, world, st, items::List::Favorites(&folders), it, &picked, &mut actions);
                    });
                    row_gap(ui);
                }
                if rows.is_empty() {
                    let loading = shown.is_none_or(|shown| !model::loaded(inv, shown));
                    let text = if total > 0 {
                        aurora_i18n::tr!("wearable-favorites-no-match")
                    } else if loading {
                        aurora_i18n::tr!("common-loading-2")
                    } else {
                        aurora_i18n::tr!("wearable-favorites-empty")
                    };
                    ui.add_space(6.0);
                    ui.label(RichText::new(text).color(p.muted));
                }
            });
        if let Some(to) = shown {
            drop_zone(ui, p, inv, list.inner_rect, to, &mut actions);
        }
    });
    dialogs(ctx, p, world, st, root, &folders, &mut actions);
    actions
}

/// The folder picker and New, Rename and Delete folder.
fn folder_bar(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, st: &mut AppearanceUi, root: Option<Uuid>, folders: &[(Uuid, String)]) {
    let fav = &mut st.wearable_favorites;
    let all = aurora_i18n::tr!("wearable-favorites-all");
    ui.horizontal(|ui| {
        let buttons = 3.0 * (ui.spacing().interact_size.y + ui.spacing().item_spacing.x);
        let width = (ui.available_width() - buttons).max(80.0);
        let current = fav
            .shown
            .and_then(|id| folders.iter().find(|(f, _)| *f == id))
            .map_or(all, |(_, name)| name.as_str());
        ui.add_enabled_ui(root.is_some(), |ui| {
            egui::ComboBox::from_id_salt("wearable_favorites_folder")
                .width(width)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(fav.shown.is_none(), all).clicked() {
                        fav.shown = None;
                    }
                    for (id, name) in folders {
                        if ui.selectable_label(fav.shown == Some(*id), name).clicked() {
                            fav.shown = Some(*id);
                        }
                    }
                });
        });
        if widgets::icon_button(
            ui,
            p,
            "folder-filled",
            aurora_i18n::tr!("wearable-favorites-new-folder-tip"),
            root.is_some(),
        ) {
            fav.dialog = Some(Dialog::New(aurora_i18n::tr!("wearable-favorites-new-folder-default").into()));
            fav.focus = true;
        }
        // folder_action_enabled: a subfolder, never All Favorites
        let sub = fav.shown;
        if widgets::icon_button(
            ui,
            p,
            "pencil-filled",
            aurora_i18n::tr!("wearable-favorites-rename-folder-tip"),
            sub.is_some(),
        ) && let Some(id) = sub
        {
            let name = inv.folders.get(&id).map(|f| f.info.name.clone()).unwrap_or_default();
            fav.dialog = Some(Dialog::Rename { id, name });
            fav.focus = true;
        }
        if widgets::icon_button(
            ui,
            p,
            "trash-filled",
            aurora_i18n::tr!("wearable-favorites-delete-folder-tip"),
            sub.is_some(),
        ) && let Some(id) = sub
        {
            let name = || folders.iter().find(|(f, _)| *f == id).map(|(_, n)| n.clone()).unwrap_or_default();
            fav.dialog = Some(if !model::loaded(inv, id) {
                Dialog::DeleteUnloaded(name())
            } else if model::holds_originals(inv, id) {
                Dialog::DeleteRefused(name())
            } else {
                Dialog::Delete(id)
            });
        }
    });
}

/// The offer to import the Legacy Viewer's favorites: shown only while the
/// Legacy folder holds favorites ours does not link yet. It links them, and
/// copies or moves nothing. Disabled while an inventory change is on its way
/// (it would be refused); the button hides once the change is accepted.
fn import_bar(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, st: &mut AppearanceUi, root: Option<Uuid>, writable: bool) {
    let fav = &mut st.wearable_favorites;
    if fav
        .plan
        .as_ref()
        .is_none_or(|(generation, folder, _)| *generation != inv.generation || *folder != root)
    {
        fav.plan = Some((inv.generation, root, root.and_then(|root| model::import_plan(inv, root))));
    }
    let Some(plan) = fav.plan.as_ref().and_then(|(_, _, plan)| plan.as_ref()) else {
        fav.import_sent = false;
        return;
    };
    if fav.import_sent {
        return;
    }
    let (links, folders) = (plan.links, plan.folders);
    let clicked = ui
        .add_enabled_ui(writable, |ui| {
            widgets::action_button(
                ui,
                p,
                "download",
                aurora_i18n::tr!("wearable-favorites-import"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, 26.0),
            )
            .on_hover_text(aurora_i18n::tr!("wearable-favorites-import-tip"))
            .clicked()
        })
        .inner;
    ui.add_space(4.0);
    if clicked && fav.import_request.is_none() {
        log::info!("wearable favorites: importing the Legacy Viewer's favorites ({links} links, {folders} new folders)");
        fav.import_request = fav
            .plan
            .as_ref()
            .and_then(|(_, _, plan)| plan.as_ref())
            .map(|plan| plan.change.clone());
    }
}

/// The filter, the sort menu and Remove.
fn filter_bar(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    st: &mut AppearanceUi,
    sort: &mut SortOrder,
    order: &[Uuid],
    actions: &mut Vec<Action>,
) {
    let fav = &mut st.wearable_favorites;
    ui.horizontal(|ui| {
        let buttons = 2.0 * (ui.spacing().interact_size.y + ui.spacing().item_spacing.x);
        let width = (ui.available_width() - buttons).max(80.0);
        let search = widgets::clearable_search_box(ui, p, &mut fav.filter, aurora_i18n::tr!("wearable-favorites-filter"), width);
        // handleKeyHere: the filter takes Ctrl+F
        if crate::ui::find_shortcut::take(ui, &[search.id]) {
            search.request_focus();
        }
        widgets::icon_menu(
            ui,
            p,
            "sort-ascending-2-filled",
            aurora_i18n::tr!("wearable-favorites-options-tip"),
            |ui| {
                for (order, icon, label) in [
                    (
                        SortOrder::Name,
                        "sort-ascending-letters",
                        aurora_i18n::tr!("wearable-favorites-sort-name"),
                    ),
                    (
                        SortOrder::MostRecent,
                        "clock-filled",
                        aurora_i18n::tr!("wearable-favorites-sort-recent"),
                    ),
                    (
                        SortOrder::TypeName,
                        "hanger-2-filled",
                        aurora_i18n::tr!("wearable-favorites-sort-type"),
                    ),
                ] {
                    if menu::check(ui, p, icon, label, *sort == order, true) {
                        *sort = order;
                    }
                }
            },
        );
        let picked = fav.selection.ordered(order);
        if widgets::icon_button(
            ui,
            p,
            "circle-minus",
            aurora_i18n::tr!("wearable-favorites-remove-tip"),
            picked.iter().any(|id| model::is_link(inv, *id)),
        ) && let Some(change) = model::remove(inv, &picked)
        {
            actions.push(Action::Folders(change));
        }
    });
}

/// FSWearableFavoritesItemsList::handleDragAndDrop: clothing, body parts
/// and objects dragged from the inventory are linked in the folder shown.
fn drop_zone(ui: &egui::Ui, p: &Palette, inv: &Inventory, rect: egui::Rect, to: Uuid, actions: &mut Vec<Action>) {
    if !ui.rect_contains_pointer(rect) {
        return;
    }
    let Some(ids) = inventory::drag::carried_ids(ui.ctx()) else {
        return;
    };
    let targets = model::droppable(inv, &ids, to);
    let allowed = !targets.is_empty();
    inventory::tree::drop_outline(ui, p, rect, allowed);
    inventory::drag::feedback(
        ui.ctx(),
        if allowed {
            egui::CursorIcon::Copy
        } else {
            egui::CursorIcon::NotAllowed
        },
    );
    if ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary)) {
        egui::DragAndDrop::clear_payload(ui.ctx());
        if let Some(change) = model::link(inv, &targets, to) {
            actions.push(Action::Folders(change));
        }
    }
}

/// The questions of New, Rename and Delete folder, and the refused name
/// (FSWearableFavoritesNewFolder, …RenameFolder, …DeleteFolder,
/// …FolderInvalidName in notifications.xml).
fn dialogs(
    ctx: &egui::Context,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    root: Option<Uuid>,
    folders: &[(Uuid, String)],
    actions: &mut Vec<Action>,
) {
    let inv = &world.inventory;
    let fav = &mut st.wearable_favorites;
    let Some(dialog) = fav.dialog.as_mut() else {
        return;
    };
    let mut done = false;
    let mut next = None;
    let response = widgets::Modal::new(egui::Id::unique("wearable_favorites_dialog")).show(ctx, |ui| {
        ui.set_width(350.0);
        let folder_name = |id: &Uuid| inv.folders.get(id).map(|f| f.info.name.clone()).unwrap_or_default();
        let text = match dialog {
            Dialog::New(_) => aurora_i18n::tr!("wearable-favorites-new-folder-prompt").to_string(),
            Dialog::Rename { id, .. } => aurora_i18n::tr!("wearable-favorites-rename-prompt", name = folder_name(id)),
            Dialog::Delete(id) => aurora_i18n::tr!("wearable-favorites-delete-confirm", name = folder_name(id)),
            Dialog::DeleteRefused(name) => aurora_i18n::tr!("wearable-favorites-delete-refused", name = name.as_str()),
            Dialog::DeleteUnloaded(name) => aurora_i18n::tr!("wearable-favorites-delete-unloaded", name = name.as_str()),
            Dialog::Invalid(name) => aurora_i18n::tr!("wearable-favorites-invalid-name", name = name.as_str()),
        };
        ui.add(egui::Label::new(RichText::new(text).color(p.ink)).wrap());
        ui.add_space(8.0);
        if let Dialog::New(name) | Dialog::Rename { name, .. } = dialog {
            let r = widgets::restricted_text_field(ui, true, name, widgets::TextPolicy::PrintableNoPipe, |b, _| {
                egui::TextEdit::singleline(b).char_limit(63).desired_width(f32::INFINITY)
            });
            if fav.focus {
                r.request_focus();
                if let Some(mut state) = egui::TextEdit::load_state(ctx, r.id) {
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(name.chars().count()),
                    )));
                    state.store(ctx, r.id);
                }
                fav.focus = false;
            }
            ui.add_space(8.0);
        }
        ui.horizontal(|ui| {
            use widgets::{ActionKind, action_button};
            let size = Vec2::new(0.0, 26.0);
            let (icon, label, kind) = match dialog {
                Dialog::Delete(_) => ("trash-filled", aurora_i18n::tr!("common-delete"), ActionKind::Danger),
                _ => ("check", aurora_i18n::tr!("common-ok"), ActionKind::Primary),
            };
            let enter = widgets::enter_pressed(ui);
            if action_button(ui, p, icon, label, kind, size).clicked() || enter {
                done = true;
                match dialog {
                    Dialog::New(name) | Dialog::Rename { name, .. } => match model::folder_name(name) {
                        Ok(name) => {
                            let change = match (&*dialog, root) {
                                (Dialog::Rename { id, .. }, _) => model::rename_folder(inv, *id, &name),
                                (_, Some(root)) => {
                                    let before = folders.iter().map(|(id, _)| *id).collect();
                                    fav.select_new = Some((name.clone(), before));
                                    Some(model::create_folder(root, &name).0)
                                }
                                _ => None,
                            };
                            actions.extend(change.map(Action::Folders));
                        }
                        Err(model::NameError::Slash(name)) => next = Some(Dialog::Invalid(name)),
                        Err(model::NameError::Empty) => {}
                    },
                    Dialog::Delete(id) => {
                        if let Some(change) = model::delete_folder(inv, *id) {
                            fav.shown = None;
                            actions.push(Action::Folders(change));
                        }
                    }
                    Dialog::Invalid(_) | Dialog::DeleteRefused(_) | Dialog::DeleteUnloaded(_) => {}
                }
            }
            if !matches!(dialog, Dialog::Invalid(_) | Dialog::DeleteRefused(_) | Dialog::DeleteUnloaded(_))
                && action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked()
            {
                done = true;
            }
        });
    });
    if done || response.should_close() {
        fav.dialog = next;
    }
}

/// `AURORA_DEMO_WEARABLE_FAVORITES=evening`: the Evening folder picked.
pub fn demo_pick(inv: &Inventory, st: &mut AppearanceUi, name: &str) {
    if let Some(root) = model::folder(inv) {
        st.wearable_favorites.shown = model::subfolders(inv, root)
            .into_iter()
            .find(|(_, n)| n.eq_ignore_ascii_case(name))
            .map(|(id, _)| id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_import_button_returns_once_the_accepted_change_is_over() {
        let mut st = State::default();
        st.import_accepted();
        assert!(st.import_hidden());
        // still on its way: stays hidden, a second click cannot link twice
        st.import_settled(true);
        assert!(st.import_hidden());
        // done or failed: the plan decides whether the button shows again
        st.import_settled(false);
        assert!(!st.import_hidden());
    }
}
