//! Account-local groups for sidebar shortcuts.
use super::*;
use crate::ui::widgets;
use aurora_i18n::tr;

#[derive(Clone, Copy)]
pub(super) struct PinDrag(Uuid, Location);

pub(super) struct Editor {
    id: Option<Uuid>,
    text: String,
    focus: bool,
    add_pin: Option<Location>,
}

impl Editor {
    fn create(add_pin: Option<Location>) -> Self {
        Self {
            id: None,
            text: String::new(),
            focus: true,
            add_pin,
        }
    }

    fn rename(group: &model::PinGroup) -> Self {
        Self {
            id: Some(group.id),
            text: group.name.clone(),
            focus: true,
            add_pin: None,
        }
    }
}

pub(super) fn pin_menu(ui: &mut Ui, inv: &Inventory, layout: &mut Preferences, place: Location) {
    if place == Location::Home || place == Location::FullTree {
        return;
    }
    if layout.pins(inv).contains(&place) {
        if ui.button(aurora_i18n::tr!("explorer-panel-unpin")).clicked() {
            layout.unpin_location(inv, place);
            ui.close();
        }
    } else if ui.button(aurora_i18n::tr!("explorer-panel-pin-to-sidebar")).clicked() {
        layout.pin_location(inv, place);
        ui.close();
    }
}

fn pin_row(
    ui: &mut Ui,
    style: &RowStyle,
    inv: &Inventory,
    layout: &mut Preferences,
    state: &mut ExplorerState,
    avatar: &str,
    place: Location,
) {
    ui.push_id(place, |ui| {
        let text = place_name(inv, layout, place, false, avatar);
        let selected = !state.here.raw && state.here.location == place;
        let glyph = style
            .resolve(inv, Row::Place(place), selected, selected)
            .with_symbol(super::folder_icons::for_row(layout, Row::Place(place)));
        let r = entry(ui, style, &text, glyph, selected);
        if r.clicked() {
            state.visit(place, false);
        }
        r.dnd_set_drag_payload(PinDrag(inv.root, place));
        if r.drag_started_by(egui::PointerButton::Primary) {
            super::drag_preview::start(ui.ctx(), inv, style, &text, glyph, None);
        }
        if let Some(from) = r.dnd_hover_payload::<PinDrag>()
            && from.0 == inv.root
            && from.1 != place
        {
            let before = ui.input(|i| i.pointer.hover_pos()).is_some_and(|pos| pos.y < r.rect.center().y);
            let y = if before { r.rect.top() } else { r.rect.bottom() };
            ui.painter()
                .hline(r.rect.x_range(), y, egui::Stroke::new(2.0, ui.visuals().hyperlink_color));
            if ui.input(|i| i.pointer.primary_released()) && r.dnd_release_payload::<PinDrag>().is_some() {
                layout.reorder_pin(inv, from.1, place, before);
            }
        } else if receive(&r, inv, layout, place) {
            state.status = aurora_i18n::tr!("explorer-panel-updated").into();
        }
        context_menu(&r, style.palette, |ui| {
            if ui.button(aurora_i18n::tr!("explorer-panel-open-tab")).clicked() {
                state.pending_open_tab = Some((place, false));
                ui.close();
            }
            super::folder_icons::menu(ui, style.palette, state, place);
            pin_menu(ui, inv, layout, place);
            move_menu(ui, inv, layout, state, place);
            if ui.button(aurora_i18n::tr!("explorer-panel-pin-up")).clicked() {
                layout.shift_location(inv, place, true);
                ui.close();
            }
            if ui.button(aurora_i18n::tr!("explorer-panel-pin-down")).clicked() {
                layout.shift_location(inv, place, false);
                ui.close();
            }
        });
    });
}

pub(super) fn sidebar(ui: &mut Ui, style: &RowStyle, inv: &Inventory, layout: &mut Preferences, state: &mut ExplorerState, avatar: &str) {
    let home = entry(
        ui,
        style,
        &place_name(inv, layout, Location::Home, false, avatar),
        style
            .resolve(inv, Row::Place(Location::Home), false, state.here.location == Location::Home)
            .with_symbol(super::folder_icons::for_row(layout, Row::Place(Location::Home))),
        state.here.location == Location::Home,
    );
    if home.clicked() {
        state.visit(Location::Home, false);
    }
    receive(&home, inv, layout, Location::Home);
    menus::context_menu_behavior(&home, style.palette, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        super::folder_icons::menu(ui, style.palette, state, Location::Home);
        pin_manager(ui, style.palette, inv, layout, state);
    });
    ui.add_space(4.0);
    group_editor(ui, inv, layout, state);
    let grouped = !layout.pin_groups().is_empty();
    egui::ScrollArea::vertical()
        .id_salt("pinned")
        .max_height(ui.available_height())
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (group, pins) in layout.pin_sections(inv) {
                if !grouped {
                    for place in pins {
                        pin_row(ui, style, inv, layout, state, avatar, place);
                    }
                    continue;
                }
                let id = group.as_ref().map(|g| g.id);
                let collapsed = group.as_ref().map_or_else(|| layout.ungrouped_collapsed(), |g| g.collapsed);
                ui.push_id(("pin-group", id), |ui| {
                    group_header(ui, style, inv, layout, state, group.as_ref(), collapsed);
                    if !collapsed {
                        ui.indent("members", |ui| {
                            for place in pins {
                                pin_row(ui, style, inv, layout, state, avatar, place);
                            }
                        });
                    }
                });
            }
        });
}

