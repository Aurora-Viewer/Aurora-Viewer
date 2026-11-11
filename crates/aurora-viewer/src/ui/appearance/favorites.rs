//! The Favorite Wearables window: port of FSFloaterWearableFavorites
//! (indra/newview/fsfloaterwearablefavorites.cpp, originally LGPL 2.1).
//! What it lists and changes is in `world::wearable_favorites`; this module
//! draws it and turns the gestures into Appearance actions.
//!
//! Drawn as picture rows instead of the Legacy name list (human decision,
//! 2026-11-11): the item's own inventory picture, or its type icon, then its
//! name and "type · point" on one line each, a full tooltip for long names.
//! The subfolders are tabs instead of a combo box with three buttons.

use super::{Action, AppearanceUi, items, selection::Selection};
use crate::{
    theme::Palette,
    ui::{icons, menu, widgets},
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
const PICTURE: f32 = 36.0;
const ROW: f32 = 46.0;
const TIP_PICTURE: f32 = 168.0;
const TIP_WIDTH: f32 = 240.0;

#[derive(Default)]
pub(crate) struct State {
    /// The tab shown: a subfolder, or None for every favorite.
    shown: Option<Uuid>,
    filter: String,
    selection: Selection,
    dialog: Option<Dialog>,
    focus: bool,
    /// The inventory root the folder was asked for, so that it is asked
    /// once per account (initCategory) and again after a relog.
    requested: Option<Uuid>,
    message: String,
}

enum Dialog {
    New(String),
    Rename(Uuid, String),
    Delete(Uuid),
}

/// The window, while `open`. `sort` is the remembered order
/// (`FSWearableFavoritesSortOrder`).
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    open: &mut bool,
    images: &HashMap<Uuid, egui::TextureHandle>,
    sort: &mut Sort,
) -> Vec<Action> {
    let mut actions = Vec::new();
    if !*open {
        return actions;
    }
    let inv = &world.inventory;
    let root = model::root(inv);
    // initCategory: made the first time the window opens.
    if root.is_none()
        && st.favorites.requested != Some(inv.root)
        && let Some(change) = model::create(inv)
    {
        st.favorites.requested = Some(inv.root);
        actions.push(Action::Folders(change));
    }
    let tabs = root.map(|root| model::folders(inv, root)).unwrap_or_default();
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
    model::sort(inv, &mut list, *sort);
    let total = list.len();
    let all = match (root, st.favorites.shown) {
        (Some(root), Some(_)) => model::entries(inv, root).len(),
        _ => total,
    };
    let filter = st.favorites.filter.trim().to_string();
    list.retain(|e| model::matches(inv, e, &filter));
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
        tab_row(ui, p, st, &tabs, all);
        ui.add_space(6.0);
        if !st.favorites.message.is_empty() {
            ui.label(egui::RichText::new(&st.favorites.message).color(p.danger).size(11.5));
        }
        list_area(ui, p, world, st, images, root, &list, &order, total, &filter, &mut actions);
    });
    dialog(ctx, p, world, st, root, &mut actions);
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
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 2.0 * 30.0).max(80.0);
        widgets::search_field(
            ui,
            p,
            &mut st.favorites.filter,
            aurora_i18n::tr!("ui-wearable-favorites-search"),
            width,
        );
        widgets::icon_menu(ui, p, "arrows-sort", aurora_i18n::tr!("ui-wearable-favorites-sort"), |ui| {
            for (value, label) in [
                (Sort::Name, aurora_i18n::tr!("ui-wearable-favorites-sort-name")),
                (Sort::Recent, aurora_i18n::tr!("ui-wearable-favorites-sort-recent")),
                (Sort::Type, aurora_i18n::tr!("ui-wearable-favorites-sort-type")),
            ] {
                if menu::check(ui, p, "", label, *sort == value, true) {
                    *sort = value;
                }
            }
        });
        let selected = st.favorites.selection.ordered(order);
        let shown = st.favorites.shown.unwrap_or(root);
        if widgets::icon_button(
            ui,
            p,
            "trash-filled",
            aurora_i18n::tr!("ui-wearable-favorites-remove"),
            !selected.is_empty(),
        ) && let Some(change) = model::remove(&world.inventory, shown, &selected)
        {
            actions.push(Action::Folders(change));
        }
    });
}

