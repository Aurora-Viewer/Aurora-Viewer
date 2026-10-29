//! Appearance floater, following LLSidepanelAppearance, LLPanelOutfitsInventory,
//! LLOutfitGallery and LLPanelOutfitEdit (Firestorm indra/newview, LGPL 2.1).

use super::{icons, menu, widgets};
use crate::{
    theme::Palette,
    world::{World, appearance as model, outfit_folders::Entry},
};
use aurora_net::inventory::InvItem;
use egui::{RichText, Vec2};
use model::Action;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;
mod folders;
mod gallery;
pub(crate) mod items;
mod layers;
mod selection;

#[derive(Default)]
pub struct AppearanceUi {
    pub tab: usize,
    pub fill_previews: bool,
    pub editing: bool,
    edit_tab: usize,
    search: String,
    outfit_selection: selection::Selection,
    reveal_outfit: Option<Uuid>,
    /// The current outfit to bring into view in the gallery or the list
    /// once its row is laid out (`locate_current`).
    locate: Option<Uuid>,
    /// The frames left to keep `locate` in view (`reveal_located`).
    locate_frames: u8,
    /// The located outfit and the time it came into view, for its flash.
    flash: Option<(Uuid, f64)>,
    selected_item: Option<Uuid>,
    /// The drag of a worn clothing layer in the outfit editor.
    layers: layers::Drag,
    /// The temporary attachments counted last frame: one more opens their
    /// section (`temporary_attachments`).
    temporary_seen: usize,
    pub save_as: Option<String>,
    /// The folder of My Outfits Save as puts the new outfit in.
    save_in: Option<Uuid>,
    select_name: bool,
    adding: bool,
    add_search: String,
    reverse_sort: bool,
    pub pending: Option<(Uuid, bool)>,
    pub pending_actor: Uuid,
    pub saved_new: bool,
    pub attachments: Vec<aurora_net::AttachRequest>,
    pub favorite_pending: HashSet<Uuid>,
    profile_item: Option<Uuid>,
    pub message: String,
    requested_items: HashSet<Uuid>,
    gallery: gallery::State,
    folders: folders::State,
    /// The groups of My Outfits closed in the list and the gallery, and a
    /// count of their changes for `shown`.
    closed_groups: HashSet<Uuid>,
    closed_version: u64,
    pub wanted_images: HashSet<Uuid>,
    /// What `prepare` walked at the last inventory change.
    prepared: Option<Prepared>,
    /// The outfits shown, filtered and sorted, and what they came from.
    shown: Option<Shown>,
    /// The list's outfits whose header was open at the last frame, `None`
    /// until the list has been drawn whole once (see `outfit_list`).
    list_open: Option<HashSet<Uuid>>,
    /// The heights of the rows last drawn, for those left out.
    rows: RowHeights,
    selection_info: Option<SelectionInfo>,
}

struct SelectionInfo {
    generation: u64,
    ids: Vec<Uuid>,
    items: usize,
    ready: bool,
}

/// The measured heights of the rows of the list and the gallery, spacing
/// included, for the rows out of sight that are not drawn. A name drawn
/// with a taller fallback font (emoji, symbols) makes a taller row; using
/// one height for all would move the rows when such a row leaves the view.
#[derive(Default)]
struct RowHeights {
    /// Zoom and text sizes the heights were measured at.
    key: (f32, f32, f32),
    /// A closed header of the list, by outfit, and the last one measured.
    list: HashMap<Uuid, f32>,
    list_typical: Option<f32>,
    /// A gallery row, by its first outfit, minus the height of the first row.
    gallery: HashMap<Uuid, f32>,
}

impl RowHeights {
    /// Forgets the heights when the zoom or a text size changed; `true`
    /// then, when the list must be drawn whole again.
    fn check(&mut self, ui: &egui::Ui) -> bool {
        let key = (
            ui.ctx().pixels_per_point(),
            ui.text_style_height(&egui::TextStyle::Body),
            ui.text_style_height(&egui::TextStyle::Button),
        );
        if key == self.key {
            return false;
        }
        *self = RowHeights { key, ..Default::default() };
        true
    }
}

/// The folders `prepare` keeps loaded, and per folder the link targets it
/// found missing with the folder's version and item count at the time.
struct Prepared {
    generation: u64,
    /// The inventory's item count: when items go away, an original once
    /// there may be missing in a folder that did not change.
    items: usize,
    folders: Vec<Uuid>,
    missing: HashMap<Uuid, ((i32, usize), Vec<Uuid>)>,
}

/// The groups and outfits of My Outfits that match the filter, in display
/// order, without what closed groups hide.
struct Shown {
    generation: u64,
    search: String,
    reverse: bool,
    closed: u64,
    order: crate::world::outfit_folders::Order,
    entries: Vec<crate::world::outfit_folders::Entry>,
    outfits: Vec<Uuid>,
    /// The outfits matching the filter, those of closed groups included.
    total: usize,
}

impl AppearanceUi {
    pub fn open_outfit(&mut self, id: Uuid) {
        self.open(1, false);
        self.outfit_selection.single(id);
        self.reveal_outfit = Some(id);
        self.selected_item = None;
        self.search.clear();
    }
    pub fn open(&mut self, tab: usize, editing: bool) {
        self.tab = tab.min(2);
        self.editing = editing;
        self.adding = false;
    }

    /// Scrolls the gallery or the list to the current outfit and selects
    /// it, after clearing a filter that hides it; from the worn tab, in the
    /// gallery. An Aurora addition (#971): the Legacy Viewer only marks the
    /// current outfit in bold (LLOutfitsList::onHighlightBaseOutfit), which a
    /// wardrobe of thousands of outfits leaves to be scrolled to by hand.
    pub fn locate_current(&mut self, inv: &crate::world::inventory::Inventory) {
        let Some(id) = locatable(inv) else {
            return;
        };
        if self.tab >= 2 {
            self.tab = 0;
        }
        self.editing = false;
        self.adding = false;
        self.open_groups_of(inv, id);
        if !self.shown_outfits(inv).contains(&id) {
            self.search.clear();
        }
        self.outfit_selection.single(id);
        self.selected_item = None;
        self.locate = Some(id);
        self.locate_frames = LOCATE_FRAMES;
        self.flash = None;
    }

    /// Scrolls to `rect` when it is the outfit to locate, and starts its
    /// flash. The rows out of sight are not laid out but given a measured
    /// height: while a window opens, its scroll bar narrows the tiles by a
    /// point, which over hundreds of rows moves the outfit by a whole row.
    /// It is brought back for `LOCATE_FRAMES` frames, at once (a jump to the
    /// same place is idempotent), until the layout has settled; the sizing
    /// pass of an opening window is skipped.
    fn reveal_located(&mut self, ui: &egui::Ui, id: Uuid, rect: egui::Rect, align: egui::Align) {
        if self.locate != Some(id) || ui.is_sizing_pass() {
            return;
        }
        ui.scroll_to_rect_animation(rect, Some(align), egui::style::ScrollAnimation::none());
        self.locate_frames = self.locate_frames.saturating_sub(1);
        if self.locate_frames == 0 {
            self.locate = None;
        }
        // "Reduce animations" keeps the selection alone
        if self.flash.is_none() && ui.style().animation_time > 0.0 {
            self.flash = Some((id, ui.input(|i| i.time)));
        }
    }

    /// How lit the located outfit `id` still is: 1 when it came into view,
    /// down to 0 after `FLASH_SECONDS`.
    fn flash(&mut self, ui: &egui::Ui, id: Uuid) -> f32 {
        let Some((flashed, start)) = self.flash else {
            return 0.0;
        };
        if flashed != id {
            return 0.0;
        }
        let t = ((ui.input(|i| i.time) - start) / FLASH_SECONDS) as f32;
        if t >= 1.0 {
            self.flash = None;
            return 0.0;
        }
        ui.ctx().request_repaint();
        1.0 - t.max(0.0)
    }

    fn select_outfit(&mut self, response: &egui::Response, id: Uuid, outfits: &[Uuid]) {
        if response.clicked() || response.secondary_clicked() {
            self.outfit_selection
                .click(id, outfits, response.ctx.input(|i| i.modifiers), response.secondary_clicked());
            self.selected_item = None;
        }
    }

    fn wear_selection(&self, outfits: &[Uuid], append: bool) -> Action {
        let folders = self.outfit_selection.ordered(outfits);
        if let [id] = folders.as_slice() {
            Action::Wear(*id, append)
        } else {
            Action::WearOutfits { folders, append }
        }
    }

    fn selection_info(&mut self, inv: &crate::world::inventory::Inventory, ids: Vec<Uuid>) -> (usize, bool) {
        if let Some(info) = &self.selection_info
            && info.generation == inv.generation
            && info.ids == ids
        {
            return (info.items, info.ready);
        }
        let mut items = HashSet::new();
        let mut ready = !ids.is_empty();
        for id in &ids {
            ready &= model::loaded(inv, *id);
            for link in model::folder_links(inv, *id).into_iter().filter(|link| !link.folder) {
                ready &= inv.items.contains_key(&link.target);
                items.insert(link.target);
            }
        }
        let count = items.len();
        self.selection_info = Some(SelectionInfo {
            generation: inv.generation,
            ids,
            items: count,
            ready,
        });
        (count, ready)
    }

    pub fn demo_multiselection(&mut self) {
        let outfits = [Uuid::from_u128(703), Uuid::from_u128(704)];
        self.outfit_selection.single(outfits[0]);
        self.outfit_selection.click(outfits[1], &outfits, egui::Modifiers::CTRL, false);
    }

    pub fn begin_save_as(&mut self, inv: &crate::world::inventory::Inventory) {
        let name = model::base(inv)
            .and_then(|id| inv.folders.get(&id))
            .map(|f| aurora_i18n::tr!("status-new", value0 = f.info.name.to_string()))
            .unwrap_or_else(|| aurora_i18n::tr!("ui-appearance-new-outfit").into());
        self.save_as = Some(name.chars().take(63).collect());
        self.select_name = true;
        // beside the outfit it comes from: a variant goes in the same group
        self.save_in = model::base(inv)
            .and_then(|id| inv.folders.get(&id))
            .map(|f| f.info.parent)
            .filter(|parent| crate::world::outfit_folders::container(inv, *parent));
    }

    /// Keeps the outfits loaded and asks for the originals their links
    /// point to. The folders are asked for at every frame (a cheap check of
    /// their state, which a refresh resets); their links are walked again
    /// only when the inventory changed, and then only those of the folders
    /// whose version or item count moved: a wardrobe of thousands of outfits
    /// cost tens of milliseconds a frame when it was walked whole each time.
    pub fn prepare(
        &mut self,
        inv: &mut crate::world::inventory::Inventory,
        agent: Uuid,
        worn: &HashSet<Uuid>,
    ) -> Vec<aurora_net::NetCommand> {
        let changed = self.prepared.as_ref().is_none_or(|p| p.generation != inv.generation);
        if changed {
            let folders = model::prepare_folders(inv);
            let mut known = self
                .prepared
                .take()
                .filter(|p| inv.items.len() >= p.items)
                .map(|p| p.missing)
                .unwrap_or_default();
            let mut missing = HashMap::with_capacity(folders.len());
            for id in &folders {
                let key = inv.folders.get(id).map_or((0, 0), |f| (f.info.version, f.items.len()));
                let targets = match known.remove(id) {
                    Some((was, targets)) if was == key => targets,
                    _ => model::missing_targets(inv, *id).collect(),
                };
                missing.insert(*id, (key, targets));
            }
            self.prepared = Some(Prepared {
                generation: inv.generation,
                items: inv.items.len(),
                folders,
                missing,
            });
        }
        let Some(prepared) = &self.prepared else {
            return Vec::new();
        };
        for id in &prepared.folders {
            inv.request(*id);
        }
        let mut missing: HashSet<_> = worn.iter().filter(|id| !inv.items.contains_key(id)).copied().collect();
        // links walked at an earlier change were asked for then
        // (`requested_items`), unless a refresh forgot them
        if changed {
            missing.extend(
                prepared
                    .missing
                    .values()
                    .flat_map(|(_, targets)| targets)
                    .filter(|id| !inv.items.contains_key(id)),
            );
        }
        self.filter_requests(model::fetch_items(missing, agent))
    }

    /// The outfits shown, in display order: those of My Outfits and of its
    /// groups matching the filter, without those of closed groups.
    fn shown_outfits(&mut self, inv: &crate::world::inventory::Inventory) -> Vec<Uuid> {
        self.shown(inv).outfits.clone()
    }

    /// The groups and outfits shown (`world/outfit_folders.rs::tree`),
    /// built again only when the inventory, the filter, the order or a
    /// group's open state changed. A filter shows every group open.
    fn shown(&mut self, inv: &crate::world::inventory::Inventory) -> &Shown {
        use crate::world::outfit_folders::{Order, tree};
        // the gallery shows a group's section before its subgroups'
        let order = if self.tab == 0 { Order::Gallery } else { Order::List };
        let fresh = self.shown.as_ref().is_some_and(|shown| {
            shown.generation == inv.generation
                && shown.search == self.search
                && shown.reverse == self.reverse_sort
                && shown.closed == self.closed_version
                && shown.order == order
        });
        if !fresh {
            let needle = self.search.trim().to_lowercase();
            let tree = tree(inv, &needle, self.reverse_sort, order);
            let total = tree.iter().filter(|e| !e.group).count();
            let mut entries = Vec::with_capacity(tree.len());
            let mut hidden_below = None;
            for entry in tree {
                if hidden_below.is_some_and(|depth| entry.depth > depth) {
                    continue;
                }
                hidden_below = (entry.group && needle.is_empty() && self.closed_groups.contains(&entry.id)).then_some(entry.depth);
                entries.push(entry);
            }
            let outfits: Vec<_> = entries.iter().filter(|e| !e.group).map(|e| e.id).collect();
            self.outfit_selection.retain(&outfits);
            self.shown = Some(Shown {
                generation: inv.generation,
                search: self.search.clone(),
                reverse: self.reverse_sort,
                closed: self.closed_version,
                order,
                entries,
                outfits,
                total,
            });
        }
        self.shown.as_ref().expect("built above")
    }

