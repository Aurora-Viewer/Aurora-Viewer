//! "Favorite wearables": port of FSFloaterWearableFavorites (indra/newview/
//! fsfloaterwearablefavorites.cpp, floater_fs_wearable_favorites.xml,
//! menu_fs_wearable_favorites.xml, originally LGPL 2.1): the folder picker
//! with New / Rename / Delete, a filter, the sort menu and Remove over the
//! list; a double click wears or takes off (RLVa applies), and an inventory
//! item dropped on the window becomes a favorite of the folder shown. The
//! rows' menu is LLWearableItemsList::ContextMenu with Assign to Folder.
//!
//! Aurora addition: "Sync from Legacy Viewer" under the list, shown while the
//! inventory holds a Legacy folder. The rules are
//! `world/wearable_favorites.rs`.

use super::appearance::items;
use super::widgets::{self, ActionKind, Floater};
use super::{inventory, menu};
use crate::theme::Palette;
use crate::world::World;
use crate::world::appearance::Action as Wear;
use crate::world::wearable_favorites::{self as favorites, Entry, Sort};
use aurora_net::inventory::InvItem;
use aurora_net::inventory::operations::Mutation;
use egui::{FontId, RichText, Sense, Vec2};
use std::collections::HashSet;
use uuid::Uuid;

/// The window's id (Floater.Toggle "fs_wearable_favorites").
pub const WINDOW: &str = "wearable_favorites";

const ROW_H: f32 = 24.0;
const BUTTON_H: f32 = 26.0;

/// What the window asks the app for.
#[derive(Debug)]
pub enum FavoritesAction {
    Wear(Wear),
    Edit {
        change: Mutation,
        /// The folder to show once it exists (its id in `change`).
        select: Option<Uuid>,
        /// The links a sync adds, to say so afterwards.
        synced: Option<usize>,
    },
    Sort(Sort),
}

enum Dialog {
    New { name: String },
    Rename { id: Uuid, name: String },
    Delete(Uuid),
}

/// An inventory change on its way.
#[derive(Debug, Clone)]
pub struct Pending {
    pub request: Uuid,
    pub select: Option<Uuid>,
    pub synced: Option<usize>,
    /// The folders to read again if the change stops half way.
    pub refresh: Vec<Uuid>,
}

#[derive(Default)]
pub struct WearableFavoritesUi {
    filter: String,
    /// The subfolder shown (None: All favorites).
    folder: Option<Uuid>,
    /// The rows selected, by original.
    selected: HashSet<Uuid>,
    anchor: Option<Uuid>,
    dialog: Option<Dialog>,
    focus: bool,
    pub message: String,
    /// A wear asked from the list, until the outfit answers.
    pub wearing: bool,
    pub pending: Option<Pending>,
    /// The originals already asked for, once each.
    pub requested: HashSet<Uuid>,
    /// The rows of the folder shown, until the inventory changes.
    rows: Option<(u64, Uuid, Sort, Vec<Entry>)>,
}

/// The filter matches the name (setFilterSubString).
fn matches(filter: &str, entry: &Entry) -> bool {
    let filter = filter.trim().to_lowercase();
    filter.is_empty() || entry.name.to_lowercase().contains(&filter)
}

impl WearableFavoritesUi {
    /// The folder shown, back to the root once it is gone.
    pub fn show_folder(&mut self, folder: Option<Uuid>) {
        self.folder = folder;
        self.selected.clear();
        self.anchor = None;
    }

    fn click(&mut self, shown: &[Entry], id: Uuid, modifiers: egui::Modifiers) {
        if modifiers.command {
            if !self.selected.remove(&id) {
                self.selected.insert(id);
            }
        } else if let Some((from, to)) = modifiers.shift.then_some(self.anchor).flatten().and_then(|anchor| {
            Some((
                shown.iter().position(|e| e.item == anchor)?,
                shown.iter().position(|e| e.item == id)?,
            ))
        }) {
            self.selected = shown[from.min(to)..=from.max(to)].iter().map(|e| e.item).collect();
            return;
        } else {
            self.selected = HashSet::from([id]);
        }
        self.anchor = Some(id);
    }

    fn rows(&mut self, world: &World, folder: Uuid, root: Uuid, sort: Sort) -> &[Entry] {
        let generation = world.inventory.generation;
        let stale = !matches!(&self.rows, Some((g, f, s, _)) if *g == generation && *f == folder && *s == sort);
        if stale {
            let mut rows = favorites::entries(&world.inventory, folder, root);
            favorites::sort(&mut rows, sort);
            self.rows = Some((generation, folder, sort, rows));
        }
        self.rows.as_ref().map_or(&[], |(.., rows)| rows)
    }

