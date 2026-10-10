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
mod properties;
pub mod thumbnail;

pub enum InvAction {
    TeleportLandmark(Uuid),
    /// « À propos du repère » (LLLandmarkBridge "about"): its profile, as
    /// item and asset.
    AboutLandmark(Uuid, Uuid),
    Edit(Mutation),
    Create {
        parent: Uuid,
        kind: NewItem,
        name: String,
    },
    Appearance(crate::world::appearance::Action),
    Animation {
        asset: Uuid,
        local: bool,
        start: bool,
    },
    Sound(Uuid),
    Restore(Uuid),
    Share {
        items: Vec<Uuid>,
        resident: Uuid,
    },
    Preview(Uuid),
    SaveContent {
        item: Uuid,
        text: String,
    },
    Environment(Uuid, u32),
    Profile(Uuid),
    ThumbnailCopied(Uuid),
    Thumbnail {
        request: Uuid,
        item: Uuid,
        input: thumbnail::Input,
    },
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
    pub names: std::collections::HashMap<Uuid, String>,
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
            names: Default::default(),
        }
    }
}

pub enum EditDialog {
    Properties(Uuid),
    Discard(Uuid),
    Share(Vec<Uuid>, Uuid),
    Delete(Vec<Uuid>, bool),
    Empty(Uuid),
    Ungroup(Uuid),
    ReplaceLinks(Uuid, Option<Uuid>),
}

struct InlineRename {
    id: Uuid,
    name: String,
    focus: bool,
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
    rename: Option<InlineRename>,
    reveal: Option<Uuid>,
    reveal_path: HashSet<Uuid>,
    roots_seen: HashSet<Uuid>,
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
    pub thumbnail: thumbnail::ThumbnailUi,
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
    pub animation_stats: Option<(Uuid, AnimationStats)>,
    pub animation_description: Option<(Uuid, String)>,
    pub animation_details: bool,
}

#[derive(Clone)]
pub struct AnimationStats {
    pub priority: i32,
    pub duration: f32,
    pub looping: bool,
    pub entry: f32,
    pub exit: f32,
    pub joints: usize,
}

impl InventoryUi {
    /// FSFloaterPartialInventory: show only descendants of the chosen folder.
    /// Firestorm indra/newview/fsfloaterpartialinventory.cpp (LGPL 2.1).
    pub fn open_folder_window(&mut self, root: Uuid) {
        self.windows.push(InventoryWindow {
            id: Uuid::new_v4(),
            root,
            state: Box::new(InventoryUi::default()),
            open: true,
        });
    }

    /// LLFolderView::startRenamingSelectedItem: edit and select the name in place.
    pub fn begin_rename(&mut self, inv: &Inventory, id: Uuid) {
        self.selected = Some(id);
        self.selection.clear();
        self.selection.insert(id);
        self.reveal = Some(id);
        self.rename = Some(InlineRename {
            id,
            name: rules::name(inv, id),
            focus: true,
        });
        self.message.clear();
    }
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
                self.reveal_path.insert(inv.root);
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
    tree_spacing(ui);
    let Some(it) = inv.items.get(id) else {
        return;
    };
    st.visible.push(*id);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        icon(ui, icons, item_icon(it.asset_type, it.inv_type), p);
        if rename_row(ui, p, inv, *id, st, prefs, actions) {
            return;
        }
        let worn = rules::original(inv, *id).is_some_and(|id| facts.worn.contains(&id));
        let text = if worn { format!("{} (porté)", it.name) } else { it.name.clone() };
        let r = ui
            .push_id(("inv_item", it.id), |ui| {
                ui.add_sized(
                    [ui.available_width(), 20.0],
                    egui::Button::selectable(st.selection.contains(id), "")
                        .left_text(RichText::new(text).size(13.0).color(p.ink))
                        .sense(egui::Sense::click_and_drag()),
                )
            })
            .inner;
        // One response handles clicks and dragging; a separate drag-source
        // interaction overlay used to consume secondary clicks on the label.
        r.dnd_set_drag_payload(InvDrag(it.id));
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
    tree_spacing(ui);
    if depth > 24 {
        return;
    }
    let Some(f) = inv.folders.get(&id) else {
        return;
    };
    let name = folder_name(inv, id);
    let tdef = f.info.type_default;
    let fetch_state = f.state;
    let id_salt = ui.make_persistent_id(("inv", id));
    let mut st = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id_salt, depth == 0);
    let first_open = depth == 0 && state.roots_seen.insert(id);
    if first_open {
        st.set_open(true);
    }
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
        if rename_row(ui, p, inv, id, state, prefs, actions) {
            return;
        }
        let r = ui.add_sized(
            [ui.available_width(), 20.0],
            egui::Button::selectable(state.selection.contains(&id), "").left_text(RichText::new(label).size(13.0).color(p.ink)),
        );
        if first_open {
            r.scroll_to_me(Some(egui::Align::Min));
        }
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
    st.store(ui.ctx());
    if st.is_open() {
        ui.indent(id_salt, |ui| {
            folder_contents(ui, p, icons, inv, id, depth + 1, expand, state, prefs, facts, actions);
        });
    }
}