/// All, the subfolders by name, and New folder; a subfolder's tab renames
/// or deletes it from its menu (rename_folder_btn / delete_folder_btn).
fn tab_row(ui: &mut egui::Ui, p: &Palette, st: &mut AppearanceUi, tabs: &[(Uuid, String)], all: usize) {
    egui::ScrollArea::horizontal()
        .id_salt("wearable_favorites_tabs")
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if widgets::pill_tab(
                    ui,
                    p,
                    "heart-filled",
                    aurora_i18n::tr!("common-all"),
                    (all > 0).then(|| all.to_string()),
                    false,
                    st.favorites.shown.is_none(),
                    true,
                )
                .clicked()
                {
                    st.favorites.shown = None;
                }
                for (id, name) in tabs {
                    let response = widgets::pill_tab(ui, p, "", name, None, false, st.favorites.shown == Some(*id), true);
                    if response.clicked() {
                        st.favorites.shown = Some(*id);
                    }
                    menu::context_menu(&response, p, |ui| {
                        if menu::item(ui, p, "pencil-filled", aurora_i18n::tr!("common-rename")) {
                            st.favorites.dialog = Some(Dialog::Rename(*id, name.clone()));
                            st.favorites.focus = true;
                        }
                        if menu::item(ui, p, "trash-filled", aurora_i18n::tr!("common-delete")) {
                            st.favorites.dialog = Some(Dialog::Delete(*id));
                        }
                    });
                }
                if widgets::icon_button(ui, p, "folder-plus", aurora_i18n::tr!("common-new-folder"), true) {
                    st.favorites.dialog = Some(Dialog::New(String::new()));
                    st.favorites.focus = true;
                }
            });
        });
}

/// The rows, the empty states, and the drop of wearables from the
/// inventory (FSWearableFavoritesItemsList::handleDragAndDrop).
#[allow(clippy::too_many_arguments)]
fn list_area(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    images: &HashMap<Uuid, egui::TextureHandle>,
    root: Uuid,
    list: &[Entry],
    order: &[Uuid],
    total: usize,
    filter: &str,
    actions: &mut Vec<Action>,
) {
    let inv = &world.inventory;
    let target = st.favorites.shown.unwrap_or(root);
    let area = Rect::from_min_size(ui.cursor().min, ui.available_size());
    let carried = (ui.rect_contains_pointer(area))
        .then(|| crate::ui::inventory::drag::carried_ids(ui.ctx()))
        .flatten()
        .map(|ids| model::addable(inv, target, &ids))
        .unwrap_or_default();
    let worn = items::worn_items(world);
    let points = world.worn_attachment_points();
    egui::ScrollArea::vertical()
        .id_salt("wearable_favorites_rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if list.is_empty() {
                empty(ui, p, inv, target, total, filter);
            }
            for entry in list {
                let Some(it) = inv.items.get(&entry.item) else {
                    continue;
                };
                let picture = picture(inv, entry);
                if let Some(image) = picture {
                    st.wanted_images.insert(image);
                }
                let texture = picture.and_then(|image| images.get(&image));
                let is_worn = worn.contains(&it.id);
                let detail = detail(world, it, points.get(&it.id).copied());
                let selected = st.favorites.selection.contains(it.id);
                let response = row(ui, p, it, texture, is_worn, &detail, selected);
                if demo_mode().is_some() && it.id == Uuid::from_u128(790) {
                    ui.ctx().data_mut(|d| d.insert_temp(demo_id(), response.rect));
                }
                if response.clicked() || response.secondary_clicked() {
                    let modifiers = ui.input(|i| i.modifiers);
                    st.favorites.selection.click(it.id, order, modifiers, response.secondary_clicked());
                }
                // onDoubleClick: wear or take off, as the outfit lists do
                items::double_click(&response, world, it, is_worn, actions);
                let response = response.on_hover_ui(|ui| tooltip(ui, p, inv, it, texture, is_worn, &detail));
                row_menu(&response, p, world, st, root, it, order, actions);
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
            }
        }
    }
    // Delete: Remove, as the Legacy remove_btn
    let selected = st.favorites.selection.ordered(order);
    if !selected.is_empty()
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
        if let Some(t) = icons::global("hanger-2-filled") {
            ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(34.0)).tint(p.muted_dim));
        }
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("ui-wearable-favorites-empty"))
                .color(p.ink)
                .size(13.5),
        );
        ui.add_space(2.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(aurora_i18n::tr!("ui-wearable-favorites-empty-hint"))
                    .color(p.muted)
                    .size(11.5),
            )
            .wrap(),
        );
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

/// "Shirt", "Object · Left Hand" (where it is worn, else its saved point).
fn detail(world: &World, it: &InvItem, worn_point: Option<u8>) -> String {
    let kind: String = match it.asset_type {
        5 | 13 => crate::world::appearance::wearables()
            .get((it.flags & 0xff) as usize)
            .map(|name| name.to_string())
            .unwrap_or_default(),
        _ => aurora_i18n::tr!("common-object").into(),
    };
    if it.asset_type != 6 {
        return kind;
    }
    let point = worn_point.or(Some((it.flags & 0xff) as u8).filter(|p| *p != 0));
    let Some(point) = point else {
        return kind;
    };
    let name = world
        .avatar_lib
        .attach_names
        .get(&point)
        .map(|name| items::point_name(name).to_owned())
        .unwrap_or_else(|| aurora_i18n::tr!("attachment-point", point = point));
    let hud = world.avatar_lib.attach_points.get(&point).is_some_and(|ap| ap.hud);
    format!("{kind} · {}{name}", if hud { "HUD : " } else { "" })
}

