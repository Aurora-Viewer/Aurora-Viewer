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
mod controls;
mod dialogs;
mod properties;
pub mod thumbnail;
mod view;
pub use controls::preferences;
pub use view::InventoryPreferences;

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

pub struct Facts {
    pub agent: Uuid,
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
            agent: world.agent_id,
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
    pub filters_open: bool,
    pub preferences_open: bool,
    filters_initialized: bool,
    filters: view::Filters,
    saved_filters: Option<view::Filters>,
    view: view::ViewCache,
    result_tree: view::TreeState,
    result_width: f32,
    creator_generation: Option<u64>,
    creators: HashSet<Uuid>,
    fetch_poll: Option<std::time::Instant>,
    fetch_generation: Option<(Uuid, u64, bool, bool, bool)>,
    pub ui_id: Uuid,
    pub demo_menu: Option<Uuid>,
    pub fetch_all: bool,
    pub search: String,
    searches: [String; 5],
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
        self.filters = view::Filters::default();
        self.saved_filters = None;
        self.filters_initialized = true;
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
            let order = if !self.search.trim().is_empty() || self.filters.active() || self.links_filter.is_some() || self.tab >= 2 {
                &self.result_tree.order
            } else {
                &self.previous_visible
            };
            if let Some((a, b)) = self
                .anchor
                .and_then(|a| order.iter().position(|id| *id == a))
                .zip(order.iter().position(|row| *row == id))
            {
                if !mods.command {
                    self.selection.clear();
                }
                self.selection.extend(order[a.min(b)..=a.max(b)].iter().copied());
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
                        .wrap_mode(egui::TextWrapMode::Extend)
                        .sense(egui::Sense::click_and_drag()),
                )
            })
            .inner;
        // One response handles clicks and dragging; a separate drag-source
        // interaction overlay used to consume secondary clicks on the label.
        r.dnd_set_drag_payload(InvDrag(it.id));
        let date = if it.created_at > 0 {
            aurora_net::inventory::created_date(it.created_at)
        } else {
            "Date inconnue".into()
        };
        let path = inventory_path(inv, it.parent);
        let r = r.on_hover_text(format!("{path}\n{date}\n{}", it.desc));
        if r.clicked() || r.secondary_clicked() {
            st.select(ui, *id, r.secondary_clicked());
        }
        if st.reveal == Some(it.id) {
            scroll_to_row(ui, &r, egui::Align::Center);
            st.reveal = None;
            st.reveal_path.clear();
        }
        if r.double_clicked() {
            context::open(inv, *id, st, prefs, actions);
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
    let mut open = st.is_open();
    folder_row(ui, p, icons, inv, id, &mut open, first_open, state, prefs, facts, actions);
    st.set_open(open);
    st.store(ui.ctx());
    if open {
        ui.indent(id_salt, |ui| {
            folder_contents(ui, p, icons, inv, id, depth + 1, expand, state, prefs, facts, actions);
        });
    }
}

fn folder_row(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &Inventory,
    id: Uuid,
    open: &mut bool,
    first_open: bool,
    state: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let Some(folder) = inv.folders.get(&id) else { return };
    let name = folder_name(inv, id);
    let fetch_state = folder.state;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let (r, resp) = ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::click());
        let c = r.center();
        let pts = if *open {
            vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(4.0, -2.0), c + egui::vec2(0.0, 3.0)]
        } else {
            vec![c + egui::vec2(-2.0, -4.0), c + egui::vec2(3.0, 0.0), c + egui::vec2(-2.0, 4.0)]
        };
        ui.painter().add(egui::Shape::convex_polygon(pts, p.muted, egui::Stroke::NONE));
        if resp.clicked() {
            *open = !*open;
        }
        icon(ui, icons, folder_icon(folder.info.type_default, *open), p);
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
            egui::Button::selectable(state.selection.contains(&id), "")
                .left_text(RichText::new(label).size(13.0).color(p.ink))
                .wrap_mode(egui::TextWrapMode::Extend),
        );
        if first_open {
            scroll_to_row(ui, &r, egui::Align::Min);
        }
        if r.clicked() || r.secondary_clicked() {
            state.select(ui, id, r.secondary_clicked());
        }
        if r.double_clicked() {
            *open = !*open;
        }
        if state.reveal == Some(id) {
            scroll_to_row(ui, &r, egui::Align::Center);
            state.reveal = None;
            state.reveal_path.clear();
        }
        super::menu::context_menu(&r, p, |ui| context::show(ui, p, inv, id, state, prefs, facts, actions));
    });
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

