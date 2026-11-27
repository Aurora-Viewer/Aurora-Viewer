//! Aurora's inventory window (`Settings::classic_inventory` off): one
//! window that widens. At its default size, the Legacy Viewer's tree under
//! icon tabs, with the selection's card at the bottom. From ~720 px the
//! tabs become shortcuts in a column at the left above the folders, and the
//! list an explorer of one folder with its columns. From ~1040 px a
//! details pane opens at the right, the filters show as chips and a status
//! bar counts the list. Its parts: the rows (`rows`), the "CMT" choice of
//! the rights the rows show (`rights`), the column (`sidebar`), the
//! toolbars and filter chips (`toolbar`) and the cards of the selection
//! (`details`). The tree, the search, the drag and drop, the menus and the
//! dialogs are the inventory's own, shared with the Legacy window.

mod breadcrumb;
mod columns;
mod details;
pub(super) mod filter;
mod gallery;
mod history;
pub(super) mod marquee;
mod rights;
pub(super) mod rows;
mod sidebar;
mod smart_bar;
pub(super) mod tabs;
mod toolbar;

pub(super) use breadcrumb::{PathEdit, SEPARATOR as PATH_SEPARATOR, path_text};
pub(super) use columns::ColumnFit;
pub(super) use history::{History, Place};
pub(super) use marquee::Marquee;
pub(super) use rights::button as rights_button;
pub(super) use sidebar::Sidebar;
pub(super) use tabs::Tabs;

use super::permissions::Rights;
use super::*;
use crate::ui::{menu, widgets};
use egui::{Rect, Sense};

/// The three widths of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// The tree under its tabs (the default size).
    Compact,
    /// The column of shortcuts and folders, the explorer.
    Medium,
    /// The details pane, the filter chips, the status bar.
    Wide,
}

impl Layout {
    /// Body widths from which the window changes (its outer width is 20 px
    /// more: about 720 and 1040 px).
    pub const MEDIUM: f32 = 700.0;
    pub const WIDE: f32 = 1020.0;

    pub fn for_width(width: f32) -> Self {
        if width >= Self::WIDE {
            Self::Wide
        } else if width >= Self::MEDIUM {
            Self::Medium
        } else {
            Self::Compact
        }
    }

    /// The layout of the window `id` as it was last drawn (its title bar is
    /// laid out before its body).
    pub fn of_window(ctx: &egui::Context, id: &str) -> Self {
        widgets::floater_size(ctx, id).map_or(Self::Compact, |size| Self::for_width(size.x - 20.0))
    }
}

/// How the rows of Aurora's window draw this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub layout: Layout,
    pub rights: Rights,
    pub columns: view::ExplorerPreferences,
    /// The explorer's list is drawn: its folders go in rather than open.
    pub explorer: bool,
}

/// The wide width the "Widen" button of the title bar gives the window.
const WIDE_SIZE: f32 = 1180.0;

/// "Widen the window": the explorer with its details pane, as tall as it is.
pub fn widen(ctx: &egui::Context, id: &str) {
    let height = widgets::floater_size(ctx, id).map_or(DEFAULT_SIZE.y, |size| size.y);
    widgets::size_floater(ctx, id, egui::vec2(WIDE_SIZE, height.max(DEFAULT_SIZE.y)));
}

/// The size "Compact view" gives a window of `size`: back to the tree at
/// the default width, as tall as it is (#4465: the wider window had no way
/// back but dragging its edge).
fn narrowed(size: Option<egui::Vec2>) -> egui::Vec2 {
    egui::vec2(DEFAULT_SIZE.x, size.map_or(DEFAULT_SIZE.y, |size| size.y))
}

/// "Compact view": the tree under its tabs, as tall as the window is.
pub fn narrow(ctx: &egui::Context, id: &str) {
    let size = egui::AreaState::load(ctx, egui::Id::unique(id)).and_then(|state| state.size);
    widgets::size_floater(ctx, id, narrowed(size));
}

/// `AURORA_DEMO_INVENTORY=aurora|aurora-medium|aurora-wide|aurora-rights`,
/// each width also with `-no-details`.
fn demo_mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| {
        std::env::var("AURORA_DEMO_INVENTORY")
            .ok()
            .filter(|mode| mode.starts_with("aurora"))
    })
    .as_deref()
}

/// Whether one of Aurora's inventory scenarios runs.
pub fn demo_active() -> bool {
    demo_mode().is_some()
}

/// The folder window of `aurora-folder-window` (its id names its layout).
pub const DEMO_FOLDER_WINDOW: Uuid = Uuid::from_u128(0xA0F0);

/// The demo's window at the width of its layout, whatever size an earlier
/// run left in the layout file.
pub fn demo_place(ctx: &egui::Context, frame: u64) {
    let Some(mode) = demo_mode() else { return };
    if frame == 2 {
        // "-no-details": the same width, the details hidden
        let size = match mode.strip_suffix("-no-details").unwrap_or(mode) {
            "aurora-medium"
            | "aurora-sidebar-medium"
            | "aurora-worn-medium"
            | "aurora-delete-confirm-medium"
            | "aurora-gallery-medium"
            | "aurora-explorer-medium" => egui::vec2(800.0, 640.0),
            "aurora-wide"
            | "aurora-gallery"
            | "aurora-system-folders"
            | "aurora-gallery-system-folders"
            | "aurora-filtered"
            | "aurora-gallery-large"
            | "aurora-explorer"
            | "aurora-explorer-path"
            | "aurora-explorer-menu"
            | "aurora-explorer-undo"
            | "aurora-explorer-tabs"
            | "aurora-explorer-split"
            | "aurora-combination"
            | "aurora-worn-wide"
            | "aurora-sidebar-wide"
            | "aurora-smart-wide"
            | "aurora-keyboard-wide"
            | "aurora-type-ahead-ime"
            | "aurora-ime"
            | "aurora-delete-confirm-wide"
            | "aurora-rights"
            | "aurora-columns"
            | "aurora-columns-hidden"
            | "aurora-columns-all"
            | "aurora-explorer-filter"
            | "aurora-explorer-preview"
            | "aurora-compact"
            | "aurora-groups"
            | "aurora-folder-previews"
            | "aurora-tiles"
            | "aurora-received-filing"
            | "aurora-received-unread"
            | "aurora-received-sort"
            | "aurora-received-sort-name" => egui::vec2(WIDE_SIZE, 720.0),
            _ if mode.starts_with("aurora-menu-") => egui::vec2(WIDE_SIZE, 720.0),
            _ => egui::vec2(DEFAULT_SIZE.x, 640.0),
        };
        if let Some(wide) = mode.strip_prefix("aurora-folder-window") {
            // the main window narrow at the left, the folder window beside it
            widgets::place_floater(ctx, "inventory", egui::pos2(30.0, 60.0), egui::vec2(DEFAULT_SIZE.x, 640.0));
            let size = if wide.is_empty() {
                egui::vec2(420.0, 560.0)
            } else {
                egui::vec2(WIDE_SIZE, 720.0)
            };
            let pos = if wide.is_empty() {
                egui::pos2(490.0, 60.0)
            } else {
                egui::pos2(300.0, 80.0)
            };
            widgets::place_floater(ctx, &format!("inventory_{DEMO_FOLDER_WINDOW}"), pos, size);
            return;
        }
        if mode == "aurora-single-folder-window" {
            // the main window narrow at the left, the single folder window
            // beside it at the explorer's width
            widgets::place_floater(ctx, "inventory", egui::pos2(30.0, 60.0), egui::vec2(DEFAULT_SIZE.x, 640.0));
            widgets::place_floater(
                ctx,
                &format!("inventory_{DEMO_FOLDER_WINDOW}"),
                egui::pos2(490.0, 60.0),
                egui::vec2(800.0, 560.0),
            );
            return;
        }
        if mode == "aurora-additional" {
            // the main window, the additional window opened on its left
            let main = egui::pos2(30.0 + DEFAULT_SIZE.x + 12.0, 60.0);
            widgets::place_floater(ctx, "inventory", main, egui::vec2(DEFAULT_SIZE.x, 640.0));
            widgets::place_floater(ctx, &floater_id(ADDITIONAL_WINDOW), egui::pos2(30.0, 60.0), DEFAULT_SIZE);
            return;
        }
        widgets::place_floater(ctx, "inventory", egui::pos2(30.0, 60.0), size);
    }
}

/// The count after the title: the number alone at the narrow width.
pub fn item_count(inv: &Inventory, layout: Layout) -> String {
    let count = inv.total_count();
    let number = aurora_i18n::integer(count as i64);
    if layout == Layout::Compact {
        number
    } else {
        aurora_i18n::tr!("inventory-items-count", count = count, number = number)
    }
}

/// The state of the title bar's right end: synchronised, or folders still
/// being fetched.
pub fn sync_state(p: &Palette, inv: &Inventory, st: &InventoryUi) -> (egui::Color32, String) {
    let fetching = !inv.queue.is_empty() || st.fetch_all || inv.folders.values().any(|f| f.state == FetchState::Fetching);
    let failed = inv.folders.values().any(|f| f.state == FetchState::Failed);
    if fetching {
        (p.muted_dim, aurora_i18n::tr!("inventory-syncing").to_owned())
    } else if failed {
        (p.amber, aurora_i18n::tr!("inventory-sync-incomplete").to_owned())
    } else {
        (p.teal, aurora_i18n::tr!("inventory-synced").to_owned())
    }
}

/// What the shortcuts count, kept until the inventory, the recent cutoff
/// or what is worn changes, and rebuilt at most twice a second.
#[derive(Default)]
pub struct Counts {
    key: Option<(u64, i64, usize, bool, bool, bool)>,
    at: Option<std::time::Instant>,
    pub recent: usize,
    pub favorites: usize,
    pub received: usize,
    pub trash: usize,
}

impl Counts {
    fn update(&mut self, inv: &Inventory, prefs: &InventoryPreferences, worn: usize) {
        use rayon::prelude::*;
        let cutoff = view::recent_cutoff(prefs, view::now());
        let key = (
            inv.generation,
            cutoff,
            worn,
            prefs.search_trash,
            prefs.search_library,
            prefs.search_outfits,
        );
        if self.key == Some(key) || self.at.is_some_and(|at| at.elapsed() < std::time::Duration::from_millis(500)) {
            return;
        }
        self.key = Some(key);
        self.at = Some(std::time::Instant::now());
        let roots = |kinds: &[i32]| kinds.iter().filter_map(|kind| rules::system(inv, *kind)).collect::<Vec<_>>();
        let trash = view::within(inv, &roots(&[14]));
        let outfits = view::within(inv, &roots(&[46, 48]));
        let library = view::within(inv, &[inv.lib_root]);
        // The Recent tab's items (LLInventoryFilter::checkAgainstFilterType
        // with the "since logout" age), in the folders it searches.
        self.recent = inv
            .items
            .par_iter()
            .filter(|(_, it)| {
                it.created_at > 0
                    && it.created_at >= cutoff
                    && (prefs.search_trash || !trash.contains(&it.parent))
                    && (prefs.search_outfits || !outfits.contains(&it.parent))
                    && (prefs.search_library || !library.contains(&it.parent))
            })
            .count();
        // Count explicit favorites, not every outfit link whose original
        // has a star (LLInventoryFavoritesItemsPanel::findAndInitRootContent).
        self.favorites = inv.items.values().filter(|it| it.favorite).count() + inv.folders.values().filter(|f| f.info.favorite).count();
        let held = |kind: i32| {
            rules::system(inv, kind)
                .and_then(|id| inv.folders.get(&id))
                .map_or(0, |f| f.items.len() + f.children.len())
        };
        self.received = crate::world::received_items::deliveries(inv).count();
        self.trash = held(14);
    }
}

