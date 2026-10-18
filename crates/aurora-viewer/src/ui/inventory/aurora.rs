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
        egui::AreaState::load(ctx, egui::Id::new(id))
            .and_then(|state| state.size)
            .map_or(Self::Compact, |size| Self::for_width(size.x - 20.0))
    }
}

/// How the rows of Aurora's window draw this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub layout: Layout,
    pub rights: Rights,
    /// The explorer's list is drawn: its folders go in rather than open.
    pub explorer: bool,
}

/// The wide width the "Widen" button of the title bar gives the window.
const WIDE_SIZE: f32 = 1180.0;

/// "Widen the window": the explorer with its details pane, as tall as it is.
pub fn widen(ctx: &egui::Context, id: &str) {
    let height = egui::AreaState::load(ctx, egui::Id::new(id))
        .and_then(|state| state.size)
        .map_or(DEFAULT_SIZE.y, |size| size.y);
    widgets::size_floater(ctx, id, egui::vec2(WIDE_SIZE, height.max(DEFAULT_SIZE.y)));
}

/// `AURORA_DEMO_INVENTORY=aurora|aurora-medium|aurora-wide|aurora-rights`.
fn demo_mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_INVENTORY").ok().filter(|mode| mode.starts_with("aurora")))
        .as_deref()
}

/// Whether one of Aurora's inventory scenarios runs.
pub fn demo_active() -> bool {
    demo_mode().is_some()
}

/// The demo's window at the width of its layout, whatever size an earlier
/// run left in the layout file.
pub fn demo_place(ctx: &egui::Context, frame: u64) {
    let Some(mode) = demo_mode() else { return };
    if frame == 2 {
        let size = match mode {
            "aurora-medium" => egui::vec2(800.0, 640.0),
            "aurora-wide" | "aurora-rights" => egui::vec2(WIDE_SIZE, 720.0),
            _ => egui::vec2(DEFAULT_SIZE.x, 640.0),
        };
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
        self.favorites = inv.items.values().filter(|it| it.favorite).count()
            + inv.folders.values().filter(|f| f.info.favorite).count();
        let held = |kind: i32| {
            rules::system(inv, kind)
                .and_then(|id| inv.folders.get(&id))
                .map_or(0, |f| f.items.len() + f.children.len())
        };
        self.received = held(50);
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
        explorer: false,
    });
    controls::init_filters(st, prefs);
    st.counts.update(inv, prefs, facts.worn.len());
    if st.browse.is_nil() || !inv.folders.contains_key(&st.browse) {
        st.browse = inv.root;
    }
    match layout {
        Layout::Compact => compact(ui, p, icons, inv, st, prefs, facts, images, actions),
        Layout::Medium | Layout::Wide => explorer(ui, p, icons, inv, st, prefs, facts, images, actions, layout),
    }
    row_menu(ui, p, inv, st, prefs, facts, actions);
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
        egui::Id::new(("inventory_row_menu", st.ui_id)),
        pos,
        &mut open,
        p,
        |ui| context::show(ui, p, inv, id, st, prefs, facts, actions),
    );
    if !open || !(inv.items.contains_key(&id) || inv.folders.contains_key(&id)) {
        st.row_menu = None;
    }
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
    toolbar::tabs(ui, p, inv, st, prefs);
    ui.add_space(6.0);
    toolbar::search_row(ui, p, inv, st, prefs, Layout::Compact, actions);
    ui.add_space(4.0);
    let (root, filtered) = list_scope(inv, st, prefs, facts);
    let message = toolbar::message(p, st);
    let below = details::STRIP + 8.0 + if message.is_some() { toolbar::MESSAGE } else { 0.0 };
    let list_h = (ui.available_height() - below).max(60.0);
    let out = list(ui, p, icons, inv, st, prefs, facts, actions, list_h, root, filtered, false);
    let zone = ui.interact(out.inner_rect, ui.id().with("aurora_list_zone"), Sense::hover());
    task_drop::target(ui, p, &zone, inv, root, prefs, actions);
    drag::target(ui, p, &zone, inv, root, st, prefs, facts, actions);
    if let Some((text, color)) = message {
        toolbar::message_line(ui, st, &text, color);
    }
    ui.add_space(6.0);
    details::strip(ui, p, icons, inv, st, prefs, facts, images, actions);
    clipboard::shortcuts(ui, inv, root, st, prefs, facts, actions);
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
    let side_w = if layout == Layout::Wide { 220.0 } else { 196.0 };
    let pane_w = if layout == Layout::Wide && !st.hide_details { 300.0 } else { 0.0 };
    const GAP: f32 = 8.0;
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
    let list_id = ui.id().with("aurora_inventory_list");
    let Some(frame) = st.aurora else { return };
    rows::header(ui, p, frame, list_id);
    ui.add_space(2.0);
    let (root, filtered) = list_scope(inv, st, prefs, facts);
    let message = toolbar::message(p, st);
    let below = match layout {
        Layout::Medium => details::STRIP + 8.0 + if message.is_some() { toolbar::MESSAGE } else { 0.0 },
        _ => toolbar::STATUS,
    };
    let list_h = (ui.available_height() - below).max(60.0);
    let explore = !filtered && !root.is_nil();
    let target = if explore { st.browse } else { root };
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
        let out = list(ui, p, icons, inv, st, prefs, facts, actions, list_h, browse, false, false);
        if let Some(frame) = &mut st.aurora {
            frame.explorer = false;
        }
        std::mem::swap(&mut st.plain, &mut st.flat);
        out
    } else {
        list(ui, p, icons, inv, st, prefs, facts, actions, list_h, root, filtered, false)
    };
    widgets::remember_scroll_bar(ui, list_id, &out);
    let zone = ui.interact(out.inner_rect, ui.id().with("aurora_list_zone"), Sense::hover());
    task_drop::target(ui, p, &zone, inv, target, prefs, actions);
    drag::target(ui, p, &zone, inv, target, st, prefs, facts, actions);
    match layout {
        Layout::Medium => {
            if let Some((text, color)) = message {
                toolbar::message_line(ui, st, &text, color);
            }
            ui.add_space(6.0);
            details::strip(ui, p, icons, inv, st, prefs, facts, images, actions);
        }
        _ => toolbar::status_bar(ui, p, st, explore, message),
    }
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
            drop(headless_output(self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
                    time: Some(self.frame as f64 / 60.0),
                    events: std::iter::once(egui::Event::ModifiersChanged(self.modifiers)).chain(events).collect(),
                    ..Default::default()
                },
                |ui| {
                    widgets::Floater::new("aurora_inventory_test", "Inventory", egui::pos2(20.0, 20.0), egui::vec2(width, 720.0))
                        .silent()
                        .show(ui.ctx(), &palette, &mut true, |ui| {
                            contents(ui, &palette, &Icons::default(), inv, st, prefs, facts, &Default::default(), actions);
                        });
                },
            )));
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
            h.st.hide_details = true;
            h.frames(2);
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
                h.actions.iter().any(|a| matches!(a, InvAction::Preview(id) if *id == Uuid::from_u128(8103))),
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
