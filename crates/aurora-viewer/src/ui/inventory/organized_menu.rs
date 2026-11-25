//! Explorer menus share the selected records, enablement and creation commands with Legacy.
use super::super::{
    context_catalog as native,
    menu_layout::{Kind, children},
};
use super::file_actions::{Menu, Request};
use super::*;
use crate::ui::menu;
use aurora_i18n::tr;

/// Keep the native menu geometry and input handling. egui passes this style to
/// the child menu config, so flyouts also get their own edge and shadow.
pub(crate) fn context_menu(response: &Response, palette: &Palette, contents: impl FnOnce(&mut Ui)) {
    context_menu_behavior(response, palette, egui::PopupCloseBehavior::CloseOnClick, contents);
}

pub(super) fn context_menu_behavior(
    response: &Response,
    palette: &Palette,
    close: egui::PopupCloseBehavior,
    contents: impl FnOnce(&mut Ui),
) {
    // Shared menu framing supplies glass, theme radii and dismissal behavior.
    // Native rows close explicitly; pin toggles deliberately keep the popup open.
    let _ = close;
    menu::context_menu(response, palette, contents);
}

pub(super) struct NewDestination {
    root: Uuid,
    origin: Location,
    key: String,
    selected: Option<Uuid>,
}

fn catalog(files: &Menu, inv: &Inventory, id: Uuid, selected: Vec<Uuid>) -> native::Catalog {
    native::build_for_selection(inv, id, files.native, files.prefs, files.facts, selected, false, false)
}

fn entry(c: &native::Catalog, key: &str) -> Option<(usize, bool)> {
    let e = c.entries.iter().find(|e| e.id == key)?;
    if let Kind::Action(index) = e.kind {
        Some((index, e.enabled))
    } else {
        None
    }
}

fn take(c: &mut native::Catalog, key: &str) -> Option<native::Command> {
    let (index, enabled) = entry(c, key)?;
    if !enabled {
        return None;
    }
    c.commands[index].take()
}

fn choose(state: &mut ExplorerState, c: &mut native::Catalog, key: &str, selected: &[Uuid]) {
    if let Some(command) = take(c, key) {
        state.file_request = Some(match command {
            native::Command::Move(_) => Request::Move(selected.to_vec()),
            native::Command::Original(target) => Request::Reveal(target),
            cmd => Request::Native(cmd),
        });
    }
}

fn action(ui: &mut Ui, p: &Palette, state: &mut ExplorerState, c: &mut native::Catalog, key: &str, selected: &[Uuid], writable: bool) {
    let Some(e) = c.entries.iter().find(|e| e.id == key) else { return };
    let (label, tip) = match key {
        "move" => (tr!("explorer-real-move"), tr!("explorer-real-move-hint")),
        "purge" => (e.label.as_str(), tr!("ui-inventory-dialogs-this-deletion-is-permanent")),
        "apply-parcel" if !e.enabled => (e.label.as_str(), tr!("explorer-set-parcel-unavailable")),
        _ => (e.label.as_str(), ""),
    };
    if menu::item_tip(ui, p, &e.icon, label, writable && e.enabled, tip) {
        choose(state, c, key, selected);
    }
}