fn folder_name(inv: &Inventory, id: Uuid) -> String {
    if id == inv.root {
        "Mon inventaire".to_owned()
    } else if id == inv.lib_root {
        "Bibliothèque".to_owned()
    } else {
        rules::name(inv, id)
    }
}

fn folder_children(inv: &Inventory, id: Uuid) -> Vec<Uuid> {
    let mut children = inv.folders.get(&id).map(|f| f.children.clone()).unwrap_or_default();
    // Present the read-only library without changing its server parent.
    if id == inv.root
        && !inv.lib_root.is_nil()
        && inv.lib_root != id
        && inv.folders.contains_key(&inv.lib_root)
        && !children.contains(&inv.lib_root)
    {
        children.push(inv.lib_root);
        children.sort_by_cached_key(|id| folder_name(inv, *id).to_lowercase());
    }
    children
}

fn folder_contents(
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
    let Some(f) = inv.folders.get(&id) else {
        return;
    };
    let items = f.items.clone();
    let children = folder_children(inv, id);
    if matches!(f.state, FetchState::Unknown | FetchState::Failed) {
        inv.request(id);
    }
    for c in &children {
        folder_tree(ui, p, icons, inv, *c, depth, expand, state, prefs, facts, actions);
    }
    for i in &items {
        item_row(ui, p, icons, inv, i, state, prefs, facts, actions);
    }
    if items.is_empty() && children.is_empty() {
        let text = if inv.folders.get(&id).is_some_and(|f| f.state == FetchState::Fetched) {
            "(vide)"
        } else {
            "Chargement du dossier…"
        };
        ui.label(RichText::new(text).size(12.0).color(p.muted_dim));
    }
}

fn folder_window_contents(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    root: Uuid,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    super::widgets::search_field(ui, &mut st.search, "Filtrer le dossier d'inventaire", ui.available_width());
    ui.add_space(3.0);
    let q = st.search.trim().to_lowercase();
    if !q.is_empty() || st.fetch_all {
        st.fetch_all = !rules::request_tree(inv, root);
    }
    let list_h = (ui.available_height() - 40.0).max(80.0);
    egui::Frame::new().fill(p.field).show(ui, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(list_h)
            .min_scrolled_height(list_h)
            .show(ui, |ui| {
                if q.is_empty() && st.links_filter.is_none() {
                    folder_contents(ui, p, icons, inv, root, 1, None, st, prefs, facts, actions);
                    return;
                }
                // Walk this subtree rather than scanning the whole inventory.
                let mut todo = vec![root];
                let mut seen = HashSet::new();
                let mut folders = Vec::new();
                let mut items = Vec::new();
                while let Some(id) = todo.pop() {
                    if !seen.insert(id) {
                        continue;
                    }
                    let Some(f) = inv.folders.get(&id) else {
                        continue;
                    };
                    if id != root && st.links_filter.is_none() && folder_name(inv, id).to_lowercase().contains(&q) {
                        folders.push(id);
                    }
                    items.extend(
                        f.items
                            .iter()
                            .filter(|id| {
                                inv.items.get(id).is_some_and(|it| {
                                    it.name.to_lowercase().contains(&q)
                                        && st
                                            .links_filter
                                            .is_none_or(|target| matches!(it.asset_type, 24 | 25) && it.asset_id == target)
                                })
                            })
                            .copied(),
                    );
                    todo.extend(folder_children(inv, id));
                }
                folders.sort_by_cached_key(|id| folder_name(inv, *id).to_lowercase());
                items.sort_by_cached_key(|id| rules::name(inv, *id).to_lowercase());
                let total = folders.len() + items.len();
                for id in folders.iter().take(500) {
                    folder_tree(ui, p, icons, inv, *id, 1, Some(false), st, prefs, facts, actions);
                }
                for id in items.iter().take(500_usize.saturating_sub(folders.len())) {
                    item_row(ui, p, icons, inv, id, st, prefs, facts, actions);
                }
                if total > 500 {
                    ui.label(RichText::new(format!("… {} autres", total - 500)).color(p.muted));
                } else if total == 0 {
                    ui.label(
                        RichText::new("Aucun résultat dans les sous-dossiers déjà chargés.")
                            .size(12.0)
                            .color(p.muted),
                    );
                }
            });
    });
    if st.fetch_all {
        ui.label(RichText::new("Recherche : chargement des sous-dossiers…").size(12.0).color(p.muted));
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
}

