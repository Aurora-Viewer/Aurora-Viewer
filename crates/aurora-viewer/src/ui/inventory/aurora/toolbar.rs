//! The bars of Aurora's inventory window: the narrow width's icon tabs and
//! search row, the explorer's toolbar (where it is, the search with its
//! Ctrl+F, the filters, the details card or pane, the rights, "New"), the wide
//! width's filter chips, the line of messages and the status bar. The
//! sorting, the search field and the preferences are the Legacy Viewer's
//! (LLPanelMainInventory's gear and view menus), gathered in one "Sort"
//! menu.

use super::super::view::{Age, Creator, Links, SearchField, types};
use super::super::*;
use super::{Layout, rights};
use crate::ui::{menu, sheet, widgets};
use egui::{Color32, Pos2, Rect, Sense, Vec2};

/// The height of the toolbars' buttons and fields.
const BAR: f32 = 30.0;
/// The height of the line of messages, when there is one.
pub const MESSAGE: f32 = 22.0;
/// The height of the wide width's status bar.
pub const STATUS: f32 = 30.0;

/// A toolbar button with an icon only: lit while `lit` (its panel is open,
/// its mode is on).
fn tool_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, lit: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(BAR), Sense::click());
    let hot = response.hovered() || response.has_focus();
    if lit || hot {
        let fill = if hot { p.raised.lerp_to_gamma(p.ink, 0.16) } else { p.raised };
        ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    }
    let ink = if lit || hot { p.ink } else { p.muted };
    widgets::paint_glyph(ui.painter(), icon, rect.center(), 16.0, ink);
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    response.on_hover_text(tip)
}

/// Shows or hides the selection's details, kept with the inventory
/// preferences: the card at the bottom, or the pane at the wide width.
fn details_button(ui: &mut egui::Ui, p: &Palette, prefs: &mut InventoryPreferences, layout: Layout) {
    let shown = prefs.show_details;
    let (icon, tip) = match (layout, shown) {
        (Layout::Wide, true) => ("layout-sidebar-right-filled", aurora_i18n::tr!("inventory-details-hide")),
        (Layout::Wide, false) => ("layout-sidebar-right-filled", aurora_i18n::tr!("inventory-details-show")),
        (_, true) => ("layout-bottombar-filled", aurora_i18n::tr!("inventory-card-hide")),
        (_, false) => ("layout-bottombar-filled", aurora_i18n::tr!("inventory-card-show")),
    };
    if tool_button(ui, p, icon, tip, shown).clicked() {
        prefs.show_details = !shown;
    }
}

/// The tabs of the narrow width: an icon over a short name each; the
/// Received tab counts deliveries not looked at yet.
pub(super) fn tabs(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences) {
    let mut entries = vec![(0, "briefcase-filled", aurora_i18n::tr!("inventory-tab-all"))];
    for (tab, icon, label, shown) in [
        (2, "clock-filled", aurora_i18n::tr!("inventory-tab-recent"), prefs.show_recent),
        (3, "hanger-2-filled", aurora_i18n::tr!("inventory-tab-worn"), prefs.show_worn),
        (4, "star-filled", aurora_i18n::tr!("inventory-tab-favorites"), prefs.show_favorites),
        // the Received Items in the main window only
        (RECEIVED_TAB, "inbox", aurora_i18n::tr!("inventory-tab-received"), st.is_main()),
    ] {
        if shown {
            entries.push((tab, icon, label));
        }
    }
    let unseen = unseen_received(inv, st);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::hover());
    let width = (rect.width() - 2.0 * (entries.len() as f32 - 1.0)) / entries.len() as f32;
    for (n, (tab, icon, label)) in entries.into_iter().enumerate() {
        let cell = Rect::from_min_size(
            Pos2::new(rect.left() + n as f32 * (width + 2.0), rect.top()),
            Vec2::new(width, rect.height()),
        );
        let response = ui.interact(cell, ui.scope_id().with(("inventory_tab", tab)), Sense::click());
        let selected = st.tab == tab;
        let hot = response.hovered();
        if selected || hot {
            let fill = if selected { p.raised } else { p.raised.gamma_multiply(0.6) };
            ui.painter().rect_filled(cell, ui.visuals().window_corner_radius, fill);
        }
        let ink = if selected { p.ink } else { p.muted };
        widgets::paint_glyph(ui.painter(), icon, Pos2::new(cell.center().x, cell.top() + 14.0), 16.0, ink);
        let galley = ui.painter().layout_job({
            let mut job =
                egui::text::LayoutJob::single_section(label.to_owned(), egui::TextFormat::simple(egui::FontId::proportional(11.0), ink));
            job.wrap = egui::text::TextWrapping::truncate_at_width(cell.width() - 6.0);
            job
        });
        widgets::paint_ink_centered(ui.painter(), galley, Pos2::new(cell.center().x, cell.bottom() - 11.0), ink);
        if tab == RECEIVED_TAB && unseen > 0 {
            widgets::unread_count(
                ui.painter(),
                p,
                Pos2::new(cell.center().x + 22.0, cell.top() + 3.0),
                unseen,
                widgets::CountSize::Bar,
            );
        }
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
        if response.clicked() {
            set_tab(st, prefs, tab);
        }
    }
}

