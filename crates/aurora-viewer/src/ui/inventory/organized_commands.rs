//! The full-width Explorer command bar. The same inventory operations back the
//! toolbar, keyboard and context menus; the overflow also works in detached panes.
use super::file_actions::{Edit, Menu, Request};
use super::*;
use crate::ui::menu;
use aurora_i18n::tr;
use egui::{Button, Image, Vec2};

const EDITS: [Edit; 4] = [Edit::Cut, Edit::Copy, Edit::Rename, Edit::Delete];

fn edit_label(edit: Edit) -> (&'static str, &'static str, &'static str) {
    match edit {
        Edit::Cut => ("cut", tr!("common-cut"), "✂"),
        Edit::Copy => ("copy-filled", tr!("common-copy"), "⧉"),
        Edit::Rename => ("pencil-filled", tr!("common-rename"), "✎"),
        Edit::Delete => ("trash-filled", tr!("common-delete"), "×"),
    }
}

const FOLDERS: i32 = 8;
const TOOL_WIDTH: f32 = 26.0;
const TOOL_HEIGHT: f32 = 28.0;
const TOOL_GAP: f32 = 4.0;

pub(super) fn matches_filter(inv: &Inventory, row: Row, kind: i32) -> bool {
    let Row::Item(id) = row else {
        return kind == FOLDERS;
    };
    // A folder link keeps its type even before its target is fetched.
    if inv.items.get(&id).is_some_and(|item| item.asset_type == 25) {
        return kind == FOLDERS;
    }
    let target = rules::original(inv, id).unwrap_or(id);
    if inv.folders.contains_key(&target) {
        return kind == FOLDERS;
    }
    inv.items.get(&target).is_some_and(|item| {
        // The Legacy filters treat attachments as objects.
        (if item.inv_type == 17 { 6 } else { item.inv_type }) == kind
    })
}

fn filter_options(ui: &mut Ui, state: &mut ExplorerState) {
    let mut filter = state.here.type_filter;
    let options = [(None, tr!("common-all")), (Some(FOLDERS), tr!("inventory-folders"))];
    for (value, label) in options
        .into_iter()
        .chain(super::super::view::types().iter().map(|&(k, label)| (Some(k), label)))
    {
        if ui.selectable_value(&mut filter, value, label).clicked() {
            ui.close();
        }
    }
    if filter != state.here.type_filter {
        state.here.type_filter = filter;
        state.rows_key = None;
        state.here.scroll = 0.0;
        state.restore_scroll = true;
    }
}

#[derive(Clone)]
pub(super) struct EditAvailability {
    generation: u64,
    layout: u64,
    protected: HashSet<Uuid>,
    worn: HashSet<Uuid>,
    active_gestures: std::collections::BTreeSet<Uuid>,
    // Immutable world snapshots are shared until lock inputs change.
    rlva: std::sync::Arc<crate::world::rlva::WearLocks>,
    allowed: [bool; 4],
}

// Large selections can contain tens of thousands of records. Do the native
// permission walk once per selection or data change, never on every repaint.
fn enabled_edits(inv: &Inventory, layout: &Preferences, state: &mut ExplorerState, files: &Menu) -> [bool; 4] {
    if state.here.selected.is_empty() || files.native.pending.is_some() {
        return [false; 4];
    }
    if let Some(c) = &state.here.edit_availability
        && c.generation == inv.generation
        && c.layout == layout.revision()
        && c.protected == files.prefs.protected
        && c.worn == files.facts.worn
        && c.active_gestures == files.facts.active_gestures
        && std::sync::Arc::ptr_eq(&c.rlva, &files.facts.rlva)
    {
        return c.allowed;
    }
    let ids = if state.dashboard_shortcuts(inv, layout) {
        None
    } else {
        file_actions::ids(state)
    };
    let allowed = ids.as_deref().map_or([false; 4], |ids| {
        let guarded = ids.iter().any(|id| model::client_managed(inv, *id));
        EDITS.map(|edit| edit.allowed(inv, ids, files, guarded))
    });
    state.here.edit_availability = Some(EditAvailability {
        generation: inv.generation,
        layout: layout.revision(),
        protected: files.prefs.protected.clone(),
        worn: files.facts.worn.clone(),
        active_gestures: files.facts.active_gestures.clone(),
        rlva: files.facts.rlva.clone(),
        allowed,
    });
    allowed
}