fn inventory_path(inv: &Inventory, mut id: Uuid) -> String {
    let mut parts = Vec::new();
    let mut seen = HashSet::new();
    while seen.insert(id) && inv.folders.contains_key(&id) {
        parts.push(folder_name(inv, id));
        id = rules::parent(inv, id).unwrap_or(Uuid::nil());
    }
    parts.reverse();
    parts.join(" / ")
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
    let fetch_state = f.state;
    state.view.prepare(inv, prefs.sort);
    let children = state.view.children(id, prefs, inv.lib_root);
    if matches!(fetch_state, FetchState::Unknown | FetchState::Failed) {
        inv.request(id);
    }
    for c in &children {
        if inv.folders.contains_key(c) {
            folder_tree(ui, p, icons, inv, *c, depth, expand, state, prefs, facts, actions);
        } else {
            item_row(ui, p, icons, inv, c, state, prefs, facts, actions);
        }
    }
    if children.is_empty() {
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
    controls::toolbar(ui, p, st, prefs);
    controls::search(ui, st, prefs, "Filtrer le dossier d'inventaire");
    ui.add_space(3.0);
    let q = st.search.trim().to_lowercase();
    let filtered = !q.is_empty() || st.links_filter.is_some() || st.filters.active();
    if filtered {
        request_search(inv, root, st, prefs);
    } else if st.fetch_all {
        st.fetch_all = !rules::request_tree(inv, root);
    }
    if filtered {
        st.view.search(
            inv,
            root,
            &q,
            prefs,
            &st.filters,
            0,
            st.links_filter,
            facts,
            facts.agent,
            view::now(),
        );
    }
    let list_h = list_height(ui);
    egui::Frame::new().fill(p.field).show(ui, |ui| {
        if filtered {
            prepare_result_tree(st, inv, root);
            search_scroll(ui, p, icons, inv, st, prefs, facts, actions, list_h);
            return;
        }
        list_scroll(ui, list_h).show(ui, |ui| {
            let expand = st.expand_all.take();
            folder_contents(ui, p, icons, inv, root, 1, expand, st, prefs, facts, actions);
        });
    });
    status(ui, p, inv, st, filtered);
}

fn tree_spacing(ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing.y = 1.0;
    ui.spacing_mut().button_padding = egui::vec2(2.0, 1.0);
    ui.spacing_mut().interact_size.y = 20.0;
}

fn prepare_result_tree(st: &mut InventoryUi, inv: &Inventory, root: Uuid) {
    if let Some(open) = st.expand_all.take() {
        st.result_tree.set_all(&st.view, root, st.tab, open);
    }
    if let Some(id) = st.reveal {
        st.result_tree.reveal(inv, id, st.tab);
    }
    if st
        .result_tree
        .prepare(&st.view, root, st.tab, root == inv.root && inv.folders.contains_key(&root))
    {
        st.result_width = 0.0;
    }
}

// LLFolderView::arrange/reshape (indra/llui/llfolderview.cpp): long labels
// enlarge the scrollable document, rather than the surrounding floater.
fn list_scroll(ui: &mut egui::Ui, height: f32) -> egui::ScrollArea {
    ui.spacing_mut().scroll = egui::style::ScrollStyle {
        dormant_background_opacity: 0.2,
        dormant_handle_opacity: 0.6,
        ..egui::style::ScrollStyle::solid()
    };
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .max_width(ui.available_width())
        .max_height(height)
        .min_scrolled_height((height - ui.spacing().scroll.allocated_width()).max(40.0))
}

fn scroll_to_row(ui: &egui::Ui, row: &egui::Response, align: egui::Align) {
    // Reveal vertically while keeping folder carets and the start of names in
    // view. Aligning a whole, potentially very wide label would also pan right.
    let rect = egui::Rect::from_x_y_ranges(ui.clip_rect().x_range(), row.rect.y_range());
    ui.scroll_to_rect(rect, Some(align));
}

fn list_height(ui: &egui::Ui) -> f32 {
    // Reserve both footer rows and their layout spacing, even while folders
    // arrive asynchronously. A variable-height footer made the window grow.
    (ui.available_height() - 44.0 - 2.0 * ui.spacing().item_spacing.y).max(40.0)
}

fn status(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, st: &InventoryUi, filtered: bool) {
    let (message, color) = if !st.message.is_empty() {
        (st.message.as_str(), p.warn)
    } else if st.pending.is_some() {
        ("Modification en cours…", p.muted)
    } else if st.fetch_all {
        ("Recherche : chargement des dossiers…", p.muted)
    } else {
        ("", p.muted)
    };
    ui.add_sized(
        [ui.available_width(), 20.0],
        egui::Label::new(RichText::new(message).size(12.0).color(color)).truncate(),
    )
    .on_hover_text(message);
    let count = if filtered {
        format!("{} résultats · {} objets chargés", st.view.results.len(), inv.item_count())
    } else {
        format!("{} objets chargés", inv.item_count())
    };
    ui.add_sized(
        [ui.available_width(), 20.0],
        egui::Label::new(RichText::new(count).size(12.0).color(p.muted)).truncate(),
    );
}

fn request_search(inv: &mut Inventory, root: Uuid, st: &mut InventoryUi, prefs: &InventoryPreferences) {
    let library = root == inv.root && prefs.search_library;
    let key = (root, inv.generation, library, prefs.search_trash, prefs.search_outfits);
    if st.fetch_generation == Some(key) && !st.fetch_all {
        return;
    }
    if st
        .fetch_poll
        .is_some_and(|poll| poll.elapsed() < std::time::Duration::from_millis(250))
    {
        return;
    }
    // Same bounded FetchInventoryDescendents2 batches as request_tree, with
    // explicit search scopes. Hidden trash/outfit subtrees aren't fetched.
    let mut todo = vec![root];
    if library {
        todo.push(inv.lib_root);
    }
    let mut seen = HashSet::new();
    let mut requests = Vec::new();
    let mut complete = true;
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(folder) = inv.folders.get(&id) else {
            continue;
        };
        let kind = folder.info.type_default;
        if kind == 50 || (kind == 14 && !prefs.search_trash) || (matches!(kind, 46 | 48) && !prefs.search_outfits) {
            continue;
        }
        complete &= folder.state == FetchState::Fetched;
        if matches!(folder.state, FetchState::Unknown | FetchState::Failed) && inv.queue.len() + requests.len() < 16 {
            requests.push(id);
        }
        todo.extend(folder.children.iter().copied());
    }
    for id in requests {
        inv.request(id);
    }
    st.fetch_all = !complete;
    st.fetch_generation = Some(key);
    st.fetch_poll = Some(std::time::Instant::now());
}