    /// Whether a group shows open: always while filtering.
    fn group_open(&self, id: Uuid) -> bool {
        !self.search.trim().is_empty() || !self.closed_groups.contains(&id)
    }

    pub fn prepare_contents(
        &mut self,
        inv: &mut crate::world::inventory::Inventory,
        agent: Uuid,
        folder: Uuid,
    ) -> Vec<aurora_net::NetCommand> {
        self.filter_requests(model::prepare_contents(inv, agent, folder))
    }

    fn filter_requests(&mut self, commands: Vec<aurora_net::NetCommand>) -> Vec<aurora_net::NetCommand> {
        commands
            .into_iter()
            .filter_map(|cmd| {
                if let aurora_net::NetCommand::FetchItems { items, owner } = cmd {
                    let items: Vec<_> = items.into_iter().filter(|id| self.requested_items.insert(*id)).collect();
                    (!items.is_empty()).then_some(aurora_net::NetCommand::FetchItems { items, owner })
                } else {
                    Some(cmd)
                }
            })
            .collect()
    }

    pub fn refresh(&mut self, inv: &mut crate::world::inventory::Inventory) {
        self.requested_items.clear();
        self.prepared = None;
        self.selection_info = None;
        for f in inv.folders.values_mut().filter(|f| matches!(f.info.type_default, 46..=48)) {
            f.state = crate::world::inventory::FetchState::Unknown;
        }
        // asked again right away: with this window closed nothing else
        // would, and every outfit change would wait for them
        for id in [model::cof(inv), model::system_folder(inv, model::FT_MY_OUTFITS), model::base(inv)]
            .into_iter()
            .flatten()
        {
            inv.request(id);
        }
    }

    /// An outfit change refused because the outfit is not loaded (a fetch
    /// lost while the agent changed region): what it needs is asked again,
    /// the link targets included, so that the next try goes through even
    /// with this window closed.
    pub fn reload(
        &mut self,
        inv: &mut crate::world::inventory::Inventory,
        agent: Uuid,
        worn: &HashSet<Uuid>,
    ) -> Vec<aurora_net::NetCommand> {
        self.requested_items.clear();
        self.prepared = None;
        self.prepare(inv, agent, worn)
    }
}

/// How long the outfit brought into view by `locate_current` stays lit.
const FLASH_SECONDS: f64 = 1.6;

/// How many frames `locate_current` keeps the outfit in view while the
/// layout settles (`AppearanceUi::reveal_located`).
const LOCATE_FRAMES: u8 = 12;

/// The current outfit when the gallery and the list can show it: an
/// outfit of My Outfits or of one of its groups, as `shown_outfits` lists them.
fn locatable(inv: &crate::world::inventory::Inventory) -> Option<Uuid> {
    model::base(inv).filter(|id| crate::world::outfit_folders::in_my_outfits(inv, *id))
}

fn icon(ui: &mut egui::Ui, name: &str, size: f32, color: egui::Color32) {
    if let Some(t) = icons::global(name) {
        ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(size)).tint(color));
    } else {
        ui.add_space(size);
    }
}

fn icon_button(ui: &mut egui::Ui, p: &Palette, name: &str, tip: &str) -> egui::Response {
    let button = if let Some(t) = icons::global(name) {
        egui::Button::image(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(16.0)).tint(p.ink))
    } else {
        egui::Button::new("·")
    };
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = Vec2::splat(4.0);
        ui.add_sized(Vec2::splat(28.0), button.fill(p.raised))
    })
    .inner
    .on_hover_text(tip)
}

fn icon_menu(ui: &mut egui::Ui, p: &Palette, name: &str, tip: &str, body: impl FnOnce(&mut egui::Ui)) {
    let response = icon_button(ui, p, name, tip);
    menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClick, body);
}

/// Pulls what follows a row (an item, a closed outfit header) up to the
/// inventory's row pitch. The list's spacing itself stays: the rows'
/// context menus inherit it.
fn row_gap(ui: &mut egui::Ui) {
    ui.add_space(1.0 - ui.spacing().item_spacing.y);
}

fn row(ui: &mut egui::Ui, p: &Palette, it: &InvItem, desc: &str, selected: bool, worn: Option<&str>) -> egui::Response {
    sensed_row(ui, p, it, desc, selected, worn, egui::Sense::click(), None)
}

/// `row`, dragged as well with `Sense::click_and_drag` (a clothing layer),
/// its tooltip ending with `hint`.
#[allow(clippy::too_many_arguments)]
fn sensed_row(
    ui: &mut egui::Ui,
    p: &Palette,
    it: &InvItem,
    desc: &str,
    selected: bool,
    worn: Option<&str>,
    sense: egui::Sense,
    hint: Option<&str>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), super::inventory::AURORA_ROW_HEIGHT), sense);
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect,
            ui.visuals().window_corner_radius,
            if selected { p.violet.gamma_multiply(0.25) } else { p.raised },
        );
    }
    let name = super::inventory::item_icon(it.asset_type, it.inv_type, it.flags);
    let color = super::inventory::item_icon_color(it.asset_type, it.inv_type, p);
    if let Some(t) = icons::global(name) {
        ui.painter().image(
            t.id(),
            egui::Rect::from_center_size(egui::pos2(rect.left() + 11.0, rect.center().y), Vec2::splat(16.0)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            color,
        );
    }
    let text = if desc.is_empty() {
        it.name.clone()
    } else {
        format!("{} {desc}", it.name)
    };
    // The worn tag follows the name, as in the inventory's rows.
    let tag_width = worn.map_or(0.0, |label| widgets::row_tag_width(ui, label) + 6.0);
    let font_id = egui::FontId::proportional(12.0);
    let format = if worn.is_some() {
        widgets::bold_format(ui, font_id, p.ink)
    } else {
        egui::TextFormat::simple(font_id, p.ink)
    };
    let mut job = egui::text::LayoutJob::single_section(text.clone(), format);
    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 30.0 - tag_width).max(20.0));
    let galley = ui.painter().layout_job(job);
    let name_end = rect.left() + 25.0 + galley.size().x;
    let text_rect = egui::Rect::from_min_max(egui::pos2(rect.left() + 25.0, rect.top()), rect.right_bottom());
    let painter = ui.painter().with_clip_rect(text_rect);
    let pos = egui::pos2(rect.left() + 25.0, rect.center().y - galley.size().y * 0.5);
    if worn.is_some()
        && let Some(offset) = widgets::faux_bold_offset(ui)
    {
        painter.galley(pos + offset, galley.clone(), p.ink);
    }
    painter.galley(pos, galley, p.ink);
    if let Some(label) = worn {
        widgets::paint_row_tag(ui, p, label, true, name_end + 6.0, rect.center().y);
    }
    let tip = match worn {
        Some(label) => format!("{text} ({label})"),
        None => text,
    };
    response.on_hover_text(match hint {
        Some(hint) => format!("{tip}\n{hint}"),
        None => tip,
    })
}

/// A wearable type worn by nothing, at the pitch, icon and text place of
/// `row` (the dummy items of LLCOFWearables::addClothingTypesDummies).
fn not_worn_row(ui: &mut egui::Ui, p: &Palette, icon_name: &str, text: &str) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), super::inventory::AURORA_ROW_HEIGHT),
        egui::Sense::hover(),
    );
    if let Some(t) = icons::global(icon_name) {
        ui.painter().image(
            t.id(),
            egui::Rect::from_center_size(egui::pos2(rect.left() + 11.0, rect.center().y), Vec2::splat(16.0)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            p.muted_dim,
        );
    }
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat::simple(egui::FontId::proportional(12.0), p.muted_dim),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 30.0).max(20.0));
    let galley = ui.painter().layout_job(job);
    let pos = egui::pos2(rect.left() + 25.0, rect.center().y - galley.size().y * 0.5);
    ui.painter().galley(pos, galley, p.muted_dim);
}

fn item_menu(response: &egui::Response, p: &Palette, it: &InvItem, actions: &mut Vec<Action>) {
    menu::context_menu(response, p, |ui| {
        let editable = matches!(it.asset_type, 5 | 13);
        if editable && menu::item(ui, p, "pencil-filled", aurora_i18n::tr!("common-edit")) {
            super::context::request(ui.ctx(), super::context::CtxAction::EditWorn(it.id));
        }
        let label = if it.asset_type == 6 {
            aurora_i18n::tr!("common-detach")
        } else {
            aurora_i18n::tr!("common-remove")
        };
        if it.asset_type != 13 && menu::item(ui, p, "circle-x-filled", label) {
            actions.push(Action::Remove(it.id));
        }
    });
}

/// The heading of a gallery section: a group's path ("Beach › Night"),
/// or, with no `group`, the outfits outside any group.
fn gallery_heading(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    group: Option<Uuid>,
    count: usize,
    actions: &mut Vec<Action>,
) {
    let inv = &world.inventory;
    let title = group.map_or_else(
        || aurora_i18n::tr!("ui-appearance-folders-other").to_string(),
        |id| crate::world::outfit_folders::path(inv, id),
    );
    let open = group.map(|id| st.group_open(id));
    let response = folders::header(ui, p, &title, count, open, 0);
    ui.add_space(4.0);
    match group {
        Some(id) => {
            if response.clicked() && st.search.trim().is_empty() {
                st.toggle_group(id);
            }
            folders::group_menu(&response, p, world, st, id, actions);
        }
        None => {
            menu::context_menu(&response, p, |ui| folders::new_folder_item(ui, p, inv, st));
        }
    }
}

/// The gallery: the outfits by section once My Outfits has groups. A
/// group's section comes when its own outfits do; a closed or empty group
/// shows its heading alone, to be opened or used.
#[allow(clippy::too_many_arguments)]
fn gallery(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    entries: &[Entry],
    outfits: &[Uuid],
    current: Option<Uuid>,
    actions: &mut Vec<Action>,
    images: &HashMap<Uuid, egui::TextureHandle>,
    list_width: f32,
) {
    // the columns follow the list's width, not what its scroll bar leaves:
    // the bar's strip must not drop a column
    let columns = if list_width >= 540.0 { 3 } else { 2 };
    let width = ((ui.available_width() - 10.0 * (columns - 1) as f32) / columns as f32).max(90.0);
    // Only the rows in sight are laid out (with their thumbnails asked
    // for); the others leave their height: that of the first row, drawn at
    // every frame to measure it, give or take what a row measured when last
    // drawn (a taller font in a name).
    st.rows.check(ui);
    let view = ui.clip_rect();
    let grouped = entries.iter().any(|e| e.group);
    let mut pitch = None;
    let mut section: Option<Option<Uuid>> = None;
    let mut at = 0;
    while at < entries.len() {
        let entry = entries[at];
        if entry.group {
            let open = st.group_open(entry.id);
            if !open || entry.outfits == 0 {
                gallery_heading(ui, p, world, st, Some(entry.id), entry.outfits, actions);
                section = Some(Some(entry.id));
                if open {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("ui-appearance-folders-empty"))
                            .size(12.0)
                            .color(p.muted),
                    );
                    ui.add_space(8.0);
                }
            }
            at += 1;
            continue;
        }
        // the outfits of one folder follow one another, after its groups
        let end = at + entries[at..].iter().take_while(|e| !e.group && e.depth == entry.depth).count();
        let folder = (entry.depth > 0)
            .then(|| world.inventory.folders.get(&entry.id).map(|f| f.info.parent))
            .flatten();
        if grouped && section != Some(folder) {
            gallery_heading(ui, p, world, st, folder, end - at, actions);
            section = Some(folder);
        }
        let run: Vec<Uuid> = entries[at..end].iter().map(|e| e.id).collect();
        at = end;
        for pair in run.chunks(columns) {
            if let Some(pitch) = pitch {
                let height = pitch + st.rows.gallery.get(&pair[0]).copied().unwrap_or(0.0);
                let located = st.locate.is_some_and(|id| pair.contains(&id));
                if !located && !in_sight(ui.cursor().top(), height, view) {
                    skip_row(ui, height);
                    continue;
                }
            }
            let top = ui.cursor().top();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                for id in pair {
                    let Some(folder) = world.inventory.folders.get(id) else {
                        continue;
                    };
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, width * 0.88), egui::Sense::click());
                        st.select_outfit(&response, *id, outfits);
                        let selected = st.outfit_selection.contains(*id);
                        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
                        ui.painter().rect_stroke(
                            rect,
                            ui.visuals().window_corner_radius,
                            egui::Stroke::new(1.0, if selected { p.violet } else { p.raised }),
                            egui::StrokeKind::Inside,
                        );
                        let image = folder.info.thumbnail;
                        if !image.is_nil() {
                            st.wanted_images.insert(image);
                        }
                        if let Some(t) = images.get(&image) {
                            let (image_rect, uv) = preview_rects(rect.shrink(3.0), t.size_vec2(), st.fill_previews);
                            ui.painter().image(t.id(), image_rect, uv, widgets::PICTURE_TINT);
                        } else if let Some(t) = icons::global("hanger-2-filled") {
                            ui.painter().image(
                                t.id(),
                                egui::Rect::from_center_size(rect.center(), Vec2::splat(width * 0.55)),
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                p.violet_light,
                            );
                        }
                        st.reveal_located(ui, *id, rect, egui::Align::Center);
                        let flash = st.flash(ui, *id);
                        if flash > 0.0 {
                            let radius = ui.visuals().window_corner_radius;
                            ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.3 * flash));
                            ui.painter().rect_stroke(
                                rect,
                                radius,
                                egui::Stroke::new(2.0, p.violet_light.gamma_multiply(flash)),
                                egui::StrokeKind::Inside,
                            );
                        }
                        if response.double_clicked() && !ui.input(|i| i.modifiers.any()) {
                            st.open_outfit(*id);
                        }
                        gallery::context_menu(&response, p, world, st, *id, actions, true);
                        let current = current == Some(*id);
                        let color = if current { p.violet_light } else { p.ink };
                        let font_id = egui::FontId::proportional(12.0);
                        let galley = outfit_label(ui, &folder.info.name, current, font_id.clone(), color, width);
                        let response = ui.add(egui::Label::new(galley.clone()));
                        if current && ui.is_rect_visible(response.rect) {
                            if let Some(offset) = widgets::faux_bold_offset(ui) {
                                let name = egui::WidgetText::from(RichText::new(&folder.info.name).font(font_id).color(color)).into_galley(
                                    ui,
                                    Some(egui::TextWrapMode::Truncate),
                                    galley.size().x,
                                    egui::TextStyle::Body,
                                );
                                let clip = response.rect.intersect(ui.clip_rect());
                                ui.painter().with_clip_rect(clip).galley(response.rect.min + offset, name, color);
                            }
                            worn_tag(ui, p, response.rect.left() + galley.size().x, response.rect.center().y);
                        }
                    });
                }
            });
            ui.add_space(12.0);
            let height = ui.cursor().top() - top;
            let first = *pitch.get_or_insert(height);
            st.rows.gallery.insert(pair[0], height - first);
        }
    }
}