// Checking paste can walk a folder hierarchy and all of its descendants. Keep
// this result on the tab, independent of navigation history; a change to any
// of the native permission inputs invalidates it.
pub(super) struct PasteAvailability {
    root: Uuid,
    generation: u64,
    layout: u64,
    into: Location,
    ids: Vec<Uuid>,
    cut: bool,
    protected: HashSet<Uuid>,
    worn: HashSet<Uuid>,
    allowed: bool,
}

fn paste_allowed(inv: &Inventory, layout: &Preferences, state: &mut ExplorerState, files: &Menu, into: Location) -> bool {
    if files.native.clipboard.is_empty() || files.native.pending.is_some() || state.dashboard_shortcuts(inv, layout) {
        state.bar_paste = None;
        return false;
    }
    if let Some(c) = &state.bar_paste
        && c.root == inv.root
        && c.generation == inv.generation
        && c.layout == layout.revision()
        && c.into == into
        && c.cut == files.native.cut
        && c.ids == files.native.clipboard
        && c.protected == files.prefs.protected
        && c.worn == files.facts.worn
    {
        return c.allowed;
    }
    let allowed = file_actions::paste_enabled(files, inv, layout, into, false);
    state.bar_paste = Some(PasteAvailability {
        root: inv.root,
        generation: inv.generation,
        layout: layout.revision(),
        into,
        ids: files.native.clipboard.clone(),
        cut: files.native.cut,
        protected: files.prefs.protected.clone(),
        worn: files.facts.worn.clone(),
        allowed,
    });
    allowed
}

fn button(ui: &Ui, icon: &str, fallback: &'static str, label: Option<&'static str>, color: Color32) -> Button<'static> {
    let image = crate::ui::icons::global(icon).map(|texture| Image::new((texture.id(), Vec2::splat(15.0))).tint(color));
    match (image, label) {
        (Some(image), Some(label)) => Button::image_and_text(image, label),
        (Some(image), None) => Button::image(image),
        (None, Some(label)) => Button::new(label),
        (None, None) => Button::new(super::item_art::glyph_image(
            ui,
            fallback,
            icon,
            false,
            crate::ui::widgets::PICTURE_TINT,
        ))
        .image_tint_follows_text_color(true),
    }
    .frame_when_inactive(false)
    .min_size(vec2(TOOL_WIDTH, TOOL_HEIGHT))
}

fn tool(ui: &mut Ui, p: &Palette, icon: &str, label: &'static str, fallback: &'static str, enabled: bool) -> bool {
    let b = button(ui, icon, fallback, None, if enabled { p.ink } else { p.muted_dim });
    let r = ui.add_enabled(enabled, b);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    r.on_hover_text(label).on_disabled_hover_text(label).clicked()
}

fn dropdown(ui: &mut Ui, p: &Palette, icon: &str, label: &'static str, enabled: bool, active: bool, body: impl FnOnce(&mut Ui)) {
    let ink = if active {
        p.violet_light
    } else if enabled {
        p.ink
    } else {
        p.muted_dim
    };
    let b = button(ui, icon, label, Some(label), ink).selected(active);
    let b = if let Some(arrow) = crate::ui::icons::global("chevron-down") {
        b.right_text(Image::new((arrow.id(), Vec2::splat(10.0))).tint(ink))
    } else {
        b.right_text(super::item_art::glyph_image(
            ui,
            "▾",
            "chevron-down",
            true,
            crate::ui::widgets::PICTURE_TINT,
        ))
        .image_tint_follows_text_color(true)
    };
    ui.add_enabled_ui(enabled, |ui| {
        egui::containers::menu::MenuButton::from_button(b).ui(ui, body);
    });
}

