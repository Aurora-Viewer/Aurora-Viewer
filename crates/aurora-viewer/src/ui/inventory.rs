//! Inventory window: lazily fetched folder tree, type icons, search.

use super::icons::Icons;
use crate::theme::Palette;
use crate::world::inventory::{FetchState, Inventory};
use crate::world::{World, inventory::actions as rules};
use aurora_net::inventory::operations::{Mutation, NewItem};
use egui::{RichText, Vec2};
use std::collections::HashSet;
use uuid::Uuid;
mod clipboard;
pub use clipboard::demo_events as clipboard_demo_events;
mod context;
mod controls;
mod delete;
pub use delete::demo_events as delete_demo_events;
mod dialogs;
mod document;
pub(crate) mod drag;
mod embedded;
mod markdown;
pub use markdown::View as NotecardView;
mod move_to;
pub use document::document_demo_events;
pub(crate) mod permissions;
mod progress;
pub(crate) mod properties;
mod rows;
pub(crate) mod task_drop;
pub mod thumbnail;
pub(crate) mod tree;
pub(crate) mod view;
pub use controls::preferences;
pub use view::InventoryPreferences;

// Keep the existing tab indices: slot 1 was unused after the single-tree layout.
const RECEIVED_TAB: usize = 1;

/// Size of the window before the user resizes it: one folder tree, like
/// Firestorm's (floater_my_inventory.xml, 333 x 570 with its smaller font).
const DEFAULT_SIZE: Vec2 = Vec2::new(440.0, 570.0);

/// The shared editor and unsaved-changes confirmation, without an inventory
/// tree. Object notecards keep their own item model and document state.
pub fn show_document(
    ctx: &egui::Context,
    p: &Palette,
    icons: &Icons,
    inv: &Inventory,
    st: &mut InventoryUi,
    drag_inventory: &Inventory,
    images: &std::collections::HashMap<Uuid, egui::TextureHandle>,
    prefs: &mut InventoryPreferences,
    script_settings: &super::scripts::settings::ScriptSettings,
) -> Vec<InvAction> {
    let mut actions = Vec::new();
    let facts = Facts {
        agent: Uuid::nil(),
        worn: Default::default(),
        active_gestures: Default::default(),
        worn_labels: Default::default(),
        points: Vec::new(),
        appearance_busy: false,
        touchable: Default::default(),
        names: Default::default(),
    };
    dialogs::preview(ctx, p, icons, inv, drag_inventory, st, images, prefs, script_settings, &mut actions);
    dialogs::show(ctx, p, icons, inv, st, prefs, &facts, images, &mut actions);
    actions
}

