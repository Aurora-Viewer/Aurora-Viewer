//! Inventory window: lazily fetched folder tree, type icons, search.

use super::icons::Icons;
use crate::theme::Palette;
use crate::world::inventory::{FetchState, Inventory};
use crate::world::{World, inventory::actions as rules};
use aurora_net::inventory::operations::{Mutation, NewItem};
use egui::{RichText, Vec2};
use std::collections::HashSet;
use uuid::Uuid;
mod context;
mod dialogs;

pub enum InvAction {
    TeleportLandmark(Uuid),
    /// « À propos du repère » (LLLandmarkBridge "about"): its profile, as
    /// item and asset.
    AboutLandmark(Uuid, Uuid),
    Edit(Mutation),
    Create { parent: Uuid, kind: NewItem, name: String },
    Appearance(crate::world::appearance::Action),
    Animation { asset: Uuid, local: bool, start: bool },
    Sound(Uuid),
    Restore(Uuid),
    Share { items: Vec<Uuid>, resident: Uuid },
    Preview(Uuid),
    SaveContent { item: Uuid, text: String },
    Environment(Uuid, u32),
    Profile(Uuid),
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct InventoryPreferences {
    pub protected: HashSet<Uuid>,
    pub upload_folders: [Uuid; 5],
}

pub struct Facts {
    pub worn: HashSet<Uuid>,
    pub points: Vec<(u8, bool, String)>,
    pub appearance_busy: bool,
}

impl Facts {
    pub fn from_world(world: &World, appearance_busy: bool) -> Self {
        let mut worn = world.worn_attachment_items();
        if let Some(cof) = crate::world::appearance::cof(&world.inventory) {
            worn.extend(
                crate::world::appearance::folder_links(&world.inventory, cof)
                    .into_iter()
                    .filter(|l| !l.folder && world.inventory.items.get(&l.target).is_some_and(|it| it.asset_type != 6))
                    .map(|l| l.target),
            );
        }
        let mut points: Vec<_> = world
            .avatar_lib
            .attach_points
            .iter()
            .map(|(id, ap)| {
                let name = world.avatar_lib.attach_names.get(id).map(String::as_str).unwrap_or("Attachement");
                (*id, ap.hud, super::appearance::items::point_name(name).to_owned())
            })
            .collect();
        points.sort_by_cached_key(|(_, _, name)| name.to_lowercase());
        Self {
            worn,
            points,
            appearance_busy,
        }
    }
}

pub enum EditDialog {
    Rename(Uuid, String),
    Folder(Uuid, String),
    Properties(Uuid),
    Discard(Uuid),
    Share(Vec<Uuid>, Uuid),
    Delete(Vec<Uuid>, bool),
    Empty(Uuid),
    Ungroup(Uuid),
    ReplaceLinks(Uuid, Option<Uuid>),
}

pub struct PropertyEdit {
    pub name: String,
    pub desc: String,
    pub next: u32,
    pub group: u32,
    pub everyone: u32,
    pub sale_type: u8,
    pub price: i32,
}
impl From<&aurora_net::inventory::InvItem> for PropertyEdit {
    fn from(it: &aurora_net::inventory::InvItem) -> Self {
        Self {
            name: it.name.clone(),
            desc: it.desc.clone(),
            next: it.next_owner_mask,
            group: it.group_mask,
            everyone: it.everyone_mask,
            sale_type: it.sale_type,
            price: it.sale_price,
        }
    }
}

pub struct InventoryWindow {
    pub id: Uuid,
    pub root: Uuid,
    pub state: Box<InventoryUi>,
    pub open: bool,
}

/// An inventory item being dragged (dropped on an object's contents in
/// the build floater, like LLToolDragAndDrop).
#[derive(Debug, Clone, Copy)]
pub struct InvDrag(pub Uuid);

#[derive(Default)]
pub struct InventoryUi {
    pub ui_id: Uuid,
    pub demo_menu: Option<Uuid>,
    pub fetch_all: bool,
    pub search: String,
    pub tab: usize,
    pub expand_all: Option<bool>,
    pub selected: Option<Uuid>,
    reveal: Option<Uuid>,
    reveal_path: HashSet<Uuid>,
    pub selection: HashSet<Uuid>,
    anchor: Option<Uuid>,
    visible: Vec<Uuid>,
    previous_visible: Vec<Uuid>,
    pub clipboard: Vec<Uuid>,
    pub cut: bool,
    pub consume_clipboard: bool,
    pub pending_refresh: Vec<Uuid>,
    pub created_selection: Option<Uuid>,
    pub merchant_requested: bool,
    pub merchant: bool,
    pub pending: Option<Uuid>,
    pub message: String,
    pub dialog: Option<EditDialog>,
    pub links_filter: Option<Uuid>,
    pub windows: Vec<InventoryWindow>,
    pub texture_picker: super::texture_picker::TexturePicker,
    pub thumbnail_target: Option<Uuid>,
    pub share_picker: super::avatar_picker::AvatarPicker,
    pub share_items: Vec<Uuid>,
    pub property_edit: Option<(Uuid, PropertyEdit)>,
    pub preview: Option<Uuid>,
    pub preview_text: Option<Result<String, String>>,
    pub preview_can_edit: bool,
    pub preview_dirty: bool,
    pub save_pending: bool,
    pub wanted_images: HashSet<Uuid>,
    pub playing: Option<(Uuid, bool)>,
}

impl InventoryUi {
    /// show_item_original: clear filters, expand ancestors, select and scroll to the original.
    pub fn show_original(&mut self, inv: &Inventory, id: Uuid) {
        self.search.clear();
        self.links_filter = None;
        self.tab = 0;
        self.expand_all = None;
        self.selected = Some(id);
        self.selection.clear();
        self.selection.insert(id);
        self.reveal = Some(id);
        self.reveal_path.clear();
        let mut parent = rules::parent(inv, id);
        while let Some(id) = parent {
            if !self.reveal_path.insert(id) {
                break;
            }
            let Some(folder) = inv.folders.get(&id) else {
                break;
            };
            if folder.library {
                self.tab = 1;
            }
            parent = (!folder.info.parent.is_nil()).then_some(folder.info.parent);
        }
    }