/// Fit keeps the whole image; Fill crops its UVs around the centre, keeping
/// its aspect ratio and all geometry inside the frame. This optional crop
/// is an Aurora extension to LLOutfitGalleryItem::draw (lloutfitgallery.cpp).
fn preview_rects(frame: egui::Rect, size: Vec2, fill: bool) -> (egui::Rect, egui::Rect) {
    let scale = frame.size() / size;
    if fill {
        let visible = if scale.x < scale.y {
            Vec2::new(scale.x / scale.y, 1.0)
        } else {
            Vec2::new(1.0, scale.y / scale.x)
        };
        (frame, egui::Rect::from_center_size(egui::pos2(0.5, 0.5), visible))
    } else {
        (
            egui::Rect::from_center_size(frame.center(), size * scale.min_elem()),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        )
    }
}

/// Whether a row of `pitch` points starting at `top` is in sight of the
/// list's viewport `view`, give or take a row.
fn in_sight(top: f32, pitch: f32, view: egui::Rect) -> bool {
    top + 2.0 * pitch >= view.top() && top - pitch <= view.bottom()
}

/// Leaves the room of a row that is not drawn, and the one automatic id
/// its child `Ui` would have taken, so that the rows drawn after it keep
/// the ids (hover, clicks, open menus) they have when every row is drawn.
fn skip_row(ui: &mut egui::Ui, pitch: f32) {
    ui.add_space(pitch);
    ui.skip_ahead_auto_ids(1);
}

/// The room between an outfit's name and its worn tag.
const WORN_TAG_GAP: f32 = 6.0;

/// An outfit's name on one line, cut short within `width`.
/// LLOutfitsList::onHighlightBaseOutfit marks the current title BOLD; it
/// leaves room for its worn tag (`worn_tag`), the tag of the worn items.
fn outfit_label(
    ui: &egui::Ui,
    name: &str,
    current: bool,
    font_id: egui::FontId,
    color: egui::Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(name.to_owned(), egui::TextFormat::simple(font_id, color));
    let mut room = 0.0;
    if current {
        if let Some(section) = job.sections.first_mut() {
            widgets::embolden(ui, &mut section.format);
        }
        room = WORN_TAG_GAP + widgets::row_tag_width(ui, aurora_i18n::tr!("ui-appearance-worn"));
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width((width - room).max(1.0));
    ui.painter().layout_job(job)
}

/// The worn tag after the current outfit's name, which ends at `name_end`.
fn worn_tag(ui: &egui::Ui, p: &Palette, name_end: f32, center_y: f32) {
    widgets::paint_row_tag(
        ui,
        p,
        aurora_i18n::tr!("ui-appearance-worn"),
        true,
        name_end + WORN_TAG_GAP,
        center_y,
    );
}

/// A group's header in the list: a click opens or closes it, a right click
/// opens its menu.
fn list_group(ui: &mut egui::Ui, p: &Palette, world: &World, st: &mut AppearanceUi, entry: &Entry, actions: &mut Vec<Action>) {
    let Some(f) = world.inventory.folders.get(&entry.id) else {
        return;
    };
    let response = folders::header(ui, p, &f.info.name, entry.outfits, Some(st.group_open(entry.id)), entry.depth);
    row_gap(ui);
    if response.clicked() && st.search.trim().is_empty() {
        st.toggle_group(entry.id);
    }
    folders::group_menu(&response, p, world, st, entry.id, actions);
}

#[allow(clippy::too_many_arguments)]
fn outfit_list(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    entries: &[Entry],
    outfits: &[Uuid],
    current: Option<Uuid>,
    actions: &mut Vec<Action>,
) {
    let worn = items::worn_labels(world);
    // Only the headers in sight are laid out; a closed one out of sight
    // leaves the height it had when last drawn. An open one (known from the
    // last frame), the focused one (open by default) and the one to reveal
    // are always drawn. The first frame draws every header, to learn which
    // ones are open and their heights, again after a zoom or text size change.
    let view = ui.clip_rect();
    let remeasure = st.rows.check(ui);
    let known_open = st.list_open.take().filter(|_| !remeasure);
    let mut open_now = HashSet::new();
    for entry in entries {
        if entry.group {
            list_group(ui, p, world, st, entry, actions);
            continue;
        }
        let id = &entry.id;
        let Some(f) = world.inventory.folders.get(id) else {
            continue;
        };
        if let Some(known_open) = &known_open
            && !known_open.contains(id)
            && st.outfit_selection.focus != Some(*id)
            && st.reveal_outfit != Some(*id)
            && st.locate != Some(*id)
            && let Some(height) = st.rows.list.get(id).copied().or(st.rows.list_typical)
            && !in_sight(ui.cursor().top(), height, view)
        {
            skip_row(ui, height);
            continue;
        }
        let top = ui.cursor().top();
        // an outfit of a group stands below its header; every outfit row is
        // one child `Ui`, the one automatic id `skip_row` leaves
        let mut room = ui.available_rect_before_wrap();
        room.min.x += entry.depth as f32 * ui.spacing().indent;
        let open = ui
            .scope_builder(egui::UiBuilder::new().max_rect(room), |ui| {
                let is_current = current == Some(*id);
                let font_id = egui::TextStyle::Button.resolve(ui.style());
                // the header's own room for its title: past the caret, 4 px short of the end
                let width = ui.available_width() - ui.spacing().indent - 4.0;
                let galley = outfit_label(ui, &f.info.name, is_current, font_id.clone(), p.ink, width);
                let reveal = st.reveal_outfit == Some(*id);
                let default_open = st.outfit_selection.focus == Some(*id);
                let forced = reveal.then_some(true).or_else(|| {
                    // Modifier selection must leave the accordion's open state alone.
                    ui.input(|i| i.modifiers.shift || i.modifiers.ctrl || i.modifiers.command)
                        .then(|| known_open.as_ref().is_some_and(|open| open.contains(id)))
                });
                let highlight = ui.painter().add(egui::Shape::Noop);
                // The headers take the inventory's row height; their body keeps the
                // window's own sizes (buttons, labels).
                let interact = ui.spacing().interact_size.y;
                ui.spacing_mut().interact_size.y = super::inventory::AURORA_ROW_HEIGHT;
                let response = widgets::collapsing_forced(ui, p, ("outfit", id), galley.clone(), default_open, forced, |ui| {
                    ui.spacing_mut().interact_size.y = interact;
                    // The header selects, like the Legacy Viewer's accordion tab
                    // (LLOutfitsList); the body opens on what to do with the
                    // outfit, replacing first, as its Wear button does
                    // (LLPanelOutfitsInventory::onWearButtonClick, "replaceoutfit").
                    ui.horizontal_wrapped(|ui| {
                        use widgets::{ActionKind, action_button};
                        let size = Vec2::new(0.0, 24.0);
                        for (icon, label, kind, append) in [
                            (
                                "shirt-filled",
                                aurora_i18n::tr!("ui-appearance-gallery-replace-your-outfit"),
                                ActionKind::Primary,
                                false,
                            ),
                            ("plus", aurora_i18n::tr!("common-add"), ActionKind::Flat, true),
                        ] {
                            if action_button(ui, p, icon, label, kind, size).clicked() {
                                actions.push(if st.outfit_selection.contains(*id) {
                                    st.wear_selection(outfits, append)
                                } else {
                                    Action::Wear(*id, append)
                                });
                            }
                        }
                    });
                    ui.add_space(2.0);
                    let mut links = model::folder_links(&world.inventory, *id);
                    items::sort_links(&world.inventory, &mut links, false);
                    for link in links.iter().filter(|l| !l.folder) {
                        if let Some(it) = world.inventory.items.get(&link.target) {
                            ui.push_id(("outfit_item", id, it.id), |ui| {
                                let response = row(
                                    ui,
                                    p,
                                    it,
                                    "",
                                    st.selected_item == Some(it.id) && st.outfit_selection.focus == Some(*id),
                                    worn.get(&it.id).map(String::as_str),
                                );
                                if response.clicked() || response.secondary_clicked() {
                                    st.outfit_selection.single(*id);
                                    st.selected_item = Some(it.id);
                                }
                                items::context_menu(&response, p, world, st, *id, it, actions);
                            });
                            row_gap(ui);
                        } else {
                            ui.label(RichText::new(&link.name).size(12.0).color(p.muted));
                        }
                    }
                    if links.is_empty() {
                        ui.label(
                            RichText::new(if model::loaded(&world.inventory, *id) {
                                aurora_i18n::tr!("ui-appearance-empty-outfit")
                            } else {
                                aurora_i18n::tr!("common-loading-2")
                            })
                            .color(p.muted),
                        );
                    }
                });
                ui.spacing_mut().interact_size.y = interact;
                st.select_outfit(&response.header, *id, outfits);
                st.reveal_located(ui, *id, response.header.rect, egui::Align::Min);
                let flash = st.flash(ui, *id);
                if st.outfit_selection.contains(*id) || flash > 0.0 {
                    ui.painter().set(
                        highlight,
                        egui::Shape::rect_filled(
                            response.header.rect,
                            ui.visuals().window_corner_radius,
                            p.violet.gamma_multiply(0.25 + 0.45 * flash),
                        ),
                    );
                }
                gallery::context_menu(&response.header, p, world, st, *id, actions, false);
                if is_current && ui.is_rect_visible(response.header.rect) {
                    let rect = response.header.rect;
                    let left = rect.left() + ui.spacing().indent;
                    if let Some(offset) = widgets::faux_bold_offset(ui) {
                        let pos = egui::pos2(left, rect.center().y - galley.size().y * 0.5);
                        let name = outfit_label(ui, &f.info.name, false, font_id, p.ink, galley.size().x);
                        ui.painter().galley(pos + offset, name, p.ink);
                    }
                    worn_tag(ui, p, left + galley.size().x, rect.center().y);
                }
                if reveal && response.body.is_some() {
                    response.header.scroll_to_me(Some(egui::Align::Min));
                    st.reveal_outfit = None;
                }
                response.body.is_some()
            })
            .inner;
        // closed headers follow one another at the inventory's row pitch
        if !open {
            row_gap(ui);
        }
        if open {
            open_now.insert(*id);
        } else {
            let height = ui.cursor().top() - top;
            st.rows.list.insert(*id, height);
            st.rows.list_typical = Some(height);
        }
    }
    st.list_open = Some(open_now);
}

fn worn_list(ui: &mut egui::Ui, p: &Palette, world: &World, st: &mut AppearanceUi, actions: &mut Vec<Action>, edit: bool) {
    let inv = &world.inventory;
    let mut links = model::cof(inv).map(|id| model::folder_links(inv, id)).unwrap_or_default();
    let worn = world.worn_attachment_items();
    let labels = items::worn_labels(world);
    for id in &worn {
        if !links.iter().any(|l| l.target == *id)
            && let Some(it) = inv.items.get(id)
        {
            links.push(aurora_net::outfits::OutfitLink {
                target: *id,
                name: it.name.clone(),
                desc: it.desc.clone(),
                folder: false,
                inv_type: it.inv_type,
            });
        }
    }
    items::sort_links(inv, &mut links, true);
    // each clothing type's layers as shown, furthest from the body first
    let mut groups: HashMap<u32, Vec<Uuid>> = HashMap::new();
    for link in &links {
        if let Some(kind) = layers::clothing_kind(inv, link) {
            groups.entry(kind).or_default().push(link.target);
        }
    }
    if edit {
        st.layers.begin(ui.ctx());
    }
    let q = st.search.trim().to_lowercase();
    let mut present = HashSet::new();
    for link in links
        .iter()
        .filter(|l| !l.folder && (edit || q.is_empty() || l.name.to_lowercase().contains(&q)))
    {
        let Some(it) = inv.items.get(&link.target) else {
            ui.label(
                RichText::new(aurora_i18n::tr!("status-loading", value0 = link.name.to_string()))
                    .size(12.0)
                    .color(p.muted),
            );
            continue;
        };
        if edit
            && !match st.edit_tab {
                0 => it.asset_type == 5,
                1 => it.asset_type == 6,
                _ => it.asset_type == 13,
            }
        {
            continue;
        }
        present.insert(it.flags & 0xff);
        // the layers of its type, when it is clothing: the editor moves them
        let group = (it.asset_type == 5).then(|| groups.get(&(it.flags & 0xff))).flatten();
        let draggable = edit && group.is_some_and(|g| g.len() > 1);
        ui.push_id(it.id, |ui| {
            let resp = sensed_row(
                ui,
                p,
                it,
                if it.asset_type == 6 && !worn.contains(&it.id) {
                    aurora_i18n::tr!("ui-appearance-attaching")
                } else {
                    ""
                },
                st.selected_item == Some(it.id),
                labels.get(&it.id).map(String::as_str),
                if draggable {
                    egui::Sense::click_and_drag()
                } else {
                    egui::Sense::click()
                },
                draggable.then(|| aurora_i18n::tr!("ui-appearance-drag-layer-hint")),
            );
            if draggable {
                st.layers.row(it.id, it.flags & 0xff, &resp);
            }
            if resp.clicked() {
                st.selected_item = Some(it.id);
            }
            item_menu(&resp, p, it, actions);
            // a body part has none of these buttons: no empty row under it
            if edit && st.selected_item == Some(it.id) && it.asset_type != 13 {
                ui.horizontal(|ui| {
                    if icon_button(
                        ui,
                        p,
                        "circle-x-filled",
                        if it.asset_type == 6 {
                            aurora_i18n::tr!("common-detach")
                        } else {
                            aurora_i18n::tr!("common-remove")
                        },
                    )
                    .clicked()
                    {
                        actions.push(Action::Remove(it.id));
                    }
                    // up goes over the others, away from the body; no arrow
                    // past either end (LLCOFWearables::buildClothingListItem)
                    if let Some(group) = group {
                        let at = group.iter().position(|id| *id == it.id);
                        if at.is_some_and(|at| at > 0)
                            && icon_button(ui, p, "caret-up-filled", aurora_i18n::tr!("ui-appearance-move-layer-up")).clicked()
                        {
                            actions.push(Action::MoveLayer(it.id, false));
                        }
                        if at.is_some_and(|at| at + 1 < group.len())
                            && icon_button(ui, p, "caret-down-filled", aurora_i18n::tr!("ui-appearance-move-layer-down")).clicked()
                        {
                            actions.push(Action::MoveLayer(it.id, true));
                        }
                    }
                });
            }
        });
        row_gap(ui);
    }
    if edit && let Some((id, body)) = st.layers.finish(ui, p) {
        actions.push(Action::PlaceLayer(id, body));
    }
    if edit && st.edit_tab != 1 {
        for (kind, name) in model::wearables()
            .iter()
            .enumerate()
            .filter(|(k, _)| if st.edit_tab == 0 { *k >= 4 } else { *k < 4 })
        {
            if !present.contains(&(kind as u32)) {
                let ending = match kind {
                    0 | 1 | 4 | 8 | 12 => "e",
                    6 | 7 | 15 => "es",
                    2 | 3 | 9 => "s",
                    _ => "",
                };
                not_worn_row(
                    ui,
                    p,
                    super::inventory::item_icon(if kind < 4 { 13 } else { 5 }, 18, kind as u32),
                    &aurora_i18n::tr!("ui-appearance-not-worn", name = name.to_string(), ending = ending.to_string()),
                );
                row_gap(ui);
            }
        }
    }
}

/// LLPanelWearing's "Temporary attachments" (tab_temp_attachments): what an
/// experience or a script attached with no item of ours, named by
/// AttachmentResources, with LLTempAttachmentsContextMenu (Touch, Edit,
/// Detach). The section opens by itself when one more arrives, so that it is
/// not missed under the worn items.
fn temporary_attachments(ui: &mut egui::Ui, p: &Palette, world: &World, st: &mut AppearanceUi) {
    let temporary = world.temp_attachments();
    let reveal = (temporary.len() > st.temporary_seen).then_some(true);
    st.temporary_seen = temporary.len();
    let title = aurora_i18n::tr!("ui-appearance-temporary-items", count = temporary.len());
    widgets::collapsing_forced(ui, p, "temporary_attachments", title, false, reveal, |ui| {
        if temporary.is_empty() {
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-appearance-no-temporary-items"))
                    .size(12.0)
                    .color(p.muted_dim),
            );
            return;
        }
        let labels = items::worn_labels(world);
        for t in &temporary {
            // a row like the worn items': the object icon, its name, its point
            let it = InvItem {
                id: t.id,
                name: world
                    .temp_attachment_names
                    .get(&t.id)
                    .map_or_else(|| aurora_i18n::tr!("ui-appearance-temporary-object").into(), str::to_owned),
                asset_type: 6,
                inv_type: 6,
                ..Default::default()
            };
            ui.push_id(t.id, |ui| {
                let resp = row(
                    ui,
                    p,
                    &it,
                    "",
                    st.selected_item == Some(t.id),
                    labels.get(&t.id).map(String::as_str),
                )
                .on_hover_text(aurora_i18n::tr!("ui-appearance-temporary-tip"));
                if resp.clicked() || resp.secondary_clicked() {
                    st.selected_item = Some(t.id);
                }
                temporary_menu(&resp, p, t);
            });
            row_gap(ui);
        }
    });
}