fn menu_row(ui: &mut Ui, p: &Palette, icon: &str, label: &'static str, enabled: bool, overflow: bool, fallback: &'static str) -> bool {
    if overflow {
        menu::item_if(ui, p, icon, label, enabled)
    } else {
        tool(ui, p, icon, label, fallback, enabled)
    }
}

fn edit_buttons(ui: &mut Ui, p: &Palette, inv: &Inventory, layout: &Preferences, state: &mut ExplorerState, files: &Menu, overflow: bool) {
    let allowed = enabled_edits(inv, layout, state, files);
    for (i, edit) in EDITS.into_iter().enumerate() {
        if edit == Edit::Rename {
            let location = match (state.here.selected.len(), state.here.selected.iter().next()) {
                (1, Some(Row::Place(place))) => *place,
                _ => state.here.location,
            };
            let can_paste = paste_allowed(inv, layout, state, files, location);
            if menu_row(ui, p, "clipboard-text-filled", tr!("common-paste"), can_paste, overflow, "▤") {
                state.file_request = Some(Request::Paste(location, false));
            }
        }
        let (icon, label, fallback) = edit_label(edit);
        if menu_row(ui, p, icon, label, allowed[i], overflow, fallback)
            && let Some(ids) = file_actions::ids(state)
        {
            state.file_request = Some(edit.request(ids));
        }
    }
}

fn presentation(ui: &mut Ui, p: &Palette, inv: &Inventory, layout: &mut Preferences, state: &mut ExplorerState, overflow: bool) {
    let filterable = !state.on_dashboard();
    if overflow {
        menu::submenu(ui, p, "arrows-sort", tr!("inventory-sort-menu"), true, |ui| {
            views::sort_options(ui, layout, state)
        });
        menu::submenu(ui, p, "layout-grid-filled", tr!("explorer-mode-view"), true, |ui| {
            views::view_options_menu(ui, p, inv, layout, state)
        });
        menu::submenu(ui, p, "filter-filled", tr!("ui-people-filter"), filterable, |ui| {
            filter_options(ui, state)
        });
    } else {
        dropdown(ui, p, "arrows-sort", tr!("inventory-sort-menu"), true, false, |ui| {
            views::sort_options(ui, layout, state)
        });
        dropdown(ui, p, "layout-grid-filled", tr!("explorer-mode-view"), true, false, |ui| {
            views::view_options_menu(ui, p, inv, layout, state)
        });
        dropdown(
            ui,
            p,
            "filter-filled",
            tr!("ui-people-filter"),
            filterable,
            state.here.type_filter.is_some(),
            |ui| filter_options(ui, state),
        );
    }
}

/// Measure translated labels in the active font; at small sizes groups move to
/// More in order rather than overflowing the pane or truncating command names.
fn label_width(ui: &Ui, label: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(label.into(), egui::TextStyle::Button.resolve(ui.style()), ui.visuals().text_color())
        .size()
        .x
}

