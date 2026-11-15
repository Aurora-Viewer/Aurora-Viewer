//! The Favorite Wearables window: port of FSFloaterWearableFavorites
//! (indra/newview/fsfloaterwearablefavorites.cpp, originally LGPL 2.1).
//! What it lists and changes is in `world::wearable_favorites`; this module
//! draws it and turns the gestures into Appearance actions.
//!
//! Drawn as picture rows instead of the Legacy name list (human decision,
//! 2026-11-11): the item's own inventory picture, or its type icon, then its
//! name and "type · point" on one line each, a full tooltip for long names.
//! The subfolders are picked from one folder menu, as the Legacy combo box
//! does, and the menu also holds New folder, Rename and Delete instead of
//! three buttons. It lists the folders downwards and never scrolls
//! sideways, at any window width (human decision, 2026-11-15).
//! A row's second line is the attachment point alone (human decision,
//! 2026-11-15); the type is in its tooltip and the search. A worn row has
//! a faint violet wash and a check at its end; on the hovered row a button
//! there wears or takes off, as a double-click does.
//! Rows can be dragged into a custom order (`widgets::reorder`; human
//! request, 2026-11-15), which the Legacy window does not have.

use super::{Action, AppearanceUi, items, selection::Selection};
use crate::{
    theme::Palette,
    ui::{icons, menu, widgets, widgets::reorder},
    world::{
        World,
        inventory::actions,
        wearable_favorites::{self as model, Entry, Sort},
    },
};
use aurora_net::inventory::InvItem;
use egui::{FontId, Rect, Sense, Vec2, Widget as _, text::LayoutJob};
use std::collections::HashMap;
use uuid::Uuid;

pub const WINDOW: &str = "wearable_favorites";
const PICTURE: f32 = 40.0;
const ROW: f32 = 54.0;
/// How long the line saying what a row's button or double-click asked
/// for stays at the bottom of the window (seconds).
const STATUS_SECONDS: f64 = 2.6;
const TIP_PICTURE: f32 = 168.0;
const TIP_WIDTH: f32 = 240.0;

#[derive(Default)]
pub(crate) struct State {
    /// The folder shown: a subfolder, or None for every favorite.
    shown: Option<Uuid>,
    filter: String,
    selection: Selection,
    dialog: Option<Dialog>,
    focus: bool,
    /// The inventory root the folder was asked for, so that it is asked
    /// once per account (initCategory) and again after a relog.
    requested: Option<Uuid>,
    message: String,
    /// The views' new custom orders (their folder, their wearables), for
    /// the app to keep in the settings.
    reordered: Vec<(Uuid, Vec<Uuid>)>,
    /// A folder just asked for, shown once the inventory lists it
    /// (onNewFolderCallback shows the new folder).
    select_new: Option<Uuid>,
    /// Another inventory change is still on its way: the app takes no new
    /// one until it lands, so the window asks for none.
    busy: bool,
    /// Where a row was let go: no tooltip until the pointer leaves that
    /// spot, so that a drop does not end on a tooltip over the list.
    quiet_at: Option<egui::Pos2>,
    /// "Wearing Boots": what was asked last, and when (`input.time`).
    status: Option<(String, f64)>,
}

impl State {
    /// The custom orders changed this frame: each view and its new order.
    pub(crate) fn take_reordered(&mut self) -> Vec<(Uuid, Vec<Uuid>)> {
        std::mem::take(&mut self.reordered)
    }
}

enum Dialog {
    New(String),
    Rename(Uuid, String),
    Delete(Uuid),
}