/// LLTempAttachmentsContextMenu::updateMenuItemsVisibility: Touch, Edit and
/// Detach; the inventory entries (Take off, Show original, favorites) do not
/// apply to an object with no item.
fn temporary_menu(response: &egui::Response, p: &Palette, t: &crate::world::temp_attachments::TempAttachment) {
    use crate::ui::context::{CtxAction, request};
    menu::context_menu(response, p, |ui| {
        if menu::item_if(ui, p, "click-filled", aurora_i18n::tr!("ui-context-touch"), t.touch) {
            request(ui.ctx(), CtxAction::Touch(t.key.local_id));
        }
        if menu::item(ui, p, "pencil-filled", aurora_i18n::tr!("common-edit")) {
            request(ui.ctx(), CtxAction::EditAttachment(t.key));
        }
        // ObjectDetach, as LLSelectMgr::sendDetach: there is no COF link
        if menu::item(ui, p, "circle-x-filled", aurora_i18n::tr!("common-detach")) {
            request(ui.ctx(), CtxAction::Detach(t.key));
        }
    });
}

fn add_list(ui: &mut egui::Ui, p: &Palette, world: &mut World, st: &mut AppearanceUi, actions: &mut Vec<Action>) {
    widgets::search_field(
        ui,
        &mut st.add_search,
        aurora_i18n::tr!("ui-appearance-filter-wearable-items"),
        ui.available_width(),
    );
    let folders: Vec<_> = world
        .inventory
        .folders
        .values()
        .filter(|f| !f.library && matches!(f.info.type_default, 5 | 6 | 13))
        .map(|f| f.info.id)
        .collect();
    for id in folders {
        if st.add_search.trim().is_empty() {
            add_folder(ui, p, &mut world.inventory, id, st, actions, 0);
        } else {
            world.inventory.request(id);
        }
    }
    let q = st.add_search.trim().to_lowercase();
    let worn: HashSet<_> = model::cof(&world.inventory)
        .map(|id| model::folder_links(&world.inventory, id))
        .unwrap_or_default()
        .into_iter()
        .map(|l| l.target)
        .collect();
    let mut items: Vec<_> = world
        .inventory
        .items
        .values()
        .filter(|it| {
            matches!(it.asset_type, 5 | 6 | 13)
                && !worn.contains(&it.id)
                && it.name.to_lowercase().contains(&q)
                && (!q.is_empty() || it.parent == world.inventory.root)
                && world.inventory.folders.get(&it.parent).is_some_and(|f| !f.library)
        })
        .collect();
    items.sort_by_key(|it| it.name.to_lowercase());
    for it in items {
        let response = row(ui, p, it, "", st.selected_item == Some(it.id), None);
        row_gap(ui);
        if response.clicked() {
            st.selected_item = Some(it.id);
        }
        if response.double_clicked() {
            actions.push(Action::Add(it.id));
            st.adding = false;
        }
    }
}

/// Lazy folder traversal keeps nested inventory available without fetching it all.
fn add_folder(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &mut crate::world::inventory::Inventory,
    id: Uuid,
    st: &mut AppearanceUi,
    actions: &mut Vec<Action>,
    depth: usize,
) {
    if depth > 24 {
        return;
    }
    let Some(f) = inv.folders.get(&id) else {
        return;
    };
    let (name, children, items) = (f.info.name.clone(), f.children.clone(), f.items.clone());
    widgets::collapsing(ui, p, ("add_outfit_folder", id), name, depth == 0, |ui| {
        inv.request(id);
        for child in children {
            add_folder(ui, p, inv, child, st, actions, depth + 1);
        }
        let worn: HashSet<_> = model::cof(inv)
            .map(|id| model::folder_links(inv, id))
            .unwrap_or_default()
            .into_iter()
            .map(|l| l.target)
            .collect();
        for id in items {
            let Some(it) = inv.items.get(&id) else {
                continue;
            };
            let it = if it.asset_type == 24 {
                inv.items.get(&it.asset_id).unwrap_or(it)
            } else {
                it
            };
            if !matches!(it.asset_type, 5 | 6 | 13) || worn.contains(&it.id) {
                continue;
            }
            let response = row(ui, p, it, "", st.selected_item == Some(it.id), None);
            row_gap(ui);
            if response.clicked() {
                st.selected_item = Some(it.id);
            }
            if response.double_clicked() {
                actions.push(Action::Add(it.id));
                st.adding = false;
            }
        }
    });
}

pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    world: &mut World,
    st: &mut AppearanceUi,
    open: &mut bool,
    complexity: u32,
    images: &HashMap<Uuid, egui::TextureHandle>,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let mut floater = widgets::Floater::new(
        "appearance",
        aurora_i18n::tr!("common-appearance"),
        egui::pos2(18.0, 60.0),
        Vec2::new(580.0, 650.0),
    )
    .help(aurora_i18n::tr!("ui-appearance-choose-an-outfit-manage-worn-items-or-save-your"));
    floater.min_size = Vec2::new(390.0, 520.0);
    if *open && !st.editing && st.tab == 2 {
        let temporary = world.temp_attachments();
        world.request_temp_attachment_names(&temporary, std::time::Instant::now());
    }
    // what the window shows is gathered only when it is drawn
    floater.show(ctx, p, open, |ui| {
        let inv = &world.inventory;
        let base = model::base(inv);
        let name = base
            .and_then(|id| inv.folders.get(&id))
            .map(|f| f.info.name.clone())
            .unwrap_or_else(|| aurora_i18n::tr!("common-current-outfit").into());
        let dirty = model::dirty(inv);
        let count = world.worn_attachment_items().len();
        if let Some(id) = st.reveal_outfit {
            st.open_groups_of(inv, id);
        }
        let (entries, outfits, total) = {
            let shown = st.shown(inv);
            (shown.entries.clone(), shown.outfits.clone(), shown.total)
        };
        ui.set_width(ui.available_width());
        if st.editing {
            ui.horizontal(|ui| {
                if icon_button(ui, p, "arrow-left", aurora_i18n::tr!("ui-appearance-back-to-outfits")).clicked() {
                    st.editing = false;
                    st.adding = false;
                }
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-appearance-edit-clothing"))
                        .size(20.0)
                        .strong()
                        .color(p.ink),
                );
            });
        }
        let shape = crate::shape_editor::worn_shape(world);
        widgets::surface(ui, p, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                // locate and edit outfit while browsing outfits, edit shape when one is worn
                let buttons = if st.editing { 0.0 } else { 2.0 } + if shape.is_some() { 1.0 } else { 0.0 };
                let spacing = ui.spacing().item_spacing.x;
                let name_width = (ui.available_width() - 30.0 - buttons * (28.0 + spacing) - spacing).max(100.0);
                icon(ui, "shirt-filled", 30.0, p.muted);
                ui.vertical(|ui| {
                    ui.set_width(name_width);
                    ui.label(
                        RichText::new(if st.pending.is_some() {
                            aurora_i18n::tr!("ui-appearance-updating-outfit")
                        } else if dirty {
                            aurora_i18n::tr!("ui-appearance-unsaved-changes")
                        } else {
                            aurora_i18n::tr!("common-current-outfit")
                        })
                        .size(11.0)
                        .color(if dirty { p.warn } else { p.muted }),
                    );
                    ui.add(egui::Label::new(RichText::new(&name).size(16.0).strong().color(p.ink)).truncate());
                });
                // right to left: edit outfit, edit shape, then locate next to the name
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !st.editing && icon_button(ui, p, "edit-filled", aurora_i18n::tr!("ui-appearance-edit-this-outfit")).clicked() {
                        st.editing = true;
                    }
                    if let Some(shape) = shape
                        && icon_button(ui, p, "man-filled", aurora_i18n::tr!("shape-edit-worn")).clicked()
                    {
                        super::context::request(ui.ctx(), super::context::CtxAction::EditWorn(shape));
                    }
                    if !st.editing {
                        let can_locate = locatable(inv).is_some();
                        let locate = ui
                            .add_enabled_ui(can_locate, |ui| {
                                icon_button(
                                    ui,
                                    p,
                                    "current-location-filled",
                                    aurora_i18n::tr!("ui-appearance-show-current-outfit"),
                                )
                            })
                            .inner
                            .on_disabled_hover_text(aurora_i18n::tr!("ui-appearance-current-outfit-not-in-my-outfits"));
                        if locate.clicked() {
                            st.locate_current(inv);
                        }
                    }
                });
            });
        });
        ui.add_space(8.0);
        if !st.editing {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 28.0 - ui.spacing().item_spacing.x).max(100.0);
                ui.add_sized(
                    Vec2::new(width, 28.0),
                    egui::TextEdit::singleline(&mut st.search)
                        .hint_text(aurora_i18n::tr!("ui-appearance-filter-outfits"))
                        .align(egui::Align2::LEFT_CENTER),
                );
                icon_menu(
                    ui,
                    p,
                    "sort-ascending-2-filled",
                    aurora_i18n::tr!("ui-appearance-sort-outfits"),
                    |ui| {
                        for (reverse, icon, label) in [
                            (false, "sort-ascending-letters", aurora_i18n::tr!("ui-appearance-name-a-z")),
                            (true, "sort-descending-letters", aurora_i18n::tr!("ui-appearance-name-z-a")),
                        ] {
                            if menu::check(ui, p, icon, label, st.reverse_sort == reverse, true) {
                                st.reverse_sort = reverse;
                            }
                        }
                    },
                );
            });
            let worn = aurora_i18n::tr!("ui-appearance-worn-38-attachments", count = format!("{}", count));
            widgets::tabs(
                ui,
                p,
                &mut st.tab,
                &[
                    (aurora_i18n::tr!("ui-appearance-outfit-gallery"), true),
                    (aurora_i18n::tr!("ui-appearance-outfits"), true),
                    (&worn, true),
                ],
            );
        } else {
            widgets::tabs(
                ui,
                p,
                &mut st.edit_tab,
                &[
                    (aurora_i18n::tr!("common-clothing"), true),
                    (aurora_i18n::tr!("ui-appearance-attached-items"), true),
                    (aurora_i18n::tr!("common-body"), true),
                ],
            );
        }
        ui.add_space(5.0);
        // The list fills what the rows beneath it leave, measured last pass:
        // a guess too small (buttons wrapped in a narrow window, a long
        // message) made the window grow to fit them, the list take the new
        // room, and the window grow again at every frame.
        let below_id = ui.scope_id().with("appearance_below_list");
        let estimate = if st.editing || st.tab == 2 { 130.0 } else { 154.0 };
        let reserved = ui.ctx().data(|d| d.get_temp::<f32>(below_id)).unwrap_or(estimate);
        let list_top = ui.cursor().top();
        let height = (ui.available_height() - reserved).max(100.0);
        egui::Frame::new()
            .fill(p.field.gamma_multiply(0.65))
            .corner_radius(ui.visuals().window_corner_radius)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let list_width = ui.available_width();
                egui::ScrollArea::vertical()
                    .id_salt("appearance_list")
                    .max_height(height)
                    .min_scrolled_height(height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_enabled_ui(st.pending.is_none(), |ui| {
                            if st.editing && st.adding {
                                add_list(ui, p, world, st, &mut actions);
                            } else if st.editing {
                                worn_list(ui, p, world, st, &mut actions, true);
                            } else {
                                match st.tab {
                                    0 => gallery(ui, p, world, st, &entries, &outfits, base, &mut actions, images, list_width),
                                    1 => outfit_list(ui, p, world, st, &entries, &outfits, base, &mut actions),
                                    _ => {
                                        ui.label(RichText::new(aurora_i18n::tr!("ui-appearance-items-to-wear")).color(p.muted));
                                        worn_list(ui, p, world, st, &mut actions, false);
                                    }
                                }
                            }
                            if !st.editing && st.tab < 2 && entries.is_empty() {
                                ui.label(RichText::new(aurora_i18n::tr!("ui-appearance-no-outfits-to-display")).color(p.muted));
                            }
                        });
                    });
            });
        if !st.editing && st.tab == 2 {
            temporary_attachments(ui, p, world, st);
        }
        if st.editing {
            ui.horizontal(|ui| {
                if widgets::flat_button(
                    ui,
                    p,
                    if st.adding {
                        aurora_i18n::tr!("ui-appearance-back-to-worn-items")
                    } else {
                        aurora_i18n::tr!("ui-appearance-add-more")
                    },
                )
                .clicked()
                {
                    st.adding = !st.adding;
                    st.selected_item = None;
                }
                if st.adding
                    && ui
                        .add_enabled(
                            st.selected_item.is_some() && st.pending.is_none(),
                            egui::Button::new(aurora_i18n::tr!("common-wear")),
                        )
                        .clicked()
                    && let Some(id) = st.selected_item
                {
                    actions.push(Action::Add(id));
                    st.adding = false;
                }
            });
        }
        if !st.editing && st.tab < 2 {
            widgets::surface(ui, p, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    icon(ui, "hanger-2-filled", 28.0, p.violet_light);
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        let selection = st.outfit_selection.ordered(&outfits);
                        let selected = st.outfit_selection.focus.and_then(|id| world.inventory.folders.get(&id));
                        let multiple = aurora_i18n::tr!("ui-appearance-selected-outfits", count = selection.len());
                        let name = if selection.len() > 1 {
                            &multiple
                        } else {
                            selected.map_or(aurora_i18n::tr!("workspace-outfit-selection"), |f| f.info.name.as_str())
                        };
                        ui.add(egui::Label::new(RichText::new(name).size(13.0).strong().color(p.ink)).truncate())
                            .on_hover_text(name);
                        if !selection.is_empty() {
                            ui.label(
                                RichText::new(aurora_i18n::tr!(
                                    "workspace-outfit-items",
                                    count = st.selection_info(&world.inventory, selection).0
                                ))
                                .size(11.0)
                                .color(p.muted),
                            );
                        }
                    });
                });
            });
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            icon_menu(ui, p, "settings-filled", aurora_i18n::tr!("ui-appearance-outfit-options"), |ui| {
                if !st.editing && st.tab == 0 {
                    menu::toggle(
                        ui,
                        p,
                        "photo-filled",
                        aurora_i18n::tr!("ui-appearance-fill-previews"),
                        &mut st.fill_previews,
                    );
                    menu::separator(ui, p);
                }
                if !st.editing && st.tab < 2 {
                    folders::new_folder_item(ui, p, &world.inventory, st);
                }
                if menu::item(ui, p, "refresh", aurora_i18n::tr!("ui-appearance-refresh")) {
                    st.refresh(&mut world.inventory);
                }
                let restore = aurora_i18n::tr!("ui-appearance-restore-saved-outfit");
                if menu::item_if(ui, p, "restore", restore, base.is_some() && st.pending.is_none()) {
                    actions.push(Action::Revert);
                }
            });
            if !st.editing && st.tab < 2 {
                ui.label(
                    RichText::new(aurora_i18n::tr!("outfit-count", count = total))
                        .size(11.0)
                        .color(p.muted),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(aurora_i18n::tr!(
                        "common-complexity",
                        value0 = super::hud::group_digits(complexity).to_string()
                    ))
                    .size(11.0)
                    .color(p.muted),
                );
            });
        });
        if !st.message.is_empty() {
            ui.add(egui::Label::new(RichText::new(&st.message).size(11.0).color(p.warn)).wrap());
        }
        widgets::divider(ui, p);
        let buttons = ui.horizontal_wrapped(|ui| {
            let available = st.pending.is_none() && model::cof(&world.inventory).is_some_and(|id| model::complete(&world.inventory, id));
            use widgets::{ActionKind, action_button, action_button_if};
            let size = Vec2::new(0.0, 26.0);
            let (save, can_save) = (aurora_i18n::tr!("common-save"), available && base.is_some() && dirty);
            if action_button_if(ui, p, "device-floppy-filled", save, ActionKind::Flat, size, can_save).clicked() {
                actions.push(Action::Save(None));
            }
            let save_as = aurora_i18n::tr!("ui-appearance-save-as");
            if action_button_if(ui, p, "device-floppy-filled", save_as, ActionKind::Flat, size, available).clicked() {
                st.begin_save_as(&world.inventory);
            }
            if st.editing {
                if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    if dirty && base.is_some() {
                        actions.push(Action::Revert);
                    }
                    st.editing = false;
                    st.adding = false;
                }
            } else if st.tab < 2 {
                let selection = st.outfit_selection.ordered(&outfits);
                let ready = available && st.selection_info(&world.inventory, selection).1;
                for (icon, label, kind, append) in [
                    ("plus", aurora_i18n::tr!("common-add"), ActionKind::Flat, true),
                    ("shirt-filled", aurora_i18n::tr!("common-wear"), ActionKind::Primary, false),
                ] {
                    if action_button_if(ui, p, icon, label, kind, size, ready).clicked() {
                        actions.push(st.wear_selection(&outfits, append));
                    }
                }
            }
        });
        let below = buttons.response.rect.bottom() - list_top - height;
        // egui's sizing pass of a new window measures them 2 points short
        if !ui.is_sizing_pass() && (below - reserved).abs() > 0.5 {
            ui.ctx().data_mut(|d| d.insert_temp(below_id, below));
            ui.ctx().request_discard("appearance rows below the list changed height");
        }
    });
    if let Some(mut name) = st.save_as.clone() {
        let mut close = false;
        let response = crate::ui::widgets::Modal::new(egui::Id::unique("outfit_save_as")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new(aurora_i18n::tr!("common-save-as")).size(17.0).strong().color(p.ink));
            ui.add_space(6.0);
            ui.label(aurora_i18n::tr!("ui-appearance-save-what-i-am-wearing-as-a-new-outfit"));
            ui.add_space(8.0);
            let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY).char_limit(63));
            if st.select_name {
                r.request_focus();
                if let Some(mut state) = egui::TextEdit::load_state(ctx, r.id) {
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(name.chars().count()),
                    )));
                    state.store(ctx, r.id);
                }
                st.select_name = false;
            }
            save_in(ui, &world.inventory, st);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let enter = widgets::enter_pressed(ui);
                let valid = !name.trim().is_empty() && st.pending.is_none();
                use widgets::{ActionKind, action_button, action_button_if};
                let size = Vec2::new(0.0, 26.0);
                if action_button_if(ui, p, "check", aurora_i18n::tr!("common-ok"), ActionKind::Primary, size, valid).clicked()
                    || (valid && enter)
                {
                    actions.push(
                        match st
                            .save_in
                            .filter(|id| crate::world::outfit_folders::is_group(&world.inventory, *id))
                        {
                            Some(parent) => Action::SaveIn(name.trim().into(), parent),
                            None => Action::Save(Some(name.trim().into())),
                        },
                    );
                    close = true;
                }
                if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    close = true;
                }
            });
        });
        st.save_as = if close || response.should_close() { None } else { Some(name) };
    }
    items::profile(ctx, p, world, st, &mut actions);
    gallery::dialogs(ctx, p, world, st, images, &mut actions);
    folders::dialogs(ctx, p, world, st, &mut actions);
    actions
}

/// "Save in": the folder of My Outfits that takes the new outfit, offered
/// once My Outfits has groups. An Aurora addition: the Legacy Viewer saves
/// every new outfit at the top of My Outfits (LLAppearanceMgr::makeNewOutfitLinks).
fn save_in(ui: &mut egui::Ui, inv: &crate::world::inventory::Inventory, st: &mut AppearanceUi) {
    let targets = crate::world::outfit_folders::targets(inv);
    if targets.len() < 2 {
        return;
    }
    let name = |id: Uuid, depth: usize| {
        if depth == 0 {
            aurora_i18n::tr!("common-my-outfits").to_string()
        } else {
            inv.folders.get(&id).map(|f| f.info.name.clone()).unwrap_or_default()
        }
    };
    let chosen = targets.iter().find(|(id, _)| Some(*id) == st.save_in).or(targets.first()).copied();
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(aurora_i18n::tr!("ui-appearance-folders-save-in"));
        egui::ComboBox::from_id_salt("outfit_save_in")
            .width(ui.available_width())
            .selected_text(chosen.map(|(id, depth)| name(id, depth)).unwrap_or_default())
            .show_ui(ui, |ui| {
                for (id, depth) in &targets {
                    let label = format!("{}{}", "   ".repeat(*depth), name(*id, *depth));
                    if ui.selectable_label(chosen.is_some_and(|(c, _)| c == *id), label).clicked() {
                        st.save_in = Some(*id);
                    }
                }
            });
    });
}

/// `AURORA_DEMO_OUTFITS=<count>,scroll`: from frame 700 the wheel scrolls
/// the outfit list or gallery of the default window, down then up every 300
/// frames, to measure the frames of a scrolled wardrobe.
pub fn demo_events(frame: u64) -> Vec<egui::Event> {
    if frame < 700 || !model::demo_outfit_mode("scroll") {
        return Vec::new();
    }
    let down = ((frame - 700) / 300).is_multiple_of(2);
    vec![
        egui::Event::PointerMoved(egui::pos2(300.0, 450.0)),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, if down { -120.0 } else { 120.0 }),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

/// The synthetic drag of `AURORA_DEMO_APPEARANCE=layers-drag|layers-hold`.
pub fn layer_demo_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    layers::demo_events(ctx, frame)
}

