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

use super::{Action, AppearanceUi, items, selection};
use crate::{
    theme::Palette,
    ui::{icons, inventory, menu, widgets, widgets::reorder},
    world::{World, inventory::Inventory, wearable_favorites as model},
};
use aurora_net::inventory::{InvItem, operations::Mutation};
use egui::{FontId, Rect, RichText, Sense, Vec2, Widget as _};
use model::SortOrder;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// The window's id (Floater.Toggle "fs_wearable_favorites").
pub const WINDOW: &str = "wearable_favorites";

/// The picture rows (from iGao's PR #3784): a square picture, the name and
/// the attachment point under it.
const PICTURE: f32 = 40.0;
const ROW: f32 = 54.0;
const WEAR_BUTTON: f32 = 28.0;
/// How strongly a worn row is washed in violet (a selected row is 0.25).
const WORN_WASH: f32 = 0.07;
/// How long the line saying what a Wear or Take off asked for stays.
const STATUS_SECONDS: f64 = 2.6;
const STATUS_HEIGHT: f32 = 26.0;
const TIP_PICTURE: f32 = 168.0;
const TIP_WIDTH: f32 = 240.0;

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
    /// The views' new custom orders (their folder, their originals), for
    /// the app to keep in the settings (`take_reordered`).
    reordered: Vec<(Uuid, Vec<Uuid>)>,
    /// "Wearing Boots": what the last Wear or Take off asked, and when.
    status: Option<(String, f64)>,
    /// Where a row was let go: no tooltip until the pointer leaves that
    /// spot, so that a drop does not end on a tooltip over the list.
    quiet_at: Option<egui::Pos2>,
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

    /// The custom orders changed this frame: each view and its new order.
    pub(crate) fn take_reordered(&mut self) -> Vec<(Uuid, Vec<Uuid>)> {
        std::mem::take(&mut self.reordered)
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
/// `writable` when an inventory change may be sent now; `brand_by_bridge`
/// while the bridge is about to make `#Aurora` (the folder waits for it,
/// and is asked for on a later frame). Deliberate
/// difference: the Legacy Viewer also calls initCategory at every login
/// (llstartup.cpp), so the folder appears in every account's inventory;
/// Aurora creates it only once the window is opened.
pub fn prepare(inv: &mut Inventory, st: &mut AppearanceUi, writable: bool, brand_by_bridge: bool) -> Option<Mutation> {
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
    let change = model::create_plan(inv, !brand_by_bridge)?;
    log::info!(
        "wearable favorites: creating the folder ({} folders to create)",
        change.operations.len()
    );
    st.created = true;
    Some(change)
}

/// Draw the window while `open`; `sort` is FSWearableFavoritesSortOrder,
/// `images` the decoded pictures, `orders` the account's custom orders.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    open: &mut bool,
    sort: &mut SortOrder,
    writable: bool,
    images: &HashMap<Uuid, egui::TextureHandle>,
    orders: Option<&model::Orders>,
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
        let points = world.worn_attachment_points();
        let saved: &[Uuid] = match (orders, shown) {
            (Some(orders), Some(view)) => orders.get(&view).map_or(&[], Vec::as_slice),
            _ => &[],
        };
        let mut rows = match (root, shown) {
            (Some(root), Some(shown)) => model::entries(inv, root, shown),
            _ => Vec::new(),
        };
        model::sort(inv, &mut rows, *sort, saved);
        let total = rows.len();
        let filter = st.wearable_favorites.filter.clone();
        rows.retain(|e| {
            inv.items
                .get(&e.target)
                .is_some_and(|it| model::matches(&filter, &it.name, labels.get(&e.target).map(String::as_str), &kind(it)))
        });
        let order: Vec<Uuid> = rows.iter().map(|e| e.link).collect();
        st.wearable_favorites.selection.retain(&order);
        let sort_before = *sort;
        filter_bar(ui, p, inv, st, sort, &order, &mut actions);
        ui.add_space(4.0);
        let now = ui.input(|i| i.time);
        let status = st.wearable_favorites.status.clone().filter(|(_, at)| now - at < STATUS_SECONDS);
        // the list takes what the status line leaves, measured: a guessed
        // height under it made the window grow at every frame
        let (mut above, footer) = widgets::above_footer(ui);
        let list = list_area(
            &mut above,
            p,
            world,
            st,
            images,
            &rows,
            &labels,
            &points,
            &folders,
            shown,
            total,
            sort,
            &mut actions,
        );
        footer.show(ui, |ui| {
            if let Some((text, at)) = &status {
                status_line(ui, p, text);
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f64((STATUS_SECONDS - (now - at)).max(0.0)));
            }
        });
        // Custom order chosen (the menu or a first drag): the other views
        // keep the order they showed instead of falling back to the type's
        if *sort == SortOrder::Custom
            && sort_before != SortOrder::Custom
            && let Some(root) = root
        {
            let views: Vec<Uuid> = std::iter::once(root).chain(folders.iter().map(|(id, _)| *id)).collect();
            let fresh: HashSet<Uuid> = st.wearable_favorites.reordered.iter().map(|(view, _)| *view).collect();
            let has = |view: Uuid| fresh.contains(&view) || orders.is_some_and(|o| o.contains_key(&view));
            let seeds = model::seeded_orders(inv, root, &views, sort_before, has);
            st.wearable_favorites.reordered.extend(seeds);
        }
        if let (Some(to), Some(list)) = (shown, list) {
            drop_zone(ui, p, inv, st, list, to, *sort, &mut actions);
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
            let name = crate::text_validation::default_name("wearable-favorites-new-folder-default");
            fav.dialog = Some(Dialog::New(name.into()));
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
                    // Aurora's own: the order the rows were dragged into
                    (
                        SortOrder::Custom,
                        "grip-vertical",
                        aurora_i18n::tr!("wearable-favorites-sort-custom"),
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

/// Where the list was drawn this frame, for a drop from the inventory: its
/// rect, the place a drop would take in Custom order (`gap`), and the
/// originals in the order shown.
struct DropPlace {
    rect: Rect,
    gap: Option<usize>,
    keys: Vec<Uuid>,
}

/// The picture rows (from iGao's PR #3784), dragged into a Custom order
/// through `widgets::reorder`, with a Wear / Take off button on the hovered
/// row, a tooltip with the picture larger, and the menu and double click of
/// the window as before.
#[allow(clippy::too_many_arguments)]
fn list_area(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    images: &HashMap<Uuid, egui::TextureHandle>,
    rows: &[model::Entry],
    labels: &HashMap<Uuid, String>,
    points: &HashMap<Uuid, u8>,
    folders: &[(Uuid, String)],
    shown: Option<Uuid>,
    total: usize,
    sort: &mut SortOrder,
    actions: &mut Vec<Action>,
) -> Option<DropPlace> {
    let inv = &world.inventory;
    let keys: Vec<Uuid> = rows.iter().map(|e| e.target).collect();
    let order: Vec<Uuid> = rows.iter().map(|e| e.link).collect();
    // a drag sorts the whole view by hand: not while filtering a part of it
    let draggable = shown.is_some() && st.wearable_favorites.filter.trim().is_empty();
    let carried = shown
        .and_then(|to| inventory::drag::carried_ids(ui.ctx()).map(|ids| model::droppable(inv, &ids, to)))
        .unwrap_or_default();
    let now = ui.input(|i| i.time);
    let area = egui::ScrollArea::vertical()
        .id_salt("wearable_favorites_rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let width = ui.available_width();
            // the places, fixed: what the pointer clicks, drags and drops on
            let responses: Vec<egui::Response> = rows
                .iter()
                .map(|_| ui.allocate_exact_size(Vec2::new(width, ROW), Sense::click_and_drag()).1)
                .collect();
            let pitch = ROW + ui.spacing().item_spacing.y;
            let pointer_y = ui
                .input(|i| i.pointer.hover_pos())
                .filter(|at| ui.clip_rect().contains(*at))
                .map(|at| at.y);
            // in Custom order, wearables carried from the inventory land
            // where they are let go: the rows make room there
            let gap = match (responses.first(), pointer_y) {
                (Some(first), Some(y)) if *sort == SortOrder::Custom && draggable && !carried.is_empty() => {
                    ui.allocate_space(Vec2::new(1.0, ROW));
                    Some(reorder::insert_slot(y - ROW / 2.0, first.rect.top(), pitch, responses.len()))
                }
                _ => None,
            };
            let id = ui.make_persistent_id(("wearable_favorites_reorder", shown));
            let layout = reorder::update(ui, id, &keys, &responses, draggable, gap);
            // the first drag sorts by hand from the order shown (`show`
            // keeps the other views' orders)
            if layout.started.is_some() && *sort != SortOrder::Custom {
                *sort = SortOrder::Custom;
            }
            if let (Some((from, to)), Some(view)) = (layout.moved, shown) {
                st.wearable_favorites.reordered.push((view, model::moved(&keys, from, to)));
            }
            let pointer = ui.input(|i| i.pointer.hover_pos());
            let fav = &mut st.wearable_favorites;
            if layout.moved.is_some() {
                fav.quiet_at = pointer;
            } else if fav.quiet_at.is_some() && pointer != fav.quiet_at {
                fav.quiet_at = None;
            }
            let quiet = layout.busy() || fav.quiet_at.is_some();
            for (i, (e, response)) in rows.iter().zip(&responses).enumerate() {
                let Some(it) = inv.items.get(&e.target) else {
                    continue;
                };
                let picture = picture(inv, e);
                if let Some(image) = picture {
                    st.wanted_images.insert(image);
                }
                let texture = picture.and_then(|image| images.get(&image));
                let worn = labels.get(&e.target).map(String::as_str);
                let is_worn = worn.is_some();
                let point = point(world, it, points.get(&it.id).copied());
                let at = layout.rect(i, response.rect);
                if layout.lifted == Some(i) {
                    reorder::lifted_card(ui, p, id, at, |card| {
                        paint_row(card, p, at, it, texture, is_worn, is_worn, point.as_deref(), None);
                    });
                    continue;
                }
                let selected = st.wearable_favorites.selection.contains(e.link);
                let face = if selected {
                    Some(p.violet.gamma_multiply(0.25))
                } else if response.hovered() && !layout.busy() {
                    Some(p.raised)
                } else if is_worn {
                    // worn: a faint wash, the check says it at the end
                    Some(p.violet.gamma_multiply(WORN_WASH))
                } else {
                    None
                };
                // the hovered row's Wear button takes the check's place
                let toggle = (!layout.busy() && ui.rect_contains_pointer(at))
                    .then(|| items::wear_toggle(world, it, is_worn, None))
                    .flatten();
                paint_row(ui, p, at, it, texture, is_worn, is_worn && toggle.is_none(), point.as_deref(), face);
                if response.clicked() || response.secondary_clicked() {
                    let modifiers = ui.input(|i| i.modifiers);
                    st.wearable_favorites
                        .selection
                        .click(e.link, &order, modifiers, response.secondary_clicked());
                }
                let asked = actions.len();
                if it.asset_type == 21 {
                    // a gesture is activated or deactivated (the inventory's double click)
                    if items::double_clicked(response) {
                        actions.push(Action::Gesture(vec![it.id], !world.active_gestures.contains(&it.id)));
                    }
                } else {
                    items::double_click(response, world, it, is_worn, None, actions);
                    // the same from a button on the hovered row, for those
                    // who would not guess the double click (Aurora's own)
                    if let Some(action) = toggle
                        && wear_button(ui, p, at, it, is_worn)
                    {
                        actions.push(action);
                    }
                    if actions.len() > asked {
                        st.wearable_favorites.status = Some((asked_status(it, is_worn), now));
                    }
                }
                let response = if quiet {
                    response.clone()
                } else {
                    response
                        .clone()
                        .on_hover_ui(|ui| tooltip(ui, p, inv, it, tooltip_picture(inv, texture), worn, point.as_deref()))
                };
                // the menu acts on the originals of the rows picked
                let picked: Vec<Uuid> = if st.wearable_favorites.selection.contains(e.link) {
                    rows.iter()
                        .filter(|r| st.wearable_favorites.selection.contains(r.link))
                        .map(|r| r.target)
                        .collect()
                } else {
                    vec![e.target]
                };
                items::context_menu(&response, p, world, st, items::List::Favorites(folders), it, &picked, actions);
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
            gap
        });
    shown.map(|_| DropPlace {
        rect: area.inner_rect,
        gap: area.inner,
        keys,
    })
}

/// FSWearableFavoritesItemsList::handleDragAndDrop: clothing, body parts
/// and objects dragged from the inventory are linked in the folder shown;
/// in Custom order they land where they are let go.
#[allow(clippy::too_many_arguments)]
fn drop_zone(
    ui: &egui::Ui,
    p: &Palette,
    inv: &Inventory,
    st: &mut AppearanceUi,
    place: DropPlace,
    to: Uuid,
    sort: SortOrder,
    actions: &mut Vec<Action>,
) {
    if !ui.rect_contains_pointer(place.rect) {
        return;
    }
    let Some(ids) = inventory::drag::carried_ids(ui.ctx()) else {
        return;
    };
    let targets = model::droppable(inv, &ids, to);
    let allowed = !targets.is_empty();
    inventory::tree::drop_outline(ui, p, place.rect, allowed);
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
            if sort == SortOrder::Custom
                && let Some(at) = place.gap
            {
                st.wearable_favorites
                    .reordered
                    .push((to, model::inserted(&place.keys, at, &targets)));
            }
        }
    }
}