pub enum InvAction {
    TakeTask(Box<crate::build::task_inventory::TransferRequest>),
    TeleportLandmark(Uuid),
    /// "About Landmark" (LLLandmarkBridge "about"): its profile, as
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
    /// Open a sky or water settings item in its editor (LLSettingsBridgeAction).
    EditEnvironment(Uuid),
    Gesture(crate::ui::gestures::Action),
    Embedded {
        editor: Uuid,
        notecard: Uuid,
        marker: char,
        copy: bool,
    },
    SaveContent {
        editor: Uuid,
        item: Uuid,
        text: String,
        close: bool,
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

#[derive(Default)]
pub struct Facts {
    pub agent: Uuid,
    pub worn: HashSet<Uuid>,
    pub active_gestures: std::collections::BTreeSet<Uuid>,
    pub worn_labels: std::collections::HashMap<Uuid, String>,
    pub points: Vec<(u8, bool, String)>,
    pub appearance_busy: bool,
    /// Worn objects that handle touches (enable_attachment_touch).
    pub touchable: HashSet<Uuid>,
    pub names: std::collections::HashMap<Uuid, String>,
}

impl Facts {
    pub fn from_world(world: &World, appearance_busy: bool) -> Self {
        let worn_labels = super::appearance::items::worn_labels(world);
        let worn = worn_labels.keys().copied().collect();
        let mut points: Vec<_> = world
            .avatar_lib
            .attach_points
            .iter()
            .map(|(id, ap)| {
                let name = world
                    .avatar_lib
                    .attach_names
                    .get(id)
                    .map(String::as_str)
                    .unwrap_or(aurora_i18n::tr!("ui-inventory-attachment"));
                (*id, ap.hud, super::appearance::items::point_name(name).to_owned())
            })
            .collect();
        points.sort_by_cached_key(|(_, _, name)| name.to_lowercase());
        Self {
            agent: world.agent_id,
            worn,
            active_gestures: world.active_gestures.clone(),
            worn_labels,
            points,
            appearance_busy,
            touchable: world.touchable_attachment_items(),
            names: Default::default(),
        }
    }

    fn worn_label(&self, inv: &Inventory, id: Uuid) -> Option<&str> {
        let original = rules::original(inv, id)?;
        if self.active_gestures.contains(&original) {
            return Some(aurora_i18n::tr!("gesture-active"));
        }
        self.worn.contains(&original).then(|| {
            self.worn_labels
                .get(&original)
                .map(String::as_str)
                .unwrap_or(aurora_i18n::tr!("common-worn"))
        })
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
    CopyEmbedded(Uuid, char),
    SaveEmbedded(Uuid),
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
    /// Nil for a standalone document, otherwise a partial inventory tree.
    pub root: Uuid,
    pub state: Box<InventoryUi>,
    pub open: bool,
}

/// Inventory cargo shared by world placement and the build contents floater.
#[derive(Debug, Clone, Copy)]
pub struct InvDrag(pub Uuid);

/// A single folder dragged to inventory or Marketplace listings.
#[derive(Debug, Clone, Copy)]
pub struct FolderDrag(pub Uuid);

pub(crate) struct WorldDrag {
    pub item: Uuid,
    pub position: egui::Pos2,
    pub released: bool,
    pub ctrl: bool,
}

/// Read before egui's end-pass clears released cargo. A dragging widget keeps
/// wants-pointer set even over the world; only the destination area counts.
pub(crate) fn world_drag(ctx: &egui::Context) -> Option<WorldDrag> {
    let cargo = egui::DragAndDrop::payload::<InvDrag>(ctx)?;
    if ctx.is_pointer_over_egui() {
        return None;
    }
    ctx.input(|i| {
        Some(WorldDrag {
            item: cargo.0,
            position: i.pointer.interact_pos()?,
            released: i.pointer.button_released(egui::PointerButton::Primary),
            ctrl: i.modifiers.ctrl,
        })
    })
}

#[derive(Default)]
pub struct InventoryUi {
    pub filters_open: bool,
    pub preferences_open: bool,
    filters_initialized: bool,
    filters: view::Filters,
    saved_filters: Option<view::Filters>,
    view: view::ViewCache,
    plain: rows::PlainTree,
    result_tree: view::TreeState,
    result_width: f32,
    creator_generation: Option<u64>,
    creators: HashSet<Uuid>,
    fetch_poll: Option<std::time::Instant>,
    fetch_generation: Option<(Uuid, u64, bool, bool, bool)>,
    /// The clipboard's folders were all loaded at this inventory generation.
    clipboard_loaded: Option<(u64, Vec<Uuid>)>,
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
    /// Received Items folders already shown once (closed on their arrival).
    received_seen: HashSet<Uuid>,
    pub selection: HashSet<Uuid>,
    anchor: Option<Uuid>,
    visible: Vec<Uuid>,
    pub clipboard: Vec<Uuid>,
    pub cut: bool,
    pub consume_clipboard: bool,
    pub pending_refresh: Vec<Uuid>,
    pub created_selection: Option<Uuid>,
    pub merchant_requested: bool,
    pub merchant: bool,
    pub pending: Option<Uuid>,
    progress: Option<progress::State>,
    copy_jobs: progress::CopyJobs,
    notice: Option<(String, std::time::Instant)>,
    pub message: String,
    pub dialog: Option<EditDialog>,
    move_to: Option<move_to::MoveTo>,
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
    pub preview_source: Option<aurora_net::inventory::notecard::Notecard>,
    /// Seeded from the shared preference on first display, then kept per window.
    pub notecard_view: Option<NotecardView>,
    notecard_markdown: markdown::Cache,
    pub attachment_preview: Option<aurora_net::inventory::InvItem>,
    pub preview_original: String,
    pub preview_message: String,
    pub content_save: Option<(Uuid, Vec<u8>, String)>,
    pub close_after_save: bool,
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
    pub fn begin_move_to(&mut self, id: Uuid) {
        self.move_to = Some(move_to::MoveTo::new(self.selected_ids(id)));
    }

    pub fn filter_move_to(&mut self, filter: &str) {
        if let Some(picker) = &mut self.move_to {
            picker.filter = filter.into();
        }
    }

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

    /// Open these folders (a demo scenario's starting state).
    pub(crate) fn open_folders(&mut self, ids: impl IntoIterator<Item = Uuid>) {
        self.plain.open_all(ids);
    }

    /// Reveal a folder with its contents expanded for direct inventory management.
    pub fn show_folder(&mut self, inv: &Inventory, id: Uuid) {
        self.show_original(inv, id);
        self.reveal_path.insert(id);
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
    /// The list is the flattened result tree, drawn row by row in sight only
    /// (`search_scroll`): a search or a filter, and every tab but the plain
    /// inventory — Received Items included, whose purchases pile up into
    /// thousands of rows.
    fn result_view(&self) -> bool {
        !self.search.trim().is_empty() || self.filters.active() || self.links_filter.is_some() || self.tab >= RECEIVED_TAB
    }

    fn select(&mut self, ui: &egui::Ui, id: Uuid, secondary: bool) {
        ui.memory_mut(|m| m.request_focus(clipboard::focus_id(self)));
        if secondary && self.selection.contains(&id) {
            self.selected = Some(id);
            return;
        }
        let mods = ui.input(|i| i.modifiers);
        if mods.shift && !secondary {
            let order = if self.result_view() {
                &self.result_tree.order
            } else {
                &self.plain.order
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

use tree::folder_icon;

/// II_FLAGS_OBJECT_HAS_MULTIPLE_ITEMS / LLInventoryIcon::getIconName
/// (Firestorm indra/{llinventory,newview}, originally LGPL 2.1).
pub const OBJECT_HAS_MULTIPLE_ITEMS: u32 = 0x200000;

pub fn item_icon(asset_type: i32, inv_type: i32, flags: u32) -> &'static str {
    match (asset_type, inv_type) {
        (6, _) if flags & OBJECT_HAS_MULTIPLE_ITEMS != 0 => "Inv_Object_Multi",
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
    tree::icon(ui, icons, name, p, None);
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
        icon(ui, icons, item_icon(it.asset_type, it.inv_type, it.flags), p);
        if rename_row(ui, p, inv, *id, st, prefs, actions) {
            return;
        }
        let worn = facts.worn_label(inv, *id);
        let suffix = worn.map_or_else(String::new, |label| format!(" ({label})"));
        let font_id = egui::FontId::proportional(13.0);
        let mut label = super::widgets::name_with_suffix(&it.name, &suffix, font_id.clone(), p.ink);
        let name = worn.map(|_| ui.painter().layout_no_wrap(it.name.clone(), font_id, p.ink));
        let permissions = permissions::Permissions::inventory(inv, *id);
        if let Some(permissions) = permissions {
            // Keep each item's rights next to its name: a very long sibling
            // must not push every row's badges outside the horizontal viewport.
            permissions.append_suffix(&mut label, p);
        }
        let r = ui
            .push_id(("inv_item", it.id), |ui| {
                super::widgets::inventory_item_label(ui, label, st.selection.contains(id), egui::Sense::click_and_drag(), name)
            })
            .inner;
        // One response handles clicks and dragging; a separate drag-source
        // interaction overlay used to consume secondary clicks on the label.
        drag::source(ui, &r, it.id, false, st);
        clipboard::demo_row(&r, st, it.id);
        task_drop::target(ui, p, &r, inv, it.parent, prefs, actions);
        drag::target(ui, p, &r, inv, it.parent, st, prefs, facts, actions);
        super::widgets::demo::rez_row(&r, it.id);
        super::widgets::demo::worn_row(&r, it.id);
        let r = r.on_hover_ui(|ui| {
            ui.label(&it.name);
            ui.label(inventory_path(inv, it.parent));
            ui.label(if it.created_at > 0 {
                aurora_net::inventory::created_date(it.created_at)
            } else {
                aurora_i18n::tr!("ui-inventory-unknown-date").into()
            });
            if !it.desc.is_empty() {
                ui.label(&it.desc);
            }
            if let Some(permissions) = permissions {
                crate::ui::widgets::divider(ui, p);
                ui.label(permissions.tooltip());
            }
        });
        if r.clicked() || r.secondary_clicked() {
            st.select(ui, *id, r.secondary_clicked());
        }
        if st.reveal == Some(it.id) {
            scroll_to_row(ui, &r, egui::Align::Center);
            st.reveal = None;
            st.reveal_path.clear();
        }
        if r.double_clicked() {
            context::double_click(inv, *id, st, prefs, facts, actions);
        }
        super::menu::context_menu(&r, p, |ui| context::show(ui, p, inv, *id, st, prefs, facts, actions));
    });
}

fn folder_row(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &Inventory,
    id: Uuid,
    open: &mut bool,
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
        let resp = tree::caret(ui, p, *open);
        clipboard::demo_caret(&resp, state, id);
        if resp.clicked() {
            *open = !*open;
        }
        icon(ui, icons, folder_icon(folder.info.type_default, *open), p);
        let label = match fetch_state {
            FetchState::Fetching => aurora_i18n::tr!("status-name-loading", name = name.to_string()),
            FetchState::Failed => aurora_i18n::tr!("ui-inventory-failed-2", name = name.to_string()),
            _ => name.clone(),
        };
        state.visible.push(id);
        if rename_row(ui, p, inv, id, state, prefs, actions) {
            return;
        }
        // Personal folders can move within inventory or to Marketplace listings.
        let draggable = rules::movable(inv, id, &prefs.protected) && folder.info.type_default == -1;
        let sense = if draggable {
            egui::Sense::click_and_drag()
        } else {
            egui::Sense::click()
        };
        let r = tree::label(ui, tree::name(&label, p), state.selection.contains(&id), sense, None);
        if draggable {
            drag::source(ui, &r, id, true, state);
        } else {
            drag::record_row(ui.ctx(), state.ui_id, id, r.rect);
        }
        clipboard::demo_row(&r, state, id);
        task_drop::target(ui, p, &r, inv, id, prefs, actions);
        // Include the caret and icon while dragging, keeping their normal
        // click gestures independent from selection and drag initiation.
        let mut destination = r.clone();
        destination.rect = destination.rect.union(resp.rect);
        drag::target(ui, p, &destination, inv, id, state, prefs, facts, actions);
        drag::navigation::folder(ui, &destination, state.ui_id, id, open);
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
        aurora_i18n::tr!("common-my-inventory").to_owned()
    } else if id == inv.lib_root {
        aurora_i18n::tr!("common-library").to_owned()
    } else {
        rules::name(inv, id)
    }
}

pub(crate) fn inventory_path(inv: &Inventory, mut id: Uuid) -> String {
    let mut parts = Vec::new();
    let mut seen = HashSet::new();
    while seen.insert(id) && inv.folders.contains_key(&id) {
        parts.push(folder_name(inv, id));
        id = rules::parent(inv, id).unwrap_or(Uuid::nil());
    }
    parts.reverse();
    parts.join(" / ")
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
    controls::search(ui, p, st, prefs, aurora_i18n::tr!("ui-inventory-filter-inventory-folder"));
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
    let zone = egui::Frame::new().fill(p.field).show(ui, |ui| {
        clipboard::list_focus(ui, st, list_h);
        if filtered {
            prepare_result_tree(st, inv, root);
            search_scroll(ui, p, icons, inv, st, prefs, facts, actions, list_h);
            return;
        }
        rows::scroll(ui, p, icons, inv, st, prefs, facts, actions, list_h, root, false);
    });
    task_drop::target(ui, p, &zone.response, inv, root, prefs, actions);
    drag::target(ui, p, &zone.response, inv, root, st, prefs, facts, actions);
    status(ui, p, inv, st, prefs, facts, actions, filtered);
    clipboard::shortcuts(ui, inv, root, st, prefs, facts, actions);
}

use tree::spacing as tree_spacing;

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
use tree::{height as list_height, scroll as list_scroll};

fn scroll_to_row(ui: &egui::Ui, row: &egui::Response, align: egui::Align) {
    // Reveal vertically while keeping folder carets and the start of names in
    // view. Aligning a whole, potentially very wide label would also pan right.
    let rect = egui::Rect::from_x_y_ranges(ui.clip_rect().x_range(), row.rect.y_range());
    ui.scroll_to_rect(rect, Some(align));
}

fn status(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
    filtered: bool,
) {
    let now = std::time::Instant::now();
    let pending_message = st.busy().then(|| st.pending_status(now));
    let (message, color) = if !st.message.is_empty() {
        (st.message.as_str(), p.warn)
    } else if let Some(message) = &pending_message {
        (message.as_str(), p.ink)
    } else if let Some((message, finished)) = &st.notice
        && now.duration_since(*finished) < progress::NOTICE_DURATION
    {
        (message.as_str(), p.success)
    } else if st.fetch_all {
        (aurora_i18n::tr!("ui-inventory-search-loading-folders"), p.muted)
    } else {
        ("", p.muted)
    };
    let message = message.to_owned();
    let count = if filtered {
        aurora_i18n::tr!(
            "ui-inventory-results-objects-loaded",
            value0 = format!("{}", st.view.results.len()),
            value1 = format!("{}", inv.item_count())
        )
    } else {
        aurora_i18n::tr!("ui-inventory-objects-loaded", value0 = format!("{}", inv.item_count()))
    };
    ui.horizontal(|ui| {
        let to = st
            .selected
            .filter(|id| inv.folders.contains_key(id))
            .or_else(|| st.selected.and_then(|id| rules::parent(inv, id)))
            .unwrap_or(inv.root);
        super::widgets::icon_menu(ui, p, "plus", aurora_i18n::tr!("common-new-folder"), |ui| {
            context::new_menu(
                ui,
                p,
                to,
                st.pending.is_none() && rules::destination(inv, to, &prefs.protected),
                st,
                prefs,
                actions,
            );
        });
        ui.label(
            RichText::new(aurora_i18n::tr!("workspace-inventory-selected", count = st.selection.len()))
                .size(11.0)
                .color(p.muted),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            delete::button(ui, p, inv, st, prefs, facts);
            if st.busy() {
                ui.add(egui::Spinner::new().size(14.0).color(p.violet));
            }
            let label = if message.is_empty() { &count } else { &message };
            ui.add(egui::Label::new(RichText::new(label).size(11.0).color(color)).truncate())
                .on_hover_text(if message.is_empty() { count } else { format!("{message}\n{count}") });
        });
    });
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
        if (kind == 50 && id != root) || (kind == 14 && !prefs.search_trash) || (matches!(kind, 46 | 48) && !prefs.search_outfits) {
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
    let mut out = if total == 0 {
        scroll.show(ui, |ui| {
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-inventory-no-results-in-the-loaded-folders"))
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
    };
    drag::navigation::scroll(ui, &mut out, st.ui_id);
    out
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
        let width = row_width(ui, p, inv, facts, row.id, row.folder, row.depth);
        st.result_width = st.result_width.max(width);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(row.depth.min(24) as f32 * ui.spacing().indent);
            if row.folder {
                let mut open = row.open;
                folder_row(ui, p, icons, inv, row.id, &mut open, st, prefs, facts, actions);
                if open != row.open {
                    st.result_tree.toggle(row.id, st.tab);
                }
            } else {
                item_row(ui, p, icons, inv, &row.id, st, prefs, facts, actions);
            }
        });
    }
}

/// The natural width of a row of the virtual lists: its indentation, its
/// label with the worn or loading suffix and the permissions, the caret and
/// the icon.
fn row_width(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, facts: &Facts, id: Uuid, folder: bool, depth: usize) -> f32 {
    let mut label = rules::name(inv, id);
    if folder {
        label = folder_name(inv, id);
        match inv.folders.get(&id).map(|folder| folder.state) {
            Some(FetchState::Fetching) => label.push_str(aurora_i18n::tr!("status-loading-3")),
            Some(FetchState::Failed) => label.push_str(aurora_i18n::tr!("ui-inventory-failed")),
            _ => {}
        }
    } else if let Some(worn) = facts.worn_label(inv, id) {
        label.push_str(&format!(" ({worn})"));
    }
    let mut label = super::widgets::name_with_suffix(&label, "", egui::FontId::proportional(13.0), p.ink);
    if !folder && let Some(permissions) = permissions::Permissions::inventory(inv, id) {
        permissions.append_suffix(&mut label, p);
    }
    let label_width = ui.fonts_mut(|fonts| fonts.layout_job(label).size().x);
    depth.min(24) as f32 * ui.spacing().indent + label_width + 40.0
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
        Ok(_) => st.message = aurora_i18n::tr!("ui-inventory-wait-for-the-current-change-to-finish").into(),
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
    controls::search(ui, p, st, prefs, aurora_i18n::tr!("ui-inventory-filter-inventory"));
    ui.add_space(3.0);
    let mut tab_ids = vec![0];
    let mut labels = vec![(aurora_i18n::tr!("common-inventory"), true)];
    for (id, label, show) in [
        (2, aurora_i18n::tr!("common-recent"), prefs.show_recent),
        (3, aurora_i18n::tr!("common-worn"), prefs.show_worn),
        (4, aurora_i18n::tr!("common-favorites"), prefs.show_favorites),
        (RECEIVED_TAB, aurora_i18n::tr!("ui-inventory-received-purchases"), true),
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
    // LLSidepanelInventory::observeInboxModifications/onInboxChanged
    // (indra/newview/llsidepanelinventory.cpp): scope and fetch the full FT_INBOX.
    // Aurora presents it as a tab, as requested, instead of a second bottom pane.
    let root = if st.tab == RECEIVED_TAB {
        rules::system(inv, 50).unwrap_or_default()
    } else {
        inv.root
    };
    let filtered = st.result_view();
    if root.is_nil() {
        st.fetch_all = false;
        st.view.results.clear();
    } else if filtered {
        request_search(inv, root, st, prefs);
    }
    if filtered && !root.is_nil() {
        st.view.search(
            inv,
            root,
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
    ui.add_space(6.0);
    let list_h = list_height(ui);
    let zone = tree::frame(ui, p, |ui| {
        clipboard::list_focus(ui, st, list_h);
        if root.is_nil() {
            list_scroll(ui, list_h).show(ui, |ui| {
                let text = if st.tab == RECEIVED_TAB {
                    aurora_i18n::tr!("ui-inventory-no-received-purchases")
                } else {
                    aurora_i18n::tr!("ui-inventory-inventory-unavailable")
                };
                ui.label(RichText::new(text).color(p.muted));
            });
            return;
        }
        if filtered {
            if st.tab == RECEIVED_TAB {
                // LLInboxInventoryPanel: the purchases are ordinary folder
                // views, closed until opened; each one closes once, when it
                // first shows up, and then keeps what the resident chose
                let fresh: Vec<Uuid> = st
                    .view
                    .result_folders()
                    .filter(|id| *id != root && st.received_seen.insert(*id))
                    .collect();
                if !fresh.is_empty() {
                    st.result_tree.collapse_folders(fresh, RECEIVED_TAB);
                }
            }
            prepare_result_tree(st, inv, root);
            search_scroll(ui, p, icons, inv, st, prefs, facts, actions, list_h);
            return;
        }
        rows::scroll(ui, p, icons, inv, st, prefs, facts, actions, list_h, root, true);
    });
    task_drop::target(ui, p, &zone.response, inv, root, prefs, actions);
    drag::target(ui, p, &zone.response, inv, root, st, prefs, facts, actions);
    status(ui, p, inv, st, prefs, facts, actions, filtered);
    clipboard::shortcuts(ui, inv, root, st, prefs, facts, actions);
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
    script_settings: &super::scripts::settings::ScriptSettings,
) -> Vec<InvAction> {
    let mut actions = Vec::new();
    // What is worn, the attachment points and the names are read for the
    // windows and dialogs that show them, not every frame while closed.
    let shown = *open || st.dialog.is_some() || st.move_to.is_some() || !st.windows.is_empty() || st.demo_menu.is_some();
    let mut facts = if shown {
        Facts::from_world(world, appearance_busy)
    } else {
        Facts::default()
    };
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
                        .unwrap_or_else(|| aurora_i18n::tr!("common-loading-2").into()),
                );
            }
        }
        for window in &st.windows {
            property_names(&window.state, world, names);
        }
    }
    property_names(st, world, &mut facts.names);
    if shown
        && matches!(prefs.search_field, view::SearchField::Creator | view::SearchField::All)
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
    st.visible.clear();
    if st.fetch_all && st.search.trim().is_empty() && !st.filters.active() && st.links_filter.is_none() && st.tab == 0 {
        // Full fetches requested by mutation dialogs must keep running even
        // when the inventory window is closed, independently of search scopes.
        st.fetch_all = !rules::request_tree(inv, inv.root);
    }
    // Copied folders load their whole tree for the paste; once loaded, they
    // are not walked again every frame until the inventory changes.
    if st
        .clipboard_loaded
        .as_ref()
        .is_none_or(|(generation, ids)| *generation != inv.generation || *ids != st.clipboard)
    {
        let mut loaded = true;
        for id in &st.clipboard {
            if inv.folders.contains_key(id) {
                loaded &= rules::request_tree(inv, *id);
            }
        }
        st.clipboard_loaded = loaded.then(|| (inv.generation, st.clipboard.clone()));
    }
    let screen = ctx.content_rect();
    super::widgets::Floater::new(
        "inventory",
        aurora_i18n::tr!("common-inventory"),
        egui::pos2(screen.right() - DEFAULT_SIZE.x - 20.0, 56.0),
        DEFAULT_SIZE,
    )
    .compact_header()
    .help(aurora_i18n::tr!("ui-inventory-right-click-item-actions-ctrl-shift-multi-select"))
    .show(ctx, p, open, |ui| {
        inventory_contents(ui, p, icons, inv, st, prefs, &facts, &mut actions)
    });
    controls::dialogs(ctx, p, st, prefs);
    finish_hidden_rename(inv, st, prefs, &mut actions);
    dialogs::show(ctx, p, icons, inv, st, prefs, &facts, images, &mut actions);
    move_to::show(ctx, p, icons, inv, st, prefs, &facts, &mut actions);
    dialogs::preview(ctx, p, icons, inv, inv, st, images, prefs, script_settings, &mut actions);
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
        if !w.root.is_nil() && !world.inventory.folders.contains_key(&w.root) {
            w.open = false;
            continue;
        }
        w.state.ui_id = w.id;
        w.state.visible.clear();
        w.state.clipboard.clone_from(&st.clipboard);
        w.state.cut = st.cut;
        w.state.pending = st.pending;
        w.state.progress.clone_from(&st.progress);
        w.state.copy_jobs.clone_from(&st.copy_jobs);
        w.state.notice.clone_from(&st.notice);
        w.state.merchant = st.merchant;
        w.state.playing = st.playing;
        let before = (w.state.clipboard.clone(), w.state.cut);
        let title = aurora_i18n::tr!("ui-inventory-inventory", value0 = folder_name(&world.inventory, w.root).to_string());
        let window_id = format!("inventory_{}", w.id);
        super::widgets::Floater::new(
            &window_id,
            title,
            screen.center() - egui::vec2(210.0, 220.0),
            Vec2::new(420.0, 440.0),
        )
        .compact_header()
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
        move_to::show(ctx, p, icons, &mut world.inventory, &mut w.state, prefs, &facts, &mut actions);
        dialogs::preview(
            ctx,
            p,
            icons,
            &world.inventory,
            &world.inventory,
            &mut w.state,
            images,
            prefs,
            script_settings,
            &mut actions,
        );
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
    windows.retain(|w| {
        w.open || w.state.preview.is_some() || w.state.dialog.is_some() || w.state.move_to.is_some() || w.state.thumbnail.item.is_some()
    });
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
mod drag_tests;
#[cfg(test)]
mod texture_tests;

#[cfg(test)]
fn headless_output(mut output: egui::FullOutput) -> egui::FullOutput {
    // These tests inspect shapes/events without a renderer to apply texture uploads.
    output.textures_delta.clear();
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_group_icons_follow_item_flags_without_changing_other_types() {
        assert_eq!(item_icon(6, 6, 0), "Inv_Object");
        assert_eq!(item_icon(6, 6, OBJECT_HAS_MULTIPLE_ITEMS | 0x100), "Inv_Object_Multi");
        assert_eq!(item_icon(6, 6, 0x100), "Inv_Object");
        assert_eq!(item_icon(0, 0, OBJECT_HAS_MULTIPLE_ITEMS), "Inv_Texture");
        assert_eq!(item_icon(0, 15, 0), "Inv_Snapshot");
        assert_eq!(item_icon(24, 24, OBJECT_HAS_MULTIPLE_ITEMS), "Inv_LinkItem");
    }

    #[test]
    fn worn_labels_follow_actual_points_and_inventory_links() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = crate::demo::DEMO_AGENT;
        crate::world::inventory::demo::seed(&mut world.inventory, world.agent_id);
        world.objects.upsert(1, crate::demo::action_avatar(false));
        let attach = |item: u128, point| {
            let mut object = crate::demo::hud::object(point, false);
            object.full_id = Uuid::from_u128(item);
            object.name_values = format!("AttachItemID STRING RW SV {}", object.full_id);
            object
        };
        let object = Uuid::from_u128(8100);
        let link = Uuid::from_u128(8113);
        let hud = Uuid::from_u128(8110);
        let unknown = Uuid::from_u128(8109);
        world.objects.upsert(1, attach(8100, 6));
        world.objects.upsert(1, attach(8110, 32));
        world.objects.upsert(1, attach(8109, 250));
        let facts = Facts::from_world(&world, false);
        assert_eq!(
            facts.worn_label(&world.inventory, object),
            Some(
                aurora_i18n::tr!(
                    "ui-appearance-items-worn-on",
                    value0 = "",
                    name = aurora_i18n::tr!("ui-appearance-items-right-hand")
                )
                .as_str()
            )
        );
        assert_eq!(
            facts.worn_label(&world.inventory, link),
            Some(
                aurora_i18n::tr!(
                    "ui-appearance-items-worn-on",
                    value0 = "",
                    name = aurora_i18n::tr!("ui-appearance-items-right-hand")
                )
                .as_str()
            )
        );
        assert_eq!(
            facts.worn_label(&world.inventory, hud),
            Some(
                aurora_i18n::tr!(
                    "ui-appearance-items-worn-on",
                    value0 = "HUD : ",
                    name = aurora_i18n::tr!("ui-appearance-items-top-right")
                )
                .as_str()
            )
        );
        assert_eq!(
            facts.worn_label(&world.inventory, unknown),
            Some(
                aurora_i18n::tr!(
                    "ui-appearance-items-worn-on",
                    value0 = "",
                    name = aurora_i18n::tr!("attachment-point", point = 250)
                )
                .as_str()
            )
        );
        assert_eq!(
            facts.worn_label(&world.inventory, Uuid::from_u128(714)),
            Some(aurora_i18n::tr!("common-worn"))
        );
        assert_eq!(
            facts.worn_label(&world.inventory, Uuid::from_u128(717)),
            None,
            "COF alone does not attach an object"
        );
        let idx = world.objects.index_of_uuid(&object).expect("attached object");
        world.objects.get_mut(idx).expect("attached object").state = 5_u8.rotate_left(4);
        let facts = Facts::from_world(&world, false);
        assert_eq!(
            facts.worn_label(&world.inventory, link),
            Some(
                aurora_i18n::tr!(
                    "ui-appearance-items-worn-on",
                    value0 = "",
                    name = aurora_i18n::tr!("ui-appearance-items-left-hand-2")
                )
                .as_str()
            )
        );
        world.objects.remove_idx(idx);
        let facts = Facts::from_world(&world, false);
        assert_eq!(facts.worn_label(&world.inventory, object), None);
        assert_eq!(facts.worn_label(&world.inventory, link), None);
    }

    pub(super) fn view_inventory(large: bool) -> Inventory {
        let mut inv = Inventory {
            root: Uuid::from_u128(1),
            ..Default::default()
        };
        inv.folders.insert(
            inv.root,
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: inv.root,
                    name: aurora_i18n::tr!("common-my-inventory").into(),
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
                worn_labels: Default::default(),
                points: vec![],
                appearance_busy: false,
                active_gestures: Default::default(),
                touchable: Default::default(),
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
                            aurora_i18n::tr!("common-inventory"),
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
        inv.items.get_mut(&Uuid::from_u128(10000)).unwrap().name = format!(
            "{}with a very long name to check horizontal scrolling. ",
            aurora_i18n::tr!("world-inventory-demo-item", n = "")
        )
        .repeat(4);
        inv.generation += 1;
        let facts = Facts {
            agent,
            worn: HashSet::new(),
            worn_labels: Default::default(),
            points: vec![],
            appearance_busy: false,
            active_gestures: Default::default(),
            touchable: Default::default(),
            names: Default::default(),
        };
        let mut prefs = InventoryPreferences::default();
        let mut st = InventoryUi {
            search: aurora_i18n::tr!("world-inventory-demo-item", n = "").trim().into(),
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
            worn_labels: Default::default(),
            points: vec![],
            appearance_busy: false,
            active_gestures: Default::default(),
            touchable: Default::default(),
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
                    let scroll = rows::scroll(
                        ui,
                        &crate::theme::Theme::default().palette(),
                        &Icons::default(),
                        &mut inv,
                        &mut st,
                        &mut prefs,
                        &facts,
                        &mut vec![],
                        180.0,
                        root,
                        true,
                    );
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
        for window_size in [DEFAULT_SIZE, Vec2::new(680.0, 610.0), Vec2::new(1000.0, 700.0)] {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let mut inv = view_inventory(false);
            let agent = Uuid::from_u128(2);
            let loading = Uuid::from_u128(8001);
            inv.folders.get_mut(&loading).unwrap().state = FetchState::Unknown;
            let facts = Facts {
                agent,
                worn: inv.items.keys().copied().collect(),
                worn_labels: Default::default(),
                points: vec![],
                appearance_busy: false,
                active_gestures: Default::default(),
                touchable: Default::default(),
                names: Default::default(),
            };
            let palette = crate::theme::Theme::default().palette();
            let mut prefs = InventoryPreferences::default();
            let mut st = InventoryUi {
                selected: Some(Uuid::from_u128(8100)),
                selection: HashSet::from([Uuid::from_u128(8100)]),
                ..Default::default()
            };
            let mut initial_size = None;
            for frame in 0..240 {
                st.visible.clear();
                st.tab = [2, 3, 0, 2][frame / 60];
                st.filters.types = if st.tab == 0 && frame >= 150 { 1 << 6 } else { u32::MAX };
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
                        super::super::widgets::Floater::new(
                            "inventory",
                            aurora_i18n::tr!("common-inventory"),
                            egui::pos2(1280.0 - window_size.x - 20.0, 56.0),
                            window_size,
                        )
                        .compact_header()
                        .silent()
                        .show(ctx, &palette, &mut true, |ui| {
                            inventory_contents(ui, &palette, &Icons::default(), &mut inv, &mut st, &mut prefs, &facts, &mut vec![]);
                        });
                    },
                ));
                if st.tab == 0 && !st.filters.active() {
                    assert!(
                        st.visible.contains(&inv.root),
                        "the complete tree stays available at every window width"
                    );
                }
                let size = ctx.memory(|memory| memory.area_rect(egui::Id::new("inventory")).unwrap().size());
                if frame == 8 {
                    assert!(
                        size.x <= window_size.x + 1.0 && size.y <= window_size.y + 1.0,
                        "initial layout exceeds requested size: {window_size:?} -> {size:?}"
                    );
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
    }

    #[test]
    fn search_selector_is_right_of_the_field_and_filters_share_the_toolbar() {
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
                controls::search(ui, &palette, &mut st, &mut prefs, "Rechercher ici");
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
        let selector = position(aurora_i18n::tr!("common-name"));
        assert!(selector.x > field.x);
        assert!((selector.y - field.y).abs() < 4.0);
        let filters = position(aurora_i18n::tr!("ui-inventory-controls-filters"));
        let preferences = position(aurora_i18n::tr!("common-preferences"));
        assert!((preferences.y - filters.y).abs() < 1.0);
        assert!(preferences.x > filters.x);
        let collapse = position(aurora_i18n::tr!("common-collapse"));
        let expand = position(aurora_i18n::tr!("common-expand"));
        assert!((collapse.y - filters.y).abs() < 1.0);
        assert!((expand.y - filters.y).abs() < 1.0);
        assert!(collapse.x > preferences.x && expand.x > collapse.x);
        assert!(field.y > filters.y);
    }

    #[test]
    fn typing_and_clearing_keep_inventory_search_layout_stable() {
        for width in [234.0, 350.0, 440.0] {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let palette = crate::theme::Theme::default().palette();
            let mut st = InventoryUi::default();
            let mut prefs = InventoryPreferences::default();
            let frame = |st: &mut InventoryUi, prefs: &mut InventoryPreferences, events| {
                headless_output(ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 240.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        controls::toolbar(ui, &palette, st, prefs);
                        controls::search(ui, &palette, st, prefs, "Chercher");
                        ui.label("below_controls");
                    },
                ))
            };
            let field_rect = |output: &egui::FullOutput| {
                output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect) if rect.fill == palette.field => Some(rect.rect),
                        _ => None,
                    })
                    .max_by(|a, b| a.width().total_cmp(&b.width()))
                    .expect("search field")
            };
            let text_rect = |output: &egui::FullOutput, label: &str| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                        }
                        _ => None,
                    })
                    .expect(label)
            };
            let pointer = |pos, pressed| {
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ]
            };
            let output = frame(&mut st, &mut prefs, vec![]);
            let field = field_rect(&output);
            let below = text_rect(&output, "below_controls");
            assert!((text_rect(&output, "Chercher").center().y - field.center().y).abs() < 1.0);
            let input_pos = field.left_center() + egui::vec2(12.0, 0.0);
            frame(&mut st, &mut prefs, pointer(input_pos, true));
            frame(&mut st, &mut prefs, pointer(input_pos, false));
            frame(&mut st, &mut prefs, vec![egui::Event::Text("fe".into())]);
            assert_eq!(st.search, "fe");
            let output = frame(&mut st, &mut prefs, vec![]);
            assert_eq!(field_rect(&output), field, "typing must not resize the field");
            assert_eq!(text_rect(&output, "below_controls"), below, "typing must not move following rows");
            assert!((text_rect(&output, "fe").center().y - field.center().y).abs() < 1.0);
            let focus = ctx.memory(|memory| memory.focused());
            st.filters.types = 1 << 6;
            frame(&mut st, &mut prefs, vec![]);
            let clear_pos = field.right_center() - egui::vec2(14.0, 0.0);
            frame(&mut st, &mut prefs, pointer(clear_pos, true));
            let output = frame(&mut st, &mut prefs, pointer(clear_pos, false));
            assert!(st.search.is_empty() && !st.filters.active());
            assert_eq!(
                ctx.memory(|memory| memory.focused()),
                focus,
                "clearing keeps the search ready for typing"
            );
            assert_eq!(field_rect(&output), field);
            assert_eq!(text_rect(&output, "below_controls"), below);

            st.search = "link-query".into();
            st.links_filter = Some(Uuid::from_u128(8100));
            st.saved_filters = Some(view::Filters {
                permissions: rules::COPY,
                ..Default::default()
            });
            frame(&mut st, &mut prefs, vec![]);
            frame(&mut st, &mut prefs, pointer(clear_pos, true));
            frame(&mut st, &mut prefs, pointer(clear_pos, false));
            assert!(st.links_filter.is_none());
            assert_eq!(
                st.filters.permissions,
                rules::COPY,
                "leaving linked mode restores the previous filters"
            );
            assert_eq!(st.search, "link-query");
        }
    }

    /// A long-time buyer's Received Items: the purchases arrive closed, one
    /// opened stays open, and only the rows in sight are drawn, even with
    /// everything expanded (the tab used to lay out its thousands of rows
    /// every frame, all open: ~17 ms of interface for 3,000 items).
    #[test]
    fn received_purchases_start_closed_and_only_rows_in_sight_are_drawn() {
        let ctx = egui::Context::default();
        let mut inv = view_inventory(false);
        let agent = Uuid::from_u128(2);
        crate::world::inventory::demo::seed_received_big(&mut inv, agent, 150, 20);
        let facts = Facts {
            agent,
            worn: Default::default(),
            worn_labels: Default::default(),
            points: vec![],
            appearance_busy: false,
            active_gestures: Default::default(),
            touchable: Default::default(),
            names: Default::default(),
        };
        let mut prefs = InventoryPreferences::default();
        let mut st = InventoryUi {
            tab: RECEIVED_TAB,
            ..Default::default()
        };
        let mut actions = vec![];
        let mut frame = |inv: &mut Inventory, st: &mut InventoryUi| {
            st.visible.clear();
            headless_output(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(440.0, 610.0))),
                    ..Default::default()
                },
                |ui| {
                    inventory_contents(
                        ui,
                        &crate::theme::Theme::default().palette(),
                        &Icons::default(),
                        inv,
                        st,
                        &mut prefs,
                        &facts,
                        &mut actions,
                    )
                },
            ));
        };
        frame(&mut inv, &mut st);
        frame(&mut inv, &mut st);
        // the demo purchase and the 150 big ones, all closed
        assert_eq!(st.result_tree.rows.len(), 151);
        assert!(st.result_tree.rows.iter().all(|r| r.folder && !r.open));
        let first = Uuid::from_u128(8_400_000);
        st.result_tree.toggle(first, RECEIVED_TAB);
        frame(&mut inv, &mut st);
        frame(&mut inv, &mut st);
        assert_eq!(st.result_tree.rows.len(), 171, "the opened purchase stays open");
        st.expand_all = Some(true);
        frame(&mut inv, &mut st);
        frame(&mut inv, &mut st);
        assert!(st.result_tree.rows.len() > 3_100);
        assert!(st.visible.len() < 60, "{} rows drawn", st.visible.len());
    }

    #[test]
    fn received_tab_fetches_and_searches_only_the_marketplace_inbox() {
        let ctx = egui::Context::default();
        let mut inv = view_inventory(false);
        let agent = Uuid::from_u128(2);
        crate::world::inventory::demo::seed_received(&mut inv, agent);
        let inbox = rules::system(&inv, 50).expect("inbox");
        inv.folders.get_mut(&inbox).expect("inbox").state = FetchState::Unknown;
        inv.folders.get_mut(&Uuid::from_u128(8000)).expect("personal folder").state = FetchState::Unknown;
        let facts = Facts {
            agent,
            worn: Default::default(),
            worn_labels: Default::default(),
            points: vec![],
            appearance_busy: false,
            active_gestures: Default::default(),
            touchable: Default::default(),
            names: Default::default(),
        };
        let mut prefs = InventoryPreferences::default();
        let mut st = InventoryUi {
            tab: RECEIVED_TAB,
            expand_all: Some(true),
            ..Default::default()
        };
        let mut actions = vec![];
        for query in ["", aurora_i18n::tr!("world-inventory-demo-instructions")] {
            st.search = query.into();
            st.visible.clear();
            headless_output(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(440.0, 610.0))),
                    ..Default::default()
                },
                |ui| {
                    inventory_contents(
                        ui,
                        &crate::theme::Theme::default().palette(),
                        &Icons::default(),
                        &mut inv,
                        &mut st,
                        &mut prefs,
                        &facts,
                        &mut actions,
                    )
                },
            ));
            assert!(st.visible.contains(&Uuid::from_u128(8_300_003)));
            assert!(!st.visible.contains(&Uuid::from_u128(8100)));
            assert!(!st.visible.contains(&inv.root));
            if !query.is_empty() {
                assert_eq!(st.view.results, vec![Uuid::from_u128(8_300_003)]);
            }
        }
        assert!(inv.queue.iter().any(|(id, _)| *id == inbox));
        assert!(!inv.queue.iter().any(|(id, _)| *id == Uuid::from_u128(8000)));
        inv.folders.remove(&inbox);
        inv.generation = inv.generation.wrapping_add(1);
        st.visible.clear();
        headless_output(ctx.run_ui(Default::default(), |ui| {
            inventory_contents(
                ui,
                &crate::theme::Theme::default().palette(),
                &Icons::default(),
                &mut inv,
                &mut st,
                &mut prefs,
                &facts,
                &mut actions,
            );
        }));
        assert!(st.visible.is_empty());
        assert!(
            st.view.results.is_empty(),
            "a missing inbox must never fall back to personal inventory"
        );
        assert!(actions.is_empty());
    }

    #[test]
    fn search_draws_only_the_viewport_and_can_reach_the_last_result() {
        let ctx = egui::Context::default();
        let mut inv = view_inventory(true);
        let agent = Uuid::from_u128(2);
        let facts = Facts {
            agent,
            worn: HashSet::new(),
            worn_labels: Default::default(),
            points: vec![],
            appearance_busy: false,
            active_gestures: Default::default(),
            touchable: Default::default(),
            names: Default::default(),
        };
        let mut prefs = InventoryPreferences::default();
        let mut st = InventoryUi {
            search: aurora_i18n::tr!("world-inventory-demo-item", n = "").trim().into(),
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
            context::double_click(
                &inv,
                Uuid::from_u128(id),
                &mut InventoryUi::default(),
                &prefs,
                &Facts::default(),
                &mut actions,
            );
            assert!(
                matches!(actions.as_slice(), [InvAction::Appearance(crate::world::appearance::Action::WearItem { replace: actual, .. })] if *actual == replace)
            );
        }
    }

    /// LLObjectBridge::openItem / LLWearableBridge::openItem: a second
    /// double-click on what is worn takes it off, through a link too; a worn
    /// body part stays put.
    #[test]
    fn double_click_takes_off_what_is_worn_except_body_parts() {
        use crate::world::appearance::Action as Wear;
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let mut physics = inv.items[&Uuid::from_u128(8111)].clone();
        physics.id = Uuid::from_u128(8190);
        physics.flags = 15;
        inv.add_items(vec![physics]);
        let worn: HashSet<Uuid> = [8100, 8111, 8112, 8190].into_iter().map(Uuid::from_u128).collect();
        let facts = Facts {
            worn,
            ..Default::default()
        };
        let click = |id: u128| {
            let mut actions = vec![];
            context::double_click(
                &inv,
                Uuid::from_u128(id),
                &mut InventoryUi::default(),
                &InventoryPreferences::default(),
                &facts,
                &mut actions,
            );
            actions
        };
        // object, its link (acts on the original), clothing, physics
        for (row, item) in [(8100, 8100), (8113, 8100), (8111, 8111), (8190, 8190)] {
            assert!(
                matches!(&click(row)[..], [InvAction::Appearance(Wear::Remove(id))] if *id == Uuid::from_u128(item)),
                "row {row}"
            );
        }
        assert!(click(8112).is_empty(), "a worn body part is only replaced");
        // not worn: the same rows put the item on again, replacing by default
        let mut actions = vec![];
        context::double_click(
            &inv,
            Uuid::from_u128(8113),
            &mut InventoryUi::default(),
            &InventoryPreferences::default(),
            &Facts::default(),
            &mut actions,
        );
        assert!(
            matches!(&actions[..], [InvAction::Appearance(Wear::WearItem { item, replace: true, point: 0 })] if *item == Uuid::from_u128(8100))
        );
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
                        worn_labels: Default::default(),
                        points: Vec::new(),
                        appearance_busy: false,
                        active_gestures: Default::default(),
                        touchable: Default::default(),
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
        for label in [
            aurora_i18n::tr!("common-my-inventory"),
            aurora_i18n::tr!("common-library"),
            aurora_i18n::tr!("world-inventory-demo-right-click-tests"),
            aurora_i18n::tr!("common-inventory"),
            aurora_i18n::tr!("common-favorites"),
        ] {
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
        st.search = inv.folders[&Uuid::from_u128(8001)].info.name.clone();
        folder_window_frame(&ctx, &mut inv, root, &mut st);
        assert_eq!(st.visible, vec![Uuid::from_u128(8001)]);
        st.search = inv.folders[&Uuid::from_u128(8002)].info.name.clone();
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
                        worn_labels: Default::default(),
                        points: Vec::new(),
                        appearance_busy: false,
                        active_gestures: Default::default(),
                        touchable: Default::default(),
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
                        worn_labels: Default::default(),
                        points: Vec::new(),
                        appearance_busy: false,
                        active_gestures: Default::default(),
                        touchable: Default::default(),
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
        for (id, name, library) in [(root, aurora_i18n::tr!("common-my-inventory"), false), (lib, "Library", true)] {
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
                        // Closed in an earlier session.
                        st.plain.set_open(root, false);
                    }
                    rows::scroll(
                        ui,
                        &p,
                        &Icons::default(),
                        &mut inv,
                        &mut st,
                        &mut InventoryPreferences::default(),
                        &Facts {
                            agent: Uuid::nil(),
                            worn: HashSet::new(),
                            worn_labels: Default::default(),
                            points: Vec::new(),
                            appearance_busy: false,
                            active_gestures: Default::default(),
                            touchable: Default::default(),
                            names: Default::default(),
                        },
                        &mut Vec::new(),
                        500.0,
                        root,
                        true,
                    );
                },
            )));
        }
        let output = output.expect("frame");
        for label in [aurora_i18n::tr!("common-my-inventory"), aurora_i18n::tr!("common-library")] {
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
                .any(|s| matches!(&s.shape, egui::epaint::Shape::Text(t) if t.galley.text() == aurora_i18n::tr!("common-properties")))
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

    #[test]
    fn world_drop_reads_released_cargo_before_end_pass_and_respects_ui_and_escape() {
        let ctx = egui::Context::default();
        let id = Uuid::from_u128(8100);
        let mut read = None;
        let mut frame = |pos, events: Vec<egui::Event>| {
            read = None;
            let mut events = events;
            events.insert(0, egui::Event::PointerMoved(pos));
            headless_output(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 600.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::Area::new(egui::Id::new("drop_test_window"))
                        .fixed_pos(egui::pos2(50.0, 50.0))
                        .show(ui.ctx(), |ui| {
                            ui.set_min_size(egui::vec2(200.0, 200.0));
                        });
                    read = world_drag(ui.ctx());
                    if read.as_ref().is_some_and(|drag| drag.released) {
                        egui::DragAndDrop::clear_payload(ui.ctx());
                    }
                },
            ));
            read.take()
        };
        for _ in 0..3 {
            frame(egui::pos2(600.0, 400.0), vec![]);
        }
        egui::DragAndDrop::set_payload(&ctx, InvDrag(id));
        assert!(frame(egui::pos2(100.0, 100.0), vec![]).is_none(), "window is not a world target");
        let pos = egui::pos2(600.0, 400.0);
        let released = frame(
            pos,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        )
        .expect("release seen inside pass");
        assert_eq!(released.item, id);
        assert!(released.released);
        assert!(egui::DragAndDrop::payload::<InvDrag>(&ctx).is_none());
        assert!(frame(pos, vec![]).is_none(), "no duplicate drop next frame");
        egui::DragAndDrop::set_payload(&ctx, InvDrag(id));
        assert!(frame(pos, vec![key(egui::Key::Escape)]).is_none());
    }
}