/// The window, while `open`. `sort` is the remembered order
/// (`FSWearableFavoritesSortOrder`), `orders` the account's custom orders,
/// `busy` while another inventory change is on its way (the app would
/// drop a new one).
#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    open: &mut bool,
    images: &HashMap<Uuid, egui::TextureHandle>,
    sort: &mut Sort,
    orders: Option<&model::Orders>,
    busy: bool,
) -> Vec<Action> {
    let mut actions = Vec::new();
    if !*open {
        return actions;
    }
    st.favorites.busy = busy;
    let sort_before = *sort;
    let inv = &world.inventory;
    let root = model::root(inv);
    // initCategory: made the first time the window opens, asked again
    // after a change in flight instead of being lost behind it.
    if root.is_none()
        && !busy
        && st.favorites.requested != Some(inv.root)
        && let Some(change) = model::create(inv)
    {
        st.favorites.requested = Some(inv.root);
        actions.push(Action::Folders(change));
    }
    let tabs = root.map(|root| model::folders(inv, root)).unwrap_or_default();
    if let Some(id) = st.favorites.select_new
        && tabs.iter().any(|(tab, _)| *tab == id)
    {
        st.favorites.shown = Some(id);
        st.favorites.select_new = None;
    }
    // A deleted tab falls back to All, once the folder is loaded: a frame
    // where it is being listed again must not lose the tab.
    if let Some(root) = root
        && model::loaded(inv, root)
        && st.favorites.shown.is_some_and(|id| !tabs.iter().any(|(tab, _)| *tab == id))
    {
        st.favorites.shown = None;
    }
    let shown = st.favorites.shown.or(root);
    let mut list = shown.map(|id| model::entries(inv, id)).unwrap_or_default();
    let saved = shown.and_then(|id| orders?.get(&id)).map_or(&[][..], Vec::as_slice);
    model::sort(inv, &mut list, *sort, saved);
    let total = list.len();
    let all = match (root, st.favorites.shown) {
        (Some(root), Some(_)) => model::entries(inv, root).len(),
        _ => total,
    };
    let filter = st.favorites.filter.trim().to_string();
    if !filter.is_empty() {
        let points = world.worn_attachment_points();
        list.retain(|e| {
            inv.items
                .get(&e.item)
                .is_some_and(|it| model::matches(inv, e, &filter, &detail(world, it, points.get(&it.id).copied())))
        });
    }
    let order: Vec<Uuid> = list.iter().map(|e| e.item).collect();
    st.favorites.selection.retain(&order);

    let mut floater = widgets::Floater::new(
        WINDOW,
        aurora_i18n::tr!("ui-wearable-favorites-title"),
        egui::pos2(620.0, 60.0),
        Vec2::new(320.0, 480.0),
    )
    .help(aurora_i18n::tr!("ui-wearable-favorites-help"));
    floater.min_size = Vec2::new(260.0, 300.0);
    floater.title_note = (all > 0).then(|| all.to_string());
    floater.show(ctx, p, open, |ui| {
        let Some(root) = root else {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| ui.label(egui::RichText::new(aurora_i18n::tr!("common-loading-2")).color(p.muted)));
            return;
        };
        header(ui, p, world, st, root, &order, sort, &mut actions);
        ui.add_space(4.0);
        folder_row(ui, p, inv, st, &tabs, all);
        ui.add_space(6.0);
        if !st.favorites.message.is_empty() {
            ui.label(egui::RichText::new(&st.favorites.message).color(p.danger).size(11.5));
        }
        let now = ui.input(|i| i.time);
        let status = st.favorites.status.clone().filter(|(_, at)| now - at < STATUS_SECONDS);
        let room = ui.available_size() - Vec2::new(0.0, if status.is_some() { STATUS_HEIGHT + 4.0 } else { 0.0 });
        ui.allocate_ui(room, |ui| {
            list_area(
                ui,
                p,
                world,
                st,
                images,
                root,
                &tabs,
                &list,
                &order,
                total,
                &filter,
                sort,
                &mut actions,
            );
        });
        if let Some((text, at)) = status {
            status_line(ui, p, &text);
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64((STATUS_SECONDS - (now - at)).max(0.0)));
        }
    });
    dialog(ctx, p, world, st, root, &mut actions);
    if let Some(root) = root
        && *sort == Sort::Custom
        && sort_before != Sort::Custom
    {
        let views: Vec<Uuid> = std::iter::once(root).chain(tabs.iter().map(|(id, _)| *id)).collect();
        let reordered = &mut st.favorites.reordered;
        let seeds = model::seeded_orders(inv, &views, sort_before, |view| {
            orders.is_some_and(|o| o.contains_key(&view)) || reordered.iter().any(|(v, _)| *v == view)
        });
        reordered.extend(seeds);
    }
    actions
}

/// The search, the sort menu and Remove (the Legacy remove_btn).
#[allow(clippy::too_many_arguments)]
fn header(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    root: Uuid,
    order: &[Uuid],
    sort: &mut Sort,
    actions: &mut Vec<Action>,
) {
    // the buttons are as tall as the field, measured on the last frame
    let height_id = search_height_id(ui);
    let side = ui.data(|d| d.get_temp::<f32>(height_id)).unwrap_or(BUTTON);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let width = (ui.available_width() - 2.0 * (side + 4.0)).max(80.0);
        // the Search window's field: its icon in front, the clear button in it
        widgets::clearable_search_box(
            ui,
            p,
            &mut st.favorites.filter,
            aurora_i18n::tr!("ui-wearable-favorites-search"),
            width,
        );
        let measured = ui.min_rect().height().round();
        if measured > 0.0 && measured != side {
            ui.data_mut(|d| d.insert_temp(height_id, measured));
            ui.ctx().request_repaint();
        }
        let sort_button = widgets::action_button(ui, p, "arrows-sort", "", widgets::ActionKind::Flat, Vec2::splat(side))
            .on_hover_text(aurora_i18n::tr!("ui-wearable-favorites-sort"));
        menu::dropdown(&sort_button, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            for (value, label) in [
                (Sort::Name, aurora_i18n::tr!("ui-wearable-favorites-sort-name")),
                (Sort::Recent, aurora_i18n::tr!("ui-wearable-favorites-sort-recent")),
                (Sort::Type, aurora_i18n::tr!("ui-wearable-favorites-sort-type")),
                (Sort::Custom, aurora_i18n::tr!("ui-wearable-favorites-sort-custom")),
            ] {
                if menu::check(ui, p, "", label, *sort == value, true) {
                    *sort = value;
                }
            }
        });
        let selected = st.favorites.selection.ordered(order);
        let shown = st.favorites.shown.unwrap_or(root);
        if widgets::action_button_if(
            ui,
            p,
            "trash-filled",
            "",
            widgets::ActionKind::Flat,
            Vec2::splat(side),
            !selected.is_empty() && !st.favorites.busy,
        )
        .on_hover_text(aurora_i18n::tr!("ui-wearable-favorites-remove"))
        .clicked()
            && let Some(change) = model::remove(&world.inventory, shown, &selected)
        {
            actions.push(Action::Folders(change));
        }
    });
}