/// One picture row: the picture (or the type icon), the name in bold when
/// worn, the detail below, a check at the end when worn.
fn row(
    ui: &mut egui::Ui,
    p: &Palette,
    it: &InvItem,
    texture: Option<&egui::TextureHandle>,
    worn: bool,
    detail: &str,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click());
    let radius = ui.visuals().window_corner_radius;
    if selected || response.hovered() {
        ui.painter()
            .rect_filled(rect, radius, if selected { p.violet.gamma_multiply(0.25) } else { p.raised });
    }
    let pic = Rect::from_center_size(egui::pos2(rect.left() + 5.0 + PICTURE / 2.0, rect.center().y), Vec2::splat(PICTURE));
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
            .paint_at(ui, Rect::from_center_size(pic.center(), Vec2::splat(20.0)));
    }
    let check = 22.0;
    let left = pic.right() + 9.0;
    let width = (rect.right() - left - if worn { check } else { 6.0 }).max(20.0);
    let name_font = FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx()));
    let name = one_line(ui, &it.name, name_font, p.ink, width);
    let sub = one_line(ui, detail, FontId::proportional(11.0), p.muted, width);
    let top = rect.center().y - (name.size().y + sub.size().y + 1.0) / 2.0;
    ui.painter().galley(egui::pos2(left, top), name.clone(), p.ink);
    ui.painter().galley(egui::pos2(left, top + name.size().y + 1.0), sub, p.muted);
    if worn && let Some(t) = icons::global("circle-check-filled") {
        egui::Image::new(&t).tint(p.violet).paint_at(
            ui,
            Rect::from_center_size(egui::pos2(rect.right() - check / 2.0 - 2.0, rect.center().y), Vec2::splat(15.0)),
        );
    }
    response
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
    it: &InvItem,
    order: &[Uuid],
    actions: &mut Vec<Action>,
) {
    let inv = &world.inventory;
    let selected = if st.favorites.selection.contains(it.id) {
        st.favorites.selection.ordered(order)
    } else {
        vec![it.id]
    };
    let shown = st.favorites.shown.unwrap_or(root);
    let tabs = model::folders(inv, root);
    let mut tail = |ui: &mut egui::Ui, actions: &mut Vec<Action>| {
        menu::separator(ui, p);
        // populate_assign_to_folder_menu: every tab, the items it lacks linked there
        menu::submenu(
            ui,
            p,
            "folder-filled",
            aurora_i18n::tr!("ui-wearable-favorites-assign"),
            !tabs.is_empty(),
            |ui| {
                for (id, name) in &tabs {
                    let add = model::addable(inv, *id, &selected);
                    if menu::item_if(ui, p, "", name, !add.is_empty())
                        && let Some(change) = model::add(inv, *id, &add)
                    {
                        actions.push(Action::Folders(change));
                    }
                }
            },
        );
        if menu::item(ui, p, "heart-off", aurora_i18n::tr!("ui-wearable-favorites-remove"))
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
                    ui.label(
                        egui::RichText::new(aurora_i18n::tr!("ui-wearable-favorites-delete-refused"))
                            .color(p.danger)
                            .size(11.5),
                    );
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
            if action_button_if(ui, p, icon, label, kind, size, valid).clicked() || (valid && enter) {
                let change = match dialog {
                    Dialog::New(name) => model::new_folder(root, name).map(|(id, change)| {
                        // onNewFolderCallback shows the new folder
                        st.favorites.shown = Some(id);
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
/// their type icon), two tabs. `new` leaves the folder to the window,
/// `empty` makes it with nothing in it, `tab` opens Date night.
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
    egui::Id::unique("wearable_favorites_demo_row")
}

/// `AURORA_DEMO_WEARABLE_FAVORITES=tooltip`: the pointer rests on the long
/// name's row at frame 520, for its tooltip.
pub fn demo_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    // moved once and left there: egui shows a tooltip under a still pointer
    if !(520..=521).contains(&frame) || demo_mode() != Some("tooltip") {
        return Vec::new();
    }
    ctx.data(|d| d.get_temp::<Rect>(demo_id()))
        .map(|rect| vec![egui::Event::PointerMoved(egui::pos2(rect.left() + 60.0, rect.center().y))])
        .unwrap_or_default()
}
