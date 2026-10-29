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

mod columns;
mod details;
mod rights;
pub(super) mod rows;
mod sidebar;
mod toolbar;

pub(super) use sidebar::Sidebar;

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
        egui::AreaState::load(ctx, egui::Id::unique(id))
            .and_then(|state| state.size)
            .map_or(Self::Compact, |size| Self::for_width(size.x - 20.0))
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
    let height = egui::AreaState::load(ctx, egui::Id::unique(id))
        .and_then(|state| state.size)
        .map_or(DEFAULT_SIZE.y, |size| size.y);
    widgets::size_floater(ctx, id, egui::vec2(WIDE_SIZE, height.max(DEFAULT_SIZE.y)));
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
            "aurora-medium" | "aurora-sidebar-medium" | "aurora-worn-medium" => egui::vec2(800.0, 640.0),
            "aurora-wide"
            | "aurora-worn-wide"
            | "aurora-sidebar-wide"
            | "aurora-keyboard-wide"
            | "aurora-rights"
            | "aurora-columns"
            | "aurora-columns-hidden"
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
    let count = inv.item_count();
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
    if fetching {
        (p.muted_dim, aurora_i18n::tr!("inventory-syncing").to_owned())
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
        self.favorites =
            inv.items.keys().filter(|id| rules::is_favorite(inv, **id)).count() + inv.folders.values().filter(|f| f.info.favorite).count();
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
    let layout = Layout::for_width(ui.available_width());
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
    match layout {
        Layout::Compact => compact(ui, p, icons, inv, st, prefs, facts, images, actions),
        Layout::Medium | Layout::Wide => explorer(ui, p, icons, inv, st, prefs, facts, images, actions, layout),
    }
    row_menu(ui, p, inv, st, prefs, facts, actions);
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
        |ui| context::show(ui, p, inv, id, st, prefs, facts, actions),
    );
    if !open || !(inv.items.contains_key(&id) || inv.folders.contains_key(&id)) {
        st.row_menu = None;
    }
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
    task_drop::target(ui, p, &zone, inv, target, prefs, actions);
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
) {
    toolbar::explorer_row(ui, p, inv, st, prefs, layout, actions);
    ui.add_space(8.0);
    let area = ui.available_rect_before_wrap();
    let pane_w = if layout == Layout::Wide && prefs.show_details { 300.0 } else { 0.0 };
    const GAP: f32 = 8.0;
    let side_w = columns::sidebar(ui, p, area, pane_w, layout, &mut prefs.explorer);
    let side = Rect::from_min_size(area.min, egui::vec2(side_w, area.height()));
    let pane = Rect::from_min_max(egui::pos2(area.right() - pane_w, area.top()), area.max);
    let content = Rect::from_min_max(
        egui::pos2(side.right() + GAP, area.top()),
        egui::pos2(if pane_w > 0.0 { pane.left() - GAP } else { area.right() }, area.bottom()),
    );
    let mut side_ui = ui.new_child(egui::UiBuilder::new().max_rect(side).id_salt("aurora_inventory_side"));
    side_ui.set_clip_rect(side.intersect(ui.clip_rect()));
    sidebar::show(&mut side_ui, p, icons, inv, st, prefs, facts, actions);
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
        _ => toolbar::breadcrumb_row(ui, p, inv, st),
    }
    let list_id = ui.scope_id().with("aurora_inventory_list");
    let Some(mut frame) = st.aurora else { return };
    frame.rights = prefs.rights;
    frame.columns = prefs.explorer;
    rows::header(ui, p, frame, list_id, &mut prefs.explorer);
    frame.columns = prefs.explorer;
    st.aurora = Some(frame);
    ui.add_space(2.0);
    let (root, filtered) = list_scope(inv, st, prefs, facts);
    let message = toolbar::message(p, st);
    let foot_h = match layout {
        Layout::Medium => card_height(prefs) + if message.is_some() { toolbar::MESSAGE } else { 0.0 },
        _ => toolbar::STATUS,
    };
    let (room, list_rect, foot) = list_and_foot(ui, foot_h);
    let list_h = list_rect.height();
    let explore = !filtered && !root.is_nil();
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
        if let Some(frame) = &mut st.aurora {
            frame.explorer = true;
        }
        let out = in_rect(ui, list_rect, "aurora_inventory_list_room", |ui| {
            list(ui, p, icons, inv, st, prefs, facts, actions, list_h, browse, false, false)
        });
        if let Some(frame) = &mut st.aurora {
            frame.explorer = false;
        }
        std::mem::swap(&mut st.plain, &mut st.flat);
        out
    } else {
        in_rect(ui, list_rect, "aurora_inventory_list_room", |ui| {
            list(ui, p, icons, inv, st, prefs, facts, actions, list_h, root, filtered, false)
        })
    };
    widgets::remember_scroll_bar(ui, list_id, &out);
    let zone = ui.interact(out.inner_rect, ui.scope_id().with("aurora_list_zone"), Sense::hover());
    task_drop::target(ui, p, &zone, inv, target, prefs, actions);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_widens_into_the_explorer_then_the_details() {
        assert_eq!(Layout::for_width(DEFAULT_SIZE.x - 20.0), Layout::Compact);
        assert_eq!(Layout::for_width(699.0), Layout::Compact);
        assert_eq!(Layout::for_width(700.0), Layout::Medium);
        assert_eq!(Layout::for_width(1019.0), Layout::Medium);
        assert_eq!(Layout::for_width(WIDE_SIZE - 20.0), Layout::Wide);
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
            for key in [
                egui::Id::unique("inventory_sidebar_handle"),
                egui::Id::unique(("inventory_column_handle", 0_usize)),
                egui::Id::unique(("inventory_column_handle", 1_usize)),
                egui::Id::unique(("inventory_column_handle", 2_usize)),
            ] {
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
            assert!(h.prefs.explorer.kind_width > 0);
            assert!(h.prefs.explorer.date_width > 0);
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
            for (header, row) in [(header.kind, row.kind), (header.rights, row.rights), (header.date, row.date)] {
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
                let sub = h.row(8001).center();
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

    #[test]
    fn a_click_selects_a_row_and_the_explorer_goes_into_a_folder() {
        for width in [DEFAULT_SIZE.x, 800.0, WIDE_SIZE] {
            let mut h = Harness::new(width);
            h.frames(2);
            // The demo object's folder, shown open (narrow) or gone into.
            let object = Uuid::from_u128(8100);
            h.st.show_original(&h.inv, object);
            h.frames(4);
            let sibling = h.row(8101);
            h.click(sibling.center(), Default::default());
            assert_eq!(h.st.selected, Some(Uuid::from_u128(8101)), "{width} px");
            assert_eq!(h.st.selection.len(), 1);
            // Ctrl adds the next one.
            let next = h.row(8102);
            h.click(next.center(), egui::Modifiers::COMMAND);
            assert_eq!(h.st.selection.len(), 2, "{width} px");
            // A double-click opens a notecard.
            let note = h.row(8103).center();
            h.actions.clear();
            h.double_click(note);
            assert!(
                h.actions
                    .iter()
                    .any(|a| matches!(a, InvAction::Preview(id) if *id == Uuid::from_u128(8103))),
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
                            .filter(|(_, rect)| viewport.intersects(**rect))
                            .min_by(|(_, a), (_, b)| {
                                let order = a.top().total_cmp(&b.top());
                                if bottom { order.reverse() } else { order }
                            })
                            .expect("visible folder");
                    h.frames(40);
                    let y = before.center().y.clamp(viewport.top() + 1.0, viewport.bottom() - 1.0);
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
}