/// All favorites or one subfolder, picked from one menu that lists them
/// downwards (the Legacy folder combo box), with New folder, Rename and
/// Delete (new_folder_btn, rename_folder_btn, delete_folder_btn) at its
/// end and on a right click. However many folders and whatever the width,
/// nothing scrolls sideways; the menu's search field finds a folder by
/// name. New folder also has its own button, as in the Legacy window.
fn folder_row(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &crate::world::inventory::Inventory,
    st: &mut AppearanceUi,
    tabs: &[(Uuid, String)],
    all: usize,
) {
    let shown = st.favorites.shown.and_then(|id| tabs.iter().find(|(tab, _)| *tab == id)).cloned();
    let all_label = aurora_i18n::tr!("ui-wearable-favorites-all");
    let (icon, tint, label, count) = match &shown {
        Some((id, name)) => ("folder-filled", p.violet_light, name.as_str(), model::entries(inv, *id).len()),
        None => ("heart-filled", p.violet_light, all_label, all),
    };
    let side = ui.data(|d| d.get_temp::<f32>(search_height_id(ui))).unwrap_or(BUTTON);
    let open_id = ui.make_persistent_id("wearable_favorites_folder_menu");
    let open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let width = (ui.available_width() - side - 4.0).max(80.0);
        let button = widgets::select_button(ui, p, icon, tint, label, &count.to_string(), open, Vec2::new(width, side));
        if demo_mode().is_some() {
            ui.ctx().data_mut(|d| d.insert_temp(demo_picker_id(), button.rect));
        }
        let open = menu::dropdown(&button, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            menu::searchable(ui, p, |ui| {
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    let current = shown.is_none();
                    if menu::current_row(ui, p, "heart-filled", all_label, &all.to_string(), current).clicked() {
                        st.favorites.shown = None;
                    }
                    for (id, name) in tabs {
                        let count = model::entries(inv, *id).len().to_string();
                        let current = shown.as_ref().is_some_and(|(tab, _)| tab == id);
                        if menu::current_row(ui, p, "folder-filled", name, &count, current).clicked() {
                            st.favorites.shown = Some(*id);
                        }
                    }
                });
                menu::separator(ui, p);
                folder_commands(ui, p, st, shown.as_ref());
            });
        });
        ui.data_mut(|d| d.insert_temp(open_id, open));
        menu::context_menu_of_button(&button, p, |ui| folder_commands(ui, p, st, shown.as_ref()));
        if widgets::action_button(ui, p, "folder-plus", "", widgets::ActionKind::Flat, Vec2::splat(side))
            .on_hover_text(aurora_i18n::tr!("common-new-folder"))
            .clicked()
        {
            st.favorites.dialog = Some(Dialog::New(String::new()));
            st.favorites.focus = true;
        }
    });
}

/// The search field's height, which the buttons of both rows take.
fn search_height_id(ui: &egui::Ui) -> egui::Id {
    ui.make_persistent_id("wearable_favorites_search_height")
}

/// New folder, and Rename and Delete of the folder `shown`.
fn folder_commands(ui: &mut egui::Ui, p: &Palette, st: &mut AppearanceUi, shown: Option<&(Uuid, String)>) {
    if menu::item(ui, p, "folder-plus", aurora_i18n::tr!("common-new-folder")) {
        st.favorites.dialog = Some(Dialog::New(String::new()));
        st.favorites.focus = true;
    }
    if let Some((id, name)) = shown {
        if menu::item(ui, p, "pencil-filled", aurora_i18n::tr!("ui-wearable-favorites-rename-folder")) {
            st.favorites.dialog = Some(Dialog::Rename(*id, name.clone()));
            st.favorites.focus = true;
        }
        if menu::item(ui, p, "trash-filled", aurora_i18n::tr!("ui-wearable-favorites-delete-folder")) {
            st.favorites.dialog = Some(Dialog::Delete(*id));
        }
    }
}

const STATUS_HEIGHT: f32 = 26.0;