/// "Shirt", "Object", "Gesture": the type the filter reads besides the
/// title, and the tooltip's line.
fn kind(it: &InvItem) -> String {
    match it.asset_type {
        5 | 13 => crate::world::appearance::wearables()
            .get((it.flags & 0xff) as usize)
            .map(|name| name.to_string())
            .unwrap_or_default(),
        21 => aurora_i18n::tr!("inventory-kind-gesture").into(),
        _ => aurora_i18n::tr!("common-object").into(),
    }
}

/// The tooltip's larger picture: none under RLVa's @viewtexture
/// (LLInspectTextureUtil::createInventoryToolTip, llinspecttexture.cpp).
fn tooltip_picture<'a>(inv: &Inventory, texture: Option<&'a egui::TextureHandle>) -> Option<&'a egui::TextureHandle> {
    texture.filter(|_| inv.can_preview(0))
}

/// The picture of a favorite: the original's own inventory picture, else
/// the link's (LLViewerInventoryItem::getThumbnailUUID). RLVa's @viewtexture
/// leaves the row's, as it leaves the inventory's tiles (LLThumbnailCtrl asks
/// nothing of RLVa), and takes the tooltip's (`tooltip_picture`).
fn picture(inv: &Inventory, entry: &model::Entry) -> Option<Uuid> {
    [entry.target, entry.link]
        .into_iter()
        .filter_map(|id| inv.items.get(&id))
        .map(|it| it.thumbnail)
        .find(|id| !id.is_nil())
}