/// The Marketplace and sender deliveries not acknowledged by this account.
pub(super) fn unseen_received(inv: &Inventory, st: &mut InventoryUi) -> usize {
    st.received_badge.unseen(inv)
}

/// The hint of the search field: what it searches.
fn search_hint(prefs: &InventoryPreferences, short: bool) -> &'static str {
    if short {
        return aurora_i18n::tr!("inventory-search-short");
    }
    match prefs.search_field {
        SearchField::Name => aurora_i18n::tr!("inventory-search-name"),
        SearchField::Description => aurora_i18n::tr!("inventory-search-description"),
        SearchField::Creator => aurora_i18n::tr!("inventory-search-creator"),
        SearchField::Uuid => aurora_i18n::tr!("inventory-search-uuid"),
        SearchField::All => aurora_i18n::tr!("inventory-search-all"),
    }
}

/// The search field, `width` wide, taking Ctrl+F for the window; its key
/// shows inside while it is empty at the wide width.
fn search(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, width: f32, layout: Layout) {
    let searching = !st.search.trim().is_empty();
    let field = ui
        .scope(|ui| {
            ui.spacing_mut().interact_size.y = BAR;
            controls::search_input(ui, p, st, search_hint(prefs, layout == Layout::Compact), width)
        })
        .inner;
    if crate::ui::find_shortcut::take(ui, &[field.id]) {
        field.request_focus();
    }
    if layout == Layout::Wide && st.search.is_empty() && !field.has_focus() {
        let binding = crate::keybinds::Binding {
            input: crate::keybinds::Input::Key("KeyF".into()),
            ctrl: true,
            shift: false,
            alt: false,
        };
        let caps = widgets::keycap::caps(ui, &binding, p.field);
        let left = field.rect.right() - 8.0 - widgets::keycap::row_width(&caps);
        widgets::keycap::paint_row(ui, p, &caps, Pos2::new(left, field.rect.center().y), p.field);
    }
    controls::after_search(inv, st, searching);
}

/// The folder "New" creates in: the explorer's, else the selected folder or
/// the selected item's.
fn new_target(inv: &Inventory, st: &InventoryUi, layout: Layout) -> Uuid {
    if layout != Layout::Compact && st.tab == 0 {
        return st.browse;
    }
    st.selected
        .filter(|id| inv.folders.contains_key(id))
        .or_else(|| st.selected.and_then(|id| rules::parent(inv, id)))
        .unwrap_or_else(|| super::base(inv, st))
}

fn new_menu(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    layout: Layout,
    actions: &mut Vec<InvAction>,
) {
    let to = new_target(inv, st, layout);
    let enabled = st.pending.is_none() && rules::destination(inv, to, &prefs.protected);
    context::new_menu(ui, p, to, enabled, st, prefs, actions);
}