/// What the last Wear or Take off asked for, at the bottom of the window,
/// under the list and never over a row.
fn status_line(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add_space(4.0);
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

/// The rows, the empty states, and the drop of wearables from the
/// inventory (FSWearableFavoritesItemsList::handleDragAndDrop). A row
/// dragged is carried to another place (`widgets::reorder`): the first drag
/// turns the sort to Custom order, starting from the order shown, as Apple
/// Reminders does. Not while searching, where the places shown are not the
/// list's. In Custom order, wearables dropped from the inventory open a gap
/// and land where they were let go.
#[allow(clippy::too_many_arguments)]
fn list_area(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    images: &HashMap<Uuid, egui::TextureHandle>,
    root: Uuid,
    tabs: &[(Uuid, String)],
    list: &[Entry],
    order: &[Uuid],
    total: usize,
    filter: &str,
    sort: &mut Sort,
    actions: &mut Vec<Action>,
) {
    let inv = &world.inventory;
    let target = st.favorites.shown.unwrap_or(root);
    let area = Rect::from_min_size(ui.cursor().min, ui.available_size());
    let carried = (ui.rect_contains_pointer(area) && !st.favorites.busy)
        .then(|| crate::ui::inventory::drag::carried_ids(ui.ctx()))
        .flatten()
        .map(|ids| model::addable(inv, target, &ids))
        .unwrap_or_default();
    let worn = items::worn_items(world);
    let points = world.worn_attachment_points();
    let draggable = filter.is_empty() && st.favorites.dialog.is_none();
    let pointer_y = ui.input(|i| i.pointer.hover_pos()).map(|at| at.y);
    let mut gap = None;
    egui::ScrollArea::vertical()
        .id_salt("wearable_favorites_rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if list.is_empty() {
                empty(ui, p, inv, target, total, filter);
            }
            // the places, fixed: what the pointer clicks, drags and drops on
            let rows: Vec<egui::Response> = list
                .iter()
                .map(|_| {
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click_and_drag())
                        .1
                })
                .collect();
            if let (Some(first), Some(y)) = (rows.first(), pointer_y)
                && *sort == Sort::Custom
                && draggable
                && !carried.is_empty()
            {
                let pitch = ROW + ui.spacing().item_spacing.y;
                gap = Some(reorder::insert_slot(y - ROW / 2.0, first.rect.top(), pitch, rows.len()));
                // room for the rows pushed down
                ui.allocate_space(Vec2::new(1.0, ROW));
            }
            let id = ui.make_persistent_id(("wearable_favorites_reorder", target));
            let layout = reorder::update(ui, id, order, &rows, draggable, gap);
            if layout.started.is_some() && *sort != Sort::Custom {
                *sort = Sort::Custom;
                st.favorites.reordered.push((target, order.to_vec()));
            }
            if let Some((from, to)) = layout.moved {
                st.favorites.reordered.push((target, model::moved(order, from, to)));
            }
            let pointer = ui.input(|i| i.pointer.hover_pos());
            if layout.moved.is_some() {
                st.favorites.quiet_at = pointer;
            } else if st.favorites.quiet_at.is_some() && pointer != st.favorites.quiet_at {
                st.favorites.quiet_at = None;
            }
            let quiet = layout.busy() || st.favorites.quiet_at.is_some();
            if demo_mode().is_some() {
                let rects: Vec<Rect> = rows.iter().map(|r| r.rect).collect();
                ui.ctx().data_mut(|d| d.insert_temp(demo_id(), rects));
            }
            for (i, (entry, response)) in list.iter().zip(&rows).enumerate() {
                let Some(it) = inv.items.get(&entry.item) else {
                    continue;
                };
                let picture = picture(inv, entry);
                if let Some(image) = picture {
                    st.wanted_images.insert(image);
                }
                let texture = picture.and_then(|image| images.get(&image));
                let is_worn = worn.contains(&it.id);
                let worn_point = points.get(&it.id).copied();
                let detail = detail(world, it, worn_point);
                let point = point(world, it, worn_point);
                let at = layout.rect(i, response.rect);
                if layout.lifted == Some(i) {
                    reorder::lifted_card(ui, p, id, at, |card| {
                        paint_row(card, p, at, it, texture, is_worn, point.as_deref(), None);
                    });
                    continue;
                }
                let selected = st.favorites.selection.contains(it.id);
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
                    .then(|| items::wear_toggle(world, it, is_worn))
                    .flatten();
                paint_row(ui, p, at, it, texture, is_worn && toggle.is_none(), point.as_deref(), face);
                if response.clicked() || response.secondary_clicked() {
                    let modifiers = ui.input(|i| i.modifiers);
                    st.favorites.selection.click(it.id, order, modifiers, response.secondary_clicked());
                }
                // onDoubleClick: wear or take off, as the outfit lists do
                let asked = actions.len();
                items::double_click(response, world, it, is_worn, actions);
                // the same from a button on the hovered row, for those who
                // would not guess the double-click (Aurora's own)
                if let Some(action) = toggle
                    && wear_button(ui, p, at, it, is_worn)
                {
                    actions.push(action);
                }
                if actions.len() > asked {
                    let now = ui.input(|i| i.time);
                    st.favorites.status = Some((asked_status(it, is_worn), now));
                }
                let response = if quiet {
                    response.clone()
                } else {
                    response
                        .clone()
                        .on_hover_ui(|ui| tooltip(ui, p, inv, it, texture, is_worn, &detail))
                };
                row_menu(&response, p, world, st, root, tabs, it, order, actions);
            }
        });
    // the drop target lights up while it would take the cargo
    if !carried.is_empty() {
        ui.painter().rect_stroke(
            area,
            ui.visuals().window_corner_radius,
            egui::Stroke::new(1.5, p.violet),
            egui::StrokeKind::Inside,
        );
        crate::ui::inventory::drag::feedback(ui.ctx(), egui::CursorIcon::Copy);
        if ui.input(|i| i.pointer.any_released()) {
            // Not `dnd_release_payload`: it takes the payload before its type
            // is known (see `marketplace_listings::released`).
            egui::DragAndDrop::clear_payload(ui.ctx());
            if let Some(change) = model::add(inv, target, &carried) {
                actions.push(Action::Folders(change));
                // in Custom order, where it was let go
                if let Some(at) = gap {
                    st.favorites.reordered.push((target, model::inserted(order, at, &carried)));
                }
            }
        }
    }
    // Delete: Remove, as the Legacy remove_btn
    let selected = st.favorites.selection.ordered(order);
    if !selected.is_empty()
        && !st.favorites.busy
        && ui.ctx().memory(|m| m.focused().is_none())
        && ui.input(|i| i.key_pressed(egui::Key::Delete))
        && ui.ui_contains_pointer()
        && let Some(change) = model::remove(inv, target, &selected)
    {
        actions.push(Action::Folders(change));
    }
}