/// "Left Hand", "HUD : Top Right": the point an object is worn on, else
/// its saved point; none for clothing, body parts, gestures and an object
/// never attached. A row's second line.
fn point(world: &World, it: &InvItem, worn_point: Option<u8>) -> Option<String> {
    if it.asset_type != 6 {
        return None;
    }
    let point = worn_point.or(Some((it.flags & 0xff) as u8).filter(|p| *p != 0))?;
    let name = world
        .avatar_lib
        .attach_names
        .get(&point)
        .map(|name| items::point_name(name).to_owned())
        .unwrap_or_else(|| aurora_i18n::tr!("attachment-point", point = point));
    let hud = world.avatar_lib.attach_points.get(&point).is_some_and(|ap| ap.hud);
    Some(format!("{}{name}", if hud { "HUD : " } else { "" }))
}

/// The row's Wear or Take off button at the right end of `row` (Detach for
/// an object, as the menus say); true when clicked.
fn wear_button(ui: &mut egui::Ui, p: &Palette, row: Rect, it: &InvItem, worn: bool) -> bool {
    let (icon, tip) = match (worn, it.asset_type == 6) {
        (false, _) => ("shirt-filled", aurora_i18n::tr!("common-wear")),
        (true, true) => ("circle-x-filled", aurora_i18n::tr!("common-detach")),
        (true, false) => ("circle-x-filled", aurora_i18n::tr!("inventory-take-off")),
    };
    let rect = Rect::from_center_size(
        egui::pos2(row.right() - 6.0 - WEAR_BUTTON / 2.0, row.center().y),
        Vec2::splat(WEAR_BUTTON),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        let button = widgets::action_button(ui, p, icon, "", widgets::ActionKind::Flat, Vec2::splat(WEAR_BUTTON)).on_hover_text(tip);
        // the second click of a double click asks nothing more: the worn
        // state has not changed yet, so it would ask the same again
        button.clicked() && !button.double_clicked()
    })
    .inner
}

