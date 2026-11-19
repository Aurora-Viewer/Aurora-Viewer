//! The explorer's tiles: the gallery and combination views of the Legacy
//! Viewer's single folder mode, port of LLInventoryGallery
//! (indra/newview/llinventorygallery.cpp, originally LGPL 2.1). Each entry
//! of the folder is a square picture over its name, as many per row as the
//! width holds; the gallery view tiles every entry, the combination view
//! the entries that have a picture, above the list of the others, with a
//! bar between them that sets their heights.
//!
//! Only the tiles in sight are laid out and only their pictures are asked
//! for: a folder of thousands of entries costs what its visible rows cost,
//! and its pictures go through the interface images' bounded cache
//! (`app/ui_image_cache.rs`) like the outfit gallery's. What a tile does
//! (selection, drag and drop, tooltip, menus, double-click) is the rows'
//! own `item_response` / `folder_response`.
//!
//! Aurora differences: the tiles' side can be set (64 to 256 px, Ctrl and
//! the wheel over them; the Legacy Viewer's is fixed at 128), and an
//! entry's rights show as the list's letters on its picture instead of a
//! suffix after its name.

use super::super::permissions::{Permissions, Rights};
use super::super::rows::Tiles;
use super::super::*;
use super::{in_rect, rights, rows as row};
use crate::ui::{sheet, widgets};
use egui::{Color32, Pos2, Rect, Sense, Vec2};
use std::collections::HashMap;

/// The Legacy Viewer's picture side (panel_inventory_gallery_item.xml).
pub const DEFAULT_SIDE: f32 = 128.0;
pub const MIN_SIDE: f32 = 64.0;
pub const MAX_SIDE: f32 = 256.0;
/// One Ctrl+wheel notch.
const STEP: f32 = 16.0;
/// Inside a tile, around its picture and name.
const PAD: f32 = 4.0;
/// Between tiles (LLInventoryGallery's horizontal and vertical gaps).
const GAP: f32 = 10.0;
const NAME: f32 = 12.5;
const ICON: f32 = 16.0;

/// The picture side the preferences give.
pub fn side(prefs: &view::ExplorerPreferences) -> f32 {
    if prefs.tile_size == 0 {
        DEFAULT_SIDE
    } else {
        f32::from(prefs.tile_size).clamp(MIN_SIDE, MAX_SIDE)
    }
}

/// The side after `notches` of the wheel (Ctrl held), in steps of `STEP`.
pub fn zoomed(side: f32, notches: i32) -> u16 {
    let side = ((side / STEP).round() + notches as f32) * STEP;
    side.clamp(MIN_SIDE, MAX_SIDE) as u16
}

/// Where the tiles go in a width: as many columns as fit
/// (LLInventoryGallery::updateRowsIfNeeded), one at least.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub columns: usize,
    pub side: f32,
    pub tile: Vec2,
    gap: f32,
}

/// How the explorer's tiles look: the gallery's pictures over their names,
/// the Compact view's names in columns, the Tiles view's cards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Picture,
    Compact,
    Card,
}

impl Style {
    pub fn of(view: view::ExplorerView) -> Self {
        match view {
            view::ExplorerView::Compact => Self::Compact,
            view::ExplorerView::Tiles => Self::Card,
            _ => Self::Picture,
        }
    }
}

/// A Compact view's cell: an icon and a name on a row's height.
const COMPACT: Vec2 = Vec2::new(232.0, 24.0);
/// A Tiles view's card: a 48 px picture beside three lines.
const CARD: Vec2 = Vec2::new(288.0, 60.0);
const CARD_PICTURE: f32 = 48.0;

impl Grid {
    pub fn new(width: f32, side: f32, name_h: f32) -> Self {
        let tile = Vec2::new(side + 2.0 * PAD, PAD + side + PAD + name_h + PAD);
        Self::cells(width, tile, side, GAP)
    }

    fn cells(width: f32, tile: Vec2, side: f32, gap: f32) -> Self {
        let columns = (((width + gap) / (tile.x + gap)).floor() as usize).max(1);
        Self { columns, side, tile, gap }
    }

    /// The grid of a view's tiles in `width`.
    pub fn for_style(style: Style, width: f32, side: f32, name_h: f32) -> Self {
        match style {
            Style::Picture => Self::new(width, side, name_h),
            Style::Compact => Self::cells(width, COMPACT, 16.0, 4.0),
            Style::Card => Self::cells(width, CARD, CARD_PICTURE, 8.0),
        }
    }

    fn pitch(&self) -> Vec2 {
        self.tile + Vec2::splat(self.gap)
    }

    pub fn rows(&self, count: usize) -> usize {
        count.div_ceil(self.columns)
    }

    /// The height of `count` tiles.
    pub fn height(&self, count: usize) -> f32 {
        (self.rows(count) as f32 * self.pitch().y - self.gap).max(0.0)
    }

    /// The tiles a band from `top` to `bottom` of the content crosses.
    pub fn visible(&self, count: usize, top: f32, bottom: f32) -> std::ops::Range<usize> {
        let pitch = self.pitch().y;
        let first = (top / pitch).floor().max(0.0) as usize;
        let last = (bottom / pitch).ceil().max(0.0) as usize;
        (first * self.columns).min(count)..(last * self.columns).min(count)
    }

    pub fn rect(&self, origin: Pos2, index: usize) -> Rect {
        let (row, column) = (index / self.columns, index % self.columns);
        Rect::from_min_size(
            origin + Vec2::new(column as f32 * self.pitch().x, row as f32 * self.pitch().y),
            self.tile,
        )
    }

    /// The offset that brings tile `index` fully in sight, moving no more
    /// than needed (LLInventoryGallery::scrollToShowItem).
    pub fn scroll_to(&self, index: usize, offset: f32, viewport: f32) -> f32 {
        let top = (index / self.columns) as f32 * self.pitch().y;
        offset.min(top).max(top + self.tile.y - viewport).max(0.0)
    }