/// The "Sort" menu: the order, Collapse / Expand for the tree, what the
/// search searches, and the inventory's preferences.
fn sort_menu(ui: &mut egui::Ui, p: &Palette, st: &mut InventoryUi, prefs: &mut InventoryPreferences, layout: Layout) {
    menu::section(ui, p, aurora_i18n::tr!("inventory-sort-menu"));
    let sort = prefs.sort_for_mut(st.tab);
    if menu::check(
        ui,
        p,
        "clock-filled",
        aurora_i18n::tr!("ui-inventory-controls-newest-first"),
        sort.by_date,
        true,
    ) {
        sort.by_date = true;
    }
    if menu::check(
        ui,
        p,
        "sort-ascending-letters",
        aurora_i18n::tr!("common-by-name"),
        !sort.by_date,
        true,
    ) {
        sort.by_date = false;
    }
    menu::toggle(
        ui,
        p,
        "folder-filled",
        aurora_i18n::tr!("ui-inventory-controls-folders-always-by-name"),
        &mut sort.folders_by_name,
    );
    menu::toggle(
        ui,
        p,
        "stack-2-filled",
        aurora_i18n::tr!("ui-inventory-controls-system-folders-first"),
        &mut sort.system_first,
    );
    menu::separator(ui, p);
    if layout == Layout::Compact || st.result_view() {
        if menu::item(
            ui,
            p,
            "chevrons-up",
            aurora_i18n::tr!("ui-inventory-controls-collapse-displayed-folders"),
        ) {
            st.expand_all = Some(false);
        }
        if menu::item(
            ui,
            p,
            "chevrons-down",
            aurora_i18n::tr!("ui-inventory-controls-expand-displayed-folders"),
        ) {
            st.expand_all = Some(true);
        }
        menu::separator(ui, p);
    }
    menu::submenu(ui, p, "search", aurora_i18n::tr!("inventory-search-in"), true, |ui| {
        for (field, label) in [
            (SearchField::Name, aurora_i18n::tr!("common-name")),
            (SearchField::Description, aurora_i18n::tr!("ui-inventory-controls-description")),
            (SearchField::Creator, aurora_i18n::tr!("common-creator")),
            (SearchField::Uuid, "UUID"),
            (SearchField::All, aurora_i18n::tr!("ui-inventory-controls-all-fields")),
        ] {
            if menu::check(ui, p, "search", label, prefs.search_field == field, true) {
                prefs.search_field = field;
            }
        }
    });
    if menu::item(
        ui,
        p,
        "settings-filled",
        aurora_i18n::tr!("ui-inventory-controls-inventory-preferences"),
    ) {
        st.preferences_open = true;
    }
}

fn filters_tip(st: &InventoryUi) -> &'static str {
    if st.filters_open {
        aurora_i18n::tr!("inventory-filters-close")
    } else {
        aurora_i18n::tr!("ui-inventory-controls-inventory-filters")
    }
}

/// The main window's button for the additional inventory window, lit while
/// that window is open (the Legacy Viewer's `new_inv_btn`); the other
/// windows have none.
fn additional_button(ui: &mut egui::Ui, p: &Palette, st: &mut InventoryUi) {
    if !st.is_main() {
        return;
    }
    let button = tool_button(
        ui,
        p,
        ADDITIONAL_ICON,
        aurora_i18n::tr!("inventory-additional-window"),
        st.additional_open(),
    );
    additional_button_drawn(ui, button.rect);
    if button.clicked() {
        st.toggle_additional_window(ui.ctx());
    }
}

/// The room `additional_button` takes in a row whose buttons are `gap` apart.
fn additional_width(st: &InventoryUi, gap: f32) -> f32 {
    if st.is_main() { BAR + gap } else { 0.0 }
}

/// The narrow width's row: the search, the filters, the details card, the
/// rights, the additional window, Sort, New.
pub(super) fn search_row(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    layout: Layout,
    actions: &mut Vec<InvAction>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let cmt = rights::button_width(ui);
        let width = (ui.available_width() - 4.0 * (BAR + 6.0) - additional_width(st, 6.0) - cmt - 6.0).max(60.0);
        search(ui, p, inv, st, prefs, width, layout);
        let lit = st.filters_open || st.filters.active();
        if tool_button(ui, p, "adjustments-horizontal-filled", filters_tip(st), lit).clicked() {
            st.filters_open = !st.filters_open;
        }
        details_button(ui, p, prefs, layout);
        rights::button(ui, p, &mut prefs.rights, BAR);
        additional_button(ui, p, st);
        let sort = tool_button(ui, p, "arrows-sort", aurora_i18n::tr!("inventory-sort-menu"), false);
        menu::dropdown(&sort, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            sort_menu(ui, p, st, prefs, layout)
        });
        let new = tool_button(ui, p, "plus", aurora_i18n::tr!("inventory-new-menu"), false);
        menu::dropdown(&new, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            new_menu(ui, p, inv, st, prefs, layout, actions);
        });
    });
}

