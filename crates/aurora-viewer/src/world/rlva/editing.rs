//! RlvActions::canEdit, enforced before local edits and network writes.
use super::super::{World, objects::ObjKey};
use aurora_net::{RegionHandle, build::BuildCmd};

impl World {
    pub fn rlva_can_edit(&self, index: usize) -> bool {
        if !self.rlva.enabled() {
            return true;
        }
        let mut root = index;
        let mut attached = false;
        for _ in 0..64 {
            let Some(object) = self.objects.get(root) else { return false };
            let Some(parent) = self.objects.parent_of(object) else { break };
            if self.objects.get(parent).is_some_and(|o| o.is_avatar()) {
                attached = true;
                break;
            }
            root = parent;
        }
        self.objects.get(root).is_some_and(|o| {
            self.rlva.allows("edit", &o.full_id.to_string())
                && !self.rlva.targets("editobj", &o.full_id.to_string())
                && !self.rlva.has(if attached { "editattach" } else { "editworld" })
        })
    }

    pub fn rlva_allows_build(&self, command: &BuildCmd) -> bool {
        if !self.rlva.enabled() {
            return true;
        }
        let local = |handle: RegionHandle, id| {
            self.objects
                .index_of(&ObjKey {
                    region: handle,
                    local_id: id,
                })
                .is_some_and(|i| self.rlva_can_edit(i))
        };
        let uuid = |id: &uuid::Uuid| self.objects.index_of_uuid(id).is_some_and(|i| self.rlva_can_edit(i));
        match command {
            BuildCmd::Transform { handle, updates } => updates.iter().all(|u| local(*handle, u.local_id)),
            BuildCmd::SetName { handle, local_id, .. }
            | BuildCmd::SetDescription { handle, local_id, .. }
            | BuildCmd::SetFlags { handle, local_id, .. }
            | BuildCmd::SetPhysicsParams { handle, local_id, .. }
            | BuildCmd::SetTextures { handle, local_id, .. }
            | BuildCmd::SetExtraParam { handle, local_id, .. }
            | BuildCmd::RemoveTaskInventory { handle, local_id, .. }
            | BuildCmd::MoveTaskInventory { handle, local_id, .. }
            | BuildCmd::UpdateTaskInventory { handle, local_id, .. }
            | BuildCmd::RezScript { handle, local_id, .. } => local(*handle, *local_id),
            BuildCmd::Select { handle, local_ids }
            | BuildCmd::Link { handle, local_ids }
            | BuildCmd::Delink { handle, local_ids }
            | BuildCmd::DeRez { handle, local_ids, .. }
            | BuildCmd::SetShape { handle, local_ids, .. }
            | BuildCmd::SetMaterial { handle, local_ids, .. }
            | BuildCmd::SetPermissions { handle, local_ids, .. }
            | BuildCmd::SetGroup { handle, local_ids, .. }
            | BuildCmd::SetOwner { handle, local_ids, .. }
            | BuildCmd::SetSaleInfo { handle, local_ids, .. }
            | BuildCmd::SetClickAction { handle, local_ids, .. }
            | BuildCmd::SetIncludeInSearch { handle, local_ids, .. } => local_ids.iter().all(|id| local(*handle, *id)),
            BuildCmd::SpinStart { object_id, .. }
            | BuildCmd::SpinUpdate { object_id, .. }
            | BuildCmd::ScriptReset { object_id, .. }
            | BuildCmd::SetScriptRunning { object_id, .. } => uuid(object_id),
            BuildCmd::Undo { ids, .. } | BuildCmd::Redo { ids, .. } => ids.iter().all(uuid),
            BuildCmd::Cap { cap, body, .. } if cap == "ModifyMaterialParams" => {
                body.as_array().iter().all(|entry| uuid(&entry["object_id"].as_uuid()))
            }
            BuildCmd::Cap { cap, body, .. } if cap == "ObjectMedia" && body["verb"].as_str() == "UPDATE" => {
                uuid(&body["object_id"].as_uuid())
            }
            BuildCmd::Cap {
                handle,
                cap,
                put: true,
                body,
                ..
            } if cap == "RenderMaterials" => crate::scene::legacy_mat::unzip_llsd(body["Zipped"].as_binary()).is_some_and(|body| {
                body["FullMaterialsPerFace"]
                    .as_array()
                    .iter()
                    .all(|entry| local(*handle, entry["ID"].as_i32() as u32))
            }),
            BuildCmd::DropAttachment { handle, local_ids } => self.rlva_can_drop(*handle, local_ids),
            // Creation, terrain, buying, read requests and release messages
            // retain their own permissions; @edit is not a rez restriction.
            _ => true,
        }
    }

    /// LLAttachmentDrop::handleEvent (llviewermenu.cpp): dropping takes the
    /// attachment off, so a locked one stays worn, and @rez forbids putting
    /// it in-world. The menus grey Drop on the same answer.
    pub fn rlva_can_drop(&self, handle: RegionHandle, local_ids: &[u32]) -> bool {
        if !self.rlva.enabled() {
            return true;
        }
        !self.rlva.has("rez")
            && local_ids.iter().all(|&local_id| {
                self.objects
                    .index_of(&ObjKey { region: handle, local_id })
                    .and_then(|i| self.objects.get(i))
                    .and_then(|o| o.attachment_item_id())
                    .is_none_or(|item| self.rlva_can_remove(item, uuid::Uuid::nil()))
            })
    }
}