#[allow(clippy::too_many_arguments)]
fn search_scroll(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
    height: f32,
) -> egui::scroll_area::ScrollAreaOutput<()> {
    tree_spacing(ui);
    let total = st.result_tree.rows.len();
    let scroll = list_scroll(ui, height);
    if total == 0 {
        scroll.show(ui, |ui| {
            ui.label(
                RichText::new("Aucun résultat parmi les dossiers déjà chargés.")
                    .size(12.0)
                    .color(p.muted),
            );
        })
    } else {
        scroll.show_rows(ui, 20.0, total, |ui, range| {
            ui.set_min_width(st.result_width);
            search_rows(ui, p, icons, inv, st, prefs, facts, actions, range);
            ui.set_min_width(st.result_width);
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn search_rows(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
    range: std::ops::Range<usize>,
) {
    tree_spacing(ui);
    // Keep the complete order for Shift selection, while drawing only rows
    // intersecting the viewport. No arbitrary 500-item result limit.
    let rows = st.result_tree.rows[range].to_vec();
    for row in rows {
        // Measure only viewport rows. Retain their natural width while scrolling
        // vertically, so the horizontal range doesn't vanish with a long label.
        // A rebuilt tree clears it; the viewport width itself is never retained.
        let mut label = rules::name(inv, row.id);
        if row.folder {
            label = folder_name(inv, row.id);
            match inv.folders.get(&row.id).map(|folder| folder.state) {
                Some(FetchState::Fetching) => label.push_str("  (chargement…)"),
                Some(FetchState::Failed) => label.push_str("  (échec)"),
                _ => {}
            }
        } else if rules::original(inv, row.id).is_some_and(|id| facts.worn.contains(&id)) {
            label.push_str(" (porté)");
        }
        let label_width = ui.fonts_mut(|fonts| fonts.layout_no_wrap(label, egui::FontId::proportional(13.0), p.ink).size().x);
        let width = row.depth.min(24) as f32 * ui.spacing().indent + label_width + 40.0;
        st.result_width = st.result_width.max(width);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(row.depth.min(24) as f32 * ui.spacing().indent);
            if row.folder {
                let mut open = row.open;
                folder_row(ui, p, icons, inv, row.id, &mut open, false, st, prefs, facts, actions);
                if open != row.open {
                    st.result_tree.toggle(row.id, st.tab);
                }
            } else {
                item_row(ui, p, icons, inv, &row.id, st, prefs, facts, actions);
            }
        });
    }
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
        scroll_to_row(ui, &r, egui::Align::Center);
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

fn inventory_contents(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    controls::toolbar(ui, p, st, prefs);
    controls::search(ui, st, prefs, "Filtrer l'inventaire");
    ui.add_space(3.0);
    let mut tab_ids = vec![0];
    let mut labels = vec![("Inventaire", true)];
    for (id, label, show) in [
        (2, "Récent", prefs.show_recent),
        (3, "Porté", prefs.show_worn),
        (4, "Favoris", prefs.show_favorites),
    ] {
        if show {
            tab_ids.push(id);
            labels.push((label, true));
        }
    }
    let mut visible_tab = tab_ids.iter().position(|id| *id == st.tab).unwrap_or(0);
    super::widgets::tabs(ui, p, &mut visible_tab, &labels);
    if prefs.separate_searches && st.tab != tab_ids[visible_tab] {
        st.searches[st.tab.min(4)] = std::mem::take(&mut st.search);
        st.search.clone_from(&st.searches[tab_ids[visible_tab]]);
    }
    st.tab = tab_ids[visible_tab];
    let filtered = !st.search.trim().is_empty() || st.tab >= 2 || st.links_filter.is_some() || st.filters.active();
    if filtered {
        request_search(inv, inv.root, st, prefs);
        st.view.search(
            inv,
            inv.root,
            &st.search,
            prefs,
            &st.filters,
            st.tab,
            st.links_filter,
            facts,
            facts.agent,
            view::now(),
        );
    }
    ui.add_space(3.0);
    let list_h = list_height(ui);
    egui::Frame::new().fill(p.field).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if filtered {
            prepare_result_tree(st, inv, inv.root);
            search_scroll(ui, p, icons, inv, st, prefs, facts, actions, list_h);
            return;
        }
        list_scroll(ui, list_h).show(ui, |ui| {
            let root = inv.root;
            if root.is_nil() {
                ui.label(RichText::new("Inventaire non disponible.").color(p.muted));
            } else {
                let expand = st.expand_all.take();
                folder_tree(ui, p, icons, inv, root, 0, expand, st, prefs, facts, actions);
            }
        });
    });
    status(ui, p, inv, st, filtered);
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
    if matches!(prefs.search_field, view::SearchField::Creator | view::SearchField::All)
        && (!st.search.trim().is_empty() || st.windows.iter().any(|w| !w.state.search.trim().is_empty()))
    {
        if st.creator_generation != Some(world.inventory.generation) {
            st.creators = world
                .inventory
                .items
                .values()
                .map(|it| it.creator)
                .filter(|id| !id.is_nil())
                .collect();
            st.creator_generation = Some(world.inventory.generation);
        }
        for id in &st.creators {
            world.social.want_name(*id);
            facts.names.insert(*id, world.social.name_of(id));
        }
    }
    let inv = &mut world.inventory;
    st.previous_visible = std::mem::take(&mut st.visible);
    if st.fetch_all && st.search.trim().is_empty() && !st.filters.active() && st.links_filter.is_none() && st.tab < 2 {
        // Full fetches requested by mutation dialogs must keep running even
        // when the inventory window is closed, independently of search scopes.
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
        egui::pos2(screen.right() - 460.0, 56.0),
        Vec2::new(440.0, 550.0),
    )
    .help("Clic droit : actions de l’élément. Ctrl / Maj : sélection multiple.")
    .show(ctx, p, open, |ui| {
        inventory_contents(ui, p, icons, inv, st, prefs, &facts, &mut actions)
    });
    controls::dialogs(ctx, p, st, prefs);
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
        controls::dialogs(ctx, p, &mut w.state, prefs);
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

    fn view_inventory(large: bool) -> Inventory {
        let mut inv = Inventory {
            root: Uuid::from_u128(1),
            ..Default::default()
        };
        inv.folders.insert(
            inv.root,
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: inv.root,
                    name: "Mon inventaire".into(),
                    type_default: 8,
                    ..Default::default()
                },
                children: vec![],
                items: vec![],
                state: FetchState::Fetched,
                library: false,
            },
        );
        let agent = Uuid::from_u128(2);
        crate::world::inventory::demo::seed(&mut inv, agent);
        crate::world::inventory::demo::seed_view(&mut inv, agent, large);
        inv
    }

    #[test]
    fn long_names_allow_narrowing_the_inventory_and_folder_windows() {
        for (folder_window, tab, filtered) in [
            (false, 0, false),
            (false, 0, true),
            (false, 2, false),
            (false, 3, false),
            (true, 0, false),
            (true, 0, true),
        ] {
            let ctx = egui::Context::default();
            let mut inv = view_inventory(false);
            let agent = Uuid::from_u128(2);
            crate::world::inventory::demo::seed_long_names(&mut inv);
            let facts = Facts {
                agent,
                worn: inv.items.keys().copied().collect(),
                points: vec![],
                appearance_busy: false,
                names: Default::default(),
            };
            let palette = crate::theme::Theme::default().palette();
            let mut prefs = InventoryPreferences::default();
            let mut st = InventoryUi {
                tab,
                expand_all: Some(true),
                search: if filtered { "JEANS_SUBSTANCE".into() } else { String::new() },
                ..Default::default()
            };
            let mut frame = 0;
            let mut run = |events| {
                frame += 1;
                st.visible.clear();
                headless_output(ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                        time: Some(frame as f64 / 60.0),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        super::super::widgets::Floater::new(
                            "narrow_inventory",
                            "Inventaire",
                            egui::pos2(100.0, 56.0),
                            Vec2::new(600.0, 550.0),
                        )
                        .silent()
                        .show(ui.ctx(), &palette, &mut true, |ui| {
                            if folder_window {
                                folder_window_contents(
                                    ui,
                                    &palette,
                                    &Icons::default(),
                                    &mut inv,
                                    Uuid::from_u128(8000),
                                    &mut st,
                                    &mut prefs,
                                    &facts,
                                    &mut vec![],
                                );
                            } else {
                                inventory_contents(ui, &palette, &Icons::default(), &mut inv, &mut st, &mut prefs, &facts, &mut vec![]);
                            }
                        });
                    },
                ));
                ctx.memory(|memory| memory.area_rect(egui::Id::new("narrow_inventory")).unwrap())
            };
            let mut rect = run(vec![]);
            for _ in 0..8 {
                rect = run(vec![]);
            }
            let original_height = rect.height();
            let start = rect.right_center();
            let target = egui::pos2(rect.left() + 234.0, start.y);
            run(vec![egui::Event::PointerMoved(start)]);
            run(vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }]);
            run(vec![egui::Event::PointerMoved(target)]);
            for _ in 0..5 {
                run(vec![]);
            }
            run(vec![egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }]);
            for _ in 0..30 {
                rect = run(vec![]);
            }
            assert!(
                rect.width() <= 250.0,
                "folder={folder_window}, tab={tab}, filtered={filtered}: {rect:?}"
            );
            assert!(
                rect.height() <= original_height + 1.0,
                "narrowing must not make the window grow vertically"
            );
        }
    }

    #[test]
    fn filtered_horizontal_scroll_reaches_long_names_and_keeps_its_range_offscreen() {
        let ctx = egui::Context::default();
        let mut inv = view_inventory(true);
        let agent = Uuid::from_u128(2);
        inv.items.get_mut(&Uuid::from_u128(10000)).unwrap().name =
            "Élément avec un nom très long pour vérifier le défilement horizontal. ".repeat(4);
        inv.generation += 1;
        let facts = Facts {
            agent,
            worn: HashSet::new(),
            points: vec![],
            appearance_busy: false,
            names: Default::default(),
        };
        let mut prefs = InventoryPreferences::default();
        let mut st = InventoryUi {
            search: "Élément".into(),
            ..Default::default()
        };
        st.view
            .search(&inv, inv.root, &st.search, &prefs, &st.filters, 0, None, &facts, agent, view::now());
        prepare_result_tree(&mut st, &inv, inv.root);
        let mut run = || {
            st.visible.clear();
            let mut scroll = None;
            headless_output(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(260.0, 200.0))),
                    ..Default::default()
                },
                |ui| {
                    scroll = Some(search_scroll(
                        ui,
                        &crate::theme::Theme::default().palette(),
                        &Icons::default(),
                        &mut inv,
                        &mut st,
                        &mut prefs,
                        &facts,
                        &mut vec![],
                        180.0,
                    ));
                },
            ));
            assert!(st.visible.len() < 20, "horizontal scrolling preserves virtual drawing");
            scroll.unwrap()
        };
        let first = run();
        assert!(first.content_size.x > first.inner_rect.width() * 2.0);
        let mut state = first.state;
        state.offset = egui::vec2(100_000.0, 100_000.0);
        state.store(&ctx, first.id);
        let _ = run();
        let last = run();
        assert!(last.state.offset.x > 200.0, "the end of the long label is reachable");
        assert!(
            (last.content_size.x - first.content_size.x).abs() < 1.0,
            "the horizontal range survives virtual scrolling"
        );
        assert!(last.state.offset.y > 10_000.0, "the last results are reachable vertically");
    }

    #[test]
    fn revealing_a_long_name_keeps_the_folder_carets_in_view() {
        let ctx = egui::Context::default();
        let mut inv = view_inventory(false);
        let agent = Uuid::from_u128(2);
        crate::world::inventory::demo::seed_long_names(&mut inv);
        let facts = Facts {
            agent,
            worn: HashSet::new(),
            points: vec![],
            appearance_busy: false,
            names: Default::default(),
        };
        let mut prefs = InventoryPreferences::default();
        let mut st = InventoryUi::default();
        st.show_original(&inv, Uuid::from_u128(8100));
        for frame in 0..30 {
            let mut horizontal_offset = 0.0;
            headless_output(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(260.0, 200.0))),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                },
                |ui| {
                    let root = inv.root;
                    let scroll = list_scroll(ui, 180.0).show(ui, |ui| {
                        folder_tree(
                            ui,
                            &crate::theme::Theme::default().palette(),
                            &Icons::default(),
                            &mut inv,
                            root,
                            0,
                            None,
                            &mut st,
                            &mut prefs,
                            &facts,
                            &mut vec![],
                        );
                    });
                    assert!(
                        scroll.content_size.x > scroll.inner_rect.width(),
                        "frame {frame}: {:?}, {:?}",
                        scroll.content_size,
                        scroll.inner_rect
                    );
                    horizontal_offset = scroll.state.offset.x;
                },
            ));
            assert!(
                horizontal_offset.abs() < 0.5,
                "revealing an item must not pan away from the folder controls"
            );
        }
        assert!(st.reveal.is_none(), "the selected item was revealed");
    }

    #[test]
    fn inventory_size_stays_stable_during_loading_filtering_and_tab_changes() {
        let ctx = egui::Context::default();
        let mut inv = view_inventory(false);
        let agent = Uuid::from_u128(2);
        let loading = Uuid::from_u128(8001);
        inv.folders.get_mut(&loading).unwrap().state = FetchState::Unknown;
        let facts = Facts {
            agent,
            worn: inv.items.keys().copied().collect(),
            points: vec![],
            appearance_busy: false,
            names: Default::default(),
        };
        let palette = crate::theme::Theme::default().palette();
        let mut prefs = InventoryPreferences::default();
        let mut st = InventoryUi::default();
        let mut initial_size = None;
        for frame in 0..240 {
            st.visible.clear();
            st.tab = [2, 3, 0, 2][frame / 60];
            st.filters.types = if st.tab == 0 { 1 << 6 } else { u32::MAX };
            if frame == 120 {
                inv.folders.get_mut(&loading).unwrap().state = FetchState::Fetched;
                inv.generation += 1;
                st.fetch_poll = None;
            }
            if frame == 150 {
                st.message = "Un message de statut très long. ".repeat(30);
            }
            if frame == 180 {
                st.message.clear();
            }
            headless_output(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx();
                    super::super::widgets::Floater::new("inventory", "Inventaire", egui::pos2(800.0, 56.0), Vec2::new(440.0, 550.0))
                        .silent()
                        .show(ctx, &palette, &mut true, |ui| {
                            inventory_contents(ui, &palette, &Icons::default(), &mut inv, &mut st, &mut prefs, &facts, &mut vec![]);
                        });
                },
            ));
            let size = ctx.memory(|memory| memory.area_rect(egui::Id::new("inventory")).unwrap().size());
            if frame == 8 {
                initial_size = Some(size);
            }
            if let Some(initial) = initial_size {
                assert!(size.y <= initial.y + 1.0, "window grew at frame {frame}: {initial:?} -> {size:?}");
                assert!(
                    size.x <= initial.x + 1.0,
                    "window widened at frame {frame}: {initial:?} -> {size:?}"
                );
            }
        }
    }

    #[test]
    fn search_selector_is_right_of_the_field_and_tree_actions_share_the_toolbar() {
        let ctx = egui::Context::default();
        let palette = crate::theme::Theme::default().palette();
        let mut st = InventoryUi::default();
        let mut prefs = InventoryPreferences::default();
        let output = headless_output(ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(440.0, 220.0))),
                ..Default::default()
            },
            |ui| {
                controls::toolbar(ui, &palette, &mut st, &mut prefs);
                controls::search(ui, &mut st, &mut prefs, "Rechercher ici");
            },
        ));
        let position = |label| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => Some(text.pos),
                    _ => None,
                })
                .expect(label)
        };
        let field = position("Rechercher ici");
        let selector = position("Nom");
        assert!(selector.x > field.x);
        assert!((selector.y - field.y).abs() < 4.0);
        for label in ["Réduire", "Développer"] {
            assert!((position(label).y - position("Filtres").y).abs() < 1.0);
            assert!(position(label).x > position("Préférences").x);
        }
    }

    #[test]
    fn search_draws_only_the_viewport_and_can_reach_the_last_result() {
        let ctx = egui::Context::default();
        let mut inv = view_inventory(true);
        let agent = Uuid::from_u128(2);
        let facts = Facts {
            agent,
            worn: HashSet::new(),
            points: vec![],
            appearance_busy: false,
            names: Default::default(),
        };
        let mut prefs = InventoryPreferences::default();
        let mut st = InventoryUi {
            search: "Élément".into(),
            ..Default::default()
        };
        st.view
            .search(&inv, inv.root, &st.search, &prefs, &st.filters, 0, None, &facts, agent, view::now());
        assert_eq!(st.view.results.len(), 1500);
        prepare_result_tree(&mut st, &inv, inv.root);
        let last = *st.view.results.last().expect("last result");
        for offset in [0.0, 100_000.0, 100_000.0] {
            st.visible.clear();
            headless_output(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 220.0))),
                    ..Default::default()
                },
                |ui| {
                    tree_spacing(ui);
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .vertical_scroll_offset(offset)
                        .show_rows(ui, 20.0, st.result_tree.rows.len(), |ui, range| {
                            search_rows(
                                ui,
                                &crate::theme::Theme::default().palette(),
                                &Icons::default(),
                                &mut inv,
                                &mut st,
                                &mut prefs,
                                &facts,
                                &mut vec![],
                                range,
                            );
                        });
                },
            ));
            assert!(st.visible.len() < 20, "only viewport rows are built");
        }
        assert!(st.visible.contains(&last), "last result remains reachable");
    }

    #[test]
    fn enabling_trash_search_fetches_its_contents_and_scopes_invalidate_fetch_cache() {
        let mut inv = Inventory {
            root: Uuid::from_u128(80),
            ..Default::default()
        };
        inv.folders.insert(
            inv.root,
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: inv.root,
                    type_default: 8,
                    ..Default::default()
                },
                children: vec![],
                items: vec![],
                state: FetchState::Fetched,
                library: false,
            },
        );
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let trash = rules::system(&inv, 14).expect("trash");
        inv.folders.get_mut(&trash).expect("trash").state = FetchState::Unknown;
        let mut prefs = InventoryPreferences::default();
        let mut st = InventoryUi::default();
        let root = inv.root;
        request_search(&mut inv, root, &mut st, &prefs);
        assert!(!inv.queue.iter().any(|(id, _)| *id == trash));
        prefs.search_trash = true;
        st.fetch_poll = None;
        request_search(&mut inv, root, &mut st, &prefs);
        assert!(inv.queue.iter().any(|(id, _)| *id == trash));
    }

    #[test]
    fn double_click_preferences_add_objects_and_clothes_but_keep_body_replacements() {
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let prefs = InventoryPreferences {
            double_click_add_objects: true,
            double_click_add_clothes: true,
            ..Default::default()
        };
        for (id, replace) in [(8100, false), (8111, false), (8112, true)] {
            let mut actions = vec![];
            context::open(&inv, Uuid::from_u128(id), &mut InventoryUi::default(), &prefs, &mut actions);
            assert!(
                matches!(actions.as_slice(), [InvAction::Appearance(crate::world::appearance::Action::WearItem { replace: actual, .. })] if *actual == replace)
            );
        }
    }

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
                        agent: Uuid::nil(),
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
        assert_eq!(st.visible, vec![Uuid::from_u128(8001), Uuid::from_u128(8103)]);
        st.search = "sous-dossier".into();
        folder_window_frame(&ctx, &mut inv, root, &mut st);
        assert_eq!(st.visible, vec![Uuid::from_u128(8001)]);
        st.search = "annonces".into();
        folder_window_frame(&ctx, &mut inv, root, &mut st);
        assert!(st.visible.is_empty());
    }

    #[test]
    fn folder_window_fetches_its_hidden_root_and_limits_search_fetches_to_descendants() {
        for (search, fetch_all) in [("", false), ("instructions", false), ("", true)] {
            let ctx = egui::Context::default();
            let mut inv = Inventory::default();
            crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
            let root = Uuid::from_u128(8000);
            for id in [root, Uuid::from_u128(8001), Uuid::from_u128(8002)] {
                inv.folders.get_mut(&id).expect("folder").state = FetchState::Unknown;
            }
            let mut st = InventoryUi {
                search: search.into(),
                fetch_all,
                ..Default::default()
            };
            folder_window_frame(&ctx, &mut inv, root, &mut st);
            assert!(inv.queue.iter().any(|(id, _)| *id == root));
            assert!(!inv.queue.iter().any(|(id, _)| *id == Uuid::from_u128(8002)));
            assert_eq!(
                inv.queue.iter().any(|(id, _)| *id == Uuid::from_u128(8001)),
                !search.is_empty() || fetch_all
            );
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
                        agent: Uuid::nil(),
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
                        agent: Uuid::nil(),
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
                            agent: Uuid::nil(),
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
