//! The Open window uses the same selection and transfer/drop rules as Build >
//! Contents (LLPanelObjectInventory, indra/newview/llpanelobjectinventory.cpp).
use super::ContentsAction;
use crate::build::{BuildTool, contents_drop, task_inventory};
use crate::theme::Palette;
use crate::ui::inventory::{self, task_selection::Selection};
use crate::ui::{icons::Icons, menu, widgets};
use crate::world::World;
use aurora_net::inventory::InvItem;
use egui::{Id, Key, Vec2};
use uuid::Uuid;

pub(super) fn state_id() -> Id {
    Id::unique("open_object_contents_selection")
}

fn focus_id(object: Uuid) -> Id {
    Id::unique(("open_object_contents_focus", object))
}

pub(super) fn clear(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<Selection>(state_id()));
}

/// Offline `open-multi` scenario: exercise the ordinary focused-list shortcut.
pub(crate) fn demo_select_all(ctx: &egui::Context, object: Uuid, raw: &mut egui::RawInput) {
    ctx.memory_mut(|m| m.request_focus(focus_id(object)));
    raw.events.push(egui::Event::ModifiersChanged(egui::Modifiers::CTRL));
    for pressed in [true, false] {
        raw.events.push(egui::Event::Key {
            key: Key::A,
            physical_key: Some(Key::A),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        });
    }
}

fn selected_items(selection: &Selection, object: Uuid, items: &[InvItem]) -> Vec<aurora_net::build::TaskItem> {
    items
        .iter()
        .filter(|item| selection.items.contains(&item.id))
        .map(|item| task_inventory::from_inventory(object, item))
        .collect()
}

/// Called before world/object shortcuts: Delete belongs to this focused list,
/// even when it is empty or read-only. Repeated keys never reach ObjectDelete.
pub(crate) fn shortcuts(
    ctx: &egui::Context,
    state: &crate::interaction::Interactions,
    world: &World,
    build: &BuildTool,
) -> Vec<ContentsAction> {
    let Some(c) = &state.contents else { return Vec::new() };
    let object = c.target.object;
    if ctx.text_edit_focused() || !ctx.memory(|m| m.has_focus(focus_id(object))) {
        return Vec::new();
    }
    let mut selection = ctx.data(|d| d.get_temp::<Selection>(state_id())).unwrap_or_default();
    let allowed = selection.object == Some(object)
        && !egui::Popup::is_any_open(ctx)
        && selection.layer.is_some_and(|layer| ctx.memory(|m| m.allows_interaction(layer)));
    let mut select_all = false;
    let mut delete = false;
    ctx.input_mut(|i| {
        i.events.retain(|event| {
            if let egui::Event::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
                ..
            } = event
                && (modifiers.is_none() && *key == Key::Delete
                    || *key == Key::A && (modifiers.command || modifiers.ctrl) && !modifiers.shift && !modifiers.alt)
            {
                if allowed && !repeat {
                    select_all |= *key == Key::A;
                    delete |= *key == Key::Delete;
                }
                false
            } else {
                true
            }
        });
    });
    let mut actions = Vec::new();
    if let Some(Ok(items)) = &c.result {
        let visible: std::collections::HashSet<_> = items.iter().map(|item| item.id).collect();
        selection.items.retain(|id| visible.contains(id));
        if select_all {
            selection.items = visible;
        }
        if delete
            && world
                .objects
                .index_of_uuid(&object)
                .is_some_and(|idx| build.contents_editable(world, idx))
        {
            let ids: Vec<_> = items
                .iter()
                .filter(|item| selection.items.contains(&item.id))
                .map(|item| item.id)
                .collect();
            if !ids.is_empty() {
                actions.push(ContentsAction::Remove(object, ids));
                selection.items.clear();
            }
        }
    }
    ctx.data_mut(|d| d.insert_temp(state_id(), selection));
    actions
}

pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    object: Uuid,
    items: &[InvItem],
    world: &World,
    build: &BuildTool,
) -> Vec<ContentsAction> {
    let mut selection = ui.ctx().data(|d| d.get_temp::<Selection>(state_id())).unwrap_or_default();
    selection.sync(object, items.iter().map(|item| item.id).collect(), ui.layer_id());
    let editable = world
        .objects
        .index_of_uuid(&object)
        .is_some_and(|idx| build.contents_editable(world, idx));
    let mut actions = Vec::new();
    let mut batch = selected_items(&selection, object, items);
    let zone = egui::Frame::new()
        .fill(p.field.gamma_multiply(0.72))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(4)
        .show(ui, |ui| {
            widgets::inventory_spacing(ui);
            let height = (ui.available_height() - 8.0).clamp(80.0, 200.0);
            let background = ui.interact(
                egui::Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), height)),
                focus_id(object),
                egui::Sense::click(),
            );
            if background.clicked() {
                background.request_focus();
                selection.items.clear();
                batch.clear();
            }
            egui::ScrollArea::both()
                .max_width(ui.available_width())
                .max_height(height)
                .min_scrolled_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if items.is_empty() {
                        ui.label(aurora_i18n::tr!("ui-object-actions-this-object-is-empty"));
                    }
                    for item in items {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if let Some(t) = icons.get(inventory::item_icon(item.asset_type, item.inv_type, item.flags)) {
                                let color = inventory::item_icon_color(item.asset_type, item.inv_type, p);
                                ui.add(egui::Image::new(t).fit_to_exact_size(Vec2::splat(16.0)).tint(color));
                            } else {
                                ui.add_space(16.0);
                            }
                            let permissions = inventory::permissions::Permissions::new(item.owner_mask, item.next_owner_mask);
                            let selected = selection.items.contains(&item.id);
                            let response = ui
                                .push_id(("object_item", item.id), |ui| permissions.row(ui, p, &item.name, selected))
                                .inner
                                .on_hover_ui(|ui| {
                                    ui.label(&item.name);
                                    if !item.desc.is_empty() {
                                        ui.label(&item.desc);
                                    }
                                    widgets::divider(ui, p);
                                    ui.label(permissions.tooltip());
                                });
                            if response.clicked() || response.secondary_clicked() || response.drag_started() && !selected {
                                selection.select_with_focus(ui, object, item.id, response.secondary_clicked(), focus_id(object));
                                batch = selected_items(&selection, object, items);
                            }
                            let single = task_inventory::from_inventory(object, item);
                            let taken = if selection.items.contains(&item.id) {
                                batch.as_slice()
                            } else {
                                std::slice::from_ref(&single)
                            };
                            if response.hovered() || response.dragged() || response.drag_started() {
                                inventory::task_drop::source(
                                    ui,
                                    &response,
                                    world,
                                    object,
                                    taken,
                                    task_inventory::owned(world, object, build.props.get(&object)),
                                );
                            }
                            let document = || match item.asset_type {
                                10 => Some(ContentsAction::Script(object, item.id)),
                                7 => Some(ContentsAction::Notecard(object, item.id)),
                                _ => None,
                            };
                            if response.double_clicked() {
                                actions.extend(document());
                            }
                            menu::context_menu(&response, p, |ui| {
                                let transfers: Result<Vec<_>, _> =
                                    taken.iter().map(|item| build.task_transfer_kind(world, object, item)).collect();
                                if matches!(item.asset_type, 7 | 10)
                                    && menu::item(ui, p, "folder-open-filled", aurora_i18n::tr!("common-open"))
                                {
                                    actions.extend(document());
                                }
                                let moves = transfers
                                    .as_ref()
                                    .is_ok_and(|kinds| kinds.iter().any(|kind| *kind != task_inventory::TransferKind::Copy));
                                let label = if moves {
                                    aurora_i18n::tr!("task-transfer-move-menu")
                                } else {
                                    aurora_i18n::tr!("build-ui-contents-copy-to-inventory")
                                };
                                let tip = transfers
                                    .as_ref()
                                    .err()
                                    .copied()
                                    .unwrap_or(aurora_i18n::tr!("build-ui-contents-like-dragging-the-item-to-inventory"));
                                if menu::item_tip(ui, p, "briefcase-filled", label, transfers.is_ok(), tip) {
                                    actions.push(ContentsAction::Take(object, taken.to_vec()));
                                }
                                if menu::item_if(ui, p, "trash-filled", aurora_i18n::tr!("common-delete"), editable) {
                                    actions.push(ContentsAction::Remove(object, taken.iter().map(|item| item.item_id).collect()));
                                    selection.items.clear();
                                }
                            });
                        });
                    }
                });
        });
    if ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|pos| !zone.response.rect.contains(pos)))
        && !egui::Popup::is_any_open(ui.ctx())
    {
        ui.memory_mut(|m| m.surrender_focus(focus_id(object)));
    }
    ui.ctx().data_mut(|d| d.insert_temp(state_id(), selection));
    if zone.response.contains_pointer()
        && let Some(ids) = inventory::drag::carried_ids(ui.ctx())
        && let Some(idx) = world.objects.index_of_uuid(&object)
    {
        let ctrl = ui.input(|i| i.modifiers.ctrl);
        let check_id = Id::unique("open_object_contents_drop");
        let allowed = ui.ctx().data_mut(|d| {
            d.get_temp_mut_or_default::<contents_drop::PlanCheck>(check_id)
                .check(world, &ids, idx, ctrl, contents_drop::Target::Panel)
        });
        inventory::tree::drop_outline(ui, p, zone.response.rect, allowed.is_ok());
        inventory::drag::feedback(
            ui.ctx(),
            if allowed.is_ok() {
                egui::CursorIcon::Copy
            } else {
                egui::CursorIcon::NotAllowed
            },
        );
        if let Err(reason) = allowed {
            zone.response.on_hover_text(contents_drop::reason_text(reason));
        }
        if ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary)) {
            egui::DragAndDrop::clear_payload(ui.ctx());
            if allowed.is_ok() {
                actions.push(ContentsAction::Drop(object, ids, ctrl));
            }
        }
    }
    actions
}