    /// The tile an arrow leads to from `index`, among `tiles`; None at the
    /// edge. Down from the last row leaves the tiles (`tiles` itself) when
    /// a list follows them (`more`).
    pub fn step(&self, index: usize, tiles: usize, arrow: egui::Key, more: bool) -> Option<usize> {
        let cols = self.columns;
        match arrow {
            egui::Key::ArrowLeft => index.checked_sub(1),
            egui::Key::ArrowRight => (index + 1 < tiles).then_some(index + 1),
            egui::Key::ArrowUp => index.checked_sub(cols),
            egui::Key::ArrowDown if index + cols < tiles => Some(index + cols),
            // a shorter last row: its last tile
            egui::Key::ArrowDown if index / cols + 1 < self.rows(tiles) => Some(tiles - 1),
            egui::Key::ArrowDown if more => Some(tiles),
            _ => None,
        }
    }
}

/// The height of a tile's name: two lines, as the Legacy Viewer's.
fn name_height(ui: &egui::Ui) -> f32 {
    2.0 * ui.fonts_mut(|fonts| fonts.row_height(&egui::FontId::proportional(NAME)))
}

/// The view of the explorer's folder.
pub fn tiles(view: view::ExplorerView) -> Tiles {
    match view {
        view::ExplorerView::List => Tiles::None,
        view::ExplorerView::Gallery => Tiles::All,
        view::ExplorerView::Combination => Tiles::Pictured,
        view::ExplorerView::Compact | view::ExplorerView::Tiles => Tiles::Listed,
    }
}

/// The gallery view: every entry of the explorer's folder as a tile, in
/// `rect`. The list's keyboard, scroll hold and focus are its own.
#[allow(clippy::too_many_arguments)]
pub(super) fn gallery(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    images: &HashMap<Uuid, egui::TextureHandle>,
    actions: &mut Vec<InvAction>,
    height: f32,
    root: Uuid,
    style: Style,
) -> egui::scroll_area::ScrollAreaOutput<()> {
    // As `list`: the place kept while the inventory changes, the keyboard.
    let key = ListKey {
        tab: st.tab,
        filtered: false,
        root,
    };
    let user = st.list_shown.is_some_and(|shown| user_on_list(ui, shown.rect));
    hold_list_scroll(st, inv.generation, key, user, ui.input(|i| i.time));
    clipboard::list_focus(ui, st, height);
    let restore = st.scroll_restore.take();
    let offset = st.plain_scroll_restore.take().or(restore);
    let out = pane(
        ui, p, icons, inv, st, prefs, facts, images, actions, height, root, offset, true, style,
    );
    st.plain_scroll = out.state.unclamped_offset().y;
    st.list_viewport = Some((out.state.unclamped_offset().y, out.inner_rect.height()));
    record_list_shown(ui, st, &out);
    drag::navigation::scroll(ui, &out, st.ui_id);
    out
}

/// The combination view in `rect`: the tiles of the entries that have a
/// picture, the bar that shares the room, the columns' titles and the list
/// of the others. A part with nothing to show leaves the room to the other
/// (LLPanelMainInventory::updateCombinationVisibility).
#[allow(clippy::too_many_arguments)]
pub(super) fn combination(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    images: &HashMap<Uuid, egui::TextureHandle>,
    actions: &mut Vec<InvAction>,
    rect: Rect,
    list_id: egui::Id,
    root: Uuid,
) -> egui::scroll_area::ScrollAreaOutput<()> {
    prepare(inv, st, prefs, root);
    let tiles = st.plain.tiles;
    let listed = st.plain.rows.len() - tiles;
    const HANDLE: f32 = 8.0;
    const HEADER: f32 = 26.0;
    const MIN: f32 = 60.0;
    let top = if tiles == 0 {
        0.0
    } else if listed == 0 {
        rect.height()
    } else {
        let room = rect.height() - HANDLE - HEADER;
        let max = (room - MIN).max(MIN);
        let shared = if prefs.explorer.gallery_height == 0 {
            let grid = Grid::new(
                rect.width() - ui.spacing().scroll.allocated_width(),
                side(&prefs.explorer),
                name_height(ui),
            );
            let pitch = rows::row_height(ui, p, st) + ui.spacing().item_spacing.y;
            split(room, grid.height(tiles), listed as f32 * pitch)
        } else {
            f32::from(prefs.explorer.gallery_height)
        };
        shared.clamp(MIN.min(max), max)
    };
    let top_rect = Rect::from_min_size(rect.min, Vec2::new(rect.width(), top));
    if listed == 0 {
        // the whole room is the tiles': the list's keyboard and focus too
        return in_rect(ui, top_rect, "aurora_inventory_gallery_room", |ui| {
            clipboard::list_focus(ui, st, top);
            pane(
                ui,
                p,
                icons,
                inv,
                st,
                prefs,
                facts,
                images,
                actions,
                top,
                root,
                None,
                true,
                Style::Picture,
            )
        });
    }
    if tiles > 0 {
        in_rect(ui, top_rect, "aurora_inventory_gallery_room", |ui| {
            pane(
                ui,
                p,
                icons,
                inv,
                st,
                prefs,
                facts,
                images,
                actions,
                top,
                root,
                None,
                false,
                Style::Picture,
            )
        });
    }
    let mut below = rect;
    if tiles > 0 {
        let hit = Rect::from_x_y_ranges(rect.x_range(), top_rect.bottom()..=top_rect.bottom() + HANDLE);
        let response = ui
            .interact(hit, ui.scope_id().with("inventory_gallery_split"), Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text(aurora_i18n::tr!("inventory-resize-gallery"));
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::unique("inventory_gallery_split"), hit));
        if response.hovered() || response.dragged() {
            ui.painter().hline(
                hit.x_range(),
                hit.center().y,
                widgets::border_stroke(p, egui::Stroke::new(1.0, p.violet_light)),
            );
        }
        if response.double_clicked() {
            prefs.explorer.gallery_height = 0;
        } else if response.dragged() && response.drag_delta().y != 0.0 {
            prefs.explorer.gallery_height = (top + response.drag_delta().y).max(MIN).round() as u16;
        }
        below.min.y = hit.bottom();
    }
    let header = Rect::from_min_size(below.min, Vec2::new(below.width(), HEADER));
    in_rect(ui, header, "aurora_inventory_gallery_header", |ui| {
        if let Some(frame) = st.aurora {
            row::header(ui, p, frame, list_id, prefs, st.tab, &mut st.column_fit);
            if let Some(frame) = &mut st.aurora {
                frame.columns = prefs.explorer;
            }
        }
    });
    let list_rect = Rect::from_min_max(egui::pos2(below.left(), header.bottom()), below.max);
    let browse = root;
    in_rect(ui, list_rect, "aurora_inventory_list_room", |ui| {
        list(
            ui,
            p,
            icons,
            inv,
            st,
            prefs,
            facts,
            actions,
            list_rect.height(),
            browse,
            false,
            false,
        )
    })
}