fn tree_spacing(ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing.y = 1.0;
    ui.spacing_mut().button_padding = egui::vec2(2.0, 1.0);
    ui.spacing_mut().interact_size.y = 20.0;
}

fn rename_row(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    id: Uuid,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    actions: &mut Vec<InvAction>,
) -> bool {
    if st.rename.is_none()
        && st.selected == Some(id)
        && st.selection.len() == 1
        && st.pending.is_none()
        && !ui.ctx().text_edit_focused()
        && ui.input(|i| i.key_pressed(egui::Key::F2))
        && rules::renameable(inv, id, &prefs.protected)
    {
        st.begin_rename(inv, id);
    }
    let Some(mut edit) = st.rename.take() else {
        return false;
    };
    if edit.id != id {
        st.rename = Some(edit);
        return false;
    }
    if !rules::renameable(inv, id, &prefs.protected) {
        return false;
    }
    let r = ui.add_sized(
        [ui.available_width(), 20.0],
        egui::TextEdit::singleline(&mut edit.name)
            .id(ui.make_persistent_id(("inventory_rename", id)))
            .font(egui::FontId::proportional(13.0))
            .text_color(p.ink)
            .char_limit(63),
    );
    if st.reveal == Some(id) {
        r.scroll_to_me(Some(egui::Align::Center));
        st.reveal = None;
        st.reveal_path.clear();
    }
    if edit.focus {
        r.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(edit.name.chars().count()),
            )));
            state.store(ui.ctx(), r.id);
        }
        edit.focus = false;
    } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        ui.memory_mut(|m| m.surrender_focus(r.id));
        return true;
    } else if !r.has_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        st.rename = Some(edit);
        finish_rename(inv, st, prefs, actions);
        return true;
    }
    st.rename = Some(edit);
    true
}

fn finish_rename(inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, actions: &mut Vec<InvAction>) {
    let Some(mut edit) = st.rename.take() else {
        return;
    };
    if !rules::renameable(inv, edit.id, &prefs.protected) {
        return;
    }
    match rules::clean_name(&edit.name) {
        Ok(name) if name == rules::name(inv, edit.id) => return,
        Ok(name) if st.pending.is_none() => match rules::patch(inv, edit.id, aurora_llsd::llsd_map! { "name" => name }) {
            Ok(change) => {
                actions.push(InvAction::Edit(change));
                st.reveal = Some(edit.id);
                return;
            }
            Err(reason) => st.message = reason,
        },
        Ok(_) => st.message = "Attendez la modification en cours.".into(),
        Err(reason) => st.message = reason,
    }
    edit.focus = true;
    st.rename = Some(edit);
}