/// Aurora's window: the layout of its width.
#[allow(clippy::too_many_arguments)]
pub(super) fn contents(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    images: &std::collections::HashMap<Uuid, egui::TextureHandle>,
    actions: &mut Vec<InvAction>,
) {
    // In the split view each pane is an explorer, without the column below
    // the medium width (Dolphin's split view keeps its places panel aside).
    let width = Layout::for_width(ui.available_width());
    let narrow = st.other_pane.is_some() && width == Layout::Compact;
    let layout = if narrow { Layout::Medium } else { width };
    st.aurora = Some(Frame {
        layout,
        rights: prefs.rights,
        columns: prefs.explorer,
        explorer: false,
    });
    controls::init_filters(st, prefs);
    if st.scope.is_nil() {
        st.counts.update(inv, prefs, facts.worn.len());
    }
    // A folder window's explorer stays under its folder.
    let base = base(inv, st);
    if st.browse.is_nil() || !inv.folders.contains_key(&st.browse) || (base != inv.root && !rules::under(inv, st.browse, base)) {
        st.browse = base;
    }
    if layout != Layout::Compact {
        st.demo_explorer_now(inv);
        st.history.retain(|id| inv.folders.contains_key(&id));
        tabs::retain(st, prefs, inv);
        if st.history.observe(history::Place::of(st)) {
            // the keyboard's place and a typed path belong to the folder left
            st.focus = None;
            st.path_edit = None;
        }
    } else {
        st.path_edit = None;
    }
    match layout {
        Layout::Compact => compact(ui, p, icons, inv, st, prefs, facts, images, actions),
        Layout::Medium | Layout::Wide => explorer(ui, p, icons, inv, st, prefs, facts, images, actions, layout, narrow),
    }
    row_menu(ui, p, inv, st, prefs, facts, actions);
}

/// Windows Explorer's Ctrl+Shift+1…6, in its order: the gallery's large,
/// medium (the Legacy Viewer's 128) and small pictures, the Compact view,
/// the Details (the list), the Tiles.
pub(super) fn set_view(prefs: &mut InventoryPreferences, folder: Uuid, key: u8) {
    use view::ExplorerView;
    let (view, side) = match key {
        1 => (ExplorerView::Gallery, Some(192)),
        2 => (ExplorerView::Gallery, Some(0)),
        3 => (ExplorerView::Gallery, Some(96)),
        4 => (ExplorerView::Compact, None),
        5 => (ExplorerView::List, None),
        _ => (ExplorerView::Tiles, None),
    };
    prefs.set_view_in(folder, view);
    if let Some(side) = side {
        prefs.explorer.tile_size = side;
    }
}

/// Where the window's folders start: the root, or a folder window's folder.
pub(super) fn base(inv: &Inventory, st: &InventoryUi) -> Uuid {
    if st.scope.is_nil() { inv.root } else { st.scope }
}

/// The menu a row's "more actions" button opened.
fn row_menu(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let Some((id, pos)) = st.row_menu else { return };
    let mut open = true;
    menu::popup_at(
        ui.ctx(),
        egui::Id::unique(("inventory_row_menu", st.ui_id)),
        pos,
        &mut open,
        p,
        |ui| {
            if st.background_menu {
                context::show_background(ui, p, inv, id, st, prefs, facts, actions);
            } else {
                context::show(ui, p, inv, id, st, prefs, facts, actions);
            }
        },
    );
    if !open || !(inv.items.contains_key(&id) || inv.folders.contains_key(&id)) {
        st.row_menu = None;
        st.background_menu = false;
    }
}

/// The letters typed to find a row, at the list's top right while they
/// count (Aurora's: the Legacy Viewer's and Explorer's type-ahead show
/// nothing, which leaves it unknown).
fn type_ahead_label(ui: &egui::Ui, p: &Palette, st: &InventoryUi, list: Rect) {
    let now = ui.input(|i| i.time);
    let Some(text) = st.type_ahead.shown(now) else { return };
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), egui::FontId::proportional(12.5), p.ink);
    let size = galley.size() + egui::vec2(16.0, 8.0);
    let rect = Rect::from_min_size(egui::pos2(list.right() - size.x - 14.0, list.top() + 6.0), size);
    let painter = ui.painter().with_clip_rect(list);
    painter.rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
    painter.rect_stroke(
        rect,
        ui.visuals().window_corner_radius,
        egui::Stroke::new(1.0, p.violet),
        egui::StrokeKind::Inside,
    );
    painter.galley(rect.min + egui::vec2(8.0, 4.0), galley, p.ink);
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
}

/// Lay `body` out in exactly `height` of the room left: the list keeps the
/// size it is given, whatever it shows (a list measuring a pixel more than
/// its room made a resizable window grow at every frame).
fn in_rect<R>(ui: &mut egui::Ui, rect: Rect, salt: &str, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .id_salt(salt)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    body(&mut child)
}

/// The room left, split into the list and a foot `foot` high under it,
/// 6 px apart: together exactly the room, whatever they draw. An empty
/// foot leaves the whole room to the list.
fn list_and_foot(ui: &egui::Ui, foot: f32) -> (Rect, Rect, Rect) {
    let gap = if foot > 0.0 { 6.0 } else { 0.0 };
    let room = ui.available_rect_before_wrap();
    let split = (room.bottom() - foot).max(room.top() + 60.0);
    let list = Rect::from_min_max(room.min, egui::pos2(room.right(), split - gap));
    let foot = Rect::from_min_max(egui::pos2(room.left(), split), room.max);
    (room, list, foot)
}

/// The height of the selection's card at the bottom: none once hidden.
fn card_height(prefs: &InventoryPreferences) -> f32 {
    if prefs.show_details { details::STRIP } else { 0.0 }
}

/// The narrow window: tabs, search, the tree, the selection's card.
#[allow(clippy::too_many_arguments)]
fn compact(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    images: &std::collections::HashMap<Uuid, egui::TextureHandle>,
    actions: &mut Vec<InvAction>,
) {
    // a folder window has no tabs
    if st.scope.is_nil() {
        toolbar::tabs(ui, p, inv, st, prefs);
        ui.add_space(6.0);
    }
    toolbar::search_row(ui, p, inv, st, prefs, Layout::Compact, actions);
    ui.add_space(4.0);
    let (root, filtered) = list_scope(inv, st, prefs, facts);
    let message = toolbar::message(p, st);
    let foot_h = card_height(prefs) + if message.is_some() { toolbar::MESSAGE } else { 0.0 };
    let (room, list_rect, foot) = list_and_foot(ui, foot_h);
    let out = in_rect(ui, list_rect, "aurora_inventory_list_room", |ui| {
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
            root,
            filtered,
            false,
        )
    });
    let zone = ui.interact(out.inner_rect, ui.scope_id().with("aurora_list_zone"), Sense::hover());
    let target = list_destination(st, root);
    task_drop::target(ui, p, &zone, inv, target, actions);
    drag::target(ui, p, &zone, inv, target, st, prefs, facts, actions);
    in_rect(ui, foot, "aurora_inventory_foot", |ui| {
        if let Some((text, color)) = message {
            toolbar::message_line(ui, st, &text, color);
        }
        if prefs.show_details {
            details::strip(ui, p, icons, inv, st, prefs, facts, images, actions);
        }
    });
    ui.advance_cursor_after_rect(room);
    clipboard::shortcuts(ui, inv, target, st, prefs, facts, actions);
    drag::drop_menu(ui, p, inv, st, prefs, facts, actions);
}

/// The medium and wide window: the column at the left, the explorer, and
/// the details pane at the wide width.
#[allow(clippy::too_many_arguments)]
fn explorer(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    images: &std::collections::HashMap<Uuid, egui::TextureHandle>,
    actions: &mut Vec<InvAction>,
    layout: Layout,
    narrow: bool,
) {
    let window = ui.max_rect();
    history::input(ui, inv, st, prefs, actions, window);
    if clipboard::list_has_focus(ui, st) && !ui.ctx().text_edit_focused() {
        tabs::input(ui, st, prefs);
    }
    tabs::strip(ui, p, inv, st, prefs, facts, actions);
    toolbar::explorer_row(ui, p, inv, st, prefs, facts, layout, actions);
    ui.add_space(8.0);
    let area = ui.available_rect_before_wrap();
    let pane_w = if layout == Layout::Wide && prefs.show_details {
        columns::details(ui, p, area, &mut prefs.explorer)
    } else {
        0.0
    };
    const GAP: f32 = 8.0;
    let side_w = if narrow {
        0.0
    } else {
        columns::sidebar(ui, p, area, pane_w, layout, &mut prefs.explorer)
    };
    let side = Rect::from_min_size(area.min, egui::vec2(side_w, area.height()));
    let pane = Rect::from_min_max(egui::pos2(area.right() - pane_w, area.top()), area.max);
    let content = Rect::from_min_max(
        egui::pos2(if narrow { area.left() } else { side.right() + GAP }, area.top()),
        egui::pos2(if pane_w > 0.0 { pane.left() - GAP } else { area.right() }, area.bottom()),
    );
    if !narrow {
        let mut side_ui = ui.new_child(egui::UiBuilder::new().max_rect(side).id_salt("aurora_inventory_side"));
        side_ui.set_clip_rect(side.intersect(ui.clip_rect()));
        sidebar::show(&mut side_ui, p, icons, inv, st, prefs, facts, actions);
    }
    let mut content_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(content)
            .id_salt("aurora_inventory_content")
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    content_ui.set_clip_rect(content.intersect(ui.clip_rect()));
    explorer_list(&mut content_ui, p, icons, inv, st, prefs, facts, images, actions, layout);
    if pane_w > 0.0 {
        let mut pane_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(pane)
                .id_salt("aurora_inventory_details")
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        pane_ui.set_clip_rect(pane.intersect(ui.clip_rect()));
        details::pane(&mut pane_ui, p, icons, inv, st, prefs, facts, images, actions);
    }
    ui.advance_cursor_after_rect(area);
}