/// The explorer's folder listed with its tiles, before anything lays it out.
fn prepare(inv: &mut Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, root: Uuid) {
    st.view.prepare(inv, prefs.sort_for(st.tab));
    st.plain.prepare(&st.view, inv, root, false, prefs.show_library);
    if inv.shown_needs_fetch(root, std::time::Instant::now()) {
        inv.request(root);
    }
}

/// The tiles in their scroll area, `height` high: every row of the tree
/// before `st.plain.tiles`. `own` when the tiles are the whole list (the
/// gallery view): the "Empty" placeholder and the rows' keyboard are theirs.
#[allow(clippy::too_many_arguments)]
fn pane(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    images: &HashMap<Uuid, egui::TextureHandle>,
    actions: &mut Vec<InvAction>,
    height: f32,
    root: Uuid,
    mut offset: Option<f32>,
    own: bool,
    style: Style,
) -> egui::scroll_area::ScrollAreaOutput<()> {
    prepare(inv, st, prefs, root);
    let count = st.plain.tiles;
    let more = st.plain.rows.len() > count && !own;
    let width = (ui.available_width() - ui.spacing().scroll.allocated_width()).max(1.0);
    let grid = Grid::for_style(style, width, side(&prefs.explorer), name_height(ui));
    let navigated = keys(ui, st, grid, count, more);
    if own {
        // the deletion's reconciling and the shown order, as a list's
        keyboard::navigate(ui, inv, st, false);
    }
    // where the tiles were scrolled last, and how much of them showed
    let place = egui::Id::unique(("inventory_tiles_place", st.ui_id, own));
    let (current, viewport) = ui.ctx().data(|d| d.get_temp::<(f32, f32)>(place)).unwrap_or((0.0, height));
    let index = |st: &InventoryUi, id: Option<Uuid>| {
        id.and_then(|id| st.plain.order.iter().position(|row| *row == id))
            .filter(|index| *index < count)
    };
    if navigated {
        st.scroll_hold = None;
        if let Some(index) = index(st, st.selected) {
            offset = Some(grid.scroll_to(index, offset.unwrap_or(current), viewport.min(height)));
        }
    }
    if let Some(index) = index(st, st.reveal) {
        let top = (index / grid.columns) as f32 * grid.pitch().y;
        offset = Some((top - (height - grid.tile.y) / 2.0).max(0.0));
    }
    // Ctrl and the wheel size the gallery's pictures
    if style == Style::Picture {
        zoom(ui, &mut prefs.explorer, height);
    }
    // a drag between the tiles is no scroll: the tiles are dragged themselves
    let mut area = egui::ScrollArea::vertical()
        .id_salt("aurora_inventory_tiles")
        .auto_shrink([false, false])
        .scroll_source(egui::scroll_area::ScrollSource {
            drag: egui::scroll_area::DragScroll::Never,
            ..Default::default()
        })
        .max_height(height);
    if let Some(offset) = offset {
        area = area.vertical_scroll_offset(offset);
    }
    let out = area.show_viewport(ui, |ui, viewport| {
        let content = grid.height(count);
        ui.set_height(content.max(viewport.height()));
        let origin = ui.max_rect().min;
        // a click between the tiles takes the keyboard to the list
        let blank = ui.interact(ui.max_rect(), ui.scope_id().with("tiles_blank"), Sense::click());
        if blank.clicked() {
            clipboard::request_focus(ui, st);
        }
        // its right-click: the menu of the folder shown
        if blank.secondary_clicked()
            && st.tab == 0
            && let Some(pos) = blank.interact_pointer_pos()
        {
            st.row_menu = Some((root, pos));
            st.background_menu = true;
        }
        if count == 0 {
            if own {
                let text = match inv.folders.get(&root).map(|f| f.state) {
                    Some(FetchState::Fetched) => aurora_i18n::tr!("inventory-folder-empty"),
                    // asked again after `SHOWN_RETRY`, not loading meanwhile
                    Some(FetchState::Failed) => aurora_i18n::tr!("ui-inventory-folder-not-loaded"),
                    _ => aurora_i18n::tr!("ui-inventory-loading-folder"),
                };
                ui.painter().text(
                    Pos2::new(ui.max_rect().center().x, origin.y + 40.0),
                    egui::Align2::CENTER_CENTER,
                    text,
                    egui::FontId::proportional(12.5),
                    p.muted_dim,
                );
            }
            return;
        }
        // F2 on a tile scrolled out of sight starts its renaming too
        let renamed = st.rename.as_ref().map(|edit| edit.id).or_else(|| {
            st.selected
                .filter(|_| st.rename.is_none() && ui.input(|i| i.key_pressed(egui::Key::F2)))
        });
        let shown = grid.visible(count, viewport.min.y, viewport.max.y);
        let extra = index(st, renamed).filter(|index| !shown.contains(index));
        for index in shown.chain(extra) {
            let Some(entry) = st.plain.rows.get(index).copied() else { break };
            let rect = grid.rect(origin, index);
            let mut cell = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("inventory_tile", entry.id)));
            cell.set_clip_rect(rect.intersect(ui.clip_rect()));
            if style == Style::Picture {
                tile(&mut cell, p, icons, inv, st, prefs, facts, images, actions, entry, grid.side);
            } else {
                small_tile(&mut cell, p, icons, inv, st, prefs, facts, images, actions, entry, style);
            }
        }
    });
    let shown = (out.state.unclamped_offset().y, out.inner_rect.height());
    ui.ctx().data_mut(|d| d.insert_temp(place, shown));
    out
}

