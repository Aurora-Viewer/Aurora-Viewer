//! Task-item adapter for the shared inventory properties panel.
//! LLFloaterProperties / show_task_item_profile (Firestorm,
//! indra/newview/llfloaterproperties.cpp, originally LGPL 2.1).

use crate::build::BuildTool;
use crate::theme::Palette;
use crate::ui::inventory::PropertyEdit;
use crate::ui::inventory::properties::{self, PropertyPanel};
use crate::ui::widgets::CascadeGroup;
use crate::world::World;
use aurora_net::build::{BuildCmd, TaskItem, perm};
use std::collections::HashMap;
use uuid::Uuid;

pub struct TaskProperties {
    pub object: Uuid,
    pub item: Uuid,
    pub edit: PropertyEdit,
}

impl TaskProperties {
    pub fn new(object: Uuid, item: &TaskItem) -> Option<Self> {
        Some(Self {
            object,
            item: item.item_id,
            edit: PropertyEdit::from(&item.to_inv_item()?),
        })
    }
}

fn editable(tool: &BuildTool, world: &World, object: Uuid, item: &TaskItem) -> bool {
    world
        .objects
        .index_of_uuid(&object)
        .is_some_and(|idx| tool.contents_editable(world, idx))
        && ((!item.group_owned && item.owner_id == world.agent_id)
            || (item.group_owned && !item.group_id.is_nil() && world.groups.is_member(&item.group_id)))
        && !matches!(item.asset_type, 2 | 24 | 25)
}

/// One window per open item. The Legacy Viewer opens one per item too
/// (LLInventoryAction::doToSelected, indra/newview/llinventoryfunctions.cpp),
/// hosted as tabs of one LLMultiItemProperties window; Aurora has no tab host
/// for them and cascades separate windows, as the inventory's PropertiesMany does.
pub(super) fn show(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World, images: &HashMap<Uuid, egui::TextureHandle>) {
    let windows = std::mem::take(&mut tool.ui.item_properties);
    let mut kept = Vec::with_capacity(windows.len());
    for state in windows {
        if let Some(state) = show_one(ctx, p, tool, world, images, state) {
            kept.push(state);
        }
    }
    // A window opened while these were drawn stays after them.
    kept.append(&mut tool.ui.item_properties);
    tool.ui.item_properties = kept;
}

fn show_one(
    ctx: &egui::Context,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    images: &HashMap<Uuid, egui::TextureHandle>,
    mut state: TaskProperties,
) -> Option<TaskProperties> {
    let item = tool
        .contents
        .objects
        .get(&state.object)
        .and_then(|inv| inv.items.iter().find(|it| it.item_id == state.item));
    let item = item.cloned().filter(|it| !it.is_folder)?;
    let inv_item = item.to_inv_item()?;
    let can_edit = editable(tool, world, state.object, &item);
    let names = [item.owner_id, item.creator_id]
        .into_iter()
        .map(|id| {
            world.social.want_name(id);
            (id, world.social.name_of(&id))
        })
        .collect::<HashMap<_, _>>();
    let mut open = true;
    let window_id = format!("task_item_props_{}_{}", state.object, state.item);
    properties::window(ctx, &window_id)
        .cascade(CascadeGroup::TaskProperties)
        .show(ctx, p, &mut open, |ui| {
            let response = properties::panel(
                ui,
                p,
                None,
                PropertyPanel {
                    inventory: None,
                    item: &inv_item,
                    edit: &mut state.edit,
                    can_edit,
                    can_next_owner: can_edit && item.owner_mask & perm::TRANSFER != 0,
                    can_describe: can_edit && item.owner_mask & perm::MODIFY != 0,
                    // TaskInventory has no thumbnail field or agent-thumbnail capability.
                    can_thumbnail: false,
                    names: &names,
                    images,
                    wanted_images: &mut tool.ui.wanted_images,
                },
            );
            if let Some(id) = response.profile {
                tool.ui.requests.push(super::Request::Profile(id));
            }
            if let Some(edit) = response.edit {
                tool.save_item_properties(
                    world,
                    &TaskProperties {
                        object: state.object,
                        item: state.item,
                        edit,
                    },
                );
            }
        });
    open.then_some(state)
}

impl BuildTool {
    /// Opens the item's window unless it is already open: one window per item.
    pub(crate) fn open_task_item_properties(&mut self, object: Uuid, item: &TaskItem) {
        if self
            .ui
            .item_properties
            .iter()
            .any(|open| open.object == object && open.item == item.item_id)
        {
            return;
        }
        self.ui.item_properties.extend(TaskProperties::new(object, item));
    }

    /// Opens a window for each item of a selection; a folder has none.
    pub(crate) fn open_task_items_properties(&mut self, object: Uuid, items: &[TaskItem]) {
        for item in items.iter().filter(|it| !it.is_folder) {
            self.open_task_item_properties(object, item);
        }
    }

    /// Re-read the source object and item at commit time, independently of the
    /// current build selection. Never turn task edits into agent-inventory edits.
    pub(super) fn save_item_properties(&mut self, world: &World, state: &TaskProperties) -> bool {
        let Some(idx) = world.objects.index_of_uuid(&state.object) else {
            return false;
        };
        let Some(object) = world.objects.get(idx) else { return false };
        let Some(mut item) = self
            .contents
            .objects
            .get(&state.object)
            .and_then(|inv| inv.items.iter().find(|it| it.item_id == state.item))
            .cloned()
        else {
            return false;
        };
        if !editable(self, world, state.object, &item) {
            return false;
        }
        let name = if state.edit.name == item.name {
            item.name.clone()
        } else {
            if crate::text_validation::require_edit(crate::text_validation::Policy::PrintableNoPipe, &item.name, &state.edit.name).is_err()
            {
                return false;
            }
            let Ok(name) = crate::world::inventory::actions::clean_name(&state.edit.name) else {
                return false;
            };
            name
        };
        if crate::text_validation::require_edit(crate::text_validation::Policy::MultilineNoPipe, &item.description, &state.edit.desc)
            .is_err()
        {
            return false;
        }
        if item.owner_mask & perm::MODIFY != 0 {
            item.name = name;
            item.description = state.edit.desc.chars().filter(|c| *c != '|').take(127).collect();
        }
        if item.owner_mask & (perm::COPY | perm::TRANSFER) == perm::COPY | perm::TRANSFER {
            item.group_mask = state.edit.group & item.base_mask;
            item.everyone_mask = state.edit.everyone & item.base_mask;
        }
        if item.owner_mask & perm::TRANSFER != 0 {
            item.next_owner_mask = state.edit.next & item.base_mask;
            if item.asset_type == 56 {
                item.next_owner_mask |= perm::COPY & item.base_mask;
            }
            if item.next_owner_mask & perm::COPY == 0 {
                item.next_owner_mask |= perm::TRANSFER & item.base_mask;
            }
            item.sale_type = if state.edit.sale_type == 2 && item.owner_mask & perm::COPY == 0 {
                1
            } else {
                state.edit.sale_type.min(3)
            };
            item.sale_price = state.edit.price.max(0);
        }
        let (handle, local_id) = (object.key.region, object.key.local_id);
        if let Some(current) = self
            .contents
            .objects
            .get_mut(&state.object)
            .and_then(|inv| inv.items.iter_mut().find(|it| it.item_id == state.item))
        {
            *current = item.clone();
        }
        self.send(BuildCmd::UpdateTaskInventory {
            handle,
            local_id,
            item: Box::new(item),
        });
        self.contents_changed(state.object);
        true
    }
}