    fn folder_bar(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &World,
        root: Option<Uuid>,
        shown: Option<Uuid>,
        sort: Sort,
        out: &mut Vec<FavoritesAction>,
    ) {
        let subfolders = root.map(|root| favorites::folders(&world.inventory, root)).unwrap_or_default();
        let all = aurora_i18n::tr!("wearable-favorites-all");
        let name = subfolders
            .iter()
            .find(|(id, _)| Some(*id) == shown)
            .map_or(all, |(_, name)| name.as_str());
        let free = self.pending.is_none();
        ui.horizontal(|ui| {
            let mut pick = None;
            egui::ComboBox::from_id_salt("wearable_favorites_folder")
                .selected_text(name)
                .width((ui.available_width() - 3.0 * (BUTTON_H + 6.0)).max(80.0))
                .show_ui(ui, |ui| {
                    if ui.selectable_label(shown.is_none(), all).clicked() {
                        pick = Some(None);
                    }
                    for (id, name) in &subfolders {
                        if ui.selectable_label(shown == Some(*id), name).clicked() {
                            pick = Some(Some(*id));
                        }
                    }
                });
            if let Some(folder) = pick {
                self.show_folder(folder);
            }
            if widgets::icon_button(ui, p, "plus", aurora_i18n::tr!("wearable-favorites-new-folder"), free) {
                self.dialog = Some(Dialog::New { name: String::new() });
                self.focus = true;
            }
            if widgets::icon_button(
                ui,
                p,
                "pencil-filled",
                aurora_i18n::tr!("wearable-favorites-rename-folder"),
                free && shown.is_some(),
            ) && let Some(id) = shown
            {
                let name = world.inventory.folders.get(&id).map(|f| f.info.name.clone()).unwrap_or_default();
                self.dialog = Some(Dialog::Rename { id, name });
                self.focus = true;
            }
            if widgets::icon_button(
                ui,
                p,
                "trash-filled",
                aurora_i18n::tr!("wearable-favorites-delete-folder"),
                free && shown.is_some(),
            ) && let Some(id) = shown
            {
                self.dialog = Some(Dialog::Delete(id));
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let removing = !self.selected.is_empty() && free;
            let width = ui.available_width() - 2.0 * (BUTTON_H + 6.0);
            widgets::search_box(ui, p, &mut self.filter, aurora_i18n::tr!("wearable-favorites-filter"), width);
            let options = widgets::action_button(ui, p, "sort-ascending-letters", "", ActionKind::Flat, Vec2::splat(BUTTON_H))
                .on_hover_text(aurora_i18n::tr!("wearable-favorites-sort"));
            menu::dropdown(&options, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                for (order, icon, label) in [
                    (
                        Sort::Name,
                        "sort-ascending-letters",
                        aurora_i18n::tr!("wearable-favorites-sort-name"),
                    ),
                    (Sort::Recent, "clock-filled", aurora_i18n::tr!("wearable-favorites-sort-recent")),
                    (Sort::Type, "shirt-filled", aurora_i18n::tr!("wearable-favorites-sort-type")),
                ] {
                    if menu::check(ui, p, icon, label, sort == order, true) {
                        out.push(FavoritesAction::Sort(order));
                    }
                }
            });
            if widgets::icon_button(ui, p, "star-filled", aurora_i18n::tr!("wearable-favorites-remove"), removing)
                && let Some(root) = root
            {
                let change = favorites::remove(&world.inventory, shown.unwrap_or(root), root, &self.selected);
                out.push(FavoritesAction::Edit {
                    change,
                    select: None,
                    synced: None,
                });
                self.selected.clear();
            }
        });
    }