/// The tiles' share of the combination view's room, before the user drags
/// the bar (LLPanelMainInventory::updateCombinationVisibility): the list
/// takes a third and the tiles the rest; tiles that fit in their two
/// thirds keep their own height when the list needs more than its third,
/// and a list shorter than its third leaves the rest to the tiles.
fn split(room: f32, tiles: f32, list: f32) -> f32 {
    let list_default = room / 3.0;
    let tiles_default = room - list_default;
    if list > list_default && tiles < tiles_default {
        tiles
    } else if list < list_default && tiles > tiles_default {
        room - list
    } else {
        tiles_default
    }
}

/// Ctrl and the wheel over the tiles change their side, a step a notch; a
/// pinch's small steps add up to one. The window's own zoom keeps Ctrl
/// with + and −.
fn zoom(ui: &egui::Ui, prefs: &mut view::ExplorerPreferences, height: f32) {
    let rect = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), height));
    let factor = ui.input(|i| i.zoom_delta());
    if !ui.rect_contains_pointer(rect) || (factor - 1.0).abs() < 1e-4 {
        return;
    }
    let key = egui::Id::unique("inventory_tiles_zoom");
    let total = ui.ctx().data(|d| d.get_temp::<f32>(key)).unwrap_or(0.0) + factor.ln();
    let (notches, left) = zoom_steps(total);
    ui.ctx().data_mut(|d| d.insert_temp(key, left));
    if notches != 0 {
        prefs.tile_size = zoomed(side(prefs), notches);
    }
}

/// The steps in a sum of zoom logarithms, and what is left of it. A wheel
/// notch is egui's 40 points times 1/200, a logarithm of 0.2: it fires a
/// step on its own, and a step starts the sum again, so that rounding
/// never adds up to a step no notch asked for.
fn zoom_steps(total: f32) -> (i32, f32) {
    const NOTCH: f32 = 0.2;
    if total.abs() < NOTCH * 0.75 {
        return (0, total);
    }
    ((total / NOTCH).round() as i32, 0.0)
}

/// The arrows over tiles: left and right step one tile, up and down one
/// row (LLInventoryGallery::handleKeyHere and moveUp/moveDown/moveLeft/
/// moveRight); Shift extends the selection from its anchor. A selection in
/// the list after the tiles is the list's to move.
fn keys(ui: &egui::Ui, st: &mut InventoryUi, grid: Grid, tiles: usize, more: bool) -> bool {
    let arrow = |event: &egui::Event| {
        matches!(
            event,
            egui::Event::Key {
                key: egui::Key::ArrowLeft | egui::Key::ArrowRight | egui::Key::ArrowUp | egui::Key::ArrowDown,
                pressed: true,
                modifiers,
                ..
            } if !modifiers.alt && !modifiers.ctrl && !modifiers.command
        )
    };
    // the selection is looked for in the folder only when an arrow is down
    if tiles == 0 || !ui.input(|i| i.events.iter().any(arrow)) || !clipboard::list_has_focus(ui, st) || st.rename.is_some() {
        return false;
    }
    let Some(current) = st
        .selected
        .filter(|id| st.selection.contains(id))
        .and_then(|id| st.plain.order.iter().position(|row| *row == id))
        .filter(|index| *index < tiles)
    else {
        return false;
    };
    let mut moves = Vec::new();
    ui.input_mut(|input| {
        input.events.retain(|event| {
            if arrow(event)
                && let egui::Event::Key { key, modifiers, .. } = event
            {
                moves.push((*key, modifiers.shift));
                false
            } else {
                true
            }
        });
    });
    let mut index = current;
    for (key, shift) in &moves {
        let Some(next) = grid.step(index, tiles, *key, more) else {
            continue;
        };
        index = next;
        let Some(id) = st.plain.order.get(index).copied() else { break };
        if *shift {
            let anchor = st
                .anchor
                .and_then(|anchor| st.plain.order.iter().position(|row| *row == anchor))
                .unwrap_or(current);
            st.selection.clear();
            st.selection
                .extend(st.plain.order[anchor.min(index)..=anchor.max(index)].iter().copied());
        } else {
            st.selection.clear();
            st.selection.insert(id);
            st.anchor = Some(id);
        }
        st.selected = Some(id);
        st.keyboard_selection = true;
    }
    !moves.is_empty()
}

/// The pill of an item: worn (or an active gesture), a link.
fn pill(inv: &Inventory, it: &aurora_net::inventory::InvItem, facts: &Facts) -> Option<(&'static str, bool)> {
    let original = rules::original(inv, it.id).unwrap_or(it.id);
    if facts.active_gestures.contains(&original) {
        Some((aurora_i18n::tr!("gesture-active"), true))
    } else if facts.worn.contains(&original) {
        Some((aurora_i18n::tr!("common-worn"), true))
    } else if matches!(it.asset_type, 24 | 25) {
        Some((aurora_i18n::tr!("inventory-pill-link"), false))
    } else {
        None
    }
}

