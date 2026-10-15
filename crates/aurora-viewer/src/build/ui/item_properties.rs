//! Task-item adapter for the shared inventory properties panel.
//! LLFloaterProperties / show_task_item_profile (Firestorm,
//! indra/newview/llfloaterproperties.cpp, originally LGPL 2.1).

use crate::build::BuildTool;
use crate::theme::Palette;
use crate::ui::inventory::PropertyEdit;
use crate::ui::inventory::properties::{self, PropertyPanel};
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

pub(super) fn show(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World, env: &super::Env) {
    let Some(mut state) = tool.ui.item_properties.take() else { return };
    let item = tool
        .contents
        .objects
        .get(&state.object)
        .and_then(|inv| inv.items.iter().find(|it| it.item_id == state.item));
    let Some(item) = item.cloned().filter(|it| !it.is_folder) else {
        return;
    };
    let Some(inv_item) = item.to_inv_item() else { return };
    let can_edit = editable(tool, world, state.object, &item);
    let names = [item.owner_id, item.creator_id]
        .into_iter()
        .map(|id| {
            world.social.want_name(id);
            (id, world.social.name_of(&id))
        })
        .collect::<HashMap<_, _>>();
    let mut open = true;
    let mut saved = false;
    properties::window(ctx, "task_item_props").show(ctx, p, &mut open, |ui| {
        let response = properties::panel(
            ui,
            p,
            None,
            PropertyPanel {
                item: &inv_item,
                edit: &mut state.edit,
                can_edit,
                can_next_owner: can_edit && item.owner_mask & perm::TRANSFER != 0,
                can_describe: can_edit && item.owner_mask & perm::MODIFY != 0,
                // TaskInventory has no thumbnail field or agent-thumbnail capability.
                can_thumbnail: false,
                names: &names,
                images: env.images,
                wanted_images: &mut tool.ui.wanted_images,
            },
        );
        if let Some(id) = response.profile {
            tool.ui.requests.push(super::Request::Profile(id));
        }
        if response.save {
            saved = tool.save_item_properties(world, &state);
        }
    });
    if open && !saved {
        tool.ui.item_properties = Some(state);
    }
}

impl BuildTool {
    pub(crate) fn open_task_item_properties(&mut self, object: Uuid, item: &TaskItem) {
        self.ui.item_properties = TaskProperties::new(object, item);
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
        let Ok(name) = crate::world::inventory::actions::clean_name(&state.edit.name) else {
            return false;
        };
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