    /// The rows' menu, on `selected` (the row alone, or the selection it is
    /// in). As in the outfit lists; Assign to Folder is shown for favorites
    /// (the is_wearable_favorite of updateItemsVisibility).
    #[allow(clippy::too_many_arguments)]
    fn context_menu(
        &self,
        response: &egui::Response,
        p: &Palette,
        world: &World,
        root: Uuid,
        it: &InvItem,
        selected: &[Uuid],
        worn_set: &HashSet<Uuid>,
        out: &mut Vec<FavoritesAction>,
    ) {
        let mut wear = Vec::new();
        menu::context_menu(response, p, |ui| {
            let list: Vec<&InvItem> = selected.iter().filter_map(|id| world.inventory.items.get(id)).collect();
            let single = list.len() <= 1;
            let (worn, unworn): (Vec<&InvItem>, Vec<&InvItem>) = list.iter().partition(|it| worn_set.contains(&it.id));
            let locks = world.rlva_wear_locks();
            let is_worn = worn_set.contains(&it.id);
            if single && is_worn && matches!(it.asset_type, 5 | 13) && menu::item(ui, p, "pencil-filled", aurora_i18n::tr!("common-edit")) {
                super::context::request(ui.ctx(), super::context::CtxAction::EditWorn(it.id));
            }
            if !unworn.is_empty() {
                let ids: Vec<Uuid> = unworn.iter().map(|it| it.id).collect();
                let allowed = |id: &Uuid| locks.wear(&world.inventory, *id, 0);
                if menu::item_if(
                    ui,
                    p,
                    "shirt-filled",
                    aurora_i18n::tr!("common-wear"),
                    ids.iter().any(|id| allowed(id).replace),
                ) {
                    wear.push(items::wear_action(ids.clone(), true, 0));
                }
                let addable: Vec<Uuid> = unworn.iter().filter(|it| it.asset_type != 13).map(|it| it.id).collect();
                if !addable.is_empty()
                    && menu::item_if(
                        ui,
                        p,
                        "plus",
                        aurora_i18n::tr!("common-add"),
                        addable.iter().any(|id| allowed(id).add),
                    )
                {
                    wear.push(items::wear_action(addable, false, 0));
                }
                if unworn.iter().all(|it| it.asset_type == 6) {
                    items::attach_menu(ui, p, world, &locks, &ids, false, &mut wear);
                    items::attach_menu(ui, p, world, &locks, &ids, true, &mut wear);
                }
            }
            if single && is_worn && it.asset_type == 6 {
                let touch = world.touchable_attachment_items().contains(&it.id);
                if menu::item_if(ui, p, "click-filled", aurora_i18n::tr!("ui-context-touch"), touch) {
                    super::context::request(ui.ctx(), super::context::CtxAction::TouchWorn(it.id));
                }
                let edit = world.worn_attachment(&it.id).is_some();
                if menu::item_if(ui, p, "pencil-filled", aurora_i18n::tr!("common-edit"), edit) {
                    super::context::request(ui.ctx(), super::context::CtxAction::EditWorn(it.id));
                }
            }
            items::remove_item(ui, p, &worn, &locks, aurora_i18n::tr!("ui-appearance-items-remove"), &mut wear);
            menu::separator(ui, p);
            let subfolders = favorites::folders(&world.inventory, root);
            menu::submenu(
                ui,
                p,
                "folder-filled",
                aurora_i18n::tr!("wearable-favorites-assign"),
                !subfolders.is_empty() && self.pending.is_none(),
                |ui| {
                    egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                        for (folder, name) in &subfolders {
                            if menu::item(ui, p, "folder-filled", name) {
                                match favorites::add(&world.inventory, Some(*folder), selected) {
                                    Ok(change) => out.push(FavoritesAction::Edit {
                                        change,
                                        select: None,
                                        synced: None,
                                    }),
                                    Err(reason) => log::warn!("favorite not assigned: {reason}"),
                                }
                            }
                        }
                    });
                },
            );
            if menu::item_if(
                ui,
                p,
                "external-link-filled",
                aurora_i18n::tr!("ui-appearance-items-show-original"),
                single,
            ) {
                wear.push(Wear::ShowOriginal(it.id));
            }
        });
        out.extend(wear.into_iter().map(FavoritesAction::Wear));
    }

    fn list(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &World,
        root: Uuid,
        shown_folder: Option<Uuid>,
        sort: Sort,
        out: &mut Vec<FavoritesAction>,
    ) {
        let folder = shown_folder.unwrap_or(root);
        let filter = self.filter.clone();
        let shown: Vec<Entry> = self
            .rows(world, folder, root, sort)
            .iter()
            .filter(|e| matches(&filter, e))
            .cloned()
            .collect();
        if shown.is_empty() {
            let key = if filter.trim().is_empty() {
                aurora_i18n::tr!("wearable-favorites-empty")
            } else {
                aurora_i18n::tr!("wearable-favorites-no-match")
            };
            ui.add(egui::Label::new(RichText::new(key).size(12.0).color(p.muted)).wrap());
            return;
        }
        let labels = items::worn_labels(world);
        let worn_set = items::worn_items(world);
        let radius = ui.visuals().window_corner_radius;
        egui::ScrollArea::vertical()
            .id_salt("wearable_favorites_rows")
            .auto_shrink([false, false])
            .show_rows(ui, ROW_H, shown.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for entry in &shown[range] {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
                    if resp.clicked() {
                        self.click(&shown, entry.item, ui.input(|i| i.modifiers));
                    } else if resp.secondary_clicked() && !self.selected.contains(&entry.item) {
                        self.selected = HashSet::from([entry.item]);
                        self.anchor = Some(entry.item);
                    }
                    if self.selected.contains(&entry.item) {
                        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.30));
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.6));
                    }
                    let glyph = inventory::item_icon(entry.asset_type, entry.inv_type, entry.flags);
                    let color = inventory::item_icon_color(entry.asset_type, entry.inv_type, p);
                    if let Some(t) = super::icons::global(glyph) {
                        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        let at = rect.left_center() + Vec2::new(14.0, 0.0);
                        ui.painter()
                            .image(t.id(), egui::Rect::from_center_size(at, Vec2::splat(16.0)), uv, color);
                    }
                    let label = labels.get(&entry.item);
                    let right = label.map_or(0.0, |_| rect.width() * 0.4);
                    let font = if label.is_some() {
                        FontId::new(12.5, super::fonts::semibold(ui.ctx()))
                    } else {
                        FontId::proportional(12.5)
                    };
                    widgets::paint_elided(
                        ui,
                        &entry.name,
                        egui::pos2(rect.left() + 28.0, rect.center().y),
                        rect.width() - 36.0 - right,
                        font,
                        p.ink,
                    );
                    if let Some(label) = label {
                        widgets::paint_elided(
                            ui,
                            label,
                            egui::pos2(rect.right() - right, rect.center().y),
                            right - 8.0,
                            FontId::proportional(11.5),
                            p.muted,
                        );
                    }
                    let Some(it) = world.inventory.items.get(&entry.item) else {
                        continue;
                    };
                    if self.pending.is_none() {
                        let mut wear = Vec::new();
                        items::double_click(&resp, world, it, worn_set.contains(&entry.item), &mut wear);
                        out.extend(wear.into_iter().map(FavoritesAction::Wear));
                    }
                    let selected: Vec<Uuid> = if self.selected.contains(&entry.item) {
                        shown.iter().map(|e| e.item).filter(|id| self.selected.contains(id)).collect()
                    } else {
                        vec![entry.item]
                    };
                    self.context_menu(&resp, p, world, root, it, &selected, &worn_set, out);
                }
            });
    }

    fn dialog(&mut self, ctx: &egui::Context, p: &Palette, world: &World, out: &mut Vec<FavoritesAction>) {
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        let mut done = false;
        let mut message = None;
        let focus = &mut self.focus;
        let response = widgets::Modal::new(egui::Id::unique("wearable_favorites_dialog")).show(ctx, |ui| {
            ui.set_width(350.0);
            let (title, name) = match dialog {
                Dialog::New { name } => (aurora_i18n::tr!("wearable-favorites-new-folder"), Some(name)),
                Dialog::Rename { name, .. } => (aurora_i18n::tr!("wearable-favorites-rename-folder"), Some(name)),
                Dialog::Delete(_) => (aurora_i18n::tr!("wearable-favorites-delete-folder"), None),
            };
            ui.label(RichText::new(title).size(17.0).color(p.ink));
            ui.add_space(8.0);
            let mut valid = true;
            if let Some(name) = name {
                let r = widgets::restricted_text_field(ui, true, name, widgets::TextPolicy::PrintableNoPipe, |b, _| {
                    egui::TextEdit::singleline(b).char_limit(63).desired_width(f32::INFINITY)
                });
                if std::mem::take(focus) {
                    r.request_focus();
                }
                valid = favorites::valid_name(name).is_ok();
                if !valid
                    && !name.trim().is_empty()
                    && let Err(reason) = favorites::valid_name(name)
                {
                    ui.label(RichText::new(reason).size(12.0).color(p.danger));
                }
            } else if let Dialog::Delete(id) = dialog {
                let folder = world.inventory.folders.get(id);
                ui.label(aurora_i18n::tr!(
                    "wearable-favorites-delete-confirm",
                    name = folder.map(|f| f.info.name.clone()).unwrap_or_default(),
                    count = folder.map_or(0, |f| f.items.len())
                ));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let size = Vec2::new(0.0, BUTTON_H);
                let (icon, label, kind) = if matches!(dialog, Dialog::Delete(_)) {
                    ("trash-filled", aurora_i18n::tr!("common-delete"), ActionKind::Danger)
                } else {
                    ("check", aurora_i18n::tr!("common-ok"), ActionKind::Primary)
                };
                let enter = widgets::enter_pressed(ui);
                if widgets::action_button_if(ui, p, icon, label, kind, size, valid).clicked() || (valid && enter) {
                    let inv = &world.inventory;
                    let change = match dialog {
                        Dialog::New { name } => favorites::new_folder(inv, name).map(|(change, id)| (change, Some(id))),
                        Dialog::Rename { id, name } => favorites::rename(inv, *id, name).map(|change| (change, None)),
                        Dialog::Delete(id) => favorites::delete_folder(inv, *id).map(|change| (change, None)),
                    };
                    match change {
                        Ok((change, select)) => out.push(FavoritesAction::Edit {
                            change,
                            select,
                            synced: None,
                        }),
                        Err(reason) => message = Some(reason),
                    }
                    done = true;
                }
                if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    done = true;
                }
            });
        });
        if let Some(message) = message {
            self.message = message;
        }
        if done || response.should_close() {
            self.dialog = None;
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, out: &mut Vec<FavoritesAction>) {
        // above the button: a refusal is read whole, whatever the window's width
        if !self.message.is_empty() {
            ui.add(egui::Label::new(RichText::new(&self.message).size(12.0).color(p.muted)).wrap());
        }
        if favorites::legacy_root(&world.inventory).is_some() {
            ui.horizontal(|ui| {
                let free = self.pending.is_none();
                let sync = widgets::action_button_if(
                    ui,
                    p,
                    "refresh",
                    aurora_i18n::tr!("wearable-favorites-sync"),
                    ActionKind::Flat,
                    Vec2::new(0.0, BUTTON_H),
                    free,
                )
                .on_hover_text(aurora_i18n::tr!("wearable-favorites-sync-tip"));
                if sync.clicked() {
                    self.sync(world, out);
                }
            });
        }
    }

    /// The Sync button: nothing to add says so, without a change.
    pub fn sync(&mut self, world: &World, out: &mut Vec<FavoritesAction>) {
        match favorites::sync(&world.inventory) {
            Ok(plan) if plan.change.operations.is_empty() => {
                self.message = aurora_i18n::tr!("wearable-favorites-synced-none").into();
            }
            Ok(plan) => out.push(FavoritesAction::Edit {
                change: plan.change,
                select: None,
                synced: Some(plan.added),
            }),
            Err(reason) => self.message = reason,
        }
    }

    /// Draw the window while `open`.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &World, sort: Sort, open: &mut bool) -> Vec<FavoritesAction> {
        let mut out = Vec::new();
        if !*open {
            return out;
        }
        let root = favorites::root(&world.inventory);
        // a subfolder deleted elsewhere: back to the root (refreshFolderCombo)
        if let Some(folder) = self.folder
            && !root.is_some_and(|root| world.inventory.folders.get(&folder).is_some_and(|f| f.info.parent == root))
        {
            self.show_folder(None);
        }
        let shown = self.folder;
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("toolbar-command-wearable-favorites"),
            egui::pos2(320.0, 180.0),
            Vec2::new(420.0, 520.0),
        );
        floater.min_size = Vec2::new(340.0, 280.0);
        floater.show(ctx, p, open, |ui| {
            self.folder_bar(ui, p, world, root, shown, sort, &mut out);
            ui.add_space(6.0);
            let zone = ui.max_rect();
            let (mut list, footer) = widgets::above_footer(ui);
            match root {
                Some(root) => self.list(&mut list, p, world, root, shown, sort, &mut out),
                None => {
                    list.add(
                        egui::Label::new(
                            RichText::new(aurora_i18n::tr!("wearable-favorites-empty"))
                                .size(12.0)
                                .color(p.muted),
                        )
                        .wrap(),
                    );
                }
            }
            footer.show(ui, |ui| self.footer(ui, p, world, &mut out));
            if let Some((ids, released)) = inventory::drag::give_target(ui, zone) {
                ui.painter().rect_stroke(
                    zone,
                    ui.visuals().window_corner_radius,
                    egui::Stroke::new(1.5, p.violet),
                    egui::StrokeKind::Inside,
                );
                if released && self.pending.is_none() {
                    match favorites::add(&world.inventory, shown, &ids) {
                        Ok(change) => out.push(FavoritesAction::Edit {
                            change,
                            select: None,
                            synced: None,
                        }),
                        Err(reason) => self.message = reason,
                    }
                }
            }
        });
        self.dialog(ctx, p, world, &mut out);
        out
    }
}