// Group headers only accept sidebar shortcuts; directory rows retain normal inventory drop behavior.
fn group_header(
    ui: &mut Ui,
    style: &RowStyle,
    inv: &Inventory,
    layout: &mut Preferences,
    state: &mut ExplorerState,
    group: Option<&model::PinGroup>,
    collapsed: bool,
) {
    let id = group.map(|g| g.id);
    let name = group.map_or_else(|| tr!("explorer-pins-ungrouped"), |g| g.name.as_str());
    let icon = if collapsed { "caret-right-filled" } else { "caret-down-filled" };
    let header = entry(ui, style, name, Glyph::navigation(icon, style.palette.teal), false);
    header.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    if header.clicked() {
        layout.toggle_pin_group(id);
    }
    if let Some(from) = header.dnd_hover_payload::<PinDrag>()
        && from.0 == inv.root
    {
        ui.painter().rect_stroke(
            header.rect.shrink(1.0),
            ui.visuals().widgets.inactive.corner_radius,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
        if ui.input(|i| i.pointer.primary_released()) && header.dnd_release_payload::<PinDrag>().is_some() {
            layout.move_pin_to_group(inv, from.1, id);
        }
    }
    context_menu(&header, style.palette, |ui| {
        if ui.button(tr!("explorer-pins-new-group")).clicked() {
            state.pin_editor = Some(Editor::create(None));
            ui.close();
        }
        if let Some(group) = group {
            if ui.button(tr!("explorer-pins-rename-group")).clicked() {
                state.pin_editor = Some(Editor::rename(group));
                ui.close();
            }
            for (up, label) in [(true, tr!("explorer-pins-up")), (false, tr!("explorer-pins-down"))] {
                let at_edge = if up {
                    layout.pin_groups().first()
                } else {
                    layout.pin_groups().last()
                };
                if ui
                    .add_enabled(at_edge.is_some_and(|g| g.id != group.id), egui::Button::new(label))
                    .clicked()
                {
                    layout.shift_pin_group(group.id, up);
                    ui.close();
                }
            }
            if ui
                .button(tr!("explorer-pins-remove-group"))
                .on_hover_text(tr!("explorer-pins-remove-hint"))
                .clicked()
            {
                layout.remove_pin_group(group.id);
                ui.close();
            }
        }
    });
}

fn group_editor(ui: &mut Ui, inv: &Inventory, layout: &mut Preferences, state: &mut ExplorerState) {
    let Some(editor) = &mut state.pin_editor else { return };
    ui.label(if editor.id.is_some() {
        tr!("explorer-pins-rename-group")
    } else {
        tr!("explorer-pins-new-group")
    });
    let field = ui.add(
        egui::TextEdit::singleline(&mut editor.text)
            .hint_text(tr!("explorer-pins-name"))
            .desired_width(ui.available_width()),
    );
    if std::mem::take(&mut editor.focus) {
        field.request_focus();
    }
    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    let escape = (field.has_focus() || field.lost_focus()) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
    let valid = !editor.text.trim().is_empty();
    let (save, cancel) = ui
        .horizontal_wrapped(|ui| {
            let save = ui.add_enabled(valid, egui::Button::new(tr!("common-save"))).clicked();
            (save, ui.button(tr!("common-cancel")).clicked())
        })
        .inner;
    if escape || cancel {
        state.pin_editor = None;
    } else if (save || enter) && valid {
        if let Some(id) = editor.id {
            layout.rename_pin_group(id, &editor.text);
        } else if let Some(id) = layout.create_pin_group(&editor.text)
            && let Some(pin) = editor.add_pin
        {
            layout.move_pin_to_group(inv, pin, Some(id));
        }
        state.pin_editor = None;
    }
}

fn move_menu(ui: &mut Ui, inv: &Inventory, layout: &mut Preferences, state: &mut ExplorerState, place: Location) {
    ui.menu_button(tr!("explorer-pins-move-group"), |ui| {
        let mut chosen = None;
        for group in layout.pin_groups() {
            if ui.push_id(group.id, |ui| ui.button(&group.name).clicked()).inner {
                chosen = Some(Some(group.id));
            }
        }
        if ui.button(tr!("explorer-pins-ungrouped")).clicked() {
            chosen = Some(None);
        }
        if let Some(group) = chosen {
            layout.move_pin_to_group(inv, place, group);
            ui.close();
        }
        ui.add_space(4.0);
        if ui.button(tr!("explorer-pins-new-group")).clicked() {
            state.pin_editor = Some(Editor::create(Some(place)));
            ui.close();
        }
    });
}
// This catalog remains available from Home's context menu and the View menu after a destination is unpinned.
pub(super) fn pin_manager(ui: &mut Ui, palette: &Palette, inv: &Inventory, layout: &mut Preferences, state: &mut ExplorerState) {
    if ui.button(tr!("explorer-pins-new-group")).clicked() {
        state.pin_editor = Some(Editor::create(None));
        ui.close();
    }
    ui.add_space(4.0);
    let pins = layout.pins(inv);
    egui::Grid::new("pin-catalog").num_columns(2).show(ui, |ui| {
        for loc in layout.catalog(inv) {
            let mut pinned = pins.contains(&loc);
            if widgets::checkbox(ui, palette, &mut pinned, layout.location_label(inv, loc)).changed() {
                if pinned {
                    layout.pin_location(inv, loc);
                } else {
                    layout.unpin_location(inv, loc);
                }
            }
            if ui.small_button(tr!("common-open")).clicked() {
                state.visit(loc, false);
                ui.close();
            }
            ui.end_row();
        }
    });
}