fn quick_button(ui: &mut Ui, icon: &str, label: &str, enabled: bool, tip: &str) -> bool {
    let font = FontId::proportional(11.0);
    let tint = if enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let caption = ui.painter().layout_no_wrap(label.to_owned(), font, tint);
    let (rect, r) = ui.allocate_exact_size(
        vec2((caption.size().x + 8.0).max(40.0), 42.0),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    if enabled && r.hovered() {
        ui.painter().rect_filled(
            rect,
            ui.visuals().widgets.inactive.corner_radius,
            ui.visuals().widgets.hovered.weak_bg_fill,
        );
    }
    if let Some(handle) = crate::ui::icons::global(icon) {
        ui.painter().image(
            handle.id(),
            Rect::from_center_size(pos2(rect.center().x, rect.top() + 11.0), vec2(16.0, 16.0)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            tint,
        );
    }
    ui.painter().galley(
        pos2(rect.center().x - caption.size().x * 0.5, rect.bottom() - caption.size().y - 2.0),
        caption,
        tint,
    );
    if r.on_hover_text(tip).clicked() && enabled {
        ui.close();
        return true;
    }
    false
}

fn quick_actions(ui: &mut Ui, c: &mut native::Catalog, state: &mut ExplorerState, selected: &[Uuid], guarded: bool, can_delete: bool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (key, label) in [
            ("cut", tr!("common-cut")),
            ("copy", tr!("common-copy")),
            ("rename", tr!("common-rename")),
            ("share", tr!("common-share")),
            ("delete", tr!("common-delete")),
        ] {
            let Some(e) = c.entries.iter().find(|e| e.id == key) else {
                continue;
            };
            let permitted = (!guarded || !matches!(key, "cut" | "rename" | "share")) && (key != "delete" || can_delete);
            if quick_button(
                ui,
                &e.icon,
                label,
                permitted && e.enabled,
                if key == "delete" { tr!("explorer-real-trash") } else { &e.label },
            ) {
                choose(state, c, key, selected);
            }
        }
    });
}

fn copy_path(
    ui: &mut Ui,
    p: &Palette,
    inv: &Inventory,
    layout: &Preferences,
    state: &mut ExplorerState,
    avatar: &str,
    place: Location,
    containing: bool,
) {
    let label = if containing {
        tr!("explorer-menu-copy-containing-path")
    } else {
        tr!("explorer-menu-copy-path")
    };
    if menu::item(ui, p, "clipboard-text-filled", label) {
        let text = state
            .navigation
            .address(inv, layout, place, state.here.raw || place == Location::FullTree, avatar);
        ui.ctx().copy_text(text);
    }
}