/// The middle of the explorer: where it is, the columns, the list, and the
/// selection's card (medium) or the status bar (wide).
#[allow(clippy::too_many_arguments)]
fn explorer_list(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    images: &std::collections::HashMap<Uuid, egui::TextureHandle>,
    actions: &mut Vec<InvAction>,
    layout: Layout,
) {
    match layout {
        Layout::Wide => toolbar::chips(ui, p, st, prefs),
        _ => toolbar::breadcrumb_row(ui, p, inv, st, prefs, facts, actions),
    }
    let list_id = ui.scope_id().with("aurora_inventory_list");
    let Some(mut frame) = st.aurora else { return };
    frame.rights = prefs.rights;
    frame.columns = prefs.explorer;
    if let Some(key) = st.view_key.take() {
        set_view(prefs, st.browse, key);
    }
    // Alt+P: the details pane, shown if hidden, previews the selection
    if std::mem::take(&mut st.toggle_preview) {
        prefs.preview_pane = !prefs.preview_pane || !prefs.show_details;
        prefs.show_details |= prefs.preview_pane;
    }
    let (root, filtered) = list_scope(inv, st, prefs, facts);
    let explore = !filtered && !root.is_nil();
    // the folder filter's bar, for the folder shown (a search has its own)
    if explore {
        filter::bar(ui, p, st);
    }
    // A search or a filter lists its results, from every folder, as rows:
    // the tiles show one folder.
    let shown = if explore {
        prefs.view_in(st.browse)
    } else {
        view::ExplorerView::List
    };
    // the combination view titles its list under its tiles
    if shown == view::ExplorerView::List {
        rows::header(ui, p, frame, list_id, prefs, st.tab, &mut st.column_fit);
        frame.columns = prefs.explorer;
    }
    st.aurora = Some(frame);
    ui.add_space(2.0);
    let message = toolbar::message(p, st);
    let foot_h = match layout {
        Layout::Medium => card_height(prefs) + if message.is_some() { toolbar::MESSAGE } else { 0.0 },
        _ => toolbar::STATUS,
    };
    let (room, list_rect, foot) = list_and_foot(ui, foot_h);
    let list_h = list_rect.height();
    let target = if explore { st.browse } else { list_destination(st, root) };
    let out = if explore {
        // A revealed entry (Find original, a new item) shows in its folder;
        // a folder to show opened is gone into.
        if let Some(id) = st.reveal {
            let into = st.reveal_path.contains(&id) && inv.folders.contains_key(&id);
            if let Some(folder) = if into { Some(id) } else { rules::parent(inv, id) } {
                st.browse = folder;
            }
            st.reveal_path.clear();
            if into {
                st.reveal = None;
            }
        }
        // Collapse and Expand belong to the tree.
        st.expand_all = None;
        let browse = st.browse;
        std::mem::swap(&mut st.plain, &mut st.flat);
        st.plain.set_tiles(gallery::tiles(shown));
        // another folder starts with an empty filter (the bar stays open)
        if st.folder_filter_at.is_some_and(|at| at != browse)
            && let Some(text) = &mut st.folder_filter
        {
            text.clear();
        }
        st.folder_filter_at = Some(browse);
        st.plain.set_filter(st.folder_filter.as_deref().unwrap_or_default());
        // the Details list's groups (the tiles keep their own order)
        let group = if shown == view::ExplorerView::List {
            prefs.explorer.group_by
        } else {
            groups::GroupBy::None
        };
        st.plain.set_group(group, st.view.context_fingerprint());
        if let Some(frame) = &mut st.aurora {
            frame.explorer = true;
        }
        // a band drawn from the empty space selects rows or tiles
        let banded = shown != view::ExplorerView::Combination;
        if banded {
            marquee::begin(ui, st, list_rect);
        }
        let out = match shown {
            view::ExplorerView::List => in_rect(ui, list_rect, "aurora_inventory_list_room", |ui| {
                list(ui, p, icons, inv, st, prefs, facts, actions, list_h, browse, false, false)
            }),
            view::ExplorerView::Gallery | view::ExplorerView::Compact | view::ExplorerView::Tiles => {
                in_rect(ui, list_rect, "aurora_inventory_list_room", |ui| {
                    let style = gallery::Style::of(shown);
                    gallery::gallery(ui, p, icons, inv, st, prefs, facts, images, actions, list_h, browse, style)
                })
            }
            view::ExplorerView::Combination => {
                gallery::combination(ui, p, icons, inv, st, prefs, facts, images, actions, list_rect, list_id, browse)
            }
        };
        if banded {
            marquee::end(ui, p, st, list_rect, out.id, out.inner_rect);
        }
        if let Some(frame) = &mut st.aurora {
            frame.explorer = false;
        }
        std::mem::swap(&mut st.plain, &mut st.flat);
        out
    } else {
        // Recent, Worn, Favorites and the results take the band too
        marquee::begin(ui, st, list_rect);
        let out = in_rect(ui, list_rect, "aurora_inventory_list_room", |ui| {
            list(ui, p, icons, inv, st, prefs, facts, actions, list_h, root, filtered, false)
        });
        marquee::end(ui, p, st, list_rect, out.id, out.inner_rect);
        out
    };
    widgets::remember_scroll_bar(ui, list_id, &out);
    type_ahead_label(ui, p, st, list_rect);
    // between the tiles too, what is dropped goes into the folder shown
    let zone_rect = if shown == view::ExplorerView::List {
        out.inner_rect
    } else {
        list_rect
    };
    let zone = ui.interact(zone_rect, ui.scope_id().with("aurora_list_zone"), Sense::hover());
    task_drop::target(ui, p, &zone, inv, target, actions);
    drag::target(ui, p, &zone, inv, target, st, prefs, facts, actions);
    in_rect(ui, foot, "aurora_inventory_foot", |ui| match layout {
        Layout::Medium => {
            if let Some((text, color)) = message {
                toolbar::message_line(ui, st, &text, color);
            }
            if prefs.show_details {
                details::strip(ui, p, icons, inv, st, prefs, facts, images, actions);
            }
        }
        _ => toolbar::status_bar(ui, p, st, explore, message),
    });
    ui.advance_cursor_after_rect(room);
    clipboard::shortcuts(ui, inv, target, st, prefs, facts, actions);
    drag::drop_menu(ui, p, inv, st, prefs, facts, actions);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_line_reports_folders_that_could_not_be_loaded() {
        let mut h = Harness::new(800.0);
        let p = crate::theme::Theme::default().palette();
        let (_, text) = sync_state(&p, &h.inv, &h.st);
        assert_eq!(text, aurora_i18n::tr!("inventory-synced"));
        let root = h.inv.root;
        h.inv.folders.get_mut(&root).expect("root").state = FetchState::Failed;
        let (color, text) = sync_state(&p, &h.inv, &h.st);
        assert_eq!(text, aurora_i18n::tr!("inventory-sync-incomplete"));
        assert_eq!(color, p.amber, "an incomplete load is a warning, not a success");
    }

    #[test]
    fn inventory_total_is_shared_by_every_title_width() {
        let mut inv = Inventory {
            root: Uuid::from_u128(1),
            ..Default::default()
        };
        inv.created_folders(vec![aurora_net::inventory::InvFolder {
            id: inv.root,
            ..Default::default()
        }]);
        let number = aurora_i18n::integer(1);
        assert_eq!(item_count(&inv, Layout::Compact), number);
        for layout in [Layout::Medium, Layout::Wide] {
            assert_eq!(
                item_count(&inv, layout),
                aurora_i18n::tr!("inventory-items-count", count = 1, number = number.clone())
            );
        }
    }

    #[test]
    fn the_window_widens_into_the_explorer_then_the_details() {
        assert_eq!(Layout::for_width(DEFAULT_SIZE.x - 20.0), Layout::Compact);
        assert_eq!(Layout::for_width(699.0), Layout::Compact);
        assert_eq!(Layout::for_width(700.0), Layout::Medium);
        assert_eq!(Layout::for_width(1019.0), Layout::Medium);
        assert_eq!(Layout::for_width(WIDE_SIZE - 20.0), Layout::Wide);
    }

    #[test]
    fn compact_view_goes_back_to_the_tree_at_the_same_height() {
        let size = narrowed(Some(egui::vec2(WIDE_SIZE, 800.0)));
        assert_eq!(Layout::for_width(size.x - 20.0), Layout::Compact);
        assert_eq!(size.y, 800.0);
        assert_eq!(narrowed(None), DEFAULT_SIZE);
    }

    #[test]
    fn expanded_inventory_splitters_resize_without_selection_or_drag_actions() {
        for (width, scale) in [(800.0, 1.0), (WIDE_SIZE, 1.0), (800.0, 1.5), (WIDE_SIZE, 1.5)] {
            let mut h = Harness::new(width);
            h.ctx.set_pixels_per_point(scale);
            h.st.show_original(&h.inv, Uuid::from_u128(8100));
            h.frames(8);
            h.actions.clear();
            let selection = h.st.selection.clone();
            let wide = Layout::for_width(width - 20.0) == Layout::Wide;
            let mut splitters = vec![
                egui::Id::unique("inventory_sidebar_handle"),
                egui::Id::unique(("inventory_column_handle", 0_usize)),
                egui::Id::unique(("inventory_column_handle", 1_usize)),
                egui::Id::unique(("inventory_column_handle", 2_usize)),
            ];
            if wide {
                splitters.push(egui::Id::unique("inventory_details_handle"));
            }
            for key in splitters {
                let hit = h.ctx.data(|data| data.get_temp::<Rect>(key)).expect("splitter drawn");
                let start = hit.center();
                h.run(vec![egui::Event::PointerMoved(start)]);
                h.run(vec![egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }]);
                h.run(vec![egui::Event::PointerMoved(start + egui::vec2(-24.0, 0.0))]);
                h.run(vec![egui::Event::PointerMoved(start + egui::vec2(-48.0, 0.0))]);
                let changed = h.prefs.explorer;
                h.frames(3);
                assert_eq!(h.prefs.explorer, changed, "holding the pointer stationary cannot keep resizing");
                h.run(vec![egui::Event::PointerButton {
                    pos: start + egui::vec2(-48.0, 0.0),
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }]);
                h.frames(3);
            }
            assert!(h.prefs.explorer.sidebar_width > 0);
            assert!(h.prefs.explorer.columns.slot(view::ColumnId::Kind).width > 0);
            assert!(h.prefs.explorer.columns.slot(view::ColumnId::Date).width > 0);
            if wide {
                assert!(h.prefs.explorer.details_width > 0);
            }
            assert_eq!(h.st.selection, selection);
            assert!(h.actions.is_empty(), "splitters cannot perform inventory operations");
            let saved = h.prefs.explorer;
            widgets::size_floater(&h.ctx, "aurora_inventory_test", egui::vec2(740.0, 560.0));
            h.frames(8);
            assert_eq!(h.prefs.explorer, saved, "fitting a smaller window preserves the requested widths");
            let header = h
                .ctx
                .data(|data| data.get_temp::<columns::Columns>(egui::Id::unique("inventory_header_columns")))
                .unwrap();
            let row = h
                .ctx
                .data(|data| data.get_temp::<columns::Columns>(egui::Id::unique(("inventory_row_columns", Uuid::from_u128(8101)))))
                .unwrap();
            assert!(
                (header.name_end - row.name_end).abs() < 0.5,
                "headers stay aligned with scrolled rows"
            );
            for id in [view::ColumnId::Kind, view::ColumnId::Rights, view::ColumnId::Date] {
                let (header, row) = (header.get(id), row.get(id));
                assert!((header.unwrap().min - row.unwrap().min).abs() < 0.5);
            }
            let hit = h
                .ctx
                .data(|data| data.get_temp::<Rect>(egui::Id::unique("inventory_sidebar_handle")))
                .unwrap();
            h.double_click(hit.center());
            assert_eq!(h.prefs.explorer.sidebar_width, 0, "double click restores the responsive default");
            h.click(h.row(8101).center(), Default::default());
            assert_eq!(h.st.selected, Some(Uuid::from_u128(8101)), "ordinary row selection still works");
            if wide {
                widgets::size_floater(&h.ctx, "aurora_inventory_test", egui::vec2(WIDE_SIZE, 720.0));
                h.frames(8);
                let hit = h
                    .ctx
                    .data(|data| data.get_temp::<Rect>(egui::Id::unique("inventory_details_handle")))
                    .unwrap();
                h.double_click(hit.center());
                assert_eq!(h.prefs.explorer.details_width, 0, "double click restores the details default");
            }
        }
    }

    #[test]
    fn header_titles_sort_and_reverse_without_selecting_or_acting() {
        use view::{SortColumn, SortHeader};
        let mut h = Harness::new(WIDE_SIZE);
        h.st.show_original(&h.inv, Uuid::from_u128(8100));
        h.frames(8);
        h.actions.clear();
        let selection = h.st.selection.clone();
        let title = |h: &Harness, header: SortHeader| {
            h.ctx
                .data(|data| data.get_temp::<Rect>(egui::Id::unique(("inventory_column_title", header))))
                .expect("title drawn")
                .center()
        };
        let kind = SortHeader::Column(SortColumn::Kind);
        h.click(title(&h, kind), Default::default());
        h.frames(2);
        assert_eq!(h.prefs.sort.header(), (kind, false));
        h.click(title(&h, kind), Default::default());
        h.frames(2);
        assert_eq!(h.prefs.sort.header(), (kind, true), "a second click reverses");
        h.click(title(&h, SortHeader::Name), Default::default());
        h.frames(2);
        assert_eq!(h.prefs.sort.header(), (SortHeader::Name, false));
        assert_eq!(h.st.selection, selection);
        assert!(h.actions.is_empty(), "sorting performs no inventory operation");
    }

    #[test]
    fn titles_drag_columns_into_place_and_dividers_fit_them() {
        use view::{ColumnId, SortHeader};
        let mut h = Harness::new(WIDE_SIZE);
        h.st.show_original(&h.inv, Uuid::from_u128(8100));
        h.frames(8);
        h.actions.clear();
        let sort = h.prefs.sort;
        let title = |h: &Harness, header: SortHeader| {
            h.ctx
                .data(|data| data.get_temp::<Rect>(egui::Id::unique(("inventory_column_title", header))))
                .expect("title drawn")
        };
        // the date carried left of the type
        let from = title(&h, SortHeader::Date).center();
        let to = title(&h, ColumnId::Kind.header()).left_center() + egui::vec2(4.0, 0.0);
        h.run(vec![egui::Event::PointerMoved(from)]);
        h.run(vec![egui::Event::PointerButton {
            pos: from,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        }]);
        for step in 1..=6 {
            h.run(vec![egui::Event::PointerMoved(from + (to - from) * (step as f32 / 6.0))]);
        }
        h.run(vec![egui::Event::PointerButton {
            pos: to,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]);
        h.frames(3);
        let order: Vec<_> = h.prefs.explorer.columns.0.iter().map(|s| s.id).collect();
        assert_eq!(order[..3], [ColumnId::Date, ColumnId::Kind, ColumnId::Rights]);
        assert_eq!(h.prefs.sort, sort, "a drag does not sort");
        assert!(h.actions.is_empty());
        // the divider right of the date fits it to its widest text
        h.prefs.explorer.columns.slot_mut(ColumnId::Date).width = 400;
        h.frames(3);
        let handle = h
            .ctx
            .data(|data| data.get_temp::<Rect>(egui::Id::unique(("inventory_column_handle", 1_usize))))
            .expect("the divider after the date");
        h.double_click(handle.center());
        h.frames(2);
        let fitted = h.prefs.explorer.columns.slot(ColumnId::Date).width;
        assert!((40..200).contains(&fitted), "fitted to its dates and title, got {fitted}");
        assert_eq!(h.prefs.sort, sort, "a fit does not sort");
    }

    #[test]
    fn alt_p_previews_a_notecards_text_and_a_texture_in_the_details_pane() {
        let mut h = Harness::new(WIDE_SIZE);
        h.st.show_original(&h.inv, Uuid::from_u128(8103));
        h.frames(4);
        let row = h.row(8103);
        h.click(egui::pos2(row.left() + 40.0, row.center().y), Default::default());
        let alt_p = if cfg!(target_os = "macos") {
            egui::Modifiers {
                shift: true,
                mac_cmd: true,
                command: true,
                ..Default::default()
            }
        } else {
            egui::Modifiers::ALT
        };
        h.actions.clear();
        h.run(vec![egui::Event::Key {
            key: egui::Key::P,
            physical_key: Some(egui::Key::P),
            pressed: true,
            repeat: false,
            modifiers: alt_p,
        }]);
        h.frames(2);
        assert!(h.prefs.preview_pane && h.prefs.show_details);
        let note = Uuid::from_u128(8103);
        let asked = |h: &Harness| h.actions.iter().any(|a| matches!(a, InvAction::PaneText(id) if *id == note));
        assert!(!asked(&h), "not before the selection has settled");
        h.frames(20);
        assert!(
            h.actions.iter().any(|a| matches!(a, InvAction::PaneText(id) if *id == note)),
            "the text is asked for"
        );
        // as the app answers
        let key = crate::ui::inventory::pane_key(&h.inv, note, 0).expect("a notecard");
        h.facts.pane_texts = std::sync::Arc::new([(key, PaneText::Text("Hello from the notecard".into()))].into());
        h.frames(2);
        let shown = h.ctx.data(|d| d.get_temp::<String>(egui::Id::unique("inventory_preview_text")));
        assert_eq!(shown.as_deref(), Some("Hello from the notecard"));
        // RLVa's @viewnote turns on: the text read before is not shown again
        h.inv.rlva_preview_mask = 2;
        h.ctx
            .data_mut(|d| d.remove_temp::<String>(egui::Id::unique("inventory_preview_text")));
        h.actions.clear();
        h.frames(2);
        let shown = h.ctx.data(|d| d.get_temp::<String>(egui::Id::unique("inventory_preview_text")));
        assert_eq!(shown, None, "@viewnote hides the text kept");
        assert!(
            h.actions.iter().any(|a| matches!(a, InvAction::PaneText(id) if *id == note)),
            "asked again under the new terms"
        );
        h.inv.rlva_preview_mask = 0;
        // a texture asks for its picture, unless RLVa's @viewtexture hides it
        let texture = Uuid::from_u128(8104);
        h.st.selection.clear();
        h.st.selection.insert(texture);
        h.st.selected = Some(texture);
        h.st.wanted_images.clear();
        h.frames(2);
        let asset = h.inv.items[&texture].asset_id;
        assert!(h.st.wanted_images.contains(&asset));
        h.inv.rlva_preview_mask = 1;
        h.st.wanted_images.clear();
        h.frames(2);
        assert!(!h.st.wanted_images.contains(&asset), "@viewtexture: the picture stays");
    }

    #[test]
    fn ctrl_shift_digits_switch_the_views_and_compact_and_tiles_keep_the_list_order() {
        let mut h = Harness::new(WIDE_SIZE);
        h.st.show_original(&h.inv, Uuid::from_u128(8100));
        h.frames(4);
        let listed = h.st.flat.order.clone();
        let row = h.row(8100);
        h.click(egui::pos2(row.left() + 40.0, row.center().y), Default::default());
        let digit = |h: &mut Harness, key| {
            let modifiers = if cfg!(target_os = "macos") {
                egui::Modifiers {
                    alt: true,
                    mac_cmd: true,
                    command: true,
                    ..Default::default()
                }
            } else {
                egui::Modifiers::CTRL | egui::Modifiers::SHIFT | egui::Modifiers::COMMAND
            };
            h.run(vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers,
            }]);
            h.frames(3);
        };
        for (key, view, side) in [
            (egui::Key::Num1, view::ExplorerView::Gallery, 192),
            (egui::Key::Num3, view::ExplorerView::Gallery, 96),
            (egui::Key::Num2, view::ExplorerView::Gallery, 0),
            (egui::Key::Num4, view::ExplorerView::Compact, 0),
            (egui::Key::Num6, view::ExplorerView::Tiles, 0),
            (egui::Key::Num5, view::ExplorerView::List, 0),
        ] {
            digit(&mut h, key);
            assert_eq!(h.prefs.explorer.view, view, "{key:?}");
            assert_eq!(h.prefs.explorer.tile_size, side, "{key:?}");
            // the selected entry stays in sight, drawn in the view
            assert!(rows::tests::drawn(&h.ctx, Uuid::from_u128(8100)).is_some(), "{view:?}");
            if matches!(view, view::ExplorerView::Compact | view::ExplorerView::Tiles) {
                assert_eq!(h.st.flat.order, listed, "{view:?} keeps the list's order");
                assert_eq!(h.st.flat.tiles, listed.len());
            }
        }
        // a cell of the Compact view selects and opens as a row
        h.prefs.explorer.view = view::ExplorerView::Compact;
        h.frames(3);
        let note = h.row(8103);
        h.actions.clear();
        h.double_click(note.left_center() + egui::vec2(30.0, 0.0));
        assert!(
            h.actions
                .iter()
                .any(|a| matches!(a, InvAction::Preview(id, _) if *id == Uuid::from_u128(8103)))
        );
    }

    #[test]
    fn group_by_type_lists_named_groups_that_close_on_a_click() {
        let mut h = Harness::new(WIDE_SIZE);
        h.st.show_original(&h.inv, Uuid::from_u128(8100));
        h.frames(4);
        let all = h.st.flat.order.len();
        h.prefs.explorer.group_by = groups::GroupBy::Kind;
        h.frames(3);
        let names: Vec<String> = h.st.flat.groups.iter().map(|(name, _)| name.clone()).collect();
        assert_eq!(
            names[0],
            aurora_i18n::tr!("inventory-group-folders"),
            "the folders' group first: {names:?}"
        );
        assert!(names.len() > 3, "{names:?}");
        assert_eq!(
            h.st.flat.groups.iter().map(|(_, n)| n).sum::<usize>(),
            all,
            "every entry in one group"
        );
        assert_eq!(h.st.flat.order.len(), all);
        // a click on a header closes its group, another opens it (the
        // second group's, in sight)
        let (group, in_group) = h.st.flat.groups[1].clone();
        let header = |h: &Harness| {
            h.ctx
                .data(|d| d.get_temp::<Rect>(egui::Id::unique(("inventory_group", group.clone()))))
                .expect("the group's header")
        };
        let first = h.st.flat.order[h.st.flat.groups[0].1];
        h.click(header(&h).left_center() + egui::vec2(60.0, 0.0), Default::default());
        h.frames(2);
        assert_eq!(h.st.flat.order.len(), all - in_group, "closed");
        assert!(!h.st.flat.order.contains(&first), "its first entry is no longer listed");
        h.click(header(&h).left_center() + egui::vec2(60.0, 0.0), Default::default());
        h.frames(2);
        assert_eq!(h.st.flat.order.len(), all, "open again");
        assert!(h.actions.is_empty());
        // the tiles keep their own order: no groups there
        h.prefs.explorer.view = view::ExplorerView::Tiles;
        h.frames(3);
        assert!(h.st.flat.groups.is_empty());
    }

    #[test]
    fn a_typed_path_to_twin_folders_shows_their_parent_filtered_to_them() {
        let mut h = Harness::new(WIDE_SIZE);
        let (here, sub) = (Uuid::from_u128(8000), Uuid::from_u128(8001));
        let twin = Uuid::from_u128(8999);
        h.inv.created_folders(vec![aurora_net::inventory::InvFolder {
            id: twin,
            parent: here,
            name: folder_name(&h.inv, sub),
            ..Default::default()
        }]);
        h.frames(2);
        let text = breadcrumb::path_text(&h.inv, h.inv.root, sub);
        h.st.path_edit = Some(PathEdit::new(text));
        h.frames(2);
        h.run(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: Some(egui::Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        h.frames(3);
        assert_eq!(h.st.browse, here, "their parent, not the first of them");
        assert_eq!(h.st.folder_filter.as_deref(), Some(folder_name(&h.inv, sub).as_str()));
        assert!(h.st.flat.order.contains(&sub) && h.st.flat.order.contains(&twin), "both listed");
        assert!(h.st.path_edit.is_none());
    }

    #[test]
    fn a_folders_tile_asks_for_its_items_pictures_by_the_tiles_size() {
        let folder = Uuid::from_u128(8000);
        for (side, wanted) in [(96, 0), (128, 1), (192, 4)] {
            let mut h = Harness::new(WIDE_SIZE);
            crate::world::inventory::demo::seed_gallery(&mut h.inv);
            h.prefs.explorer.view = view::ExplorerView::Gallery;
            h.prefs.explorer.tile_size = side;
            h.st.demo_reveal(folder);
            h.frames(6);
            let previews = thumbnail::folder_preview(&h.inv, folder, 4);
            let asked = previews.iter().filter(|asset| h.st.wanted_images.contains(asset)).count();
            assert!(asked >= wanted, "{side} px: {asked} of {wanted}");
            if wanted == 0 {
                // small tiles: the folder's icon, nothing asked for it
                assert!(rows::tests::drawn(&h.ctx, folder).is_some());
                assert_eq!(asked, 0);
            }
        }
    }

    /// Aurora's window `width` wide in a headless context.
    struct Harness {
        ctx: egui::Context,
        inv: Inventory,
        st: InventoryUi,
        prefs: InventoryPreferences,
        facts: Facts,
        width: f32,
        frame: u64,
        actions: Vec<InvAction>,
        modifiers: egui::Modifiers,
    }

    impl Harness {
        fn new(width: f32) -> Self {
            let agent = Uuid::from_u128(2);
            Self {
                ctx: egui::Context::default(),
                inv: super::super::tests::view_inventory(false),
                st: InventoryUi::default(),
                prefs: InventoryPreferences::default(),
                facts: Facts {
                    agent,
                    ..Default::default()
                },
                width,
                frame: 0,
                actions: Vec::new(),
                modifiers: Default::default(),
            }
        }

        fn run(&mut self, events: Vec<egui::Event>) {
            self.frame += 1;
            self.st.visible.clear();
            let palette = crate::theme::Theme::default().palette();
            let (inv, st, prefs, facts, actions, width) = (
                &mut self.inv,
                &mut self.st,
                &mut self.prefs,
                &self.facts,
                &mut self.actions,
                self.width,
            );
            drop(headless_output(
                self.ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
                        time: Some(self.frame as f64 / 60.0),
                        events: std::iter::once(egui::Event::ModifiersChanged(self.modifiers))
                            .chain(events)
                            .collect(),
                        ..Default::default()
                    },
                    |ui| {
                        widgets::Floater::new(
                            "aurora_inventory_test",
                            "Inventory",
                            egui::pos2(20.0, 20.0),
                            egui::vec2(width, 720.0),
                        )
                        .silent()
                        .show(ui.ctx(), &palette, &mut true, |ui| {
                            contents(ui, &palette, &Icons::default(), inv, st, prefs, facts, &Default::default(), actions);
                        });
                    },
                ),
            ));
        }

        fn frames(&mut self, n: usize) {
            for _ in 0..n {
                self.run(vec![]);
            }
        }

        fn click(&mut self, pos: egui::Pos2, modifiers: egui::Modifiers) {
            self.modifiers = modifiers;
            self.run(vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                self.run(vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                }]);
            }
            self.modifiers = Default::default();
            self.frames(2);
        }

        fn double_click(&mut self, pos: egui::Pos2) {
            // apart from an earlier click: three within 0.6 s are a triple click
            self.frames(40);
            self.run(vec![egui::Event::PointerMoved(pos)]);
            for _ in 0..2 {
                for pressed in [true, false] {
                    self.run(vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ]);
                }
            }
            self.frames(2);
        }

        fn row(&self, id: u128) -> Rect {
            rows::tests::drawn(&self.ctx, Uuid::from_u128(id)).unwrap_or_else(|| panic!("row {id} drawn at {} px", self.width))
        }
    }

    /// Press at `from`, move to `to` in steps, and report what was carried
    /// before the release.
    fn press_and_move(h: &mut Harness, from: egui::Pos2, to: egui::Pos2) -> bool {
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        h.run(vec![egui::Event::PointerMoved(from)]);
        h.run(vec![button(from, true)]);
        for step in 1..=6 {
            h.run(vec![egui::Event::PointerMoved(from.lerp(to, step as f32 / 6.0))]);
        }
        let carried = drag::carried_ids(&h.ctx).is_some();
        h.run(vec![button(to, false)]);
        h.frames(2);
        carried
    }

    #[test]
    fn ctrl_alt_f_filters_the_folder_shown_until_esc() {
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        };
        let mut h = Harness::new(WIDE_SIZE);
        h.st.show_original(&h.inv, Uuid::from_u128(8100));
        h.frames(4);
        let all = h.st.flat.order.len();
        assert!(all > 3, "the right-click tests' folder");
        let row = h.row(8100);
        h.click(egui::pos2(row.left() + 40.0, row.center().y), Default::default());
        // as a keyboard sends them: Ctrl (Cmd on a Mac) is also "command"
        let filter = if cfg!(target_os = "macos") {
            let cmd_option = egui::Modifiers {
                alt: true,
                mac_cmd: true,
                command: true,
                ..Default::default()
            };
            key(egui::Key::F, cmd_option)
        } else {
            key(
                egui::Key::F,
                egui::Modifiers::CTRL | egui::Modifiers::COMMAND | egui::Modifiers::ALT,
            )
        };
        h.run(vec![filter]);
        h.frames(3);
        assert_eq!(h.st.folder_filter.as_deref(), Some(""), "the bar opens empty");
        assert!(
            h.ctx
                .data(|d| d.get_temp::<Rect>(egui::Id::unique("inventory_folder_filter")))
                .is_some()
        );
        // everything selected, as Ctrl+A
        h.st.selection = h.st.flat.order.iter().copied().collect();
        h.run(vec![egui::Event::Text("demo OBJ".into())]);
        h.frames(3);
        assert_eq!(h.st.folder_filter.as_deref(), Some("demo OBJ"), "the field has the keyboard");
        assert!(
            !h.st.selection.is_empty() && h.st.selection.iter().all(|id| h.st.flat.order.contains(id)),
            "what the filter hides leaves the selection"
        );
        let names: Vec<String> =
            h.st.flat
                .order
                .iter()
                .map(|id| {
                    h.inv
                        .items
                        .get(id)
                        .map_or_else(|| folder_name(&h.inv, *id), |it| it.name.clone())
                        .to_lowercase()
                })
                .collect();
        assert!(!names.is_empty() && names.len() < all, "{names:?}");
        assert!(
            names.iter().all(|n| n.contains("demo") && n.contains("obj")),
            "every word, any case: {names:?}"
        );
        // a folder with nothing that matches says so; the filter stays open
        h.run(vec![egui::Event::Text("zzz".into())]);
        h.frames(3);
        assert!(h.st.flat.order.is_empty() && h.st.flat.filtered_out == all);
        // another folder: the bar stays open, its text goes
        let here = h.st.browse;
        h.st.browse = h.inv.root;
        h.frames(2);
        assert_eq!(h.st.folder_filter.as_deref(), Some(""), "emptied in another folder");
        h.st.browse = here;
        h.frames(2);
        assert_eq!(h.st.flat.order.len(), all);
        h.run(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
        h.frames(3);
        assert_eq!(h.st.folder_filter, None, "Esc closes the bar");
        assert_eq!(h.st.flat.order.len(), all, "and lists everything again");
        assert!(h.actions.is_empty());
    }

    #[test]
    fn f2_renames_and_alt_enter_opens_the_properties_after_a_click() {
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        };
        let mut h = Harness::new(WIDE_SIZE);
        h.st.show_original(&h.inv, Uuid::from_u128(8100));
        h.frames(4);
        let row = h.row(8100);
        h.click(egui::pos2(row.left() + 40.0, row.center().y), Default::default());
        assert_eq!(h.st.selected, Some(Uuid::from_u128(8100)));
        h.run(vec![key(egui::Key::Enter, egui::Modifiers::ALT)]);
        h.frames(2);
        // in a window of its own; a second Alt+Enter on another row opens
        // another, the first kept
        let open = |h: &Harness| {
            h.st.windows
                .iter()
                .filter_map(|w| match w.state.dialog {
                    Some(EditDialog::Properties(id)) => Some(id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(open(&h), vec![Uuid::from_u128(8100)], "selection {:?}", h.st.selection.len());
        assert!(h.st.dialog.is_none(), "the list stays free");
        let other = h.row(8102);
        // the first press let go, as a hand does (else egui reads a repeat)
        h.run(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: Some(egui::Key::Enter),
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::ALT,
        }]);
        h.click(egui::pos2(other.left() + 40.0, other.center().y), Default::default());
        h.run(vec![key(egui::Key::Enter, egui::Modifiers::ALT)]);
        h.frames(2);
        assert_eq!(open(&h), vec![Uuid::from_u128(8100), Uuid::from_u128(8102)]);
        h.click(egui::pos2(row.left() + 40.0, row.center().y), Default::default());
        h.run(vec![key(egui::Key::F2, egui::Modifiers::NONE)]);
        h.frames(2);
        assert_eq!(h.st.rename.as_ref().map(|r| r.id), Some(Uuid::from_u128(8100)), "F2 renames");
    }

    #[test]
    fn a_pin_takes_left_and_right_clicks_and_drags_to_reorder() {
        let mut h = Harness::new(WIDE_SIZE);
        let (a, b) = (Uuid::from_u128(8000), Uuid::from_u128(8001));
        h.prefs.pinned = vec![a, b];
        h.frames(4);
        let pin = |h: &Harness, id: Uuid| {
            h.ctx
                .data(|d| d.get_temp::<Rect>(egui::Id::unique(("aurora_pin", id))))
                .expect("pin drawn")
        };
        // a left click shows it
        h.click(pin(&h, b).center(), Default::default());
        assert_eq!(h.st.browse, b);
        // a right click opens its menu, not a drag
        let at = pin(&h, a).center();
        h.run(vec![egui::Event::PointerMoved(at)]);
        for pressed in [true, false] {
            h.run(vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Default::default(),
            }]);
        }
        h.frames(2);
        assert!(egui::Popup::is_any_open(&h.ctx), "the pin's menu");
        assert!(drag::carried_ids(&h.ctx).is_none());
        h.run(vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        h.frames(2);
        // dragged onto the other pin, it takes its place
        let (from, to) = (pin(&h, b).center(), pin(&h, a).center());
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        h.run(vec![egui::Event::PointerMoved(from)]);
        h.run(vec![button(from, true)]);
        for step in 1..=6 {
            h.run(vec![egui::Event::PointerMoved(from.lerp(to, step as f32 / 6.0))]);
        }
        h.run(vec![button(to, false)]);
        h.frames(2);
        assert_eq!(h.prefs.pinned, vec![b, a]);
    }

    #[test]
    fn letters_typed_after_a_click_find_a_row() {
        let mut h = Harness::new(WIDE_SIZE);
        h.st.show_original(&h.inv, Uuid::from_u128(8100));
        h.frames(4);
        let row = h.row(8100);
        h.click(egui::pos2(row.left() + 40.0, row.center().y), Default::default());
        let before = h.st.selected;
        let name = |h: &Harness| h.st.selected.map(|id| rules::name(&h.inv, id)).unwrap_or_default();
        h.run(vec![egui::Event::Text("i".into())]);
        h.frames(2);
        assert!(
            name(&h).to_lowercase().starts_with('i'),
            "typed i: {} (before {:?})",
            name(&h),
            before
        );
    }

    #[test]
    fn esc_during_a_drag_drops_nothing() {
        let mut h = Harness::new(WIDE_SIZE);
        h.st.show_original(&h.inv, Uuid::from_u128(8100));
        h.frames(4);
        let row = h.row(8100);
        let folder = h.row(8001);
        let name = egui::pos2(row.left() + 40.0, row.center().y);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        h.run(vec![egui::Event::PointerMoved(name)]);
        h.run(vec![button(name, true)]);
        for step in 1..=4 {
            h.run(vec![egui::Event::PointerMoved(name + egui::vec2(0.0, 6.0 * step as f32))]);
        }
        assert!(drag::carried_ids(&h.ctx).is_some(), "the drag started");
        h.run(vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        let over = folder.center();
        for _ in 0..3 {
            h.run(vec![egui::Event::PointerMoved(over)]);
        }
        assert!(drag::carried_ids(&h.ctx).is_none(), "Esc ended the drag");
        h.actions.clear();
        h.run(vec![button(over, false)]);
        h.frames(2);
        assert!(
            !h.actions.iter().any(|a| matches!(a, InvAction::Drop(_) | InvAction::Edit(_))),
            "nothing dropped"
        );
    }

    #[test]
    fn a_row_drags_by_its_name_and_its_other_columns_draw_the_band() {
        let mut h = Harness::new(WIDE_SIZE);
        h.st.show_original(&h.inv, Uuid::from_u128(8100));
        h.frames(4);
        let row = h.row(8100);
        // from the date column, down over the next rows: a band, no cargo
        let from = egui::pos2(row.right() - 12.0, row.center().y);
        let to = egui::pos2(from.x, row.bottom() + 70.0);
        assert!(!press_and_move(&mut h, from, to), "the other columns do not carry the item");
        assert!(
            h.st.selection.len() >= 3,
            "the band selects the rows it crossed: {:?}",
            h.st.selection.len()
        );
        // a click there still selects the row alone
        h.click(from, Default::default());
        assert_eq!(h.st.selection, HashSet::from([Uuid::from_u128(8100)]));
        // from its name: the item is carried, nothing banded
        let name = egui::pos2(row.left() + 40.0, row.center().y);
        assert!(
            press_and_move(&mut h, name, egui::pos2(name.x + 10.0, row.bottom() + 70.0)),
            "the name carries the item"
        );
    }

    #[test]
    fn results_rows_drag_by_their_name_and_their_other_columns_draw_the_band() {
        // Recent, Worn, Favorites and a search list results, not a folder
        let mut h = Harness::new(WIDE_SIZE);
        h.frames(3);
        h.st.search = "Demo".into();
        h.frames(6);
        let drawn: Vec<Rect> = (8000..8300u128)
            .filter_map(|n| rows::tests::drawn(&h.ctx, Uuid::from_u128(n)))
            .collect();
        let first = drawn.iter().min_by(|a, b| a.top().total_cmp(&b.top())).copied().expect("rows");
        let from = egui::pos2(first.right() - 12.0, first.center().y);
        let to = egui::pos2(from.x, first.bottom() + 70.0);
        assert!(!press_and_move(&mut h, from, to), "the other columns do not carry the row");
        assert!(h.st.selection.len() >= 3, "banded: {}", h.st.selection.len());
    }

    #[test]
    fn folder_buttons_fold_each_view_and_search_has_its_own_row() {
        for width in [DEFAULT_SIZE.x, 800.0, WIDE_SIZE] {
            for scoped in [false, true] {
                let mut h = Harness::new(width);
                let folder = Uuid::from_u128(8000);
                let child = Uuid::from_u128(8001);
                if scoped {
                    h.st.scope = folder;
                }
                h.frames(5);
                let tool = |h: &Harness, icon| {
                    h.ctx
                        .data(|data| data.get_temp::<Rect>(egui::Id::unique(("inventory_tool_button", icon))))
                        .expect("toolbar button drawn")
                };
                let collapse = tool(&h, "chevrons-up");
                let expand = tool(&h, "chevrons-down");
                let search = h
                    .ctx
                    .data(|data| data.get_temp::<Rect>(egui::Id::unique("inventory_search_row")))
                    .unwrap();
                assert!(expand.right() <= search.right() + 0.5, "{width}: tools fit inside the window");
                assert!(search.top() >= expand.bottom(), "{width}: search is below the tools");
                assert!(
                    search.contains_rect(tool(&h, "filters-filled")),
                    "advanced search stays beside its field"
                );
                h.click(expand.center(), Default::default());
                if width == DEFAULT_SIZE.x {
                    assert!(h.st.plain.is_open(child), "expand reaches nested folders");
                } else {
                    assert!(h.st.side.rows.iter().any(|row| row.id == child), "expand reaches the sidebar");
                }
                h.click(collapse.center(), Default::default());
                if width == DEFAULT_SIZE.x {
                    assert!(!h.st.plain.is_open(child), "collapse closes nested folders too");
                } else if !scoped {
                    assert!(
                        !h.st.side.rows.iter().any(|row| row.id == child),
                        "the parent is collapsed in the sidebar"
                    );
                }
                assert_eq!(h.st.browse, if scoped { folder } else { h.inv.root }, "folding never navigates");
                assert!(h.actions.is_empty(), "folding does not mutate inventory");

                h.st.filters.types = 1 << 6;
                h.frames(3);
                let filters = h.st.filters.clone();
                h.click(tool(&h, "chevrons-up").center(), Default::default());
                if !scoped {
                    assert!(!h.st.result_tree.is_open(folder, 0), "filtered results consume the command");
                }
                h.click(tool(&h, "chevrons-down").center(), Default::default());
                assert!(h.st.result_tree.is_open(folder, 0));
                assert_eq!(h.st.filters, filters, "the filter is retained");

                h.click(tool(&h, "filters-filled").center(), Default::default());
                assert!(h.st.advanced, "the relocated advanced-search button still works");
                h.click(tool(&h, "filters-filled").center(), Default::default());
                assert!(!h.st.advanced);
            }
        }
    }

    #[test]
    fn every_width_draws_every_selection_filter_tab_and_choice_of_rights() {
        for width in [DEFAULT_SIZE.x, 800.0, WIDE_SIZE] {
            let mut h = Harness::new(width);
            h.frames(3);
            let layout = h.st.aurora.map(|frame| frame.layout);
            assert_eq!(layout, Some(Layout::for_width(width - 20.0)), "{width} px");
            let (object, other, folder) = (Uuid::from_u128(8100), Uuid::from_u128(8101), Uuid::from_u128(8000));
            for selection in [vec![], vec![object], vec![folder], vec![object, other]] {
                h.st.selection = selection.iter().copied().collect();
                h.st.selected = selection.first().copied();
                for rights in [Rights::Hidden, Rights::Mine, Rights::Next, Rights::Both] {
                    h.prefs.rights = rights;
                    h.frames(2);
                }
            }
            for tab in [2, 3, 4, RECEIVED_TAB, 0] {
                h.st.tab = tab;
                h.frames(2);
            }
            h.st.filters.types = 1 << 6;
            h.st.filters.permissions = rules::MODIFY;
            h.frames(2);
            h.st.filters = view::Filters::default();
            h.st.search = "demo".into();
            h.frames(2);
            h.st.search.clear();
            h.prefs.show_details = false;
            h.frames(2);
        }
    }

    #[test]
    fn a_folder_window_stays_under_its_folder_at_every_width() {
        let folder = Uuid::from_u128(8000);
        for width in [DEFAULT_SIZE.x, 800.0, WIDE_SIZE] {
            let mut h = Harness::new(width);
            h.st.scope = folder;
            h.frames(4);
            assert_eq!(h.st.browse, folder, "{width} px: the explorer starts at the folder");
            // neither the root nor a folder outside it can be shown
            h.st.browse = h.inv.root;
            h.frames(2);
            assert_eq!(h.st.browse, folder, "{width} px");
            h.st.tab = RECEIVED_TAB;
            h.frames(2);
            assert_eq!(h.st.tab, 0, "{width} px: a folder window has no tabs");
            assert!(
                h.st.side.rows.iter().all(|row| rules::under(&h.inv, row.id, folder)),
                "{width} px: the column lists the folder's subfolders only"
            );
            let area = |h: &Harness| {
                h.ctx
                    .memory(|m| m.area_rect(egui::Id::unique("aurora_inventory_test")))
                    .expect("window")
            };
            let before = area(&h).height();
            h.frames(30);
            assert!(area(&h).height() <= before + 0.5, "{width} px: the window does not grow");
            // its rows are the folder's: one is clicked
            h.frames(40);
            let row = h.row(8101);
            h.click(row.center(), Default::default());
            assert_eq!(h.st.selected, Some(Uuid::from_u128(8101)), "{width} px");
            if width >= 800.0 {
                let sub = h.row(8001).left_center() + egui::vec2(60.0, 0.0);
                h.double_click(sub);
                assert_eq!(h.st.browse, Uuid::from_u128(8001), "{width} px: going into a subfolder");
            }
        }
    }

    #[test]
    fn no_tab_makes_the_window_grow() {
        for width in [DEFAULT_SIZE.x, 800.0, WIDE_SIZE] {
            for (tab, received) in [
                (0, false),
                (2, false),
                (3, false),
                (4, false),
                (RECEIVED_TAB, false),
                (RECEIVED_TAB, true),
            ] {
                let mut h = Harness::new(width);
                if received {
                    crate::world::inventory::demo::seed_received(&mut h.inv, Uuid::from_u128(2));
                }
                h.st.tab = tab;
                h.frames(4);
                let area = |h: &Harness| {
                    h.ctx
                        .memory(|m| m.area_rect(egui::Id::unique("aurora_inventory_test")))
                        .expect("window")
                };
                let before = area(&h).height();
                h.frames(30);
                let after = area(&h).height();
                assert!(
                    after <= before + 0.5,
                    "{width} px, tab {tab}, received {received}: {before} → {after}"
                );
            }
        }
    }

    #[test]
    fn hover_add_preserves_worn_items_regardless_of_double_click_preferences() {
        use crate::world::appearance::Action as Wear;

        for width in [DEFAULT_SIZE.x, 800.0, WIDE_SIZE] {
            for add_on_double_click in [false, true] {
                for (kind, linked, library) in [
                    (6, false, false),
                    (5, false, false),
                    (6, true, false),
                    (6, false, true),
                    (5, true, true),
                ] {
                    let mut h = Harness::new(width);
                    h.prefs.double_click_add_objects = add_on_double_click;
                    h.prefs.double_click_add_clothes = add_on_double_click;
                    let row = Uuid::from_u128(8100);
                    let mut item = h.inv.items[&row].clone();
                    item.asset_type = kind;
                    item.flags = 4;
                    if library {
                        h.inv.folders.get_mut(&item.parent).unwrap().library = true;
                    }
                    let original = if linked {
                        item.id = Uuid::from_u128(8999);
                        let original = item.id;
                        h.inv.items.insert(original, item);
                        let link = h.inv.items.get_mut(&row).unwrap();
                        link.asset_type = 24;
                        link.asset_id = original;
                        original
                    } else {
                        h.inv.items.insert(row, item);
                        row
                    };
                    h.st.show_original(&h.inv, row);
                    h.frames(4);
                    h.run(vec![egui::Event::PointerMoved(h.row(8100).center())]);
                    h.frames(2);
                    let button = h
                        .ctx
                        .data(|data| data.get_temp::<Rect>(egui::Id::unique(("inventory_quick", row, 0_usize))))
                        .expect("hover action drawn");
                    h.actions.clear();
                    h.click(button.center(), Default::default());
                    if library {
                        assert!(matches!(&h.actions[..], [InvAction::WearCopy { item, replace: false }] if *item == original));
                    } else {
                        assert!(
                            matches!(&h.actions[..], [InvAction::Appearance(Wear::WearItem { item, replace: false, point: 0 })] if *item == original)
                        );
                    }
                    h.facts.appearance_busy = true;
                    h.actions.clear();
                    // Exercise a separate single click, not the row's double-click gesture.
                    h.frames(40);
                    h.click(button.center(), Default::default());
                    assert!(h.actions.is_empty(), "hover actions wait for the current appearance operation");
                }
            }
        }
    }

    fn press(h: &mut Harness, key: egui::Key) {
        h.run(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        h.frames(2);
    }

    /// The explorer on the right-click tests' folder, with the demo's
    /// pictures, in `view`; its list's order first.
    fn tiled(width: f32, view: view::ExplorerView) -> (Harness, Vec<Uuid>) {
        let mut h = Harness::new(width);
        crate::world::inventory::demo::seed_gallery(&mut h.inv);
        h.frames(2);
        h.st.show_original(&h.inv, Uuid::from_u128(8100));
        h.frames(4);
        let listed = h.st.flat.order.clone();
        h.prefs.explorer.view = view;
        // small tiles: the whole folder in sight at both widths
        h.prefs.explorer.tile_size = 64;
        h.st.selection.clear();
        h.st.selected = None;
        h.frames(4);
        (h, listed)
    }

    /// A tile's name, below its picture (and its wear buttons).
    fn name_of(rect: Rect) -> egui::Pos2 {
        egui::pos2(rect.center().x, rect.bottom() - 8.0)
    }

    #[test]
    fn the_gallery_tiles_folders_then_items_pictures_first() {
        for width in [800.0, WIDE_SIZE] {
            let (mut h, listed) = tiled(width, view::ExplorerView::Gallery);
            assert_eq!(h.st.browse, Uuid::from_u128(8000));
            let pictured = |id: &Uuid| thumbnail::tile_picture(&h.inv, *id).is_some();
            // LLInventoryGallery::compareGalleryItem: the folders, then the
            // items, the pictured ones first, in the list's order otherwise
            let mut expected = listed.clone();
            expected.sort_by_key(|id| (h.inv.items.contains_key(id), !pictured(id)));
            assert_eq!(h.st.flat.order, expected, "{width} px");
            assert_eq!(h.st.flat.tiles, listed.len(), "every entry is a tile");
            assert_eq!(h.st.flat.order[0], Uuid::from_u128(8001), "the pictured folder leads");
            // a tile is a row: a click selects it, Ctrl adds one
            h.click(name_of(h.row(8101)), Default::default());
            assert_eq!(h.st.selected, Some(Uuid::from_u128(8101)), "{width} px");
            h.click(name_of(h.row(8102)), egui::Modifiers::COMMAND);
            assert_eq!(h.st.selection.len(), 2, "{width} px");
            // a double-click opens a notecard, and goes into a folder
            h.actions.clear();
            h.double_click(name_of(h.row(8103)));
            assert!(
                h.actions
                    .iter()
                    .any(|a| matches!(a, InvAction::Preview(id, _) if *id == Uuid::from_u128(8103))),
                "{width} px"
            );
            h.double_click(name_of(h.row(8001)));
            assert_eq!(h.st.browse, Uuid::from_u128(8001), "{width} px");
        }
    }

    #[test]
    fn the_arrows_move_across_the_tiles_and_down_a_row() {
        let (mut h, _) = tiled(WIDE_SIZE, view::ExplorerView::Gallery);
        let order = h.st.flat.order.clone();
        let first = h.row(order[0].as_u128());
        let second = h.row(order[1].as_u128());
        // the tiles' row: as many as the width holds
        let columns = order
            .iter()
            .take_while(|id| (h.row(id.as_u128()).top() - first.top()).abs() < 0.5)
            .count();
        assert!(columns >= 2 && second.left() > first.right(), "tiles side by side");
        h.click(name_of(first), Default::default());
        press(&mut h, egui::Key::ArrowRight);
        assert_eq!(h.st.selected, Some(order[1]));
        press(&mut h, egui::Key::ArrowDown);
        assert_eq!(h.st.selected, Some(order[1 + columns]), "down one row");
        press(&mut h, egui::Key::ArrowLeft);
        press(&mut h, egui::Key::ArrowUp);
        assert_eq!(h.st.selected, Some(order[0]));
        assert_eq!(h.st.selection.len(), 1);
    }

    #[test]
    fn the_combination_view_tiles_the_pictured_entries_above_the_list() {
        for width in [800.0, WIDE_SIZE] {
            let (mut h, listed) = tiled(width, view::ExplorerView::Combination);
            let pictured: Vec<Uuid> = listed
                .iter()
                .copied()
                .filter(|id| thumbnail::tile_picture(&h.inv, *id).is_some())
                .collect();
            let others: Vec<Uuid> = listed.iter().copied().filter(|id| !pictured.contains(id)).collect();
            assert_eq!(pictured.len(), 7, "five items, a link to one and a folder have pictures");
            assert_eq!(h.st.flat.tiles, pictured.len());
            assert_eq!(h.st.flat.order, [pictured.clone(), others.clone()].concat(), "{width} px");
            // the tiles above, the list's rows (as tall as rows) below
            let tile = h.row(pictured[0].as_u128());
            let row = h.row(others[0].as_u128());
            assert!(tile.height() > rows::HEIGHT * 2.0 && row.height() < rows::HEIGHT + 1.0);
            assert!(row.top() > tile.bottom(), "{width} px");
            // down from the last tiles' row goes on into the list
            h.click(name_of(h.row(pictured[pictured.len() - 1].as_u128())), Default::default());
            press(&mut h, egui::Key::ArrowDown);
            assert_eq!(h.st.selected, Some(others[0]), "{width} px");
            press(&mut h, egui::Key::ArrowDown);
            assert_eq!(h.st.selected, Some(others[1]), "the list steps its rows");
            // the bar between them sets the tiles' height, a double-click resets it
            let hit = h
                .ctx
                .data(|data| data.get_temp::<Rect>(egui::Id::unique("inventory_gallery_split")))
                .expect("split drawn");
            let start = hit.center();
            h.run(vec![egui::Event::PointerMoved(start)]);
            h.run(vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }]);
            h.run(vec![egui::Event::PointerMoved(start + egui::vec2(0.0, 40.0))]);
            h.run(vec![egui::Event::PointerButton {
                pos: start + egui::vec2(0.0, 40.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }]);
            h.frames(2);
            assert!(h.prefs.explorer.gallery_height > 0, "{width} px");
            let hit = h
                .ctx
                .data(|data| data.get_temp::<Rect>(egui::Id::unique("inventory_gallery_split")))
                .expect("split drawn");
            h.double_click(hit.center());
            assert_eq!(h.prefs.explorer.gallery_height, 0);
        }
    }

    #[test]
    fn a_folder_with_no_picture_leaves_the_combination_view_to_its_list() {
        let mut h = Harness::new(WIDE_SIZE);
        h.frames(2);
        h.st.show_original(&h.inv, Uuid::from_u128(8100));
        h.prefs.explorer.view = view::ExplorerView::Combination;
        h.frames(4);
        assert_eq!(h.st.flat.tiles, 0);
        assert!(
            h.ctx
                .data(|data| data.get_temp::<Rect>(egui::Id::unique("inventory_gallery_split")))
                .is_none(),
            "no bar without tiles"
        );
        let row = h.row(8101);
        assert!(row.height() < rows::HEIGHT + 1.0, "the entries are rows");
        // a search lists its results as rows, in every view
        h.prefs.explorer.view = view::ExplorerView::Gallery;
        h.st.search = "demo".into();
        h.frames(6);
        assert!(h.row(8101).height() < rows::HEIGHT + 1.0);
    }

    #[test]
    fn a_folder_s_or_a_link_s_picture_shows_and_a_texture_is_not_its_own() {
        let mut h = Harness::new(WIDE_SIZE);
        crate::world::inventory::demo::seed_gallery(&mut h.inv);
        let id = Uuid::from_u128;
        // its own, a folder's
        assert_eq!(thumbnail::tile_picture(&h.inv, id(8100)), Some(crate::demo::TEX_SNAPSHOT));
        assert_eq!(thumbnail::tile_picture(&h.inv, id(8001)), Some(crate::demo::TEX_TILES));
        // the link to the object shows the object's
        assert_eq!(thumbnail::tile_picture(&h.inv, id(8113)), Some(crate::demo::TEX_SNAPSHOT));
        // a texture without one is not its own picture (its tooltip shows it)
        assert_eq!(thumbnail::tile_picture(&h.inv, id(8104)), None);
        // an outfit folder: its only texture, none with two
        let outfit = id(0x0ff1);
        h.inv.folders.insert(
            outfit,
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: outfit,
                    parent: id(8000),
                    type_default: 47,
                    ..Default::default()
                },
                children: vec![],
                items: vec![id(8104)],
                state: FetchState::Fetched,
                library: false,
            },
        );
        assert_eq!(thumbnail::tile_picture(&h.inv, outfit), Some(crate::demo::TEX_SNAPSHOT));
        h.inv.folders.get_mut(&outfit).expect("outfit").items.push(id(8104));
        assert_eq!(thumbnail::tile_picture(&h.inv, outfit), None, "two textures: no single picture");
        h.st.selection.clear();
    }

    #[test]
    fn a_click_selects_a_row_and_the_explorer_goes_into_a_folder() {
        for width in [DEFAULT_SIZE.x, 800.0, WIDE_SIZE] {
            let mut h = Harness::new(width);
            h.frames(2);
            // The demo object's folder, shown open (narrow) or gone into.
            let object = Uuid::from_u128(8100);
            h.st.show_original(&h.inv, object);
            h.frames(4);
            // on the name, left of the quick buttons the hovered row shows
            let sibling = h.row(8101);
            h.click(sibling.left_center() + egui::vec2(60.0, 0.0), Default::default());
            assert_eq!(h.st.selected, Some(Uuid::from_u128(8101)), "{width} px");
            assert_eq!(h.st.selection.len(), 1);
            // Ctrl adds the next one.
            let next = h.row(8102);
            h.click(next.left_center() + egui::vec2(60.0, 0.0), egui::Modifiers::COMMAND);
            assert_eq!(h.st.selection.len(), 2, "{width} px");
            // A double-click opens a notecard.
            let note = h.row(8103).left_center() + egui::vec2(60.0, 0.0);
            h.actions.clear();
            h.double_click(note);
            assert!(
                h.actions
                    .iter()
                    .any(|a| matches!(a, InvAction::Preview(id, _) if *id == Uuid::from_u128(8103))),
                "{width} px"
            );
            if width >= 800.0 {
                assert_eq!(h.st.browse, Uuid::from_u128(8000), "the explorer shows the revealed item's folder");
                // A double-click on a folder row goes into it.
                let sub = h.row(8001).center();
                h.double_click(sub);
                assert_eq!(h.st.browse, Uuid::from_u128(8001), "{width} px");
            }
        }
    }

    fn sidebar_inventory(width: f32) -> Harness {
        let mut h = Harness::new(width);
        crate::world::inventory::demo::seed_sidebar(&mut h.inv);
        h.frames(4);
        h
    }

    #[test]
    fn clicking_sidebar_folders_keeps_their_position() {
        for width in [800.0, WIDE_SIZE] {
            let mut h = sidebar_inventory(width);
            for scrolled in [false, true] {
                if scrolled {
                    let (_, viewport) = h.st.side.scroll.expect("sidebar");
                    h.run(vec![
                        egui::Event::PointerMoved(viewport.center()),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, -493.0),
                            phase: egui::TouchPhase::Move,
                            source: egui::MouseWheelSource::Wheel,
                            modifiers: Default::default(),
                        },
                    ]);
                    h.frames(60);
                }
                let (before_offset, viewport) = h.st.side.scroll.expect("sidebar");
                if scrolled {
                    assert!(before_offset > 0.0, "the mouse wheel scrolled the sidebar");
                }
                for bottom in [true, false] {
                    let (&id, &before) =
                        h.st.side
                            .drawn
                            .iter()
                            .filter(|(_, rect)| viewport.intersect(**rect).height() > 1.0)
                            .min_by(|(_, a), (_, b)| {
                                let order = a.top().total_cmp(&b.top());
                                if bottom { order.reverse() } else { order }
                            })
                            .expect("visible folder");
                    h.frames(40);
                    // A sliver of a row can be less than the old 1 px inset:
                    // target its visible part rather than its neighbour.
                    let y = before.intersect(viewport).center().y;
                    assert!(before.contains(egui::pos2(before.right() - 30.0, y)));
                    h.click(egui::pos2(before.right() - 30.0, y), Default::default());
                    assert_eq!(h.st.browse, id, "{width} px: the click opens the folder");
                    let after = h.st.side.drawn.get(&id).expect("clicked folder still drawn");
                    assert!((after.top() - before.top()).abs() < 0.1, "{width} px: {before:?} -> {after:?}");
                    let (after_offset, _) = h.st.side.scroll.expect("sidebar");
                    assert!((after_offset - before_offset).abs() < 0.1, "a click leaves the scroll offset alone");
                }
            }
        }
    }

    #[test]
    fn navigating_to_offscreen_sidebar_folders_reveals_them_at_the_nearest_edge() {
        for width in [800.0, WIDE_SIZE] {
            let mut h = sidebar_inventory(width);
            let last = Uuid::from_u128(30_079);
            assert!(!h.st.side.drawn.contains_key(&last), "the target starts below the viewport");
            h.st.show_folder(&h.inv, last);
            h.frames(4);
            let (_, viewport) = h.st.side.scroll.expect("sidebar");
            let row = h.st.side.drawn.get(&last).expect("revealed folder");
            assert!(viewport.contains_rect(*row), "{width} px: {row:?} in {viewport:?}");
            assert!(
                (row.bottom() - viewport.bottom()).abs() < 1.0,
                "{width} px: reveal only scrolls to the nearest edge: {row:?} in {viewport:?}"
            );
            let first = Uuid::from_u128(30_000);
            h.st.show_folder(&h.inv, first);
            h.frames(4);
            let (_, viewport) = h.st.side.scroll.expect("sidebar");
            let row = h.st.side.drawn.get(&first).expect("revealed folder above");
            assert!(viewport.contains_rect(*row), "{width} px: {row:?} in {viewport:?}");
            assert!(
                (row.top() - viewport.top()).abs() < 1.0,
                "{width} px: reveal above uses the top edge: {row:?} in {viewport:?}"
            );
        }
    }

    #[test]
    fn the_counts_follow_the_recent_tab_and_the_system_folders() {
        use crate::world::inventory::Folder;
        use aurora_net::inventory::{InvFolder, InvItem};
        let id = Uuid::from_u128;
        let mut inv = Inventory {
            root: id(1),
            ..Default::default()
        };
        for (n, parent, kind) in [(1, 0, 8), (14, 1, 14), (50, 1, 50), (20, 1, -1)] {
            inv.folders.insert(
                id(n),
                Folder {
                    info: InvFolder {
                        id: id(n),
                        parent: id(parent),
                        type_default: kind,
                        favorite: n == 20,
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: Vec::new(),
                    state: FetchState::Fetched,
                    library: false,
                },
            );
        }
        let now = view::now();
        inv.add_items(vec![
            InvItem {
                id: id(100),
                parent: id(20),
                created_at: now - 60,
                favorite: true,
                ..Default::default()
            },
            InvItem {
                id: id(101),
                parent: id(14),
                created_at: now - 60,
                ..Default::default()
            },
            InvItem {
                id: id(102),
                parent: id(50),
                created_at: now - 3 * 86400,
                ..Default::default()
            },
        ]);
        let prefs = InventoryPreferences::default();
        let mut counts = Counts::default();
        counts.update(&inv, &prefs, 0);
        assert_eq!(counts.recent, 1, "the trash is not searched by default");
        assert_eq!(counts.favorites, 2, "a favourite item and a favourite folder");
        assert_eq!(counts.received, 1);
        assert_eq!(counts.trash, 1);
    }

    #[test]
    fn favorites_count_uses_own_flags_not_link_targets() {
        use aurora_net::inventory::{InvFolder, InvItem};
        let id = Uuid::from_u128;
        let mut inv = Inventory {
            root: id(1),
            ..Default::default()
        };
        inv.created_folders(vec![
            InvFolder {
                id: inv.root,
                type_default: 8,
                ..Default::default()
            },
            InvFolder {
                id: id(2),
                parent: inv.root,
                favorite: true,
                ..Default::default()
            },
        ]);
        inv.add_items(vec![InvItem {
            id: id(10),
            parent: inv.root,
            favorite: true,
            ..Default::default()
        }]);
        inv.add_items(
            (100..200)
                .map(|n| InvItem {
                    id: id(n),
                    parent: id(2),
                    asset_type: 24,
                    asset_id: id(10),
                    ..Default::default()
                })
                .collect(),
        );
        inv.add_items(vec![InvItem {
            id: id(20),
            parent: inv.root,
            asset_type: 25,
            asset_id: id(2),
            ..Default::default()
        }]);
        let prefs = InventoryPreferences::default();
        let mut counts = Counts::default();
        counts.update(&inv, &prefs, 0);
        assert_eq!(counts.favorites, 2, "one explicit item and one explicit folder, not their aliases");
        assert!(rules::is_favorite(&inv, id(100)) && rules::is_favorite(&inv, id(20)));
        inv.items.get_mut(&id(100)).expect("link").favorite = true;
        inv.generation += 1;
        counts.at = None;
        counts.update(&inv, &prefs, 0);
        assert_eq!(counts.favorites, 3, "an explicitly flagged link still counts");
        inv.items.get_mut(&id(10)).expect("original").favorite = false;
        inv.generation += 1;
        counts.at = None;
        counts.update(&inv, &prefs, 0);
        assert_eq!(counts.favorites, 2, "a changed original must not add or remove alias entries");
    }
}