/// "Wearing Boots", "Taking off Boots": the status line after the button or
/// a double click.
fn asked_status(it: &InvItem, worn: bool) -> String {
    let name = it.name.clone();
    if worn {
        aurora_i18n::tr!("wearable-favorites-taking-off", name = name).to_string()
    } else {
        aurora_i18n::tr!("wearable-favorites-wearing", name = name).to_string()
    }
}

/// What the last Wear or Take off asked for, at the bottom of the window,
/// under the list and never over a row.
fn status_line(ui: &mut egui::Ui, p: &Palette, text: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), STATUS_HEIGHT), Sense::hover());
    ui.painter()
        .rect_filled(rect, ui.visuals().window_corner_radius, p.violet.gamma_multiply(0.22));
    widgets::paint_elided(
        ui,
        text,
        egui::pos2(rect.left() + 10.0, rect.center().y),
        rect.width() - 20.0,
        FontId::proportional(12.0),
        p.ink,
    );
}

/// One picture row at `rect`: its face (selected, hovered, worn, none),
/// the picture (or the type icon), the name (bold when `worn`), the
/// attachment point below it when there is one, and a check at the right
/// end when `check` (a worn row, unless its Wear button is shown there).
#[allow(clippy::too_many_arguments)]
fn paint_row(
    ui: &egui::Ui,
    p: &Palette,
    rect: Rect,
    it: &InvItem,
    texture: Option<&egui::TextureHandle>,
    worn: bool,
    check: bool,
    point: Option<&str>,
    face: Option<egui::Color32>,
) {
    let radius = ui.visuals().window_corner_radius;
    if let Some(face) = face {
        ui.painter().rect_filled(rect, radius, face);
    }
    let pic = Rect::from_center_size(egui::pos2(rect.left() + 6.0 + PICTURE / 2.0, rect.center().y), Vec2::splat(PICTURE));
    ui.painter().rect_filled(pic, radius, p.field);
    if let Some(t) = texture {
        egui::Image::new(t)
            .uv(crop_square(t.size_vec2()))
            .corner_radius(radius)
            .tint(widgets::PICTURE_TINT)
            .paint_at(ui, pic);
    } else if let Some(t) = icons::global(inventory::item_icon(it.asset_type, it.inv_type, it.flags)) {
        let tint = inventory::item_icon_color(it.asset_type, it.inv_type, p);
        egui::Image::new(&t)
            .tint(tint)
            .paint_at(ui, Rect::from_center_size(pic.center(), Vec2::splat(22.0)));
    }
    let left = pic.right() + 10.0;
    let width = (rect.right() - left - WEAR_BUTTON - 10.0).max(20.0);
    let name = one_line(ui, &it.name, name_format(ui, worn, p.ink), width);
    let sub = point.map(|point| {
        let format = egui::TextFormat::simple(FontId::proportional(11.0), p.muted);
        one_line(ui, point, format, width)
    });
    let height = name.size().y + sub.as_ref().map_or(0.0, |sub| sub.size().y + 2.0);
    let top = rect.center().y - height / 2.0;
    if worn && let Some(offset) = widgets::faux_bold_offset(ui) {
        ui.painter().galley(egui::pos2(left, top) + offset, name.clone(), p.ink);
    }
    ui.painter().galley(egui::pos2(left, top), name.clone(), p.ink);
    if let Some(sub) = sub {
        ui.painter().galley(egui::pos2(left, top + name.size().y + 2.0), sub, p.muted);
    }
    if check && let Some(t) = icons::global("circle-check-filled") {
        let center = egui::pos2(rect.right() - 6.0 - WEAR_BUTTON / 2.0, rect.center().y);
        egui::Image::new(&t)
            .tint(p.violet)
            .paint_at(ui, Rect::from_center_size(center, Vec2::splat(15.0)));
    }
}