/// A button with an icon, a label and a count on the accent at its end
/// (the active filters).
fn counted_button(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, count: usize, lit: bool) -> egui::Response {
    let font = egui::FontId::proportional(12.5);
    let text = ui.painter().layout_no_wrap(label.to_owned(), font, p.ink);
    let number = (count > 0).then(|| ui.painter().layout_job(sheet::semibold(ui, &count.to_string(), 10.5, p.on_accent)));
    let badge_w = number.as_ref().map_or(0.0, |n| (n.size().x + 8.0).max(16.0) + 6.0);
    let size = Vec2::new(12.0 + 16.0 + 6.0 + text.size().x + badge_w + 12.0, BAR);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hot = response.hovered() || response.has_focus();
    let fill = if hot { p.raised.lerp_to_gamma(p.ink, 0.16) } else { p.raised };
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    let ink = if lit || hot { p.ink } else { p.ink.gamma_multiply(0.92) };
    widgets::paint_glyph(ui.painter(), icon, Pos2::new(rect.left() + 12.0 + 8.0, rect.center().y), 16.0, ink);
    let text_left = rect.left() + 12.0 + 16.0 + 6.0;
    widgets::paint_row_text(ui.painter(), text.clone(), text_left, rect.center().y, ink);
    if let Some(number) = number {
        let badge = Rect::from_min_size(
            Pos2::new(text_left + text.size().x + 6.0, rect.center().y - 8.0),
            Vec2::new((number.size().x + 8.0).max(16.0), 16.0),
        );
        ui.painter().rect_filled(badge, widgets::inner_radius(ui.visuals()), p.violet);
        widgets::paint_ink_centered(ui.painter(), number, badge.center(), p.on_accent);
    }
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    response
}

/// The explorer's toolbar. Medium: the search, the filters, the details
/// card, the rights, the columns, the additional window, Sort and New. Wide:
/// where it is, the search with its key, the filters with their count, the
/// details pane, the rights, the columns, the additional window and New.
pub(super) fn explorer_row(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    layout: Layout,
    actions: &mut Vec<InvAction>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let cmt = rights::button_width(ui);
        if layout == Layout::Wide {
            let crumbs = (ui.available_width() * 0.32).clamp(140.0, 380.0);
            ui.allocate_ui_with_layout(Vec2::new(crumbs, BAR), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                breadcrumb(ui, p, inv, st)
            });
            let new_label = aurora_i18n::tr!("inventory-new");
            let new_w = widgets::action_button_fit(ui, "plus", new_label);
            let filters_w = 150.0;
            let width =
                (ui.available_width() - (BAR + 8.0) - filters_w - (BAR + 8.0) - additional_width(st, 8.0) - cmt - 8.0 - new_w - 8.0 - 8.0)
                    .max(120.0);
            search(ui, p, inv, st, prefs, width, layout);
            let count = chip_list(st).len();
            let lit = st.filters_open || count > 0;
            if counted_button(
                ui,
                p,
                "adjustments-horizontal-filled",
                aurora_i18n::tr!("ui-inventory-controls-filters"),
                count,
                lit,
            )
            .on_hover_text(filters_tip(st))
            .clicked()
            {
                st.filters_open = !st.filters_open;
            }
            details_button(ui, p, prefs, layout);
            rights::button(ui, p, &mut prefs.rights, BAR);
            let columns = tool_button(ui, p, "layout-columns", aurora_i18n::tr!("inventory-columns"), false);
            menu::dropdown(&columns, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                super::columns::options(ui, p, &mut prefs.explorer, &mut prefs.rights);
            });
            additional_button(ui, p, st);
            let new = widgets::action_button(ui, p, "plus", new_label, widgets::ActionKind::Primary, Vec2::new(new_w, BAR));
            menu::dropdown(&new, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                new_menu(ui, p, inv, st, prefs, layout, actions);
            });
        } else {
            let width = (ui.available_width() - 4.0 * (BAR + 8.0) - additional_width(st, 8.0) - 112.0 - cmt - 8.0).max(120.0);
            search(ui, p, inv, st, prefs, width, layout);
            let count = chip_list(st).len();
            let lit = st.filters_open || count > 0;
            if counted_button(
                ui,
                p,
                "adjustments-horizontal-filled",
                aurora_i18n::tr!("ui-inventory-controls-filters"),
                count,
                lit,
            )
            .on_hover_text(filters_tip(st))
            .clicked()
            {
                st.filters_open = !st.filters_open;
            }
            details_button(ui, p, prefs, layout);
            rights::button(ui, p, &mut prefs.rights, BAR);
            let columns = tool_button(ui, p, "layout-columns", aurora_i18n::tr!("inventory-columns"), false);
            menu::dropdown(&columns, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                super::columns::options(ui, p, &mut prefs.explorer, &mut prefs.rights);
            });
            additional_button(ui, p, st);
            let sort = tool_button(ui, p, "arrows-sort", aurora_i18n::tr!("inventory-sort-menu"), false);
            menu::dropdown(&sort, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                sort_menu(ui, p, st, prefs, layout)
            });
            let new = widgets::action_button(ui, p, "plus", "", widgets::ActionKind::Primary, Vec2::splat(BAR))
                .on_hover_text(aurora_i18n::tr!("inventory-new-menu"));
            menu::dropdown(&new, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                new_menu(ui, p, inv, st, prefs, layout, actions);
            });
        }
    });
}