fn empty(ui: &mut egui::Ui, p: &Palette, inv: &crate::world::inventory::Inventory, target: Uuid, total: usize, filter: &str) {
    ui.add_space(28.0);
    ui.vertical_centered(|ui| {
        if total > 0 && !filter.is_empty() {
            ui.label(egui::RichText::new(aurora_i18n::tr!("ui-wearable-favorites-no-match", filter = filter.to_string())).color(p.muted));
            return;
        }
        if !model::loaded(inv, target) {
            ui.label(egui::RichText::new(aurora_i18n::tr!("common-loading-2")).color(p.muted));
            return;
        }
        // a subfolder says how to fill it, All how to start
        let folder = model::root(inv).is_some_and(|root| root != target);
        let (icon, title, hint) = if folder {
            (
                "folder-filled",
                aurora_i18n::tr!("ui-wearable-favorites-empty-folder"),
                aurora_i18n::tr!("ui-wearable-favorites-empty-folder-hint"),
            )
        } else {
            (
                "hanger-2-filled",
                aurora_i18n::tr!("ui-wearable-favorites-empty"),
                aurora_i18n::tr!("ui-wearable-favorites-empty-hint"),
            )
        };
        if let Some(t) = icons::global(icon) {
            ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(34.0)).tint(p.muted_dim));
        }
        ui.add_space(6.0);
        ui.label(egui::RichText::new(title).color(p.ink).size(13.5));
        ui.add_space(2.0);
        ui.add(egui::Label::new(egui::RichText::new(hint).color(p.muted).size(11.5)).wrap());
    });
}

/// The picture of a favorite: the wearable's own inventory picture, else
/// the link's (LLViewerInventoryItem::getThumbnailUUID), none while RLVa
/// forbids previews (as the inventory's picture tooltips).
fn picture(inv: &crate::world::inventory::Inventory, entry: &Entry) -> Option<Uuid> {
    if !inv.can_preview(0) {
        return None;
    }
    [entry.item, entry.link]
        .into_iter()
        .filter_map(|id| inv.items.get(&id))
        .map(|it| it.thumbnail)
        .find(|id| !id.is_nil())
}

/// "Shirt", "Object · Left Hand": the tooltip's line, and what the search
/// reads besides the name.
fn detail(world: &World, it: &InvItem, worn_point: Option<u8>) -> String {
    let kind: String = match it.asset_type {
        5 | 13 => crate::world::appearance::wearables()
            .get((it.flags & 0xff) as usize)
            .map(|name| name.to_string())
            .unwrap_or_default(),
        _ => aurora_i18n::tr!("common-object").into(),
    };
    match point(world, it, worn_point) {
        Some(point) => format!("{kind} · {point}"),
        None => kind,
    }
}

/// "Left Hand", "HUD : Top Right": the point an object is worn on, else
/// its saved point; none for clothing, body parts and an object that was
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
        // the second click of a double-click asks nothing more: the worn
        // state has not changed yet, so it would ask the same again
        button.clicked() && !button.double_clicked()
    })
    .inner
}

const WEAR_BUTTON: f32 = 28.0;
/// The folder button's height and the square buttons beside it and the
/// search field (until the field's own height is measured).
const BUTTON: f32 = 28.0;
/// How strongly a worn row is washed in violet (a selected row is 0.25).
const WORN_WASH: f32 = 0.07;

/// "Wearing Boots", "Taking off Boots": the status line after a button or
/// a double-click.
fn asked_status(it: &InvItem, worn: bool) -> String {
    let name = it.name.clone();
    if worn {
        aurora_i18n::tr!("ui-wearable-favorites-taking-off", name = name).to_string()
    } else {
        aurora_i18n::tr!("ui-wearable-favorites-wearing", name = name).to_string()
    }
}