/// One tile: its picture (or its type's icon, large), its type's icon in a
/// corner (LLInventoryGalleryItem's thumbnail and type icon), its pill and
/// favourite star, its rights, and its name on two lines under it.
#[allow(clippy::too_many_arguments)]
fn tile(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    images: &HashMap<Uuid, egui::TextureHandle>,
    actions: &mut Vec<InvAction>,
    entry: rows::Row,
    side: f32,
) {
    let id = entry.id;
    let Some(frame) = st.aurora else { return };
    let item = inv.items.get(&id);
    let folder = inv.folders.get(&id);
    if item.is_none() && folder.is_none() {
        return;
    }
    let rect = ui.max_rect();
    let opacity = ui.opacity();
    ui.multiply_opacity(clipboard::opacity(inv, st, id));
    st.visible.push(id);
    let selected = st.selection.contains(&id);
    let picture = Rect::from_min_size(rect.min + Vec2::splat(PAD), Vec2::splat(side));
    let name_rect = Rect::from_min_max(
        Pos2::new(rect.left() + PAD, picture.bottom() + PAD),
        Pos2::new(rect.right() - PAD, rect.bottom() - PAD),
    );
    let radius = ui.visuals().window_corner_radius;
    let fill = |ui: &egui::Ui| {
        if selected {
            ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.28));
        } else if ui.rect_contains_pointer(rect) && ui.ctx().dragged_id().is_none() {
            ui.painter().rect_filled(rect, radius, p.raised);
        }
    };
    // the type's icon: an item's (a link's original's), a folder's
    let (icon, tint) = match (item, folder) {
        (Some(it), _) => inventory_item_icon(inv, it, p),
        (_, Some(f)) => (
            folder_icon(f.info.type_default, false),
            folder_colors::tint(prefs, id).unwrap_or(if selected { p.violet_pale } else { p.muted }),
        ),
        _ => return,
    };
    let draw_picture = |ui: &egui::Ui, st: &mut InventoryUi| {
        ui.painter().rect_filled(picture, radius, p.field);
        let shown = thumbnail::tile_picture(inv, id);
        // a folder without a picture of its own: those of its items
        if shown.is_none() && folder.is_some() {
            let previews = thumbnail::folder_preview(inv, id, thumbnail::preview_count(side));
            if paint_previews(ui, p, &previews, picture, images, &mut st.wanted_images) {
                let corner = Rect::from_min_size(
                    Pos2::new(picture.left() + 4.0, picture.bottom() - 4.0 - ICON - 4.0),
                    Vec2::splat(ICON + 4.0),
                );
                ui.painter().rect_filled(corner, radius, p.panel.gamma_multiply(0.85));
                paint_icon(ui, icons, icon, corner.center(), ICON, tint);
                return;
            }
        }
        let texture = shown.and_then(|asset| {
            st.wanted_images.insert(asset);
            images.get(&asset)
        });
        if let Some(texture) = texture {
            // the whole picture, its proportions kept (an outfit's texture
            // need not be square)
            let size = texture.size_vec2();
            let scale = (side / size.x.max(1.0)).min(side / size.y.max(1.0));
            let shown = Rect::from_center_size(picture.center(), size * scale);
            ui.painter().image(
                texture.id(),
                shown,
                Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                widgets::PICTURE_TINT,
            );
        } else {
            // no picture (or not yet): the type's icon, large, as
            // LLThumbnailCtrl's fallback
            let faded = if shown.is_some() { tint.gamma_multiply(0.45) } else { tint };
            paint_icon(ui, icons, icon, picture.center(), (side * 0.36).max(ICON), faded);
        }
        if texture.is_some() {
            // the type's icon in the corner, over a plate (a picture still
            // loading shows the large icon alone)
            let corner = Rect::from_min_size(
                Pos2::new(picture.left() + 4.0, picture.bottom() - 4.0 - ICON - 4.0),
                Vec2::splat(ICON + 4.0),
            );
            ui.painter().rect_filled(corner, radius, p.panel.gamma_multiply(0.85));
            paint_icon(ui, icons, icon, corner.center(), ICON, tint);
        }
    };
    // Renaming: the field in place of the name.
    let mut field = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_size(name_rect.min, Vec2::new(name_rect.width(), 22.0)))
            .id_salt(("inventory_rename_tile", id)),
    );
    if rename_row(&mut field, p, inv, id, st, actions) {
        fill(ui);
        draw_picture(ui, st);
        ui.set_opacity(opacity);
        return;
    }
    let sense = if item.is_some() || folder_draggable(inv, id, prefs) {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let key = if item.is_some() { "inv_item" } else { "inv_folder" };
    let r = ui.interact(rect, ui.make_persistent_id((key, id)), sense);
    super::marquee::record(st, id, rect);
    if st.selected == Some(id) {
        st.selected_rect = Some(rect);
    }
    #[cfg(test)]
    row::tests::record(ui, id, rect);
    fill(ui);
    draw_picture(ui, st);
    let hovered = ui.rect_contains_pointer(rect) && ui.ctx().dragged_id().is_none();
    let quick = match (hovered, item) {
        (true, Some(_)) => row::quick_buttons(inv, id, facts, frame.layout),
        _ => Vec::new(),
    };
    // the pill at the picture's top left, the star at its top right
    let pill = item.and_then(|it| pill(inv, it, facts));
    let locked = folder.and_then(|_| prefs.protected.contains(&id).then(|| aurora_i18n::tr!("inventory-pill-protected")));
    let tag_y = picture.top() + 4.0 + 9.0;
    let clip = ui.clip_rect();
    ui.set_clip_rect(clip.intersect(picture));
    if let Some((text, worn)) = pill.or(locked.map(|text| (text, false))) {
        widgets::paint_row_tag(ui, p, text, worn, picture.left() + 4.0, tag_y);
    }
    if rules::is_favorite(inv, id) {
        widgets::paint_glyph(
            ui.painter(),
            "star-filled",
            Pos2::new(picture.right() - 4.0 - 8.0, tag_y),
            14.0,
            p.amber,
        );
    }
    // the rights the list shows, at the bottom right, unless a quick
    // button takes the place
    let permissions = item.and_then(|_| Permissions::inventory(inv, id));
    if quick.is_empty()
        && frame.rights != Rights::Hidden
        && let Some(permissions) = permissions
    {
        let width = rights::width(frame.rights);
        let plate = Rect::from_min_size(
            Pos2::new(picture.right() - 4.0 - width - 8.0, picture.bottom() - 4.0 - ICON - 4.0),
            Vec2::new(width + 8.0, ICON + 4.0),
        );
        ui.painter().rect_filled(plate, radius, p.panel.gamma_multiply(0.85));
        rights::paint(ui, p, permissions, frame.rights, plate.left() + 4.0, plate.center().y, selected);
    }
    ui.set_clip_rect(clip);
    // the name, on two lines (LLInventoryGalleryItem's name box): a folder
    // loading says so, a worn item's is in the tag's face, a link's leans
    let (label, worn, link) = match (item, folder) {
        (Some(it), _) => (
            shown_name(it).to_owned(),
            pill.is_some_and(|(_, worn)| worn),
            matches!(it.asset_type, 24 | 25),
        ),
        (_, Some(f)) => {
            let name = folder_name(inv, id);
            let label = match f.state {
                FetchState::Fetching => aurora_i18n::tr!("status-name-loading", name = name),
                FetchState::Failed => aurora_i18n::tr!("ui-inventory-failed-2", name = name),
                _ => name,
            };
            (label, false, false)
        }
        _ => return,
    };
    let ink = if worn { widgets::row_tag_ink(p, true) } else { p.ink };
    let mut job = if selected || worn {
        sheet::semibold(ui, &label, NAME, ink)
    } else {
        egui::text::LayoutJob::single_section(label, egui::TextFormat::simple(egui::FontId::proportional(NAME), ink))
    };
    for section in &mut job.sections {
        section.format.italics = link;
    }
    job.wrap = egui::text::TextWrapping {
        max_width: name_rect.width().max(1.0),
        max_rows: 2,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    job.halign = egui::Align::Center;
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(Pos2::new(name_rect.center().x, name_rect.top()), galley, ink);
    ui.set_opacity(opacity);
    match (item, folder) {
        (Some(it), _) => {
            item_response(ui, p, inv, it, r, permissions, st, prefs, facts, actions);
            if !quick.is_empty() {
                ui.set_clip_rect(clip.intersect(picture));
                row::show_quick(
                    ui,
                    p,
                    inv,
                    id,
                    &quick,
                    picture.right() - 4.0,
                    picture.bottom() - 4.0 - 11.0,
                    st,
                    prefs,
                    facts,
                    actions,
                );
                ui.set_clip_rect(clip);
            }
        }
        (_, Some(_)) => {
            let mut open = false;
            let r = folder_response(ui, p, inv, id, r, None, &mut open, st, prefs, facts, actions);
            // a double-click, or cargo held over it, goes into the folder
            // (LLInventoryGalleryItem::handleDoubleClick)
            if r.double_clicked() || open {
                st.browse = id;
                st.selection.clear();
                st.selected = None;
            }
        }
        _ => {}
    }
}

/// A tile of the Compact view (an icon and the name on one line) or of the
/// Tiles view (a picture beside the name, the type and the date). What it
/// does is a row's: selection, drag, menus, renaming, double-click.
#[allow(clippy::too_many_arguments)]
fn small_tile(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    images: &HashMap<Uuid, egui::TextureHandle>,
    actions: &mut Vec<InvAction>,
    entry: rows::Row,
    style: Style,
) {
    let id = entry.id;
    let item = inv.items.get(&id);
    let folder = inv.folders.get(&id);
    if item.is_none() && folder.is_none() {
        return;
    }
    let rect = ui.max_rect();
    let opacity = ui.opacity();
    ui.multiply_opacity(clipboard::opacity(inv, st, id));
    st.visible.push(id);
    let selected = st.selection.contains(&id);
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let card = style == Style::Card;
    let picture_side = if card { CARD_PICTURE } else { ICON };
    let picture = Rect::from_min_size(
        Pos2::new(rect.left() + PAD, rect.center().y - picture_side / 2.0),
        Vec2::splat(picture_side),
    );
    let text_left = picture.right() + if card { 10.0 } else { 6.0 };
    let name_rect = Rect::from_min_max(Pos2::new(text_left, rect.top()), Pos2::new(rect.right() - PAD, rect.bottom()));
    let (icon, tint) = match (item, folder) {
        (Some(it), _) => inventory_item_icon(inv, it, p),
        (_, Some(f)) => (
            folder_icon(f.info.type_default, false),
            folder_colors::tint(prefs, id).unwrap_or(if selected { p.violet_pale } else { p.muted }),
        ),
        _ => return,
    };
    let fill = |ui: &egui::Ui| {
        if selected {
            ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.28));
        } else if ui.rect_contains_pointer(rect) && ui.ctx().dragged_id().is_none() {
            ui.painter().rect_filled(rect, radius, p.raised);
        }
    };
    let draw_picture = |ui: &egui::Ui, st: &mut InventoryUi| {
        // the card's picture, the type's icon until it has one
        // a folder without a picture borrows one of its items' (a card's
        // picture is smaller than a medium tile's)
        if card && folder.is_some() && thumbnail::tile_picture(inv, id).is_none() {
            let previews = thumbnail::folder_preview(inv, id, 1);
            if images_ready(&previews, images) {
                ui.painter().rect_filled(picture, radius, p.field);
            }
            if paint_previews(ui, p, &previews, picture, images, &mut st.wanted_images) {
                return;
            }
        }
        let texture = card.then(|| thumbnail::tile_picture(inv, id)).flatten().and_then(|asset| {
            st.wanted_images.insert(asset);
            images.get(&asset)
        });
        match texture {
            Some(texture) => {
                ui.painter().rect_filled(picture, radius, p.field);
                let size = texture.size_vec2();
                let scale = (picture_side / size.x.max(1.0)).min(picture_side / size.y.max(1.0));
                ui.painter().image(
                    texture.id(),
                    Rect::from_center_size(picture.center(), size * scale),
                    Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    widgets::PICTURE_TINT,
                );
            }
            None => {
                if card {
                    ui.painter().rect_filled(picture, radius, p.field);
                }
                paint_icon(ui, icons, icon, picture.center(), if card { 24.0 } else { ICON }, tint);
            }
        }
    };
    let mut field = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_size(
                Pos2::new(name_rect.left(), rect.center().y - 11.0),
                Vec2::new(name_rect.width(), 22.0),
            ))
            .id_salt(("inventory_rename_tile", id)),
    );
    if rename_row(&mut field, p, inv, id, st, actions) {
        fill(ui);
        draw_picture(ui, st);
        ui.set_opacity(opacity);
        return;
    }
    let sense = if item.is_some() || folder_draggable(inv, id, prefs) {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let key = if item.is_some() { "inv_item" } else { "inv_folder" };
    let r = ui.interact(rect, ui.make_persistent_id((key, id)), sense);
    super::marquee::record(st, id, rect);
    if st.selected == Some(id) {
        st.selected_rect = Some(rect);
    }
    #[cfg(test)]
    row::tests::record(ui, id, rect);
    fill(ui);
    draw_picture(ui, st);
    // the name, a worn item's in the tag's face, a link's leaning
    let pill = item.and_then(|it| pill(inv, it, facts));
    let (label, worn, link) = match (item, folder) {
        (Some(it), _) => (
            shown_name(it).to_owned(),
            pill.is_some_and(|(_, worn)| worn),
            matches!(it.asset_type, 24 | 25),
        ),
        (_, Some(_)) => (folder_name(inv, id), false, false),
        _ => return,
    };
    let ink = if worn { widgets::row_tag_ink(p, true) } else { p.ink };
    let mut job = if selected || worn || card {
        sheet::semibold(ui, &label, NAME, ink)
    } else {
        egui::text::LayoutJob::single_section(label, egui::TextFormat::simple(egui::FontId::proportional(NAME), ink))
    };
    for section in &mut job.sections {
        section.format.italics = link;
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width(name_rect.width().max(1.0));
    let galley = ui.painter().layout_job(job);
    let muted = if selected { p.violet_pale } else { p.muted };
    if card {
        // the name, then the type, then the date (Explorer's tile)
        let line = |text: String| {
            let mut job = egui::text::LayoutJob::single_section(text, egui::TextFormat::simple(egui::FontId::proportional(11.5), muted));
            job.wrap = egui::text::TextWrapping::truncate_at_width(name_rect.width().max(1.0));
            ui.painter().layout_job(job)
        };
        let kind = line(row::kind_name(inv, id));
        let seconds = item.map_or_else(|| st.view.folder_date(id), |it| it.created_at);
        let date = line(pill.map_or_else(|| row::short_date(seconds, super::super::view::now()), |(text, _)| text.to_owned()));
        let top = rect.center().y - (galley.size().y + kind.size().y + date.size().y) / 2.0;
        let (name_h, kind_h) = (galley.size().y, kind.size().y);
        ui.painter().galley(Pos2::new(text_left, top), galley, ink);
        ui.painter().galley(Pos2::new(text_left, top + name_h), kind, muted);
        ui.painter().galley(Pos2::new(text_left, top + name_h + kind_h), date, muted);
    } else {
        widgets::paint_row_text(ui.painter(), galley, text_left, rect.center().y, ink);
    }
    if rules::is_favorite(inv, id) {
        widgets::paint_glyph(
            ui.painter(),
            "star-filled",
            Pos2::new(rect.right() - PAD - 7.0, rect.center().y),
            12.0,
            p.amber,
        );
    }
    ui.set_opacity(opacity);
    match (item, folder) {
        (Some(it), _) => {
            let permissions = Permissions::inventory(inv, id);
            item_response(ui, p, inv, it, r, permissions, st, prefs, facts, actions);
        }
        (_, Some(_)) => {
            let mut open = false;
            let r = folder_response(ui, p, inv, id, r, None, &mut open, st, prefs, facts, actions);
            if r.double_clicked() || open {
                st.browse = id;
                st.selection.clear();
                st.selected = None;
            }
        }
        _ => {}
    }
}

/// Whether any of `previews` is loaded.
fn images_ready(previews: &[Uuid], images: &HashMap<Uuid, egui::TextureHandle>) -> bool {
    previews.iter().any(|asset| images.contains_key(asset))
}

/// A folder's borrowed pictures in `rect`: one whole, or two to four in a
/// two by two grid (Dolphin's folder previews), each with its proportions
/// kept. Asks for them; false while none of them is loaded, the folder's
/// icon showing meanwhile.
fn paint_previews(
    ui: &egui::Ui,
    p: &Palette,
    previews: &[Uuid],
    rect: Rect,
    images: &HashMap<Uuid, egui::TextureHandle>,
    wanted: &mut HashSet<Uuid>,
) -> bool {
    wanted.extend(previews.iter().copied());
    if !images_ready(previews, images) {
        return false;
    }
    let cells: Vec<Rect> = if previews.len() == 1 {
        vec![rect]
    } else {
        let half = (rect.size() - Vec2::splat(2.0)) / 2.0;
        (0..4)
            .map(|n| {
                let offset = Vec2::new((n % 2) as f32 * (half.x + 2.0), (n / 2) as f32 * (half.y + 2.0));
                Rect::from_min_size(rect.min + offset, half)
            })
            .collect()
    };
    let radius = ui.visuals().widgets.inactive.corner_radius;
    for (asset, cell) in previews.iter().zip(cells) {
        if previews.len() > 1 {
            ui.painter().rect_filled(cell, radius, p.panel);
        }
        if let Some(texture) = images.get(asset) {
            let size = texture.size_vec2();
            let scale = (cell.width() / size.x.max(1.0)).min(cell.height() / size.y.max(1.0));
            ui.painter().image(
                texture.id(),
                Rect::from_center_size(cell.center(), size * scale),
                Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                widgets::PICTURE_TINT,
            );
        }
    }
    true
}

fn paint_icon(ui: &egui::Ui, icons: &Icons, name: &str, center: Pos2, size: f32, tint: Color32) {
    // sharp up to 256-point tiles at 200%
    if let Some(texture) = icons.sized(ui.ctx(), name, size) {
        ui.painter().image(
            texture.id(),
            Rect::from_center_size(center, Vec2::splat(size)),
            Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
            tint,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_many_tiles_per_row_as_the_width_holds_and_one_at_least() {
        let name = 32.0;
        // the Legacy Viewer's 128 px picture: 136 px tiles, 10 px apart
        assert_eq!(Grid::new(136.0, 128.0, name).columns, 1);
        assert_eq!(Grid::new(281.0, 128.0, name).columns, 1);
        assert_eq!(Grid::new(282.0, 128.0, name).columns, 2);
        assert_eq!(Grid::new(700.0, 128.0, name).columns, 4);
        assert_eq!(Grid::new(40.0, 256.0, name).columns, 1, "a narrow pane keeps one column");
        // 72 px tiles: (700 + 10) / 82
        assert_eq!(Grid::new(700.0, 64.0, name).columns, 8);
    }

    #[test]
    fn only_the_tiles_in_sight_are_laid_out() {
        let grid = Grid::new(700.0, 128.0, 32.0);
        let pitch = grid.tile.y + GAP;
        assert_eq!(grid.visible(2000, 0.0, 400.0), 0..4 * 3);
        assert_eq!(grid.visible(2000, pitch * 10.0 + 1.0, pitch * 12.0), 40..48);
        assert_eq!(grid.visible(6, 0.0, 4000.0), 0..6, "never past the last tile");
        assert_eq!(grid.visible(0, 0.0, 400.0), 0..0);
        assert_eq!(grid.rows(2000), 500);
        assert!((grid.height(2000) - (500.0 * pitch - GAP)).abs() < 1e-3);
        assert_eq!(grid.height(0), 0.0);
    }

    #[test]
    fn tiles_follow_each_other_row_by_row() {
        let grid = Grid::new(700.0, 128.0, 32.0);
        let origin = Pos2::new(10.0, 20.0);
        assert_eq!(grid.rect(origin, 0).min, origin);
        assert_eq!(grid.rect(origin, 1).min, origin + Vec2::new(grid.tile.x + GAP, 0.0));
        assert_eq!(grid.rect(origin, 4).min, origin + Vec2::new(0.0, grid.tile.y + GAP));
        for index in 0..16 {
            let (a, b) = (grid.rect(origin, index), grid.rect(origin, index + 1));
            assert!(!a.intersects(b), "tiles never overlap");
        }
    }

    #[test]
    fn the_arrows_step_a_tile_or_a_row() {
        use egui::Key::*;
        let grid = Grid::new(700.0, 128.0, 32.0); // 4 columns
        assert_eq!(grid.step(5, 10, ArrowLeft, false), Some(4));
        assert_eq!(grid.step(0, 10, ArrowLeft, false), None);
        assert_eq!(grid.step(5, 10, ArrowRight, false), Some(6));
        assert_eq!(grid.step(9, 10, ArrowRight, false), None);
        assert_eq!(grid.step(5, 10, ArrowUp, false), Some(1));
        assert_eq!(grid.step(2, 10, ArrowUp, false), None);
        assert_eq!(grid.step(1, 10, ArrowDown, false), Some(5));
        // 10 tiles: the last row holds 8 and 9; from 7, down is its last tile
        assert_eq!(grid.step(7, 10, ArrowDown, false), Some(9));
        assert_eq!(grid.step(9, 10, ArrowDown, false), None);
        // the combination view's list follows the tiles
        assert_eq!(grid.step(9, 10, ArrowDown, true), Some(10));
    }

    #[test]
    fn ctrl_and_the_wheel_change_the_side_within_its_bounds() {
        assert_eq!(zoomed(128.0, 1), 144);
        assert_eq!(zoomed(128.0, -1), 112);
        assert_eq!(zoomed(256.0, 1), 256);
        assert_eq!(zoomed(64.0, -3), 64);
        let mut prefs = view::ExplorerPreferences::default();
        assert_eq!(side(&prefs), DEFAULT_SIDE);
        prefs.tile_size = 9000;
        assert_eq!(side(&prefs), MAX_SIDE);
        // the Legacy Viewer's split: a third for the list, unless one part
        // needs less than its share
        assert_eq!(split(300.0, 120.0, 400.0), 120.0, "short tiles keep their height");
        assert_eq!(split(300.0, 500.0, 40.0), 260.0, "a short list leaves the rest");
        assert_eq!(split(300.0, 500.0, 400.0), 200.0, "both long: two thirds");
        assert_eq!(split(300.0, 120.0, 40.0), 200.0, "both short: two thirds");
        // a wheel notch is a step; a pinch's small steps add up
        assert_eq!(zoom_steps(0.2), (1, 0.0));
        assert_eq!(zoom_steps(-0.2), (-1, 0.0));
        assert_eq!(zoom_steps(0.4).0, 2);
        let (none, left) = zoom_steps(0.05);
        assert_eq!(none, 0);
        assert_eq!(zoom_steps(left + 0.05 + 0.05).0, 1);
    }

    #[test]
    fn the_scroll_moves_only_as_far_as_the_tile() {
        let grid = Grid::new(700.0, 128.0, 32.0);
        let pitch = grid.tile.y + GAP;
        // in sight: the place stays
        assert_eq!(grid.scroll_to(4, 0.0, 600.0), 0.0);
        // below: its bottom at the viewport's bottom
        let row = 10;
        let offset = grid.scroll_to(row * 4, 0.0, 600.0);
        assert!((offset - (row as f32 * pitch + grid.tile.y - 600.0)).abs() < 1e-3);
        // above: its top at the viewport's top
        assert!((grid.scroll_to(4, 5000.0, 600.0) - pitch).abs() < 1e-3);
    }
}