/// The name of the shortcut (tab) shown, for where the list is.
fn tab_name(tab: usize) -> &'static str {
    match tab {
        2 => aurora_i18n::tr!("inventory-tab-recent"),
        3 => aurora_i18n::tr!("inventory-tab-worn"),
        4 => aurora_i18n::tr!("inventory-tab-favorites"),
        RECEIVED_TAB => aurora_i18n::tr!("inventory-tab-received"),
        _ => aurora_i18n::tr!("common-my-inventory"),
    }
}

/// The folders from the root to the explorer's, each a link back to it
/// (the last one, where the list is, in semi-bold); a shortcut's name
/// instead while one is shown.
fn breadcrumb(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, st: &mut InventoryUi) {
    ui.spacing_mut().item_spacing.x = 4.0;
    if st.tab != 0 {
        ui.add(egui::Label::new(sheet::semibold(ui, tab_name(st.tab), 13.0, p.ink)).truncate());
        return;
    }
    let base = super::base(inv, st);
    let mut path = Vec::new();
    let mut id = st.browse;
    let mut seen = HashSet::new();
    while seen.insert(id) && inv.folders.contains_key(&id) {
        path.push(id);
        if id == inv.root || id == base {
            break;
        }
        id = rules::parent(inv, id).unwrap_or(Uuid::nil());
    }
    path.reverse();
    // A deep folder keeps its last levels in sight.
    let shown = path.len().min(4);
    if path.len() > shown {
        ui.label(egui::RichText::new("…").size(13.0).color(p.muted));
        widgets::glyph(ui, "chevron-right", p.muted_dim, 12.0);
    }
    let start = path.len() - shown;
    for (n, folder) in path[start..].iter().enumerate() {
        let last = start + n + 1 == path.len();
        let name = folder_name(inv, *folder);
        if last {
            ui.add(egui::Label::new(sheet::semibold(ui, &name, 13.0, p.ink)).truncate());
        } else {
            let link = ui
                .add(egui::Label::new(egui::RichText::new(&name).size(13.0).color(p.muted)).sense(Sense::click()))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if link.clicked() {
                st.browse = *folder;
                st.selection.clear();
                st.selected = None;
            }
            widgets::glyph(ui, "chevron-right", p.muted_dim, 12.0);
        }
    }
}

/// The medium width's line over the list: where it is, how much it holds.
pub(super) fn breadcrumb_row(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, st: &mut InventoryUi) {
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), 26.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let count = (st.tab == 0)
                .then(|| inv.folders.get(&st.browse))
                .flatten()
                .filter(|f| f.state == FetchState::Fetched)
                .map(|f| f.items.len() + f.children.len());
            let count_w = 110.0;
            ui.allocate_ui_with_layout(
                Vec2::new((ui.available_width() - count_w).max(40.0), 26.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| breadcrumb(ui, p, inv, st),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(count) = count {
                    ui.label(
                        egui::RichText::new(aurora_i18n::tr!(
                            "inventory-items-count",
                            count = count,
                            number = aurora_i18n::integer(count as i64)
                        ))
                        .size(12.0)
                        .color(p.muted),
                    );
                }
            });
        },
    );
}

/// One active filter: its text, and how to take it off.
pub(super) struct Chip {
    pub text: String,
    remove: fn(&mut InventoryUi),
}

fn type_names(types_mask: u32) -> String {
    let names: Vec<&str> = types()
        .iter()
        .filter(|(kind, _)| types_mask & (1_u32 << kind) != 0)
        .map(|(_, name)| *name)
        .collect();
    if names.len() > 3 {
        aurora_i18n::tr!("inventory-chip-types", count = names.len())
    } else {
        names.join(", ")
    }
}