/// Local test cards: portrait, landscape and square, with a central disc
/// and edge marks that make cropping and distortion visible without assets.
pub fn demo_previews(ctx: &egui::Context, inv: &mut crate::world::inventory::Inventory, images: &mut HashMap<Uuid, egui::TextureHandle>) {
    let p = crate::theme::Theme::default().palette();
    for (folder, size, color) in [(703, [240, 400], p.rose), (704, [400, 180], p.teal), (705, [256, 256], p.amber)] {
        let id = Uuid::from_u128(80_000 + folder);
        let mut image = egui::ColorImage::filled(size, p.field);
        let centre = egui::vec2(size[0] as f32, size[1] as f32) * 0.5;
        let radius = centre.min_elem() * 0.6;
        for y in 0..size[1] {
            for x in 0..size[0] {
                let point = egui::vec2(x as f32, y as f32);
                let edge = x < 10 || y < 10 || x + 10 >= size[0] || y + 10 >= size[1];
                image[(x, y)] = if edge || (point - centre).length_sq() < radius * radius {
                    color
                } else if x % 40 < 2 || y % 40 < 2 {
                    p.muted
                } else {
                    p.field
                };
            }
        }
        if let Some(folder) = inv.folders.get_mut(&Uuid::from_u128(folder)) {
            folder.info.thumbnail = id;
        }
        images.insert(
            id,
            ctx.load_texture(format!("demo-outfit-{folder}"), image, egui::TextureOptions::LINEAR),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outfit_previews_fit_or_fill_without_distortion_or_overflow() {
        let frame = egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(240.0, 210.0));
        for size in [egui::vec2(240.0, 400.0), egui::vec2(400.0, 180.0), Vec2::splat(256.0), frame.size()] {
            for fill in [false, true] {
                let (rect, uv) = preview_rects(frame, size, fill);
                assert!(frame.expand(1e-4).contains_rect(rect), "{size:?}, fill={fill}: {rect:?}");
                assert_eq!(rect.center(), frame.center());
                assert_eq!(uv.center(), egui::pos2(0.5, 0.5));
                assert!(uv.min.x >= 0.0 && uv.min.y >= 0.0 && uv.max.x <= 1.0 && uv.max.y <= 1.0);
                let source = size * uv.size();
                assert!((source.x / source.y - rect.width() / rect.height()).abs() < 1e-5);
                if fill {
                    assert_eq!(rect, frame);
                    assert!((uv.width().max(uv.height()) - 1.0).abs() < 1e-5);
                } else {
                    assert_eq!(uv.min, egui::Pos2::ZERO);
                    assert_eq!(uv.max, egui::pos2(1.0, 1.0));
                    assert!((rect.width() - frame.width()).abs() < 1e-4 || (rect.height() - frame.height()).abs() < 1e-4);
                }
            }
        }
        let (_, portrait) = preview_rects(frame, egui::vec2(240.0, 400.0), true);
        assert_eq!(portrait.min.x, 0.0);
        assert!(portrait.min.y > 0.0);
        let (_, landscape) = preview_rects(frame, egui::vec2(400.0, 180.0), true);
        assert!(landscape.min.x > 0.0);
        assert_eq!(landscape.min.y, 0.0);
    }

    #[test]
    fn outfit_previews_default_to_fit_and_remember_the_saved_choice() {
        use crate::settings::Settings;
        let old: Settings = serde_json::from_str("{}").expect("old settings");
        assert!(!old.appearance_fill_previews);
        for fill in [true, false] {
            let settings = Settings {
                appearance_fill_previews: fill,
                ..Default::default()
            };
            let saved = serde_json::to_vec(&settings).expect("save settings");
            let restored: Settings = serde_json::from_slice(&saved).expect("restore settings");
            assert_eq!(restored.appearance_fill_previews, fill);
        }
    }

    #[test]
    fn outfit_preview_option_toggles_both_ways_in_the_gallery_menu() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let _icons = icons::Icons::load(&ctx, None);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        model::seed_demo(&mut world.inventory, Uuid::from_u128(1));
        let mut st = AppearanceUi::default();
        for _ in 0..4 {
            draw(&ctx, &mut world, &mut st, None);
        }
        let click = |st: &mut AppearanceUi, world: &mut World, pos| {
            for pressed in [true, false] {
                draw_input(
                    &ctx,
                    world,
                    st,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
                        events: vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ],
                        ..Default::default()
                    },
                );
            }
        };
        let rect = ctx.memory(|m| m.area_rect(egui::Id::unique("appearance"))).expect("appearance");
        // The options button is left of the count, in the row above the footer.
        let count = aurora_i18n::tr!("outfit-count", count = 4usize);
        let pos = label_position(&ctx, &mut world, &mut st, &count);
        let label = aurora_i18n::tr!("ui-appearance-fill-previews");
        for fill in [true, false] {
            click(&mut st, &mut world, egui::pos2(rect.left() + 20.0, pos.y));
            let pos = menu_label_position(&ctx, &mut world, &mut st, label);
            click(&mut st, &mut world, pos);
            assert_eq!(st.fill_previews, fill);
        }
    }

    /// A refused outfit change (no capabilities in the region just entered)
    /// used to leave the current outfit unloaded: "still loading" for every
    /// later change while the Appearance window stayed closed.
    #[test]
    fn a_refused_outfit_change_loads_the_outfit_again() {
        use crate::world::inventory::FetchState;
        let mut inv = crate::world::inventory::Inventory::default();
        let agent = crate::demo::DEMO_AGENT;
        model::seed_demo(&mut inv, agent);
        let cof = model::cof(&inv).expect("current outfit");
        let mut st = AppearanceUi::default();
        st.refresh(&mut inv);
        assert_eq!(inv.folders[&cof].state, FetchState::Fetching);
        assert!(inv.queue.contains(&(cof, false)));
        assert!(model::plan(&inv, Action::Add(Uuid::from_u128(723)), &HashMap::new()).is_err());
        // the fetch failed too, and a link target is missing: the next
        // refused change asks for both
        inv.queue.clear();
        inv.failed(&[cof]);
        let target = model::folder_links(&inv, cof)
            .into_iter()
            .find(|l| !l.folder)
            .expect("COF link")
            .target;
        inv.items.remove(&target);
        let commands = st.reload(&mut inv, agent, &HashSet::new());
        assert!(inv.queue.contains(&(cof, false)));
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, aurora_net::NetCommand::FetchItems { items, .. } if items.contains(&target)))
        );
    }

    #[test]
    fn wearable_rows_precede_attachments_in_outfits_worn_and_editor() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let palette = crate::theme::Theme::default().palette();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = crate::demo::DEMO_AGENT;
        model::seed_demo(&mut world.inventory, world.agent_id);
        model::seed_demo_wearables(&mut world.inventory, world.agent_id);
        let folder = Uuid::from_u128(705);
        for view in 0..3 {
            let mut st = AppearanceUi::default();
            st.open_outfit(folder);
            let mut render = || {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 1600.0))),
                        ..Default::default()
                    },
                    |ui| {
                        ui.set_width(600.0);
                        if view == 0 {
                            let entry = Entry {
                                id: folder,
                                depth: 0,
                                group: false,
                                outfits: 0,
                            };
                            outfit_list(
                                ui,
                                &palette,
                                &world,
                                &mut st,
                                &[entry],
                                &[folder],
                                model::base(&world.inventory),
                                &mut vec![],
                            );
                        } else {
                            worn_list(ui, &palette, &world, &mut st, &mut vec![], view == 2);
                        }
                    },
                )
            };
            for _ in 0..3 {
                render();
            }
            let output = render();
            let y = |id| {
                let name = &world.inventory.items[&Uuid::from_u128(id)].name;
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text().starts_with(name) => Some(text.pos.y),
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("missing {name} in view {view}"))
            };
            // The real row painter must show every system wearable above every attachment.
            for link in model::folder_links(&world.inventory, folder) {
                let item = &world.inventory.items[&link.target];
                if matches!(item.asset_type, 5 | 13) && view != 2 {
                    for attachment in [717, 718, 719, 720, 721] {
                        assert!(
                            y(item.id.as_u128()) < y(attachment),
                            "{} below attachment in view {view}",
                            item.name
                        );
                    }
                }
            }
            if view != 0 {
                assert!(y(725) < y(724) && y(724) < y(714), "tattoo layers reversed in view {view}");
            }
        }
    }

    #[test]
    fn outfits_worn_and_editor_show_actual_attachment_points() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let palette = crate::theme::Theme::default().palette();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = crate::demo::DEMO_AGENT;
        model::seed_demo(&mut world.inventory, world.agent_id);
        let folder = model::cof(&world.inventory).expect("current outfit");
        world.objects.upsert(1, crate::demo::action_avatar(false));
        for (item, point) in [(717, 2), (721, 32)] {
            let mut object = crate::demo::hud::object(point, false);
            object.full_id = Uuid::from_u128(item);
            object.name_values = format!("AttachItemID STRING RW SV {}", object.full_id);
            world.objects.upsert(1, object);
        }
        for view in 0..3 {
            let mut st = AppearanceUi::default();
            st.open_outfit(folder);
            st.edit_tab = 1;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(430.0);
                if view == 0 {
                    let entry = Entry {
                        id: folder,
                        depth: 0,
                        group: false,
                        outfits: 0,
                    };
                    outfit_list(
                        ui,
                        &palette,
                        &world,
                        &mut st,
                        &[entry],
                        &[folder],
                        model::base(&world.inventory),
                        &mut vec![],
                    );
                } else {
                    worn_list(ui, &palette, &world, &mut st, &mut vec![], view == 2);
                }
            });
            output.textures_delta.clear();
            for label in [
                aurora_i18n::tr!(
                    "ui-appearance-items-worn-on",
                    value0 = "",
                    name = aurora_i18n::tr!("ui-appearance-items-skull")
                ),
                aurora_i18n::tr!(
                    "ui-appearance-items-worn-on",
                    value0 = "HUD : ",
                    name = aurora_i18n::tr!("ui-appearance-items-top-right")
                ),
            ] {
                assert!(
                    output
                        .shapes
                        .iter()
                        .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == label)),
                    "missing attachment label in view {view}: {label}"
                );
            }
        }
    }

    fn draw(ctx: &egui::Context, world: &mut World, st: &mut AppearanceUi, key: Option<egui::Key>) -> Vec<Action> {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
            ..Default::default()
        };
        if let Some(key) = key {
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            });
        }
        draw_input(ctx, world, st, input)
    }

    fn draw_input(ctx: &egui::Context, world: &mut World, st: &mut AppearanceUi, input: egui::RawInput) -> Vec<Action> {
        draw_output(ctx, world, st, input).0
    }

    fn modified_click(
        ctx: &egui::Context,
        world: &mut World,
        st: &mut AppearanceUi,
        pos: egui::Pos2,
        modifiers: egui::Modifiers,
        button: egui::PointerButton,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        for pressed in [true, false] {
            actions.extend(draw_input(
                ctx,
                world,
                st,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
                    time: Some(ctx.input(|i| i.time) + 0.4),
                    events: vec![
                        egui::Event::ModifiersChanged(modifiers),
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button,
                            pressed,
                            modifiers,
                        },
                    ],
                    ..Default::default()
                },
            ));
        }
        actions
    }

    fn outfit_position(ctx: &egui::Context, world: &mut World, st: &mut AppearanceUi, id: Uuid) -> egui::Pos2 {
        let name = world.inventory.folders[&id].info.name.clone();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
            ..Default::default()
        };
        // The current outfit also names the banner above the scroll area.
        // The row/tile has the smaller, scrolling clip rectangle.
        let pos = draw_output(ctx, world, st, input)
            .1
            .iter()
            .filter_map(|s| text_position(&s.shape, &name).map(|pos| (s.clip_rect.height(), pos)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .expect("outfit row/tile")
            .1;
        pos + if st.tab == 0 { egui::vec2(10.0, -50.0) } else { Vec2::ZERO }
    }

    #[test]
    fn appearance_multiselection_pointer_events_and_bulk_buttons_in_both_views() {
        for tab in [0, 1] {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let _icons = icons::Icons::load(&ctx, None);
            let mut world = wardrobe(0);
            for (id, name) in [(702, "00 Empty"), (703, "01 Base"), (704, "02 Accessories"), (705, "03 Current")] {
                world
                    .inventory
                    .folders
                    .get_mut(&Uuid::from_u128(id))
                    .expect("fixture outfit")
                    .info
                    .name = name.into();
            }
            world.inventory.generation += 1;
            let mut st = AppearanceUi { tab, ..Default::default() };
            for _ in 0..5 {
                draw(&ctx, &mut world, &mut st, None);
            }
            let order = st.shown_outfits(&world.inventory);
            let first = order.iter().position(|id| *id == Uuid::from_u128(702)).expect("empty outfit");
            for (step, (index, modifiers)) in [
                (first, egui::Modifiers::NONE),
                (2, egui::Modifiers::CTRL),
                (1, egui::Modifiers::SHIFT),
            ]
            .into_iter()
            .enumerate()
            {
                let pos = outfit_position(&ctx, &mut world, &mut st, order[index]);
                assert!(modified_click(&ctx, &mut world, &mut st, pos, modifiers, egui::PointerButton::Primary).is_empty());
                let expected = match step {
                    0 => vec![order[first]],
                    1 => vec![order[first], order[2]],
                    _ => order[1..=2].to_vec(),
                };
                assert_eq!(st.outfit_selection.ordered(&order), expected, "view {tab}, click {index}");
                for _ in 0..4 {
                    draw(&ctx, &mut world, &mut st, None);
                }
            }
            for (label, append) in [(aurora_i18n::tr!("common-add"), true), (aurora_i18n::tr!("common-wear"), false)] {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
                    ..Default::default()
                };
                let pos = draw_output(&ctx, &mut world, &mut st, input)
                    .1
                    .iter()
                    .filter_map(|s| text_position(&s.shape, label))
                    .max_by(|a, b| a.y.total_cmp(&b.y))
                    .expect("footer button");
                let actions = modified_click(&ctx, &mut world, &mut st, pos, egui::Modifiers::NONE, egui::PointerButton::Primary);
                assert!(
                    matches!(actions.as_slice(), [Action::WearOutfits { folders, append: actual }]
                    if *actual == append && folders == &order[1..=2]),
                    "view {tab}: {actions:?}"
                );
                assert!(model::plan(&world.inventory, actions[0].clone(), &HashMap::new()).is_ok());
            }
            let pos = outfit_position(&ctx, &mut world, &mut st, order[1]);
            modified_click(
                &ctx,
                &mut world,
                &mut st,
                pos,
                egui::Modifiers::NONE,
                egui::PointerButton::Secondary,
            );
            assert_eq!(st.outfit_selection.ordered(&order), order[1..=2]);
            for _ in 0..4 {
                draw(&ctx, &mut world, &mut st, None);
            }
            let pos = menu_label_position(
                &ctx,
                &mut world,
                &mut st,
                aurora_i18n::tr!("ui-appearance-gallery-add-to-your-outfit"),
            );
            let actions = modified_click(&ctx, &mut world, &mut st, pos, egui::Modifiers::NONE, egui::PointerButton::Primary);
            assert!(
                matches!(actions.as_slice(), [Action::WearOutfits { folders, append: true }] if folders == &order[1..=2]),
                "{actions:?}"
            );
        }
    }

    #[test]
    fn appearance_multiselection_filtering_clears_hidden_actions_and_rechecks_loading() {
        let mut world = wardrobe(0);
        let mut st = AppearanceUi::default();
        st.demo_multiselection();
        let order = st.shown_outfits(&world.inventory);
        let ids = st.outfit_selection.ordered(&order);
        assert!(st.selection_info(&world.inventory, ids.clone()).1);
        st.refresh(&mut world.inventory);
        assert!(!st.selection_info(&world.inventory, ids.clone()).1);
        world.inventory.folders.get_mut(&ids[0]).expect("folder").state = crate::world::inventory::FetchState::Fetching;
        world.inventory.generation += 1;
        assert!(!st.selection_info(&world.inventory, ids).1);
        st.search = "no matching outfit".into();
        assert!(st.shown_outfits(&world.inventory).is_empty());
        assert!(st.outfit_selection.ordered(&order).is_empty());
        assert!(!st.selection_info(&world.inventory, vec![]).1);
    }

    fn text_position(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(text) if text.galley.text().starts_with(label) => Some(text.pos + egui::vec2(3.0, 5.0)),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| text_position(s, label)),
            _ => None,
        }
    }

    fn label_position(ctx: &egui::Context, world: &mut World, st: &mut AppearanceUi, label: &str) -> egui::Pos2 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
            ..Default::default()
        };
        draw_output(ctx, world, st, input)
            .1
            .iter()
            .find_map(|s| text_position(&s.shape, label))
            .expect(label)
    }

    /// `label_position` in an open menu: its row rather than a button of the
    /// same name that the menu covers. A menu is clipped to the screen, the
    /// window's widgets to the window.
    fn menu_label_position(ctx: &egui::Context, world: &mut World, st: &mut AppearanceUi, label: &str) -> egui::Pos2 {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0));
        let input = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        draw_output(ctx, world, st, input)
            .1
            .iter()
            .filter(|s| s.clip_rect.contains_rect(screen))
            .find_map(|s| text_position(&s.shape, label))
            .expect(label)
    }

    fn draw_output(
        ctx: &egui::Context,
        world: &mut World,
        st: &mut AppearanceUi,
        input: egui::RawInput,
    ) -> (Vec<Action>, Vec<egui::epaint::ClippedShape>) {
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(input, |ui| {
            actions = show(
                ui.ctx(),
                &crate::theme::Theme::default().palette(),
                world,
                st,
                &mut true,
                41_748,
                &HashMap::new(),
            );
        });
        // Headless tests do not submit texture uploads to a renderer.
        output.textures_delta.clear();
        (actions, output.shapes)
    }

    #[test]
    fn floater_stays_compact_and_save_as_rejects_empty_names() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let _icons = icons::Icons::load(&ctx, None);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        model::seed_demo(&mut world.inventory, Uuid::from_u128(1));
        let mut st = AppearanceUi::default();
        for _ in 0..80 {
            draw(&ctx, &mut world, &mut st, None);
        }
        let rect = ctx.memory(|m| m.area_rect(egui::Id::unique("appearance"))).expect("floater");
        assert!(rect.width() < 620.0 && rect.height() < 720.0, "{rect:?}");
        st.save_as = Some("   ".into());
        draw(&ctx, &mut world, &mut st, None);
        assert!(draw(&ctx, &mut world, &mut st, Some(egui::Key::Enter)).is_empty());
        assert!(st.save_as.is_some());
        st.save_as = Some(" Tenue de test ".into());
        let actions = draw(&ctx, &mut world, &mut st, Some(egui::Key::Enter));
        assert!(matches!(&actions[..],[Action::Save(Some(name))] if name=="Tenue de test"));
        assert!(st.save_as.is_none());
    }

    /// The list used to take the window's height minus a fixed guess of the
    /// rows beneath it; once those rows wrapped (a narrow window, French
    /// labels) the window grew to fit them, the list took the new room, and
    /// the window grew again at every frame, while dragging and after.
    #[test]
    fn narrow_window_keeps_its_height_with_an_empty_gallery() {
        for (edge, delta) in [((1.0, 0.5), Vec2::new(-250.0, 0.0)), ((1.0, 1.0), Vec2::new(-250.0, -300.0))] {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let _icons = icons::Icons::load(&ctx, None);
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            model::seed_demo(&mut world.inventory, Uuid::from_u128(1));
            let mut st = AppearanceUi {
                search: "no outfit has this name".into(),
                ..Default::default()
            };
            let area = || ctx.memory(|m| m.area_rect(egui::Id::unique("appearance"))).expect("floater");
            for _ in 0..10 {
                draw(&ctx, &mut world, &mut st, None);
            }
            let rect = area();
            assert_eq!(rect.height(), 650.0, "opens at its default height");
            // just inside the grabbed edge or corner
            let grab = rect.min + rect.size() * Vec2::new(edge.0, edge.1) - Vec2::new(1.0, if edge.1 == 1.0 { 1.0 } else { 0.0 });
            let input = |pos: egui::Pos2, pressed: Option<bool>| egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
                events: std::iter::once(egui::Event::PointerMoved(pos))
                    .chain(pressed.map(|pressed| egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    }))
                    .collect(),
                ..Default::default()
            };
            draw_input(&ctx, &mut world, &mut st, input(grab, None));
            draw_input(&ctx, &mut world, &mut st, input(grab, Some(true)));
            for i in 1..=40 {
                draw_input(&ctx, &mut world, &mut st, input(grab + delta * (i as f32 / 20.0).min(1.0), None));
            }
            let dragged = area();
            assert_eq!(dragged.width(), 390.0, "down to its minimum width");
            draw_input(&ctx, &mut world, &mut st, input(grab + delta, Some(false)));
            for _ in 0..20 {
                draw(&ctx, &mut world, &mut st, None);
            }
            let rest = area();
            assert_eq!(rest.size(), dragged.size(), "the window stops where it was released");
            assert!(rest.height() <= rect.height() + 0.5, "{edge:?}: {rect:?} -> {rest:?}");
        }
    }

    #[test]
    fn right_click_outfit_item_opens_a_persistent_menu() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let _icons = icons::Icons::load(&ctx, None);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        model::seed_demo(&mut world.inventory, Uuid::from_u128(1));
        let target = Uuid::from_u128(723);
        let links: Vec<_> = world.inventory.folders[&Uuid::from_u128(704)]
            .items
            .iter()
            .copied()
            .filter(|id| world.inventory.items[id].asset_id == target)
            .collect();
        world.inventory.folders.get_mut(&Uuid::from_u128(704)).expect("outfit").items = links;
        let mut st = AppearanceUi::default();
        st.open_outfit(Uuid::from_u128(704));
        st.reveal_outfit = None; // keep preceding headings visible for these item clicks
        for _ in 0..4 {
            draw(&ctx, &mut world, &mut st, None);
        }
        let label = world.inventory.items[&Uuid::from_u128(723)].name.clone();
        let pos = label_position(&ctx, &mut world, &mut st, &label);
        for pressed in [true, false] {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            };
            draw_input(&ctx, &mut world, &mut st, input);
        }
        assert_eq!(st.selected_item, Some(Uuid::from_u128(723)), "clicked outfit item");
        assert!(egui::Popup::is_any_open(&ctx), "right-click menu");
        for _ in 0..4 {
            draw(&ctx, &mut world, &mut st, None);
        }
        assert!(egui::Popup::is_any_open(&ctx), "menu must remain open");
    }

    #[test]
    fn add_from_context_menu_dispatches_one_action_for_objects_and_clothes() {
        for item in [Uuid::from_u128(722), Uuid::from_u128(723)] {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let _icons = icons::Icons::load(&ctx, None);
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            model::seed_demo(&mut world.inventory, Uuid::from_u128(1));
            let folder = Uuid::from_u128(704);
            let mut link = world.inventory.items[&world.inventory.folders[&folder].items[0]].clone();
            link.asset_id = item;
            link.name = world.inventory.items[&item].name.clone();
            link.id = Uuid::from_u128(990);
            let first = link.id;
            world.inventory.items.insert(first, link.clone());
            link.id = Uuid::from_u128(991);
            let second = link.id;
            world.inventory.items.insert(second, link);
            world.inventory.folders.get_mut(&folder).expect("outfit").items = vec![first, second];
            let mut st = AppearanceUi::default();
            st.open_outfit(folder);
            st.reveal_outfit = None;
            for _ in 0..4 {
                draw(&ctx, &mut world, &mut st, None);
            }
            let mut actions = Vec::new();
            let label = world.inventory.items[&item].name.clone();
            for (label, button, menu) in [
                (label.as_str(), egui::PointerButton::Secondary, false),
                (aurora_i18n::tr!("common-add"), egui::PointerButton::Primary, true),
            ] {
                let pos = if menu {
                    menu_label_position(&ctx, &mut world, &mut st, label)
                } else {
                    label_position(&ctx, &mut world, &mut st, label)
                };
                for pressed in [true, false] {
                    actions.extend(draw_input(
                        &ctx,
                        &mut world,
                        &mut st,
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
                            events: vec![
                                egui::Event::PointerMoved(pos),
                                egui::Event::PointerButton {
                                    pos,
                                    button,
                                    pressed,
                                    modifiers: Default::default(),
                                },
                            ],
                            ..Default::default()
                        },
                    ));
                }
                for _ in 0..4 {
                    actions.extend(draw(&ctx, &mut world, &mut st, None));
                }
            }
            assert!(
                matches!(&actions[..], [Action::WearItem { item: id, replace: false, point: 0 }] if *id == item),
                "{actions:?}"
            );
        }
    }

    #[test]
    fn gallery_double_click_reopens_the_folder_without_wearing_it() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let _icons = icons::Icons::load(&ctx, None);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        model::seed_demo(&mut world.inventory, Uuid::from_u128(1));
        let mut st = AppearanceUi::default();
        st.open(1, false);
        // Establish persisted closed headers before returning to the gallery.
        for _ in 0..4 {
            draw(&ctx, &mut world, &mut st, None);
        }
        st.open(0, false);
        for _ in 0..4 {
            draw(&ctx, &mut world, &mut st, None);
        }
        let label = world.inventory.folders[&Uuid::from_u128(703)].info.name.clone();
        let pos = label_position(&ctx, &mut world, &mut st, &label) + egui::vec2(10.0, -50.0);
        let mut actions = Vec::new();
        for _ in 0..2 {
            for pressed in [true, false] {
                actions.extend(draw_input(
                    &ctx,
                    &mut world,
                    &mut st,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
                        events: vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ],
                        ..Default::default()
                    },
                ));
            }
        }
        assert!(actions.is_empty(), "double-click must not dispatch Wear: {actions:?}");
        assert_eq!(st.tab, 1);
        assert_eq!(st.outfit_selection.focus, Some(Uuid::from_u128(703)));
        for _ in 0..4 {
            draw(&ctx, &mut world, &mut st, None);
        }
        assert!(
            st.reveal_outfit.is_none(),
            "the previously closed folder has opened and been scrolled to"
        );
    }

    /// A closed group hides its outfits from the list, the selection and
    /// the ranges; a filter shows every group open; locating the worn outfit
    /// opens the groups that hold it.
    #[test]
    fn closed_groups_hide_their_outfits_until_a_filter_or_a_locate() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        model::seed_demo(&mut world.inventory, Uuid::from_u128(1));
        crate::world::outfit_folders::seed_demo(&mut world.inventory, Uuid::from_u128(1));
        let (beach, night, club) = (Uuid::from_u128(741), Uuid::from_u128(742), Uuid::from_u128(740));
        let mut st = AppearanceUi {
            tab: 1,
            ..Default::default()
        };
        assert_eq!(st.shown_outfits(&world.inventory).len(), 8);
        st.outfit_selection.single(Uuid::from_u128(747));
        st.toggle_group(beach);
        let shown = st.shown_outfits(&world.inventory);
        assert_eq!(shown.len(), 6);
        assert!(!shown.contains(&Uuid::from_u128(746)) && !shown.contains(&Uuid::from_u128(747)));
        assert!(
            st.outfit_selection.ordered(&shown).is_empty(),
            "the hidden outfit leaves the selection"
        );
        let entries = &st.shown(&world.inventory).entries;
        assert!(entries.iter().any(|e| e.id == beach) && !entries.iter().any(|e| e.id == night));
        assert_eq!(st.shown(&world.inventory).total, 8, "the count keeps them");
        st.search = "night".into();
        assert_eq!(st.shown_outfits(&world.inventory), vec![Uuid::from_u128(747)]);
        st.search.clear();
        // the worn outfit moves into Night, inside the closed Beach
        let worn = model::base(&world.inventory).expect("worn outfit");
        world.inventory.folders.get_mut(&worn).expect("worn").info.parent = night;
        world.inventory.folders.get_mut(&night).expect("night").children.push(worn);
        let my = model::system_folder(&world.inventory, model::FT_MY_OUTFITS).expect("My Outfits");
        world
            .inventory
            .folders
            .get_mut(&my)
            .expect("My Outfits")
            .children
            .retain(|id| *id != worn);
        world.inventory.generation += 1;
        st.toggle_group(club);
        st.locate_current(&world.inventory);
        assert!(st.group_open(beach) && st.group_open(night) && !st.group_open(club));
        assert!(st.shown_outfits(&world.inventory).contains(&worn));
    }

    fn wardrobe(count: usize) -> World {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = crate::demo::DEMO_AGENT;
        model::seed_demo(&mut world.inventory, world.agent_id);
        model::seed_demo_many(&mut world.inventory, world.agent_id, count);
        world
    }

    fn fetched(commands: &[aurora_net::NetCommand]) -> HashSet<Uuid> {
        commands
            .iter()
            .flat_map(|c| match c {
                aurora_net::NetCommand::FetchItems { items, .. } => items.clone(),
                _ => Vec::new(),
            })
            .collect()
    }

    /// A wardrobe of thousands of outfits used to have all its links walked
    /// and copied at every frame the window was open (25 ms a frame with
    /// 3,000 outfits). They are walked again only when the inventory changed.
    #[test]
    fn prepare_walks_the_outfit_links_only_after_an_inventory_change() {
        let mut world = wardrobe(50);
        let agent = world.agent_id;
        let inv = &mut world.inventory;
        let mut st = AppearanceUi::default();
        assert!(
            fetched(&st.prepare(inv, agent, &HashSet::new())).is_empty(),
            "every original is there"
        );
        let outfit = Uuid::from_u128(2_000_010);
        assert!(model::missing_targets(inv, outfit).next().is_none());
        let target = model::folder_links(inv, outfit)[0].target;
        // unchanged generation: the links are not walked again
        let original = inv.items.remove(&target).expect("original");
        assert!(
            fetched(&st.prepare(inv, agent, &HashSet::new())).is_empty(),
            "idle frame walks nothing"
        );
        // an inventory change (an item fewer): walked again and asked for once
        inv.generation += 1;
        assert!(fetched(&st.prepare(inv, agent, &HashSet::new())).contains(&target));
        assert!(fetched(&st.prepare(inv, agent, &HashSet::new())).is_empty(), "asked for once");
        // a refresh forgets what was asked for: asked for again
        st.refresh(inv);
        assert!(fetched(&st.prepare(inv, agent, &HashSet::new())).contains(&target));
        // the original arrives and the outfit is loaded again; a new link
        // to a missing original is found by its outfit's change alone
        inv.add_items(vec![original]);
        inv.folders.get_mut(&outfit).expect("outfit").state = crate::world::inventory::FetchState::Fetched;
        let ghost = Uuid::from_u128(99);
        let mut link = inv.items[&inv.folders[&outfit].items[0]].clone();
        link.id = Uuid::from_u128(98);
        link.asset_id = ghost;
        inv.add_items(vec![link]);
        assert_eq!(fetched(&st.prepare(inv, agent, &HashSet::new())), HashSet::from([ghost]));
        // the folders are asked for at every frame: a reset state is seen at once
        inv.queue.clear();
        inv.folders.get_mut(&outfit).expect("outfit").state = crate::world::inventory::FetchState::Unknown;
        st.prepare(inv, agent, &HashSet::new());
        assert!(inv.queue.contains(&(outfit, false)));
    }

    #[test]
    fn the_outfit_list_is_built_again_only_when_its_inputs_change() {
        let mut world = wardrobe(30);
        let inv = &mut world.inventory;
        let mut st = AppearanceUi::default();
        let first = st.shown_outfits(inv);
        assert_eq!(first.len(), 34, "4 demo outfits and 30 generated");
        let name = |inv: &crate::world::inventory::Inventory, id: &Uuid| inv.folders[id].info.name.to_lowercase();
        assert!(first.windows(2).all(|w| name(inv, &w[0]) <= name(inv, &w[1])), "sorted by name");
        // renamed without an inventory change: the list is not rebuilt
        let last = *first.last().expect("outfit");
        inv.folders.get_mut(&last).expect("outfit").info.name = "  a first outfit".into();
        assert_eq!(st.shown_outfits(inv), first, "idle frame keeps the list");
        // the inventory changed: rebuilt
        inv.generation += 1;
        assert_eq!(st.shown_outfits(inv)[0], last);
        // the filter and the order rebuild it too
        st.search = " BEACH ".into();
        let beach = st.shown_outfits(inv);
        assert_eq!(beach.len(), 3);
        assert!(beach.iter().all(|id| name(inv, id).contains("beach")));
        st.reverse_sort = true;
        let reversed = st.shown_outfits(inv);
        assert_eq!(reversed.iter().rev().copied().collect::<Vec<_>>(), beach);
    }

    #[test]
    fn rows_within_a_row_of_the_view_are_in_sight() {
        let view = egui::Rect::from_min_max(egui::pos2(0.0, 100.0), egui::pos2(300.0, 400.0));
        assert!(in_sight(100.0, 30.0, view));
        assert!(in_sight(45.0, 30.0, view), "the row before the view");
        assert!(!in_sight(30.0, 30.0, view));
        assert!(in_sight(425.0, 30.0, view), "the row after the view");
        assert!(!in_sight(431.0, 30.0, view));
    }

    /// Where the outfit list or gallery shows `name` within its view: the
    /// room above and below its text, `None` out of sight. The banner names
    /// the current outfit too, clipped to the whole window; the list's clip
    /// rectangle is the smaller one.
    fn listed_in_view(ctx: &egui::Context, world: &mut World, st: &mut AppearanceUi, name: &str) -> Option<(f32, f32)> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
            ..Default::default()
        };
        let shapes = draw_output(ctx, world, st, input).1;
        let hits: Vec<_> = shapes
            .iter()
            .filter_map(|s| text_position(&s.shape, name).map(|pos| (s.clip_rect, pos)))
            .collect();
        let window = hits.iter().map(|(clip, _)| clip.height()).fold(0.0, f32::max);
        hits.iter()
            .find(|(clip, pos)| clip.height() < window && clip.contains(*pos))
            .map(|(clip, pos)| (pos.y - clip.top(), clip.bottom() - pos.y))
    }

    /// The banner's locate button brings the current outfit into view and
    /// selects it, in the gallery and the list, through a filter that hid
    /// it and from the worn tab (#971).
    #[test]
    fn locating_the_current_outfit_scrolls_to_it_and_selects_it() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let _icons = icons::Icons::load(&ctx, None);
        let mut world = wardrobe(400);
        let current = model::base(&world.inventory).expect("current outfit");
        // in the middle of 404 outfits, far out of sight: an outfit at the
        // end would be reached by any scroll that overshoots
        let name = "Lzz Current";
        world.inventory.folders.get_mut(&current).expect("outfit").info.name = name.into();
        world.inventory.generation += 1;
        assert_eq!(locatable(&world.inventory), Some(current));
        // last, a window opened on the outfit to locate (the demo's `locate`)
        for (tab, filter, shown_tab, opening) in [(0, "", 0, false), (1, "beach", 1, false), (2, "", 0, false), (0, "", 0, true)] {
            let ctx = if opening { egui::Context::default() } else { ctx.clone() };
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let _icons = icons::Icons::load(&ctx, None);
            let mut st = AppearanceUi::default();
            st.open(tab, false);
            st.search = filter.into();
            if !opening {
                for _ in 0..3 {
                    draw(&ctx, &mut world, &mut st, None);
                }
                assert!(
                    listed_in_view(&ctx, &mut world, &mut st, name).is_none(),
                    "out of sight first, tab {tab}"
                );
            }
            st.locate_current(&world.inventory);
            // the scroll is animated
            for _ in 0..120 {
                draw(&ctx, &mut world, &mut st, None);
            }
            assert!(st.locate.is_none(), "located, tab {tab}");
            assert_eq!(st.tab, shown_tab);
            assert!(st.search.is_empty(), "the filter that hid it is cleared");
            let shown = st.shown_outfits(&world.inventory);
            assert_eq!(st.outfit_selection.ordered(&shown), vec![current]);
            let (above, below) = listed_in_view(&ctx, &mut world, &mut st, name).expect("in sight");
            if shown_tab == 0 {
                // the tile above its name is whole, in the middle of the view
                assert!(above > 160.0 && below > 20.0, "centred, tab {tab}: {above} above, {below} below");
            } else {
                assert!(above < 40.0, "header at the top: {above} above");
            }
        }
        // an outfit worn from outside My Outfits cannot be shown there
        let my = model::system_folder(&world.inventory, model::FT_MY_OUTFITS).expect("My Outfits");
        world
            .inventory
            .folders
            .get_mut(&my)
            .expect("My Outfits")
            .children
            .retain(|id| *id != current);
        assert_eq!(locatable(&world.inventory), None);
        let mut st = AppearanceUi::default();
        st.locate_current(&world.inventory);
        assert!(st.locate.is_none());
    }

    fn outfit_names(shapes: &[egui::epaint::ClippedShape], world: &World) -> HashMap<String, egui::Pos2> {
        fn collect(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
            match shape {
                egui::Shape::Text(t) => out.push((t.galley.text().to_owned(), t.pos)),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let names: HashSet<_> = world.inventory.folders.values().map(|f| f.info.name.as_str()).collect();
        let mut out = Vec::new();
        shapes.iter().for_each(|s| collect(&s.shape, &mut out));
        out.into_iter().filter(|(text, _)| names.contains(text.as_str())).collect()
    }

    /// Only the outfits in sight are laid out, where the whole list puts
    /// them, and the widgets keep their ids: an item's menu stays open
    /// across frames that draw the whole list.
    #[test]
    fn a_long_outfit_list_lays_out_what_is_in_sight_where_the_whole_list_does() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let _icons = icons::Icons::load(&ctx, None);
        let mut world = wardrobe(400);
        let mut st = AppearanceUi::default();
        let outfit = Uuid::from_u128(2_000_200);
        st.open_outfit(outfit);
        // the scroll to the open outfit is animated
        for _ in 0..120 {
            draw(&ctx, &mut world, &mut st, None);
        }
        assert!(st.reveal_outfit.is_none(), "scrolled to the open outfit");
        assert!(st.list_open.as_ref().is_some_and(|open| open.contains(&outfit)));
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
            ..Default::default()
        };
        // right click an item of the open outfit, then draw the whole list
        let mut links = model::folder_links(&world.inventory, outfit);
        items::sort_links(&world.inventory, &mut links, false);
        let label = links
            .into_iter()
            .find_map(|l| world.inventory.items.get(&l.target).map(|it| it.name.clone()))
            .expect("item");
        let pos = label_position(&ctx, &mut world, &mut st, &label);
        for pressed in [true, false] {
            let mut input = input();
            input.events = vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: Default::default(),
                },
            ];
            draw_input(&ctx, &mut world, &mut st, input);
        }
        assert!(egui::Popup::is_any_open(&ctx), "right-click menu");
        for _ in 0..3 {
            st.list_open = None;
            draw(&ctx, &mut world, &mut st, None);
            draw(&ctx, &mut world, &mut st, None);
        }
        assert!(egui::Popup::is_any_open(&ctx), "the menu stays open across whole-list frames");
        draw(&ctx, &mut world, &mut st, Some(egui::Key::Escape));
        // up a little: closed headers above the open outfit come into sight
        let mut wheel = input();
        wheel.events = vec![
            egui::Event::PointerMoved(egui::pos2(300.0, 400.0)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, 200.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            },
        ];
        draw_input(&ctx, &mut world, &mut st, wheel);
        for _ in 0..120 {
            draw(&ctx, &mut world, &mut st, None);
        }
        let settled = outfit_names(&draw_output(&ctx, &mut world, &mut st, input()).1, &world);
        st.list_open = None;
        let whole = outfit_names(&draw_output(&ctx, &mut world, &mut st, input()).1, &world);
        let in_sight = outfit_names(&draw_output(&ctx, &mut world, &mut st, input()).1, &world);
        assert!(in_sight.len() > 8, "{}", in_sight.len());
        assert_eq!(whole, settled, "the whole list puts the headers where they were");
        assert_eq!(in_sight, whole, "the headers in sight sit where the whole list puts them");
    }

    /// The gallery lays out only its rows in sight, where the whole gallery
    /// puts them, and keeps the whole gallery's height.
    #[test]
    fn the_gallery_lays_out_its_rows_in_sight_where_the_whole_gallery_does() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let _icons = icons::Icons::load(&ctx, None);
        let palette = crate::theme::Theme::default().palette();
        let world = wardrobe(200);
        let mut st = AppearanceUi::default();
        let outfits = st.shown_outfits(&world.inventory);
        let entries = st.shown(&world.inventory).entries.clone();
        let base = model::base(&world.inventory);
        let mut run = |clip: Option<egui::Rect>| {
            let mut height = 0.0;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(560.0);
                if let Some(clip) = clip {
                    ui.set_clip_rect(clip);
                }
                let top = ui.cursor().top();
                gallery(
                    ui,
                    &palette,
                    &world,
                    &mut st,
                    &entries,
                    &outfits,
                    base,
                    &mut vec![],
                    &HashMap::new(),
                    560.0,
                );
                height = ui.cursor().top() - top;
            });
            output.textures_delta.clear();
            (outfit_names(&output.shapes, &world), height)
        };
        let (whole, whole_height) = run(Some(egui::Rect::from_min_max(egui::pos2(0.0, -1e6), egui::pos2(560.0, 1e6))));
        let (part, part_height) = run(Some(egui::Rect::from_min_max(egui::pos2(0.0, 6000.0), egui::pos2(560.0, 6600.0))));
        assert_eq!(whole.len(), outfits.len());
        assert!(part.len() < 30 && part.len() > 6, "{}", part.len());
        assert_eq!(part_height, whole_height);
        for (text, pos) in &part {
            if pos.y > 1000.0 {
                assert_eq!(whole.get(text), Some(pos), "{text} moved");
            }
        }
    }

    /// The list keeps the whole list's height and positions while it lays
    /// out only the headers in sight (an open outfit included).
    #[test]
    fn the_outfit_list_keeps_its_height_while_it_lays_out_what_is_in_sight() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let _icons = icons::Icons::load(&ctx, None);
        let palette = crate::theme::Theme::default().palette();
        let world = wardrobe(300);
        let mut st = AppearanceUi {
            tab: 1,
            ..Default::default()
        };
        let outfits = st.shown_outfits(&world.inventory);
        let entries = st.shown(&world.inventory).entries.clone();
        st.outfit_selection.single(outfits[150]);
        let base = model::base(&world.inventory);
        let everything = egui::Rect::from_min_max(egui::pos2(0.0, -1e6), egui::pos2(560.0, 1e6));
        let mut run = |clip: egui::Rect| {
            let mut height = 0.0;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(560.0);
                ui.set_clip_rect(clip);
                let top = ui.cursor().top();
                outfit_list(ui, &palette, &world, &mut st, &entries, &outfits, base, &mut vec![]);
                height = ui.cursor().top() - top;
            });
            output.textures_delta.clear();
            (outfit_names(&output.shapes, &world), height)
        };
        // a few frames for the selected outfit to open
        for _ in 0..30 {
            run(everything);
        }
        let (whole, whole_height) = run(everything);
        assert_eq!(whole.len(), outfits.len());
        // past the open outfit, whose body is laid out above the view
        let view = egui::Rect::from_min_max(egui::pos2(0.0, 6000.0), egui::pos2(560.0, 6600.0));
        let (part, part_height) = run(view);
        assert_eq!(part_height, whole_height);
        assert!(part.len() > 10 && part.len() < 40, "{}", part.len());
        for (text, pos) in &part {
            assert_eq!(whole.get(text), Some(pos), "{text} moved");
        }
    }
}