/// One picture row at `rect`: its face (selected, hovered, worn, none),
/// the picture (or the type icon), the name, the attachment point below it
/// when there is one, and a check at the right end when `check` (a worn
/// row, unless its Wear button is shown there).
#[allow(clippy::too_many_arguments)]
fn paint_row(
    ui: &egui::Ui,
    p: &Palette,
    rect: Rect,
    it: &InvItem,
    texture: Option<&egui::TextureHandle>,
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
        let uv = crop_square(t.size_vec2());
        egui::Image::new(t)
            .uv(uv)
            .corner_radius(radius)
            .tint(widgets::PICTURE_TINT)
            .paint_at(ui, pic);
    } else if let Some(t) = icons::global(crate::ui::inventory::item_icon(it.asset_type, it.inv_type, it.flags)) {
        let tint = crate::ui::inventory::item_icon_color(it.asset_type, it.inv_type, p);
        egui::Image::new(&t)
            .tint(tint)
            .paint_at(ui, Rect::from_center_size(pic.center(), Vec2::splat(22.0)));
    }
    let left = pic.right() + 10.0;
    let width = (rect.right() - left - WEAR_BUTTON - 10.0).max(20.0);
    let name_font = FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx()));
    let name = one_line(ui, &it.name, name_font, p.ink, width);
    let sub = point.map(|point| one_line(ui, point, FontId::proportional(11.0), p.muted, width));
    let height = name.size().y + sub.as_ref().map_or(0.0, |sub| sub.size().y + 2.0);
    let top = rect.center().y - height / 2.0;
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

fn one_line(ui: &egui::Ui, text: &str, font: FontId, color: egui::Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(font, color));
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
    inv: &crate::world::inventory::Inventory,
    it: &InvItem,
    texture: Option<&egui::TextureHandle>,
    worn: bool,
    detail: &str,
) {
    ui.set_max_width(TIP_WIDTH);
    if let Some(t) = texture {
        let side = TIP_PICTURE;
        egui::Image::new(t)
            .uv(crop_square(t.size_vec2()))
            .corner_radius(ui.visuals().window_corner_radius)
            .fit_to_exact_size(Vec2::splat(side))
            .ui(ui);
        ui.add_space(4.0);
    }
    ui.add(egui::Label::new(egui::RichText::new(&it.name).color(p.ink).strong()).wrap());
    ui.label(egui::RichText::new(detail).color(p.muted));
    if worn {
        ui.label(egui::RichText::new(aurora_i18n::tr!("common-worn")).color(p.violet));
    }
    widgets::divider(ui, p);
    let permissions = crate::ui::inventory::permissions::Permissions::new(actions::held(inv, it), it.next_owner_mask);
    ui.label(egui::RichText::new(permissions.tooltip()).color(p.muted).size(11.0));
}

/// The wearable lists' menu (Wear, Add, Attach to, Take off, Touch,
/// Edit, Show original…), then Assign to folder and Remove
/// (menu_wearable_list_item.xml's favorites_assign_folder, remove_btn).
#[allow(clippy::too_many_arguments)]
fn row_menu(
    response: &egui::Response,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    root: Uuid,
    tabs: &[(Uuid, String)],
    it: &InvItem,
    order: &[Uuid],
    actions: &mut Vec<Action>,
) {
    let inv = &world.inventory;
    let busy = st.favorites.busy;
    let selected = if st.favorites.selection.contains(it.id) {
        st.favorites.selection.ordered(order)
    } else {
        vec![it.id]
    };
    let shown = st.favorites.shown.unwrap_or(root);
    let mut tail = |ui: &mut egui::Ui, actions: &mut Vec<Action>| {
        menu::separator(ui, p);
        // populate_assign_to_folder_menu: every tab, the items it lacks linked there
        menu::submenu(
            ui,
            p,
            "folder-filled",
            aurora_i18n::tr!("ui-wearable-favorites-assign"),
            !tabs.is_empty() && !busy,
            |ui| {
                for (id, name) in tabs {
                    let add = model::addable(inv, *id, &selected);
                    if menu::item_if(ui, p, "", name, !add.is_empty())
                        && let Some(change) = model::add(inv, *id, &add)
                    {
                        actions.push(Action::Folders(change));
                    }
                }
            },
        );
        if menu::item_if(ui, p, "heart-off", aurora_i18n::tr!("ui-wearable-favorites-remove"), !busy)
            && let Some(change) = model::remove(inv, shown, &selected)
        {
            actions.push(Action::Folders(change));
        }
    };
    items::context_menu(response, p, world, st, items::Tail::Custom(&mut tail), it, &selected, actions);
}