/// The active filters, as the chips show them.
pub(super) fn chip_list(st: &InventoryUi) -> Vec<Chip> {
    let f = &st.filters;
    let mut chips = Vec::new();
    if st.links_filter.is_some() {
        chips.push(Chip {
            text: aurora_i18n::tr!("inventory-chip-links-of").to_owned(),
            remove: |st| {
                st.links_filter = None;
                if let Some(filters) = st.saved_filters.take() {
                    st.filters = filters;
                }
            },
        });
        return chips;
    }
    if f.types != u32::MAX {
        chips.push(Chip {
            text: aurora_i18n::tr!("inventory-chip-type", types = type_names(f.types)),
            remove: |st| st.filters.types = u32::MAX,
        });
    }
    for (bit, label, remove) in [
        (
            rules::MODIFY,
            aurora_i18n::tr!("inventory-chip-modify"),
            (|st: &mut InventoryUi| st.filters.permissions &= !rules::MODIFY) as fn(&mut InventoryUi),
        ),
        (rules::COPY, aurora_i18n::tr!("inventory-chip-copy"), |st| {
            st.filters.permissions &= !rules::COPY
        }),
        (rules::TRANSFER, aurora_i18n::tr!("inventory-chip-transfer"), |st| {
            st.filters.permissions &= !rules::TRANSFER
        }),
    ] {
        if f.permissions & bit != 0 {
            chips.push(Chip {
                text: label.to_owned(),
                remove,
            });
        }
    }
    match f.links {
        Links::Include => {}
        Links::Only => chips.push(Chip {
            text: aurora_i18n::tr!("ui-inventory-controls-links-only").to_owned(),
            remove: |st| st.filters.links = Links::Include,
        }),
        Links::Exclude => chips.push(Chip {
            text: aurora_i18n::tr!("ui-inventory-controls-hide-links").to_owned(),
            remove: |st| st.filters.links = Links::Include,
        }),
    }
    match f.creator {
        Creator::All => {}
        Creator::Me => chips.push(Chip {
            text: aurora_i18n::tr!("ui-inventory-controls-created-by-me").to_owned(),
            remove: |st| st.filters.creator = Creator::All,
        }),
        Creator::Others => chips.push(Chip {
            text: aurora_i18n::tr!("ui-inventory-controls-created-by-others").to_owned(),
            remove: |st| st.filters.creator = Creator::All,
        }),
    }
    let age = match f.age {
        Age::Any => None,
        Age::SinceLogout => Some(aurora_i18n::tr!("ui-inventory-controls-since-last-logout").to_owned()),
        Age::Newer => Some(aurora_i18n::tr!("inventory-chip-newer", hours = f.hours)),
        Age::Older => Some(aurora_i18n::tr!("inventory-chip-older", hours = f.hours)),
    };
    if let Some(text) = age {
        chips.push(Chip {
            text,
            remove: |st| st.filters.age = Age::Any,
        });
    }
    if f.always_folders {
        chips.push(Chip {
            text: aurora_i18n::tr!("inventory-chip-all-folders").to_owned(),
            remove: |st| st.filters.always_folders = false,
        });
    }
    if f.coalesced {
        chips.push(Chip {
            text: aurora_i18n::tr!("ui-inventory-controls-coalesced-objects-only").to_owned(),
            remove: |st| st.filters.coalesced = false,
        });
    }
    chips
}

/// A chip: its text, and a cross that takes the filter off.
fn chip(ui: &mut egui::Ui, p: &Palette, text: &str) -> bool {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), egui::FontId::proportional(12.0), p.ink);
    let size = Vec2::new(10.0 + galley.size().x + 6.0 + 14.0 + 6.0, 24.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hot = response.hovered();
    let fill = if hot { p.raised.lerp_to_gamma(p.ink, 0.16) } else { p.raised };
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
    widgets::paint_row_text(ui.painter(), galley, rect.left() + 10.0, rect.center().y, p.ink);
    widgets::paint_glyph(
        ui.painter(),
        "x",
        Pos2::new(rect.right() - 6.0 - 7.0, rect.center().y),
        13.0,
        if hot { p.ink } else { p.muted },
    );
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let tip = aurora_i18n::tr!("inventory-chip-remove");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    response.on_hover_text(tip).clicked()
}

/// The text of the sort order, at the right end of the chips.
fn sort_text(prefs: &InventoryPreferences, tab: usize) -> String {
    let sort = prefs.sort_for(tab);
    let order = if sort.by_date {
        aurora_i18n::tr!("inventory-sort-date")
    } else {
        aurora_i18n::tr!("inventory-sort-name")
    };
    if sort.system_first {
        aurora_i18n::tr!("inventory-sort-folders-first", order = order)
    } else {
        order.to_owned()
    }
}