    pub fn selected_ids(&self, id: Uuid) -> Vec<Uuid> {
        let mut ids: Vec<_> = if self.selection.contains(&id) {
            self.selection.iter().copied().collect()
        } else {
            vec![id]
        };
        ids.sort();
        ids
    }
    fn select(&mut self, ui: &egui::Ui, id: Uuid, secondary: bool) {
        if secondary && self.selection.contains(&id) {
            self.selected = Some(id);
            return;
        }
        let mods = ui.input(|i| i.modifiers);
        if mods.shift && !secondary {
            if let Some((a, b)) = self
                .anchor
                .and_then(|a| self.previous_visible.iter().position(|id| *id == a))
                .zip(self.previous_visible.iter().position(|row| *row == id))
            {
                if !mods.command {
                    self.selection.clear();
                }
                self.selection.extend(self.previous_visible[a.min(b)..=a.max(b)].iter().copied());
            } else {
                self.selection.insert(id);
            }
        } else if mods.command && !secondary {
            if !self.selection.remove(&id) {
                self.selection.insert(id);
            }
            self.anchor = Some(id);
        } else {
            self.selection.clear();
            self.selection.insert(id);
            self.anchor = Some(id);
        }
        self.selected = Some(id);
    }
}

fn folder_icon(type_default: i32, open: bool) -> &'static str {
    match type_default {
        14 => "Inv_TrashClosed",
        8 | 9 => "Inv_FolderOpen",
        _ if open => "Inv_FolderOpen",
        _ => "Inv_FolderClosed",
    }
}