fn pins_and_icons(ui: &mut Ui, p: &Palette, inv: &Inventory, layout: &mut Preferences, state: &mut ExplorerState, place: Location) {
    if !state.here.raw && !matches!(place, Location::Home | Location::FullTree | Location::MyFolders) {
        if layout.pins(inv).contains(&place) {
            if menu::item(ui, p, "pin-filled", tr!("explorer-panel-unpin")) {
                layout.unpin_location(inv, place);
            }
        } else if menu::item(ui, p, "pin-filled", tr!("explorer-panel-pin-to-sidebar")) {
            layout.pin_location(inv, place);
        }
    }
    folder_icons::menu(ui, p, state, place);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn selected(
    ui: &mut Ui,
    p: &Palette,
    inv: &Inventory,
    layout: &mut Preferences,
    state: &mut ExplorerState,
    avatar: &str,
    row: Row,
    open: &mut Option<Row>,
    files: &Menu,
    context: bool,
) {
    let single = state.here.selected.len() == 1;
    let dashboard = state.dashboard_shortcuts(inv, layout);
    let ids: Vec<_> = state.here.selected.iter().filter_map(|row| row.id()).collect();
    let guarded = ids.iter().any(|id| model::client_managed(inv, *id));
    let mut c = row
        .id()
        .filter(|_| !dashboard && ids.len() == state.here.selected.len() && !ids.is_empty())
        .map(|id| catalog(files, inv, id, ids.clone()));
    if context && let Some(c) = &mut c {
        let can_delete = file_actions::delete_enabled(inv, &ids, files, guarded, false);
        quick_actions(ui, c, state, &ids, guarded, can_delete);
        ui.add_space(4.0);
    }
    if single && menu::item_keys(ui, p, "folder-open-filled", tr!("common-open"), "Enter") {
        match row {
            Row::Item(id) if !rules::original(inv, id).is_some_and(|to| inv.folders.contains_key(&to)) => {
                state.file_request = Some(Request::Native(native::Command::Open(id)))
            }
            _ => *open = Some(row),
        }
    }
    if let Some(c) = &mut c {
        if !context {
            action(ui, p, state, c, "share", &ids, !guarded);
        }
        action(ui, p, state, c, "apply-environment", &ids, true);
        action(ui, p, state, c, "apply-parcel", &ids, true);
    }
    if single && let Row::Place(place) = row {
        if menu::item(ui, p, "layout-grid-filled", tr!("explorer-panel-open-tab")) {
            state.pending_open_tab = Some((place, state.here.raw));
        }
        if context && !dashboard {
            new_menu(ui, p, inv, layout, state, files, place);
        }
        pins_and_icons(ui, p, inv, layout, state, place);
        copy_path(ui, p, inv, layout, state, avatar, place, false);
    }
    if !dashboard && single {
        if !state.here.query.trim().is_empty()
            && let Some(parent) = navigation::containing(inv, layout, row, state.here.raw)
            && menu::item(ui, p, "folder-open-filled", tr!("explorer-address-open-containing"))
        {
            state.visit(parent, state.here.raw);
            state.clear_filter();
            state.select_and_reveal(row);
        }
        if let Row::Item(_) = row
            && let Some(parent) = navigation::containing(inv, layout, row, state.here.raw)
        {
            copy_path(ui, p, inv, layout, state, avatar, parent, true);
        }
    }
    if dashboard {
        return;
    }
    let real_parent = row.id().and_then(|id| rules::parent(inv, id)).filter(|_| single);
    let local_placement = row.id().filter(|id| single && !state.here.raw && layout.is_placed(*id));
    let removable = if single && !state.here.raw {
        if let Row::Place(Location::Custom(id)) = row {
            Some(id)
        } else {
            None
        }
    } else {
        None
    };
    if c.is_some() || real_parent.is_some() || removable.is_some() {
        menu::submenu(ui, p, "folder-filled", tr!("inventory-menu-organize"), true, |ui| {
            if let Some(c) = &mut c {
                action(ui, p, state, c, "move", &ids, !guarded);
                action(
                    ui,
                    p,
                    state,
                    c,
                    "restore",
                    &ids,
                    !guarded && ids.iter().all(|id| rules::in_type(inv, *id, 14)),
                );
                action(ui, p, state, c, "original", &ids, true);
            }
            if let Some(parent) = real_parent
                && menu::item(ui, p, "folder-open-filled", tr!("explorer-panel-show-original"))
            {
                file_actions::show_parent(inv, state, row, parent);
            }
            if let Some(id) = local_placement
                && menu::item(ui, p, "arrow-back-up", tr!("explorer-panel-restore"))
            {
                layout.restore(id);
            }
            if let Some(id) = removable
                && menu::item(ui, p, "folder-filled", tr!("explorer-panel-remove-virtual"))
            {
                layout.remove_custom(id);
            }
        });
    }
    if let Some(c) = &mut c {
        for key in [
            "wear",
            "add",
            "detach",
            "play-local",
            "play-world",
            "play-sound",
            "play-sound-world",
            "image",
        ] {
            action(ui, p, state, c, key, &ids, true);
        }
    }
    if ids
        .iter()
        .any(|id| super::super::organized_permissions::selectable(inv, *id, files.facts.agent))
        && menu::item_if(
            ui,
            p,
            "lock-filled",
            tr!("explorer-permissions-title"),
            files.native.pending.is_none() && !files.native.next_owner.running(),
        )
    {
        state.file_request = Some(Request::NextOwner(ids.clone()));
    }
    if single {
        let destination = if let Row::Place(place) = row { place } else { state.here.location };
        paste(ui, p, inv, layout, state, destination, files);
    }
    if let Some(c) = &mut c {
        if ids.iter().all(|id| rules::in_type(inv, *id, 14)) {
            let can_purge = file_actions::delete_enabled(inv, &ids, files, guarded, true);
            action(ui, p, state, c, "purge", &ids, can_purge);
        }
        action(ui, p, state, c, "properties", &ids, true);
    }
}

fn paste(ui: &mut Ui, p: &Palette, inv: &Inventory, layout: &Preferences, state: &mut ExplorerState, place: Location, files: &Menu) {
    for (links, icon, label) in [
        (false, "clipboard-text-filled", tr!("common-paste")),
        (true, "link", tr!("ui-inventory-context-paste-as-link")),
    ] {
        if file_actions::paste_enabled(files, inv, layout, place, links) && menu::item(ui, p, icon, label) {
            state.file_request = Some(Request::Paste(place, links));
        }
    }
}

// Resolve a writable SL destination again when the command executes. A pin or
// combined category with several destinations cannot implicitly choose a parent.
fn allowed(inv: &Inventory, id: Uuid) -> bool {
    model::personal_contents(inv, id)
        && rules::destination(inv, id)
        && inv.folders.get(&id).is_some_and(|folder| folder.info.type_default != 16)
}
fn targets(inv: &Inventory, layout: &Preferences, place: Location) -> Vec<Uuid> {
    if matches!(place, Location::Library | Location::Group(Group::System)) {
        return Vec::new();
    }
    if let Some(id) = folder::fixed_parent(place, inv) {
        return allowed(inv, id).then_some(id).into_iter().collect();
    }
    folder::choices(inv, layout, place)
        .into_iter()
        .filter(|id| allowed(inv, *id))
        .collect()
}

fn home_target(inv: &Inventory, kind: aurora_net::inventory::operations::NewItem) -> Option<Uuid> {
    let (asset, _, _) = kind.types();
    model::preferred_root(inv, i32::from(asset)).filter(|id| allowed(inv, *id))
}

fn creation(c: &native::Catalog, ui: &mut Ui, p: &Palette, zone: &str, home: Option<(&Inventory, &Preferences)>) -> Option<String> {
    for e in c.entries.iter().filter(|entry| entry.zone == zone) {
        let mut pressed = None;
        match e.kind {
            Kind::Group { .. } if e.id.starts_with("new-") => {
                menu::submenu(ui, p, &e.icon, &e.label, e.enabled, |ui| {
                    pressed = creation(c, ui, p, &children(&e.id), home)
                });
            }
            Kind::Action(index)
                if matches!(
                    c.commands[index],
                    Some(native::Command::Folder(_) | native::Command::Invoke(InvAction::Create { .. }))
                ) =>
            {
                let mut enabled = e.enabled;
                let text = if let Some((inv, layout)) = home {
                    let destination = match &c.commands[index] {
                        Some(native::Command::Invoke(InvAction::Create { kind, .. })) => home_target(inv, *kind).map(|id| {
                            path(inv, layout, Location::Folder(id), false)
                                .into_iter()
                                .filter(|p| *p != Location::Home)
                                .map(|p| place_name(inv, layout, p, false, ""))
                                .collect::<Vec<_>>()
                                .join(" › ")
                        }),
                        _ => Some(tr!("explorer-new-home-container").into()),
                    };
                    enabled &= destination.is_some();
                    destination.map_or_else(|| e.label.clone(), |path| format!("{}    ›    {path}", e.label))
                } else {
                    e.label.clone()
                };
                if menu::item_if(ui, p, &e.icon, &text, enabled) {
                    pressed = Some(e.id.clone());
                }
            }
            _ => {}
        }
        if pressed.is_some() {
            return pressed;
        }
    }
    None
}
fn new_menu(ui: &mut Ui, p: &Palette, inv: &Inventory, layout: &Preferences, state: &mut ExplorerState, files: &Menu, place: Location) {
    let actual = if let Some(id) = folder::fixed_parent(place, inv) {
        allowed(inv, id).then_some(id)
    } else {
        targets(inv, layout, place).into_iter().next()
    };
    let Some(_) = actual else { return };
    menu::submenu(ui, p, "plus", tr!("inventory-menu-new"), files.native.pending.is_none(), |ui| {
        new_options(ui, p, inv, layout, state, files, place);
    });
}

pub(super) fn new_options(
    ui: &mut Ui,
    p: &Palette,
    inv: &Inventory,
    layout: &Preferences,
    state: &mut ExplorerState,
    files: &Menu,
    place: Location,
) {
    let Some(parent) = targets(inv, layout, place).first().copied() else {
        return;
    };
    let c = catalog(files, inv, parent, vec![parent]);
    let home = (place == Location::Home && !state.here.raw).then_some((inv, layout));
    if let Some(key) = creation(&c, ui, p, &children("new"), home) {
        state.file_request = Some(Request::Create(place, key));
    }
}

pub(super) fn background(
    ui: &mut Ui,
    p: &Palette,
    inv: &Inventory,
    layout: &mut Preferences,
    state: &mut ExplorerState,
    avatar: &str,
    files: &Menu,
    context: bool,
) {
    let place = state.here.location;
    if context {
        new_menu(ui, p, inv, layout, state, files, place);
        views::menu(ui, p, inv, layout, state);
    }
    paste(ui, p, inv, layout, state, place, files);
    copy_path(ui, p, inv, layout, state, avatar, place, false);
    folder_icons::menu(ui, p, state, place);
}

pub(super) fn create(
    inv: &Inventory,
    layout: &Preferences,
    state: &mut ExplorerState,
    files: &Menu,
    place: Location,
    key: &str,
    _actions: &mut Vec<InvAction>,
) {
    if files.native.pending.is_some() || !layout.available(inv, place) {
        state.status = tr!("explorer-real-unavailable").into();
        return;
    }
    let choices = targets(inv, layout, place);
    if choices.is_empty() {
        state.status = tr!("explorer-create-unavailable").into();
        return;
    }
    // Let the existing folder draft show the physical path and capture the user's new name.
    if key == "new-folder" {
        state.visit(place, state.here.raw);
        folder::begin(inv, layout, state);
        return;
    }
    let Some(to) = folder::fixed_parent(place, inv) else {
        state.new_destination = Some(NewDestination {
            root: inv.root,
            origin: place,
            key: key.into(),
            selected: None,
        });
        return;
    };
    let mut c = catalog(files, inv, to, vec![to]);
    if let Some(native::Command::Invoke(InvAction::Create { kind, name, .. })) = take(&mut c, key) {
        let destination = if place == Location::Home && !state.here.raw {
            let Some(id) = home_target(inv, kind) else {
                state.status = tr!("explorer-real-unavailable").into();
                return;
            };
            id
        } else {
            to
        };
        let shown = if destination == inv.root {
            if state.here.raw { Location::FullTree } else { Location::MyFolders }
        } else {
            Location::Folder(destination)
        };
        if state.folder_draft.is_some() {
            return;
        }
        state.visit(shown, state.here.raw);
        folder::start(state, Some(destination), Some(kind), &name, false);
    } else {
        state.status = tr!("explorer-create-unavailable").into();
    }
}

pub(super) fn destination(ui: &mut Ui, inv: &Inventory, layout: &Preferences, state: &mut ExplorerState, files: &Menu) {
    let Some(mut draft) = state.new_destination.take() else { return };
    if draft.root != inv.root {
        return;
    }
    let choices = targets(inv, layout, draft.origin);
    if draft.selected.is_some_and(|id| !choices.contains(&id)) {
        draft.selected = None;
    }
    ui.label(tr!("explorer-create-location"));
    egui::ComboBox::from_id_salt("explorer-new-item-destination")
        .selected_text(draft.selected.map_or_else(
            || tr!("explorer-create-choose-parent").to_string(),
            |id| super::super::inventory_path(inv, id),
        ))
        .show_ui(ui, |ui| {
            for id in choices {
                ui.selectable_value(&mut draft.selected, Some(id), super::super::inventory_path(inv, id));
            }
        });
    let mut close = false;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                files.native.pending.is_none() && draft.selected.is_some(),
                egui::Button::new(tr!("explorer-panel-create")),
            )
            .clicked()
            && let Some(to) = draft.selected
        {
            state.file_request = Some(Request::Create(Location::Folder(to), draft.key.clone()));
            close = true;
        }
        close |= ui.small_button(tr!("common-cancel")).clicked();
    });
    if !close {
        state.new_destination = Some(draft);
    }
}