/// A row's name: bold for a worn item, the regular face for the rest, as
/// the Legacy Viewer's wearable lists mark what is worn (`IS_WORN` draws
/// `worn_style`, `font.style="BOLD"` in `wearable_outfit_list_item.xml`).
/// A static system font has no bold face: `paint_row` then paints a worn
/// name twice.
fn name_format(ui: &egui::Ui, worn: bool, color: egui::Color32) -> egui::TextFormat {
    let font = FontId::proportional(12.5);
    if worn {
        widgets::bold_format(ui, font, color)
    } else {
        egui::TextFormat::simple(font, color)
    }
}

fn one_line(ui: &egui::Ui, text: &str, format: egui::TextFormat, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), format);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    ui.painter().layout_job(job)
}

/// The middle square of a picture, so that a wide or tall one fills the
/// frame without being squeezed.
fn crop_square(size: Vec2) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
    }
    let side = size.x.min(size.y);
    let (u, v) = (side / size.x, side / size.y);
    Rect::from_min_size(egui::pos2((1.0 - u) / 2.0, (1.0 - v) / 2.0), Vec2::new(u, v))
}

/// The full name (wrapped), the picture larger, type and point, worn
/// state and permissions: what a cut row cannot show.
fn tooltip(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    it: &InvItem,
    texture: Option<&egui::TextureHandle>,
    worn: Option<&str>,
    point: Option<&str>,
) {
    ui.set_max_width(TIP_WIDTH);
    if let Some(t) = texture {
        egui::Image::new(t)
            .uv(crop_square(t.size_vec2()))
            .corner_radius(ui.visuals().window_corner_radius)
            .fit_to_exact_size(Vec2::splat(TIP_PICTURE))
            .ui(ui);
        ui.add_space(4.0);
    }
    ui.add(egui::Label::new(RichText::new(&it.name).color(p.ink).strong()).wrap());
    let detail = match point {
        Some(point) => format!("{} · {point}", kind(it)),
        None => kind(it),
    };
    ui.label(RichText::new(detail).color(p.muted));
    if let Some(worn) = worn {
        ui.label(RichText::new(worn).color(p.violet));
    }
    widgets::divider(ui, p);
    let held = crate::world::inventory::actions::held(inv, it);
    let permissions = inventory::permissions::Permissions::new(held, it.next_owner_mask);
    ui.label(RichText::new(permissions.tooltip()).color(p.muted).size(11.0));
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
    fn the_status_line_keeps_the_window_steady() {
        let agent = Uuid::from_u128(2);
        let status = aurora_i18n::tr!("wearable-favorites-wearing", name = "Boots").to_string();
        crate::ui::widgets::steady_tests::assert_window_steady(
            WINDOW,
            &[Vec2::new(360.0, 430.0), Vec2::new(260.0, 220.0)],
            &status,
            || {
                let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
                world.agent_id = agent;
                crate::world::appearance::seed_demo(&mut world.inventory, agent);
                model::seed_demo(&mut world.inventory, agent);
                let mut st = AppearanceUi::default();
                // a Wear just asked: the line shows under the list
                st.wearable_favorites.status = Some((status.clone(), 0.0));
                (world, st, SortOrder::Custom)
            },
            |(world, st, sort), ctx, p| {
                // asked again at every frame: the line stays the whole run
                if let Some((_, at)) = &mut st.wearable_favorites.status {
                    *at = ctx.input(|i| i.time);
                }
                let mut open = true;
                show(ctx, p, world, st, &mut open, sort, true, &HashMap::new(), None);
            },
        );
    }

    /// Where the pointer is while the window is painted: away from it, over
    /// the worn row (its Wear button takes the check's place), or carrying
    /// that row down a Custom-order list (painted as the lifted card).
    #[derive(Clone, Copy, PartialEq)]
    enum Pointer {
        Away,
        Over,
        Carrying,
    }

    /// The window as painted under the interface font `font` (`None`: no
    /// font set up, egui's own, which has no heavier face): does that font
    /// have one, each text's format and position every time it is painted,
    /// and the filled rectangles.
    struct Painted {
        has_semibold: bool,
        semibold: egui::FontFamily,
        ink: egui::Color32,
        raised: egui::Color32,
        texts: HashMap<String, Vec<(egui::TextFormat, egui::Pos2)>>,
        fills: Vec<(Rect, egui::Color32)>,
    }

    impl Painted {
        /// Where the worn name is first painted.
        fn worn_at(&self) -> egui::Pos2 {
            let worn = aurora_i18n::tr!("world-appearance-leather-jacket-2").to_string();
            self.texts[&worn][0].1
        }
    }

    fn painted(font: Option<&str>, pointer: Pointer) -> Painted {
        let agent = Uuid::from_u128(2);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = agent;
        // the root folder a login brings, which the favorites hang from
        let root = world.inventory.root;
        world.inventory.folders.insert(
            root,
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: root,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: crate::world::inventory::FetchState::Fetched,
                library: false,
            },
        );
        crate::world::appearance::seed_demo(&mut world.inventory, agent);
        model::seed_demo(&mut world.inventory, agent);
        let mut st = AppearanceUi::default();
        let p = crate::theme::Theme::default().palette();
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        crate::theme::Theme::default().apply(&frames.ctx, 1.0);
        if let Some(font) = font {
            crate::ui::fonts::apply(&frames.ctx, font);
        }
        crate::ui::widgets::place_floater(&frames.ctx, WINDOW, egui::pos2(40.0, 40.0), Vec2::new(360.0, 640.0));
        let mut sort = SortOrder::Custom;
        let mut frame = |events: Vec<egui::Event>| {
            frames.shapes(events, |ui| {
                let mut open = true;
                show(ui.ctx(), &p, &world, &mut st, &mut open, &mut sort, true, &HashMap::new(), None);
            })
        };
        let mut shapes = Vec::new();
        for _ in 0..5 {
            shapes = frame(Vec::new());
        }
        if pointer != Pointer::Away {
            let worn = aurora_i18n::tr!("world-appearance-leather-jacket-2").to_string();
            let name = shapes
                .iter()
                .find_map(|s| match s {
                    egui::Shape::Text(t) if t.galley.text() == worn => Some(t.pos),
                    _ => None,
                })
                .expect("the worn row is painted");
            let at = name + Vec2::new(4.0, 6.0);
            match pointer {
                Pointer::Over => {
                    for _ in 0..3 {
                        shapes = frame(vec![egui::Event::PointerMoved(at)]);
                    }
                }
                _ => {
                    let press = egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    };
                    shapes = frame(vec![egui::Event::PointerMoved(at), press]);
                    for step in 1..=4 {
                        shapes = frame(vec![egui::Event::PointerMoved(at + Vec2::new(0.0, 8.0 * step as f32))]);
                    }
                }
            }
        }
        let mut texts: HashMap<String, Vec<_>> = HashMap::new();
        let mut fills = Vec::new();
        for shape in shapes {
            match shape {
                egui::Shape::Text(t) => {
                    if let Some(section) = t.galley.job.sections.first() {
                        texts
                            .entry(t.galley.text().to_owned())
                            .or_default()
                            .push((section.format.clone(), t.pos));
                    }
                }
                egui::Shape::Rect(r) => fills.push((r.rect, r.fill)),
                _ => {}
            }
        }
        Painted {
            has_semibold: crate::ui::fonts::has_semibold(&frames.ctx),
            semibold: crate::ui::fonts::semibold(&frames.ctx),
            ink: p.ink,
            raised: p.raised,
            texts,
            fills,
        }
    }

    /// The leather jacket is worn, the spare shirt is not (`model::seed_demo`,
    /// both clothing, worn by the current outfit): only the worn name is
    /// drawn heavier, in the face `font` has for it.
    fn assert_only_the_worn_name_is_bold(w: &Painted, font: &str) {
        let worn = aurora_i18n::tr!("world-appearance-leather-jacket-2").to_string();
        let spare = aurora_i18n::tr!("world-appearance-spare-shirt").to_string();
        let regular = egui::TextFormat::simple(FontId::proportional(12.5), w.ink);
        let spare_paints = &w.texts[&spare];
        assert_eq!(spare_paints.len(), 1, "{font}: {spare:?} is painted once");
        assert_eq!(spare_paints[0].0, regular, "{font}: {spare:?} is in the regular face");
        let worn_paints = &w.texts[&worn];
        let bold = worn_paints[0].0.clone();
        assert_eq!(bold.font_id.family, w.semibold, "{font}: {worn:?} is in the semi-bold family");
        assert_ne!(bold.coords, regular.coords, "{font}: {worn:?} asks a variable font for its weight");
        if w.has_semibold {
            assert_eq!(worn_paints.len(), 1, "{font}: {worn:?} is painted once, in its heavier face");
        } else {
            // no heavier face: a copy a pixel to the right is painted
            // first, then the name itself
            assert_eq!(worn_paints.len(), 2, "{font}: {worn:?} is painted twice");
            let shift = worn_paints[0].1 - worn_paints[1].1;
            assert!(shift.y == 0.0 && shift.x > 0.0, "{font}: the copy is to the right, by {shift:?}");
        }
    }

    #[test]
    fn only_a_worn_favorite_has_a_bold_name() {
        let w = painted(Some(crate::ui::fonts::DEFAULT), Pointer::Away);
        assert!(w.has_semibold, "the default font has a bold face");
        assert_only_the_worn_name_is_bold(&w, crate::ui::fonts::DEFAULT);
    }

    /// A font without a heavier face, on every machine: the worn name is
    /// painted twice (`faux_bold_offset`).
    #[test]
    fn a_worn_favorite_is_bold_in_a_font_without_a_bold_face() {
        let w = painted(None, Pointer::Away);
        assert!(!w.has_semibold, "egui's own font has no bold face");
        assert_only_the_worn_name_is_bold(&w, "egui's font");
    }

    /// The hovered worn row shows its Take off button where the check was;
    /// its name stays bold.
    #[test]
    fn a_hovered_worn_favorite_keeps_its_bold_name() {
        let w = painted(Some(crate::ui::fonts::DEFAULT), Pointer::Over);
        let name = w.worn_at();
        assert!(
            w.fills
                .iter()
                .any(|(rect, fill)| *fill == w.raised && rect.contains(name + Vec2::new(4.0, 6.0))),
            "the worn row is hovered"
        );
        assert_only_the_worn_name_is_bold(&w, "hovered");
    }

    /// The worn row carried in a Custom-order drag, as the lifted card: its
    /// name stays bold.
    #[test]
    fn a_carried_worn_favorite_keeps_its_bold_name() {
        let still = painted(Some(crate::ui::fonts::DEFAULT), Pointer::Away).worn_at();
        let w = painted(Some(crate::ui::fonts::DEFAULT), Pointer::Carrying);
        assert!(
            w.worn_at().y > still.y + 8.0,
            "the worn row is carried down: {still:?} → {:?}",
            w.worn_at()
        );
        assert_only_the_worn_name_is_bold(&w, "carried");
    }

    /// A system font as the interface font: a variable one draws a worn
    /// name at its semi-bold weight, a static one twice. Which kinds this
    /// machine has varies; a few of its fonts are enough.
    #[test]
    fn a_worn_favorite_stays_bold_with_a_system_font() {
        let (mut variable, mut fixed) = (false, false);
        for font in crate::ui::fonts::system_fonts().into_iter().take(8) {
            let font = format!("system:{}", font.path.display());
            let w = painted(Some(&font), Pointer::Away);
            assert_only_the_worn_name_is_bold(&w, &font);
            variable |= w.has_semibold;
            fixed |= !w.has_semibold;
            if variable && fixed {
                return;
            }
        }
        eprintln!("system fonts checked: variable {variable}, static {fixed}");
    }

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

    /// RLVa's @viewtexture leaves a favorite's row picture, as the inventory's
    /// tiles (LLThumbnailCtrl), and takes the tooltip's
    /// (LLInspectTextureUtil::createInventoryToolTip).
    #[test]
    fn rlva_viewtexture_leaves_the_row_picture_and_takes_the_tooltip_one() {
        let mut inv = Inventory::default();
        let (item, thumbnail) = (Uuid::from_u128(0x7401), Uuid::from_u128(0x7402));
        inv.items.insert(
            item,
            InvItem {
                id: item,
                thumbnail,
                ..Default::default()
            },
        );
        let entry = model::Entry { link: item, target: item };
        let ctx = egui::Context::default();
        let image = ctx.load_texture(
            "favorite",
            egui::ColorImage::filled([4, 4], egui::Color32::GRAY),
            Default::default(),
        );
        assert_eq!(picture(&inv, &entry), Some(thumbnail));
        assert!(tooltip_picture(&inv, Some(&image)).is_some());
        inv.rlva_preview_mask = 1;
        assert_eq!(picture(&inv, &entry), Some(thumbnail), "the row keeps it");
        assert!(tooltip_picture(&inv, Some(&image)).is_none(), "the tooltip loses it");
    }
}