pub fn item_icon(asset_type: i32, inv_type: i32) -> &'static str {
    match (asset_type, inv_type) {
        (_, 15) => "Inv_Snapshot",
        (0, _) => "Inv_Texture",
        (1, _) => "Inv_Sound",
        (2, _) => "Inv_CallingCard",
        (3, _) => "Inv_Landmark",
        (5, _) => "Inv_Clothing",
        (6, _) => "Inv_Object",
        (7, _) => "Inv_Notecard",
        (10, _) => "Inv_Script",
        (13, _) => "Inv_BodyShape",
        (20, _) => "Inv_Animation",
        (21, _) => "Inv_Gesture",
        (24, _) => "Inv_LinkItem",
        (25, _) => "Inv_LinkFolder",
        (49, _) => "Inv_Mesh",
        (56, _) => "Inv_Settings",
        (57, _) => "Inv_Material",
        _ => "Inv_Invalid",
    }
}

fn icon(ui: &mut egui::Ui, icons: &Icons, name: &str, p: &Palette) {
    if let Some(t) = icons.get(name) {
        let tint = if name.starts_with("Inv_Folder") { p.violet_light } else { p.ink };
        ui.add(egui::Image::new(t).fit_to_exact_size(Vec2::splat(16.0)).tint(tint));
    } else {
        ui.add_space(16.0);
    }
}

fn item_row(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &Inventory,
    id: &Uuid,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let Some(it) = inv.items.get(id) else {
        return;
    };
    st.visible.push(*id);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        icon(ui, icons, item_icon(it.asset_type, it.inv_type), p);
        let worn = rules::original(inv, *id).is_some_and(|id| facts.worn.contains(&id));
        let text = if worn { format!("{} (porté)", it.name) } else { it.name.clone() };
        let r = ui
            .dnd_drag_source(egui::Id::new(("inv_drag", it.id)), InvDrag(it.id), |ui| {
                ui.add_sized(
                    [ui.available_width(), 20.0],
                    egui::Button::selectable(st.selection.contains(id), RichText::new(text).size(13.0).color(p.ink)).frame(false),
                )
            })
            .inner;
        let r = if it.desc.is_empty() { r } else { r.on_hover_text(&it.desc) };
        if r.clicked() || r.secondary_clicked() {
            st.select(ui, *id, r.secondary_clicked());
        }
        if st.reveal == Some(it.id) {
            r.scroll_to_me(Some(egui::Align::Center));
            st.reveal = None;
            st.reveal_path.clear();
        }
        if r.double_clicked() {
            context::open(inv, *id, st, actions);
        }
        super::menu::context_menu(&r, p, |ui| context::show(ui, p, inv, *id, st, prefs, facts, actions));
    });
}