/// The wide width's chips: one per active filter, "Add a filter", and the
/// sort order, which opens the Sort menu.
pub(super) fn chips(ui: &mut egui::Ui, p: &Palette, st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), 32.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let sort = sort_text(prefs, st.tab);
            let sort_w = ui
                .painter()
                .layout_no_wrap(sort.clone(), egui::FontId::proportional(12.0), p.muted)
                .size()
                .x
                + 22.0;
            let mut removed = None;
            let room = ui.available_width() - sort_w - 8.0;
            ui.allocate_ui_with_layout(
                Vec2::new(room.max(40.0), 32.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                    for chip_entry in chip_list(st) {
                        if chip(ui, p, &chip_entry.text) {
                            removed = Some(chip_entry.remove);
                        }
                    }
                    let add = ui
                        .add(
                            egui::Label::new(
                                egui::RichText::new(format!("+ {}", aurora_i18n::tr!("inventory-chip-add")))
                                    .size(12.0)
                                    .color(p.violet_light),
                            )
                            .sense(Sense::click()),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if add.clicked() {
                        st.filters_open = true;
                    }
                },
            );
            if let Some(remove) = removed {
                remove(st);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (rect, response) = ui.allocate_exact_size(Vec2::new(sort_w, 24.0), Sense::click());
                let hot = response.hovered();
                let ink = if hot { p.ink } else { p.muted };
                widgets::paint_glyph(
                    ui.painter(),
                    "arrows-sort",
                    Pos2::new(rect.left() + 7.0, rect.center().y),
                    13.0,
                    ink,
                );
                let galley = ui.painter().layout_no_wrap(sort, egui::FontId::proportional(12.0), ink);
                widgets::paint_row_text(ui.painter(), galley, rect.left() + 18.0, rect.center().y, ink);
                if hot {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let response = response.on_hover_text(aurora_i18n::tr!("inventory-sort-menu"));
                menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    sort_menu(ui, p, st, prefs, Layout::Wide)
                });
            });
        },
    );
}

/// The message to show: a refused change, an operation's progress, its
/// end, or the folders still loading for a search.
pub(super) fn message(p: &Palette, st: &InventoryUi) -> Option<(String, Color32)> {
    let now = std::time::Instant::now();
    if !st.message.is_empty() {
        Some((st.message.clone(), p.warn))
    } else if st.busy() {
        Some((st.pending_status(now), p.ink))
    } else if let Some(message) = st.success_notice(now) {
        Some((message.to_owned(), p.success))
    } else if st.fetch_all {
        Some((aurora_i18n::tr!("ui-inventory-search-loading-folders").to_owned(), p.muted))
    } else {
        None
    }
}

/// The line of a message, with the spinner of an operation.
pub(super) fn message_line(ui: &mut egui::Ui, st: &InventoryUi, text: &str, color: Color32) {
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), MESSAGE),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            if st.busy() {
                ui.add(egui::Spinner::new().size(13.0).color(color));
            }
            tree::status_text(ui, text, color).on_hover_text(text);
        },
    );
}

/// The wide width's status bar: what the list holds and what is selected,
/// or a message; the hint about wearing at its right end.
pub(super) fn status_bar(ui: &mut egui::Ui, p: &Palette, st: &InventoryUi, explore: bool, message: Option<(String, Color32)>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), STATUS), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.top() + 0.5, widgets::divider_stroke(p));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(8.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let ui = &mut child;
    match message {
        Some((text, color)) => {
            if st.busy() {
                ui.add(egui::Spinner::new().size(13.0).color(color));
            }
            tree::status_text(ui, &text, color).on_hover_text(text);
        }
        None => {
            let listed = if explore { st.flat.order.len() } else { st.result_tree.order.len() };
            let text = aurora_i18n::tr!(
                "inventory-status-counts",
                count = listed,
                number = aurora_i18n::integer(listed as i64),
                selected = st.selection.len()
            );
            tree::status_text(ui, &text, p.muted);
        }
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        tree::status_text(ui, aurora_i18n::tr!("inventory-status-hint"), p.muted);
    });
}

#[cfg(test)]
mod tests {
    use super::super::super::view::Filters;
    use super::*;

    #[test]
    fn restored_menu_feedback_uses_the_success_color_and_disappears_after_expiry() {
        let palette = crate::theme::Theme::default().palette();
        let mut st = InventoryUi::default();
        let mut prefs = InventoryPreferences::default();
        crate::ui::inventory::context_notecards::demo_restore(&mut st, &mut prefs);
        assert_eq!(
            message(&palette, &st),
            Some((aurora_i18n::tr!("inventory-menu-loaded").into(), palette.success))
        );
        st.notice.as_mut().unwrap().1 -= std::time::Duration::from_secs(5);
        assert!(message(&palette, &st).is_none());
    }