/// New folder, Rename and Delete (FSWearableFavoritesNewFolder,
/// FSWearableFavoritesRenameFolder, FSWearableFavoritesDeleteFolder).
fn dialog(ctx: &egui::Context, p: &Palette, world: &World, st: &mut AppearanceUi, root: Option<Uuid>, actions: &mut Vec<Action>) {
    let inv = &world.inventory;
    let Some(root) = root else {
        st.favorites.dialog = None;
        return;
    };
    let Some(dialog) = st.favorites.dialog.as_mut() else {
        return;
    };
    let mut done = false;
    let response = widgets::Modal::new(egui::Id::unique("wearable_favorites_dialog")).show(ctx, |ui| {
        ui.set_width(320.0);
        let title = match dialog {
            Dialog::New(_) => aurora_i18n::tr!("common-new-folder"),
            Dialog::Rename(..) => aurora_i18n::tr!("ui-wearable-favorites-rename-folder"),
            Dialog::Delete(_) => aurora_i18n::tr!("ui-wearable-favorites-delete-folder"),
        };
        ui.label(egui::RichText::new(title).size(17.0).color(p.ink));
        ui.add_space(8.0);
        let valid;
        match dialog {
            Dialog::New(name) | Dialog::Rename(_, name) => {
                let r = widgets::restricted_text_field(ui, true, name, widgets::TextPolicy::PrintableNoPipe, |b, _| {
                    egui::TextEdit::singleline(b).char_limit(63).desired_width(f32::INFINITY)
                });
                if st.favorites.focus {
                    r.request_focus();
                    st.favorites.focus = false;
                }
                let checked = model::folder_name(name);
                valid = checked.is_ok();
                // the slash is explained; an empty name only greys OK
                if let Err(reason) = checked
                    && !name.trim().is_empty()
                {
                    ui.label(egui::RichText::new(reason).color(p.danger).size(11.5));
                }
            }
            Dialog::Delete(id) => {
                let folder_name = inv.folders.get(id).map(|f| f.info.name.clone()).unwrap_or_default();
                let count = model::entries(inv, *id).len();
                ui.add(
                    egui::Label::new(aurora_i18n::tr!(
                        "ui-wearable-favorites-delete-confirm",
                        name = folder_name,
                        count = count
                    ))
                    .wrap(),
                );
                valid = model::delete_folder(inv, root, *id).is_some();
                if !valid {
                    // why: gone meanwhile, still being read, or holding more than links
                    let (why, color) = if !inv.folders.contains_key(id) {
                        (aurora_i18n::tr!("common-item-not-found"), p.danger)
                    } else if !model::loaded(inv, *id) {
                        (aurora_i18n::tr!("common-loading-2"), p.muted)
                    } else {
                        (aurora_i18n::tr!("ui-wearable-favorites-delete-refused"), p.danger)
                    };
                    ui.label(egui::RichText::new(why).color(color).size(11.5));
                }
            }
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            use widgets::{ActionKind, action_button, action_button_if};
            let size = Vec2::new(0.0, 26.0);
            let (icon, label, kind) = if matches!(dialog, Dialog::Delete(_)) {
                ("trash-filled", aurora_i18n::tr!("common-delete"), ActionKind::Danger)
            } else {
                ("check", aurora_i18n::tr!("common-ok"), ActionKind::Primary)
            };
            let enter = widgets::enter_pressed(ui);
            // another inventory change on its way: OK waits for it
            let valid = valid && !st.favorites.busy;
            if action_button_if(ui, p, icon, label, kind, size, valid).clicked() || (valid && enter) {
                let change = match dialog {
                    Dialog::New(name) => model::new_folder(root, name).map(|(id, change)| {
                        // onNewFolderCallback shows the new folder, once
                        // the inventory lists it
                        st.favorites.select_new = Some(id);
                        change
                    }),
                    Dialog::Rename(id, name) => model::rename_folder(inv, root, *id, name),
                    Dialog::Delete(id) => {
                        model::delete_folder(inv, root, *id).ok_or_else(|| aurora_i18n::tr!("ui-wearable-favorites-delete-refused").into())
                    }
                };
                match change {
                    Ok(change) => {
                        st.favorites.message.clear();
                        actions.push(Action::Folders(change));
                    }
                    Err(reason) => st.favorites.message = reason,
                }
                done = true;
            }
            if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                done = true;
            }
        });
    });
    if done || response.should_close() {
        st.favorites.dialog = None;
    }
}

/// `AURORA_DEMO_WEARABLE_FAVORITES=<mode>`, read once.
pub fn demo_mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_WEARABLE_FAVORITES").ok()).as_deref()
}