pub(super) fn bar(
    ui: &mut Ui,
    p: &Palette,
    inv: &Inventory,
    layout: &mut Preferences,
    state: &mut ExplorerState,
    avatar: &str,
    files: &Menu,
    open: &mut Option<Row>,
) {
    let width = ui.available_width();
    let place = state.here.location;
    if state.on_dashboard() {
        state.clear_filter();
    }
    let available = layout.available(inv, place);
    let height = TOOL_HEIGHT
        .max(ui.spacing().interact_size.y)
        .max(ui.text_style_height(&egui::TextStyle::Button) + 2.0 * ui.spacing().button_padding.y);
    ui.allocate_ui_with_layout(vec2(width, height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = TOOL_GAP;
        let new_label = tr!("inventory-new");
        let edit_width = 5.0 * (TOOL_WIDTH + TOOL_GAP) + 12.0;
        let dropdown_size = |name: &str| label_width(ui, name) + 48.0;
        let base = dropdown_size(new_label) + 44.0;
        let display_width = [tr!("inventory-sort-menu"), tr!("explorer-mode-view"), tr!("ui-people-filter")]
            .into_iter()
            .map(dropdown_size)
            .sum::<f32>();
        let full_display = width >= base + display_width;
        let full_edit = width >= base + display_width + edit_width;
        dropdown(
            ui,
            p,
            "plus",
            new_label,
            available && folder::available(inv, place) && files.native.pending.is_none(),
            false,
            |ui| {
                menus::new_options(ui, p, inv, layout, state, files, place);
            },
        );
        if full_edit {
            ui.add_space(4.0);
            edit_buttons(ui, p, inv, layout, state, files, false);
        }
        if full_display {
            ui.add_space(4.0);
            presentation(ui, p, inv, layout, state, false);
        }
        let (more, _) = egui::containers::menu::MenuButton::from_button(
            Button::new(super::item_art::glyph_image(
                ui,
                "…",
                "dots-filled",
                false,
                crate::ui::widgets::PICTURE_TINT,
            ))
            .image_tint_follows_text_color(true)
            .frame_when_inactive(false)
            .min_size(vec2(TOOL_HEIGHT, TOOL_HEIGHT)),
        )
        .ui(ui, |ui| {
            if !full_edit {
                edit_buttons(ui, p, inv, layout, state, files, true);
                ui.add_space(4.0);
            }
            if !full_display {
                presentation(ui, p, inv, layout, state, true);
                ui.add_space(4.0);
            }
            if let Some(row) = state
                .anchor
                .filter(|r| state.here.selected.contains(r))
                .or_else(|| state.here.selected.iter().copied().next())
            {
                menus::selected(ui, p, inv, layout, state, avatar, row, open, files, false);
            } else {
                menus::background(ui, p, inv, layout, state, avatar, files, false);
            }
        });
        more.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr!("inventory-more-actions")));
        more.on_hover_text(tr!("inventory-more-actions"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn explorer_edit_cache_tracks_gestures_and_locks_without_inventory_changes() {
        let inv = super::super::tests::fixture();
        let layout = Preferences::default();
        let mut state = ExplorerState::at(Location::Folder(Uuid::from_u128(3)));
        let item = Uuid::from_u128(20);
        state.here.selected.insert(Row::Item(item));
        let native = InventoryUi::default();
        let prefs = InventoryPreferences::default();
        let mut facts = Facts::default();
        let enabled = |state: &mut ExplorerState, facts: &Facts| {
            enabled_edits(
                &inv,
                &layout,
                state,
                &Menu {
                    native: &native,
                    prefs: &prefs,
                    facts,
                },
            )
        };
        let generation = inv.generation;
        assert!(enabled(&mut state, &facts)[0]);
        assert!(enabled(&mut state, &facts)[3]);
        let snapshot = state.here.edit_availability.as_ref().unwrap().rlva.clone();
        enabled(&mut state, &facts);
        assert!(Arc::ptr_eq(&snapshot, &state.here.edit_availability.as_ref().unwrap().rlva));
        facts.active_gestures.insert(item);
        assert!(!enabled(&mut state, &facts)[3]);
        facts.active_gestures.clear();
        assert!(enabled(&mut state, &facts)[3]);
        facts.rlva = Arc::new(crate::world::rlva::WearLocks::holding_folders([inv.items[&item].parent]));
        let locked = enabled(&mut state, &facts);
        assert!(!locked[0] && !locked[3]);
        facts.rlva = Arc::default();
        let unlocked = enabled(&mut state, &facts);
        assert!(unlocked[0] && unlocked[3]);
        assert_eq!(inv.generation, generation);
    }
}