    #[test]
    fn received_sort_controls_edit_only_the_active_order_in_both_layouts() {
        fn label_rect(shape: &egui::Shape, label: &str) -> Option<Rect> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label => Some(Rect::from_min_size(text.pos, text.galley.size())),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| label_rect(shape, label)),
                _ => None,
            }
        }
        for classic in [false, true] {
            let ctx = egui::Context::default();
            let palette = crate::theme::Theme::default().palette();
            let mut prefs = InventoryPreferences::default();
            let mut frame_index = 0;
            let mut frame = |tab, prefs: &mut InventoryPreferences, events| {
                frame_index += 1;
                headless_output(ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 1000.0))),
                        time: Some(f64::from(frame_index) / 60.0),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        if classic {
                            controls::preferences_for_tab(ui, &palette, prefs, tab);
                        } else {
                            let mut st = InventoryUi { tab, ..Default::default() };
                            sort_menu(ui, &palette, &mut st, prefs, Layout::Wide);
                        }
                    },
                ))
            };
            for tab in [RECEIVED_TAB, 0] {
                let other_tab = if tab == RECEIVED_TAB { 0 } else { RECEIVED_TAB };
                let other_order = prefs.sort_for(other_tab);
                for (label, switch) in [
                    (aurora_i18n::tr!("common-by-name"), false),
                    (aurora_i18n::tr!("ui-inventory-controls-folders-always-by-name"), true),
                    (aurora_i18n::tr!("ui-inventory-controls-system-folders-first"), true),
                ] {
                    drop(frame(tab, &mut prefs, vec![]));
                    let output = frame(tab, &mut prefs, vec![]);
                    let rect = output
                        .shapes
                        .iter()
                        .find_map(|shape| label_rect(&shape.shape, label))
                        .expect("sort control drawn");
                    let pos = if classic && switch {
                        egui::pos2(
                            rect.left() - egui::Style::default().spacing.item_spacing.x - widgets::SWITCH_SIZE.x / 2.0,
                            rect.center().y,
                        )
                    } else {
                        rect.center()
                    };
                    for pressed in [true, false] {
                        drop(frame(
                            tab,
                            &mut prefs,
                            vec![
                                egui::Event::PointerMoved(pos),
                                egui::Event::PointerButton {
                                    pos,
                                    button: egui::PointerButton::Primary,
                                    pressed,
                                    modifiers: Default::default(),
                                },
                            ],
                        ));
                    }
                    assert_eq!(
                        prefs.sort_for(other_tab),
                        other_order,
                        "changing {label} leaves the other tab intact"
                    );
                }
                let order = prefs.sort_for(tab);
                assert!(!order.by_date);
                assert_eq!(order.folders_by_name, tab == RECEIVED_TAB);
                assert!(!order.system_first);
                assert_eq!(sort_text(&prefs, tab), aurora_i18n::tr!("inventory-sort-name"));
            }
        }
    }

    #[test]
    fn each_active_filter_has_a_chip_that_takes_it_off() {
        let mut st = InventoryUi::default();
        assert!(chip_list(&st).is_empty());
        st.filters = Filters {
            types: 1 << 6,
            permissions: rules::MODIFY | rules::COPY,
            links: Links::Exclude,
            creator: Creator::Me,
            age: Age::SinceLogout,
            always_folders: true,
            coalesced: true,
            ..Filters::default()
        };
        let chips = chip_list(&st);
        assert_eq!(chips.len(), 8);
        for chip in chips {
            (chip.remove)(&mut st);
        }
        assert!(!st.filters.active(), "every chip removed its filter");
    }

    #[test]
    fn the_links_mode_is_one_chip_that_restores_the_filters() {
        let mut st = InventoryUi {
            saved_filters: Some(Filters {
                coalesced: true,
                ..Filters::default()
            }),
            links_filter: Some(Uuid::from_u128(5)),
            ..Default::default()
        };
        st.filters.links = Links::Only;
        let chips = chip_list(&st);
        assert_eq!(chips.len(), 1);
        (chips[0].remove)(&mut st);
        assert!(st.links_filter.is_none());
        assert!(st.filters.coalesced, "the filters from before the links mode are back");
    }

    #[test]
    fn new_items_go_to_the_explorer_folder_or_the_selection() {
        let mut inv = Inventory {
            root: Uuid::from_u128(1),
            ..Default::default()
        };
        for (n, parent) in [(1_u128, 0_u128), (2, 1), (3, 1)] {
            inv.folders.insert(
                Uuid::from_u128(n),
                crate::world::inventory::Folder {
                    info: aurora_net::inventory::InvFolder {
                        id: Uuid::from_u128(n),
                        parent: Uuid::from_u128(parent),
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: Vec::new(),
                    state: FetchState::Fetched,
                    library: false,
                },
            );
        }
        let mut st = InventoryUi {
            browse: Uuid::from_u128(2),
            selected: Some(Uuid::from_u128(3)),
            ..Default::default()
        };
        assert_eq!(new_target(&inv, &st, Layout::Wide), Uuid::from_u128(2));
        assert_eq!(new_target(&inv, &st, Layout::Compact), Uuid::from_u128(3));
        st.selected = None;
        assert_eq!(new_target(&inv, &st, Layout::Compact), inv.root);
    }
}