/// The demo's favorites over `appearance::seed_demo`'s wardrobe: a long
/// name, pictures of several shapes (and items with none, which show
/// their type icon), seven folders (one empty). `new` leaves the folder to
/// the window, `empty` makes it with nothing in it, `tab` opens Date night.
pub fn demo_seed(
    ctx: &egui::Context,
    inv: &mut crate::world::inventory::Inventory,
    agent: Uuid,
    images: &mut HashMap<Uuid, egui::TextureHandle>,
    st: &mut AppearanceUi,
    mode: &str,
) {
    use crate::world::inventory::actions::demo_mutate;
    // The demo server answers the root with its own folders only (version
    // 2): held as read at a newer version, the root keeps the folders made
    // here, as a grid's answer would list them.
    if let Some(root) = inv.folders.get_mut(&inv.root) {
        root.state = crate::world::inventory::FetchState::Fetched;
        root.info.version = root.info.version.max(3);
    }
    if mode == "new" {
        return;
    }
    let Some(change) = model::create(inv) else {
        return;
    };
    if let Err(e) = demo_mutate(inv, agent, change) {
        log::warn!("demo favorites: {e}");
        return;
    }
    let Some(root) = model::root(inv) else {
        return;
    };
    if mode == "empty" {
        return;
    }
    let gown = Uuid::from_u128(790);
    inv.add_items(vec![InvItem {
        id: gown,
        parent: inv.root,
        name: "Velvet evening gown with layered tulle skirt (rigged mesh, all bodies)".into(),
        asset_type: 6,
        inv_type: 6,
        flags: 0,
        creator: agent,
        owner: agent,
        base_mask: 0x7fff_ffff,
        owner_mask: 0x0000_e000,
        next_owner_mask: 0x0000_8000,
        ..Default::default()
    }]);
    let p = crate::theme::Theme::default().palette();
    for (item, size, color) in [
        (715u128, [256, 256], p.amber),
        (716, [400, 220], p.teal),
        (717, [200, 360], p.rose),
        (790, [256, 256], p.violet),
        (720, [256, 256], p.indigo),
    ] {
        let id = Uuid::from_u128(81_000 + item);
        let mut image = egui::ColorImage::filled(size, p.field);
        let centre = egui::vec2(size[0] as f32, size[1] as f32) * 0.5;
        let radius = centre.min_elem() * 0.62;
        for y in 0..size[1] {
            for x in 0..size[0] {
                let point = egui::vec2(x as f32, y as f32);
                image[(x, y)] = if (point - centre).length_sq() < radius * radius {
                    color
                } else if (x + y) % 48 < 3 {
                    p.raised
                } else {
                    p.field
                };
            }
        }
        if let Some(it) = inv.items.get_mut(&Uuid::from_u128(item)) {
            it.thumbnail = id;
        }
        images.insert(
            id,
            ctx.load_texture(format!("demo-favorite-{item}"), image, egui::TextureOptions::LINEAR),
        );
    }
    let link = |n: u128| Uuid::from_u128(n);
    let apply = |inv: &mut crate::world::inventory::Inventory, change: Option<aurora_net::inventory::operations::Mutation>| {
        if let Some(change) = change
            && let Err(e) = demo_mutate(inv, agent, change)
        {
            log::warn!("demo favorites: {e}");
        }
    };
    let all = [link(715), link(716), link(717), link(790), link(714), link(721), link(710)];
    let change = model::add(inv, root, &all);
    apply(inv, change);
    for (name, members) in [
        ("Date night", [link(790), link(718), link(720)].as_slice()),
        ("Work", &[link(715), link(716)]),
        ("Beach", &[link(714)]),
        ("Club", &[link(715), link(790), link(721)]),
        ("Gym", &[link(710)]),
        ("Roleplay - medieval court", &[link(790), link(717)]),
        ("Spring", &[]),
    ] {
        let Ok((id, change)) = model::new_folder(root, name) else {
            continue;
        };
        apply(inv, Some(change));
        let change = model::add(inv, id, members);
        apply(inv, change);
        if name == "Date night" && mode == "tab" {
            st.favorites.shown = Some(id);
        }
    }
    // the long name on screen, for its tooltip
    if mode == "tooltip" {
        st.favorites.filter = "gown".into();
    }
    // distinct dates for "most recent first"
    for (n, id) in model::entries(inv, root).iter().map(|e| e.link).enumerate() {
        if let Some(it) = inv.items.get_mut(&id) {
            it.created_at = 1_700_000_000 + n as i64 * 3_600;
        }
    }
}

fn demo_id() -> egui::Id {
    egui::Id::unique("wearable_favorites_demo_rows")
}

fn demo_picker_id() -> egui::Id {
    egui::Id::unique("wearable_favorites_demo_picker")
}

/// The demo's pointer: `tooltip` rests on the long name's row at frame
/// 520, for its tooltip; `drag` picks up the second row at frame 500 and
/// carries it two rows down, held there for the capture; `drop` lets it go
/// at frame 560, so that the capture shows it landed in Custom order;
/// `menu` opens the folder menu at frame 540; `hover` rests on the second
/// row's Wear button at frame 520.
pub fn demo_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    let Some(mode) = demo_mode() else {
        return Vec::new();
    };
    let rects: Vec<Rect> = ctx.data(|d| d.get_temp(demo_id())).unwrap_or_default();
    let button = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    match mode {
        // moved once and left there: egui shows a tooltip under a still pointer
        "tooltip" if (520..=521).contains(&frame) => rects
            .first()
            .map(|rect| vec![egui::Event::PointerMoved(egui::pos2(rect.left() + 60.0, rect.center().y))])
            .unwrap_or_default(),
        "menu" => {
            let Some(rect) = ctx.data(|d| d.get_temp::<Rect>(demo_picker_id())) else {
                return Vec::new();
            };
            match frame {
                539 => vec![egui::Event::PointerMoved(rect.center())],
                540 => vec![button(rect.center(), true)],
                541 => vec![button(rect.center(), false)],
                _ => Vec::new(),
            }
        }
        "hover" if (520..=521).contains(&frame) => rects
            .get(1)
            .map(|rect| {
                vec![egui::Event::PointerMoved(egui::pos2(
                    rect.right() - 6.0 - WEAR_BUTTON / 2.0,
                    rect.center().y,
                ))]
            })
            .unwrap_or_default(),
        "drag" | "drop" => {
            let Some(rect) = rects.get(1) else {
                return Vec::new();
            };
            let start = egui::pos2(rect.left() + 60.0, rect.center().y);
            // two rows down, 3 points a frame, as a hand moves
            let at = |f: u64| start + Vec2::new(0.0, (f.saturating_sub(502) as f32 * 3.0).min(2.0 * (ROW + 2.0)));
            match frame {
                500 => vec![egui::Event::PointerMoved(start)],
                501 => vec![button(start, true)],
                502..=559 => vec![egui::Event::PointerMoved(at(frame))],
                560 if mode == "drop" => vec![button(at(frame), false)],
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}