fn folder_tree(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    id: Uuid,
    depth: usize,
    expand: Option<bool>,
    state: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    if depth > 24 {
        return;
    }
    let Some(f) = inv.folders.get(&id) else {
        return;
    };
    let name = f.info.name.clone();
    let tdef = f.info.type_default;
    let fetch_state = f.state;
    let children = f.children.clone();
    let items = f.items.clone();
    let id_salt = ui.make_persistent_id(("inv", id));
    let mut st = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id_salt, depth == 0);
    if state.reveal_path.contains(&id) {
        st.set_open(true);
    }
    if let Some(e) = expand {
        // "Développer" only opens folders already fetched, to avoid fetching everything.
        if !e || fetch_state == FetchState::Fetched || depth == 0 {
            st.set_open(e || depth == 0);
        }
    }
    let open = st.is_open();
    let header = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let (r, resp) = ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::click());
        let c = r.center();
        let pts = if open {
            vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(4.0, -2.0), c + egui::vec2(0.0, 3.0)]
        } else {
            vec![c + egui::vec2(-2.0, -4.0), c + egui::vec2(3.0, 0.0), c + egui::vec2(-2.0, 4.0)]
        };
        ui.painter().add(egui::Shape::convex_polygon(pts, p.muted, egui::Stroke::NONE));
        if resp.clicked() {
            st.toggle(ui);
        }
        icon(ui, icons, folder_icon(tdef, open), p);
        let label = match fetch_state {
            FetchState::Fetching => format!("{name}  (chargement…)"),
            FetchState::Failed => format!("{name}  (échec)"),
            _ => name.clone(),
        };
        state.visible.push(id);
        let r = ui.add_sized(
            [ui.available_width(), 20.0],
            egui::Button::selectable(state.selection.contains(&id), RichText::new(label).size(13.0).color(p.ink)).frame(false),
        );
        if r.clicked() || r.secondary_clicked() {
            state.select(ui, id, r.secondary_clicked());
        }
        if r.double_clicked() {
            st.toggle(ui);
        }
        if state.reveal == Some(id) {
            r.scroll_to_me(Some(egui::Align::Center));
            state.reveal = None;
            state.reveal_path.clear();
        }
        super::menu::context_menu(&r, p, |ui| context::show(ui, p, inv, id, state, prefs, facts, actions));
    });
    let _ = header;
    if st.is_open() && matches!(fetch_state, FetchState::Unknown | FetchState::Failed) {
        inv.request(id);
    }
    st.store(ui.ctx());
    if st.is_open() {
        ui.indent(id_salt, |ui| {
            for c in children {
                folder_tree(ui, p, icons, inv, c, depth + 1, expand, state, prefs, facts, actions);
            }
            for i in &items {
                item_row(ui, p, icons, inv, i, state, prefs, facts, actions);
            }
            if items.is_empty() && fetch_state == FetchState::Fetched && inv.folders.get(&id).is_some_and(|f| f.children.is_empty()) {
                ui.label(RichText::new("(vide)").size(12.0).color(p.muted_dim));
            }
        });
    }
}

pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    icons: &Icons,
    world: &mut World,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    open: &mut bool,
    appearance_busy: bool,
    images: &std::collections::HashMap<Uuid, egui::TextureHandle>,
) -> Vec<InvAction> {
    let mut actions = Vec::new();
    let facts = Facts::from_world(world, appearance_busy);
    let inv = &mut world.inventory;
    st.previous_visible = std::mem::take(&mut st.visible);
    if st.fetch_all {
        st.fetch_all = !rules::request_tree(inv, inv.root);
    }
    for id in st.clipboard.clone() {
        if inv.folders.contains_key(&id) {
            rules::request_tree(inv, id);
        }
    }
    let screen = ctx.content_rect();
    super::widgets::Floater::new(
        "inventory",
        "Inventaire",
        egui::pos2(screen.right() - 390.0, 56.0),
        Vec2::new(370.0, 420.0),
    )
    .help("Clic droit : actions de l’élément. Ctrl / Maj : sélection multiple.")
    .show(ctx, p, open, |ui| {
        super::widgets::search_field(ui, &mut st.search, "Filtrer l'inventaire", ui.available_width());
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            if super::widgets::flat_button(ui, p, "Réduire").clicked() {
                st.expand_all = Some(false);
            }
            if super::widgets::flat_button(ui, p, "Développer").clicked() {
                st.expand_all = Some(true);
            }
        });
        ui.add_space(3.0);
        super::widgets::tabs(
            ui,
            p,
            &mut st.tab,
            &[
                ("Inventaire", true),
                ("Bibliothèque", true),
                ("Récent", true),
                ("Porté", true),
                ("Favoris", true),
            ],
        );
        ui.add_space(3.0);
        let list_h = (ui.available_height() - 48.0).max(80.0);
        egui::Frame::new().fill(p.field).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(list_h)
                .min_scrolled_height(list_h)
                .show(ui, |ui| {
                    let q = st.search.trim().to_lowercase();
                    if !q.is_empty() || st.tab >= 2 || st.links_filter.is_some() {
                        let mut hits: Vec<Uuid> = inv
                            .items
                            .values()
                            .filter(|i| i.name.to_lowercase().contains(&q))
                            .filter(|i| !rules::in_type(inv, i.id, 14))
                            .filter(|i| {
                                st.links_filter
                                    .is_none_or(|target| matches!(i.asset_type, 24 | 25) && i.asset_id == target)
                            })
                            .filter(|i| match st.tab {
                                1 => rules::library(inv, i.id),
                                2 => {
                                    i.created_at
                                        >= std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .map_or(0, |d| d.as_secs() as i64)
                                            - 86400
                                }
                                3 => rules::original(inv, i.id).is_some_and(|id| facts.worn.contains(&id)),
                                4 => i.favorite,
                                _ => !rules::library(inv, i.id),
                            })
                            .map(|i| i.id)
                            .collect();
                        hits.sort_by_key(|i| inv.items.get(i).map(|x| x.name.to_lowercase()));
                        let mut folders: Vec<_> = inv
                            .folders
                            .values()
                            .filter(|f| f.info.id != inv.root && f.info.id != inv.lib_root && !rules::in_type(inv, f.info.id, 14))
                            .filter(|f| f.info.name.to_lowercase().contains(&q) && st.links_filter.is_none())
                            .filter(|f| match st.tab {
                                1 => f.library,
                                2 | 3 => false,
                                4 => f.info.favorite,
                                _ => !f.library,
                            })
                            .map(|f| f.info.id)
                            .collect();
                        folders.sort_by_key(|id| rules::name(inv, *id).to_lowercase());
                        for id in &folders {
                            folder_tree(ui, p, icons, inv, *id, 1, Some(false), st, prefs, &facts, &mut actions);
                        }
                        let total = hits.len() + folders.len();
                        for id in hits.iter().take(500) {
                            item_row(ui, p, icons, inv, id, st, prefs, &facts, &mut actions);
                        }
                        if total > 500 {
                            ui.label(RichText::new(format!("… {} autres", total - 500)).color(p.muted));
                        }
                        if total == 0 {
                            ui.label(
                                RichText::new("Aucun résultat parmi les dossiers déjà chargés.")
                                    .size(12.0)
                                    .color(p.muted),
                            );
                        }
                        return;
                    }
                    let root = if st.tab == 1 { inv.lib_root } else { inv.root };
                    if root.is_nil() {
                        ui.label(RichText::new("Inventaire non disponible.").color(p.muted));
                    } else {
                        let expand = st.expand_all.take();
                        folder_tree(ui, p, icons, inv, root, 0, expand, st, prefs, &facts, &mut actions);
                    }
                });
        });
        if st.fetch_all {
            ui.label(RichText::new("Recherche : chargement des dossiers…").color(p.muted));
        }
        if st.links_filter.is_some() && super::widgets::flat_button(ui, p, "Quitter la recherche de liens").clicked() {
            st.links_filter = None;
        }
        if !st.message.is_empty() {
            ui.label(RichText::new(&st.message).size(12.0).color(p.warn));
        }
        if st.pending.is_some() {
            ui.label(RichText::new("Modification en cours…").size(12.0).color(p.muted));
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} objets", inv.item_count())).size(12.0).color(p.muted));
        });
    });
    dialogs::show(ctx, p, inv, st, prefs, &facts, &mut actions);
    dialogs::preview(ctx, p, inv, st, images, &mut actions);
    if let Some(image) = st.texture_picker.show(ctx, p, world, images)
        && let Some(id) = st.thumbnail_target.take()
    {
        match rules::patch(&world.inventory, id, aurora_net::inventory::operations::metadata_thumbnail(image)) {
            Ok(change) => actions.push(InvAction::Edit(change)),
            Err(reason) => st.message = reason,
        }
    }
    st.wanted_images.extend(st.texture_picker.wanted_images.drain());
    if let Some(residents) = st.share_picker.show(ctx, p, world)
        && let Some(resident) = residents.first()
    {
        st.dialog = Some(EditDialog::Share(std::mem::take(&mut st.share_items), *resident));
    }
    let mut windows = std::mem::take(&mut st.windows);
    let mut spawned = Vec::new();
    for w in &mut windows {
        if !world.inventory.folders.contains_key(&w.root) {
            w.open = false;
            continue;
        }
        w.state.ui_id = w.id;
        w.state.previous_visible = std::mem::take(&mut w.state.visible);
        w.state.clipboard.clone_from(&st.clipboard);
        w.state.cut = st.cut;
        w.state.pending = st.pending;
        w.state.merchant = st.merchant;
        w.state.save_pending = st.save_pending;
        w.state.playing = st.playing;
        let before = (w.state.clipboard.clone(), w.state.cut);
        let title = rules::name(&world.inventory, w.root);
        let window_id = format!("inventory_{}", w.id);
        super::widgets::Floater::new(
            &window_id,
            title,
            screen.center() - egui::vec2(210.0, 220.0),
            Vec2::new(420.0, 440.0),
        )
        .show(ctx, p, &mut w.open, |ui| {
            egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                folder_tree(
                    ui,
                    p,
                    icons,
                    &mut world.inventory,
                    w.root,
                    0,
                    None,
                    &mut w.state,
                    prefs,
                    &facts,
                    &mut actions,
                );
            });
            if !w.state.message.is_empty() {
                ui.label(RichText::new(&w.state.message).color(p.warn));
            }
        });
        dialogs::show(ctx, p, &world.inventory, &mut w.state, prefs, &facts, &mut actions);
        dialogs::preview(ctx, p, &world.inventory, &mut w.state, images, &mut actions);
        if let Some(image) = w.state.texture_picker.show(ctx, p, world, images)
            && let Some(id) = w.state.thumbnail_target.take()
            && let Ok(change) = rules::patch(&world.inventory, id, aurora_net::inventory::operations::metadata_thumbnail(image))
        {
            actions.push(InvAction::Edit(change));
        }
        if let Some(residents) = w.state.share_picker.show(ctx, p, world)
            && let Some(resident) = residents.first()
        {
            w.state.dialog = Some(EditDialog::Share(std::mem::take(&mut w.state.share_items), *resident));
        }
        st.wanted_images.extend(w.state.wanted_images.drain());
        st.wanted_images.extend(w.state.texture_picker.wanted_images.drain());
        if before != (w.state.clipboard.clone(), w.state.cut) {
            st.clipboard.clone_from(&w.state.clipboard);
            st.cut = w.state.cut;
        }
        st.consume_clipboard |= w.state.consume_clipboard;
        w.state.consume_clipboard = false;
        spawned.append(&mut w.state.windows);
    }
    // Documents and confirmations outlive the inventory window which opened
    // them, just like the main inventory's auxiliary floaters.
    windows.retain(|w| w.open || w.state.preview.is_some() || w.state.dialog.is_some());
    windows.append(&mut spawned);
    st.windows = windows;
    if let Some(id) = st.demo_menu {
        let mut menu_open = true;
        super::menu::popup_at(
            ctx,
            egui::Id::new("inventory_demo_menu"),
            screen.center() - egui::vec2(200.0, 330.0),
            &mut menu_open,
            p,
            |ui| context::show(ui, p, &world.inventory, id, st, prefs, &facts, &mut actions),
        );
        if !menu_open {
            st.demo_menu = None;
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn showing_an_original_clears_filters_and_expands_its_ancestors() {
        let mut inv = Inventory::default();
        crate::world::appearance::seed_demo(&mut inv, Uuid::from_u128(1));
        let original = Uuid::from_u128(715);
        let mut parent = inv.folders[&Uuid::from_u128(704)].clone();
        parent.info.parent = inv.root;
        inv.items.get_mut(&original).expect("original").parent = parent.info.id;
        let parent_id = parent.info.id;
        inv.folders.insert(parent_id, parent);
        let mut st = InventoryUi {
            search: "ancien filtre".into(),
            tab: 1,
            expand_all: Some(false),
            ..Default::default()
        };
        st.show_original(&inv, original);
        assert!(st.search.is_empty());
        assert_eq!(st.tab, 0);
        assert_eq!(st.selected, Some(original));
        assert_eq!(st.reveal, Some(original));
        assert!(st.reveal_path.contains(&parent_id));
        assert!(st.expand_all.is_none());
    }
}