fn finish_hidden_rename(inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, actions: &mut Vec<InvAction>) {
    // Closing the window, filtering or collapsing a parent also ends editing.
    if st.rename.as_ref().is_some_and(|edit| !st.visible.contains(&edit.id)) {
        finish_rename(inv, st, prefs, actions);
        st.rename = None;
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
    let mut facts = Facts::from_world(world, appearance_busy);
    fn property_names(st: &InventoryUi, world: &World, names: &mut std::collections::HashMap<Uuid, String>) {
        if let Some(EditDialog::Properties(id)) = st.dialog
            && let Some(it) = world.inventory.items.get(&id)
        {
            for resident in [it.owner, it.creator].into_iter().filter(|id| !id.is_nil()) {
                names.insert(
                    resident,
                    world
                        .social
                        .avatar_names
                        .complete(&resident)
                        .unwrap_or_else(|| "Chargement…".into()),
                );
            }
        }
        for window in &st.windows {
            property_names(&window.state, world, names);
        }
    }
    property_names(st, world, &mut facts.names);
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
        let mut visible_tab = match st.tab {
            2 => 1,
            3 => 2,
            4 => 3,
            _ => 0,
        };
        super::widgets::tabs(
            ui,
            p,
            &mut visible_tab,
            &[("Inventaire", true), ("Récent", true), ("Porté", true), ("Favoris", true)],
        );
        st.tab = [0, 2, 3, 4][visible_tab];
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
                                2 => {
                                    i.created_at
                                        >= std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .map_or(0, |d| d.as_secs() as i64)
                                            - 86400
                                }
                                3 => rules::original(inv, i.id).is_some_and(|id| facts.worn.contains(&id)),
                                4 => i.favorite,
                                _ => true,
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
                                2 | 3 => false,
                                4 => f.info.favorite,
                                _ => true,
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
                    let root = inv.root;
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
    finish_hidden_rename(inv, st, prefs, &mut actions);
    dialogs::show(ctx, p, icons, inv, st, prefs, &facts, images, &mut actions);
    dialogs::preview(ctx, p, inv, st, images, prefs, &mut actions);
    st.thumbnail.show(
        ctx,
        p,
        icons,
        world,
        prefs,
        st.ui_id,
        images,
        &mut st.wanted_images,
        st.pending.is_some(),
        &mut actions,
    );
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
        let title = format!("Inventaire : {}", folder_name(&world.inventory, w.root));
        let window_id = format!("inventory_{}", w.id);
        super::widgets::Floater::new(
            &window_id,
            title,
            screen.center() - egui::vec2(210.0, 220.0),
            Vec2::new(420.0, 440.0),
        )
        .show(ctx, p, &mut w.open, |ui| {
            folder_window_contents(
                ui,
                p,
                icons,
                &mut world.inventory,
                w.root,
                &mut w.state,
                prefs,
                &facts,
                &mut actions,
            );
        });
        finish_hidden_rename(&world.inventory, &mut w.state, prefs, &mut actions);
        dialogs::show(ctx, p, icons, &world.inventory, &mut w.state, prefs, &facts, images, &mut actions);
        dialogs::preview(ctx, p, &world.inventory, &mut w.state, images, prefs, &mut actions);
        w.state.thumbnail.show(
            ctx,
            p,
            icons,
            world,
            prefs,
            w.id,
            images,
            &mut w.state.wanted_images,
            st.pending.is_some(),
            &mut actions,
        );
        if let Some(residents) = w.state.share_picker.show(ctx, p, world)
            && let Some(resident) = residents.first()
        {
            w.state.dialog = Some(EditDialog::Share(std::mem::take(&mut w.state.share_items), *resident));
        }
        st.wanted_images.extend(w.state.wanted_images.drain());
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
    windows.retain(|w| w.open || w.state.preview.is_some() || w.state.dialog.is_some() || w.state.thumbnail.item.is_some());
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
fn headless_output(mut output: egui::FullOutput) -> egui::FullOutput {
    // These tests inspect shapes/events without a renderer to apply texture uploads.
    output.textures_delta.clear();
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder_window_frame(ctx: &egui::Context, inv: &mut Inventory, root: Uuid, st: &mut InventoryUi) -> egui::FullOutput {
        st.visible.clear();
        headless_output(ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 700.0))),
                ..Default::default()
            },
            |ui| {
                folder_window_contents(
                    ui,
                    &crate::theme::Theme::default().palette(),
                    &Icons::default(),
                    inv,
                    root,
                    st,
                    &mut InventoryPreferences::default(),
                    &Facts {
                        worn: HashSet::new(),
                        points: Vec::new(),
                        appearance_busy: false,
                        names: Default::default(),
                    },
                    &mut Vec::new(),
                );
            },
        ))
    }

    #[test]
    fn folder_window_shows_children_without_its_root_or_other_inventory_branches() {
        let ctx = egui::Context::default();
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let root = Uuid::from_u128(8000);
        let mut st = InventoryUi::default();
        let output = folder_window_frame(&ctx, &mut inv, root, &mut st);
        assert!(st.visible.contains(&Uuid::from_u128(8001)));
        assert!(st.visible.contains(&Uuid::from_u128(8100)));
        for id in [inv.root, inv.lib_root, root, Uuid::from_u128(8002)] {
            assert!(!st.visible.contains(&id), "outside or enclosing folder: {id}");
        }
        for label in ["Mon inventaire", "Bibliothèque", "Essais du clic droit", "Inventaire", "Favoris"] {
            assert!(
                !output
                    .shapes
                    .iter()
                    .any(|s| matches!(&s.shape, egui::epaint::Shape::Text(t) if t.galley.text() == label))
            );
        }
    }

    #[test]
    fn folder_window_search_finds_nested_items_and_excludes_matching_items_elsewhere() {
        let ctx = egui::Context::default();
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let root = Uuid::from_u128(8000);
        let mut nested = inv.items[&Uuid::from_u128(8103)].clone();
        nested.parent = Uuid::from_u128(8001);
        let mut outside = nested.clone();
        outside.id = Uuid::from_u128(8200);
        outside.parent = Uuid::from_u128(8002);
        inv.add_items(vec![nested, outside]);
        let mut st = InventoryUi {
            search: "instructions".into(),
            ..Default::default()
        };
        folder_window_frame(&ctx, &mut inv, root, &mut st);
        assert_eq!(st.visible, vec![Uuid::from_u128(8103)]);
        st.search = "sous-dossier".into();
        folder_window_frame(&ctx, &mut inv, root, &mut st);
        assert_eq!(st.visible, vec![Uuid::from_u128(8001)]);
        st.search = "annonces".into();
        folder_window_frame(&ctx, &mut inv, root, &mut st);
        assert!(st.visible.is_empty());
    }

    #[test]
    fn folder_window_fetches_its_hidden_root_and_limits_search_fetches_to_descendants() {
        for search in ["", "instructions"] {
            let ctx = egui::Context::default();
            let mut inv = Inventory::default();
            crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
            let root = Uuid::from_u128(8000);
            for id in [root, Uuid::from_u128(8001), Uuid::from_u128(8002)] {
                inv.folders.get_mut(&id).expect("folder").state = FetchState::Unknown;
            }
            let mut st = InventoryUi {
                search: search.into(),
                ..Default::default()
            };
            folder_window_frame(&ctx, &mut inv, root, &mut st);
            assert!(inv.queue.iter().any(|(id, _)| *id == root));
            assert!(!inv.queue.iter().any(|(id, _)| *id == Uuid::from_u128(8002)));
            assert_eq!(inv.queue.iter().any(|(id, _)| *id == Uuid::from_u128(8001)), !search.is_empty());
        }
    }

    fn rename_frame(ctx: &egui::Context, inv: &Inventory, state: &mut InventoryUi, events: Vec<egui::Event>) -> Vec<InvAction> {
        let mut actions = Vec::new();
        let _ = headless_output(ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                item_row(
                    ui,
                    &crate::theme::Theme::default().palette(),
                    &Icons::default(),
                    inv,
                    &Uuid::from_u128(8100),
                    state,
                    &mut InventoryPreferences::default(),
                    &Facts {
                        worn: HashSet::new(),
                        points: Vec::new(),
                        appearance_busy: false,
                        names: Default::default(),
                    },
                    &mut actions,
                );
                ui.add(egui::TextEdit::singleline(&mut String::new()).id(egui::Id::new("other_field")));
            },
        ));
        actions
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn inline_rename_submits_on_enter_and_leaves_the_item_unchanged_until_confirmation() {
        let ctx = egui::Context::default();
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let id = Uuid::from_u128(8100);
        let old = inv.items[&id].name.clone();
        let mut st = InventoryUi::default();
        st.begin_rename(&inv, id);
        rename_frame(&ctx, &inv, &mut st, Vec::new());
        rename_frame(&ctx, &inv, &mut st, vec![egui::Event::Text("Objet renommé".into())]);
        let actions = rename_frame(&ctx, &inv, &mut st, vec![key(egui::Key::Enter)]);
        assert!(st.rename.is_none());
        assert!(st.dialog.is_none());
        assert_eq!(inv.items[&id].name, old);
        assert!(matches!(&actions[..], [InvAction::Edit(change)] if matches!(&change.operations[..],
            [aurora_net::inventory::operations::Operation::Patch { id: target, body, .. }] if *target == id && body["name"].as_str() == "Objet renommé")));
    }

    #[test]
    fn inline_rename_cancels_on_escape_and_keeps_the_default_on_enter_or_focus_loss() {
        for finish in [egui::Key::Escape, egui::Key::Enter, egui::Key::Tab] {
            let ctx = egui::Context::default();
            let mut inv = Inventory::default();
            crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
            let id = Uuid::from_u128(8100);
            let mut st = InventoryUi::default();
            st.begin_rename(&inv, id);
            rename_frame(&ctx, &inv, &mut st, Vec::new());
            if finish == egui::Key::Escape {
                rename_frame(&ctx, &inv, &mut st, vec![egui::Event::Text("Annulé".into())]);
            }
            let mut actions = rename_frame(&ctx, &inv, &mut st, vec![key(finish)]);
            actions.extend(rename_frame(&ctx, &inv, &mut st, Vec::new()));
            assert!(actions.is_empty(), "{finish:?}");
            assert!(st.rename.is_none(), "{finish:?}");
            assert!(st.dialog.is_none());
        }
    }

    #[test]
    fn inline_rename_saves_changes_when_another_field_takes_focus() {
        let ctx = egui::Context::default();
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let mut st = InventoryUi::default();
        st.begin_rename(&inv, Uuid::from_u128(8100));
        rename_frame(&ctx, &inv, &mut st, Vec::new());
        rename_frame(&ctx, &inv, &mut st, vec![egui::Event::Text("Nouveau nom".into())]);
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("other_field")));
        let actions = rename_frame(&ctx, &inv, &mut st, Vec::new());
        assert!(st.rename.is_none());
        assert!(matches!(&actions[..], [InvAction::Edit(change)] if matches!(&change.operations[..],
            [aurora_net::inventory::operations::Operation::Patch { body, .. }] if body["name"].as_str() == "Nouveau nom")));
        st.begin_rename(&inv, Uuid::from_u128(8100));
        rename_frame(&ctx, &inv, &mut st, Vec::new());
        rename_frame(&ctx, &inv, &mut st, vec![egui::Event::Text("Nom avant fermeture".into())]);
        st.visible.clear();
        let mut actions = Vec::new();
        finish_hidden_rename(&inv, &mut st, &InventoryPreferences::default(), &mut actions);
        assert!(st.rename.is_none());
        assert!(matches!(&actions[..], [InvAction::Edit(change)] if matches!(&change.operations[..],
            [aurora_net::inventory::operations::Operation::Patch { body, .. }] if body["name"].as_str() == "Nom avant fermeture")));
    }

    fn item_frame(ctx: &egui::Context, inv: &Inventory, state: &mut InventoryUi, events: Vec<egui::Event>) -> egui::FullOutput {
        headless_output(ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                item_row(
                    ui,
                    &crate::theme::Theme::default().palette(),
                    &Icons::default(),
                    inv,
                    &Uuid::from_u128(8100),
                    state,
                    &mut InventoryPreferences::default(),
                    &Facts {
                        worn: HashSet::new(),
                        points: Vec::new(),
                        appearance_busy: false,
                        names: Default::default(),
                    },
                    &mut Vec::new(),
                );
            },
        ))
    }

    #[test]
    fn root_opens_once_and_library_is_a_left_aligned_virtual_child() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let root = Uuid::from_u128(1);
        let lib = Uuid::from_u128(2);
        let mut inv = Inventory {
            root,
            lib_root: lib,
            ..Default::default()
        };
        for (id, name, library) in [(root, "Mon inventaire", false), (lib, "Library", true)] {
            inv.folders.insert(
                id,
                crate::world::inventory::Folder {
                    info: aurora_net::inventory::InvFolder {
                        id,
                        name: name.into(),
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: Vec::new(),
                    state: FetchState::Fetched,
                    library,
                },
            );
        }
        let mut st = InventoryUi::default();
        let mut output = None;
        for frame in 0..4 {
            output = Some(headless_output(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                    ..Default::default()
                },
                |ui| {
                    if frame == 0 {
                        let mut old = egui::collapsing_header::CollapsingState::load_with_default_open(
                            &ctx,
                            ui.make_persistent_id(("inv", root)),
                            false,
                        );
                        old.set_open(false);
                        old.store(&ctx);
                    }
                    folder_tree(
                        ui,
                        &p,
                        &Icons::default(),
                        &mut inv,
                        root,
                        0,
                        None,
                        &mut st,
                        &mut InventoryPreferences::default(),
                        &Facts {
                            worn: HashSet::new(),
                            points: Vec::new(),
                            appearance_busy: false,
                            names: Default::default(),
                        },
                        &mut Vec::new(),
                    );
                },
            )));
        }
        let output = output.expect("frame");
        for label in ["Mon inventaire", "Bibliothèque"] {
            let text = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::epaint::Shape::Text(t) if t.galley.text() == label => Some(t),
                    _ => None,
                })
                .expect("tree label");
            assert!(text.pos.x < 100.0, "{label} must remain next to its icon");
        }
        assert!(st.visible.contains(&lib));
        assert_eq!(inv.folders[&lib].info.parent, Uuid::nil());
        assert!(inv.folders[&lib].library);
    }

    #[test]
    fn a_secondary_click_on_an_item_selects_it_and_opens_its_menu() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let p = crate::theme::Theme::default().palette();
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let id = Uuid::from_u128(8100);
        let mut state = InventoryUi::default();
        let pos = egui::pos2(200.0, 18.0);
        let mut output = None;
        for n in 0..8 {
            let mut events = vec![egui::Event::PointerMoved(pos)];
            if matches!(n, 4 | 5) {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed: n == 4,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            output = Some(item_frame(&ctx, &inv, &mut state, events));
        }
        assert_eq!(state.selected, Some(id));
        let output = output.expect("frame");
        assert!(
            output
                .shapes
                .iter()
                .any(|s| matches!(&s.shape, egui::epaint::Shape::Text(t) if t.galley.text() == "Propriétés"))
        );
        assert!(
            output
                .shapes
                .iter()
                .any(|s| matches!(&s.shape, egui::epaint::Shape::Rect(r) if r.fill == p.violet)),
            "selection must be visible"
        );
    }

    #[test]
    fn f2_starts_inline_rename_after_selecting_a_row() {
        let ctx = egui::Context::default();
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let mut st = InventoryUi::default();
        let pos = egui::pos2(200.0, 18.0);
        for n in 0..8 {
            let mut events = vec![egui::Event::PointerMoved(pos)];
            if matches!(n, 4 | 5) {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: n == 4,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            item_frame(&ctx, &inv, &mut st, events);
        }
        assert_eq!(st.selected, Some(Uuid::from_u128(8100)));
        item_frame(&ctx, &inv, &mut st, vec![key(egui::Key::F2)]);
        assert_eq!(st.rename.as_ref().map(|e| e.id), st.selected);
        assert!(st.dialog.is_none());
    }

    #[test]
    fn item_rows_still_supply_the_inventory_payload_when_dragged() {
        let ctx = egui::Context::default();
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let mut state = InventoryUi::default();
        for n in 0..7 {
            let pos = egui::pos2(if n < 5 { 200.0 } else { 260.0 }, 18.0);
            let mut events = vec![egui::Event::PointerMoved(pos)];
            if n == 4 {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            item_frame(&ctx, &inv, &mut state, events);
        }
        assert_eq!(
            egui::DragAndDrop::payload::<InvDrag>(&ctx).expect("drag payload").0,
            Uuid::from_u128(8100)
        );
    }

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
