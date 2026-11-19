//! RlvActions::canEdit, enforced before local edits and network writes.
use super::super::{World, objects::ObjKey};
use aurora_net::{
    RegionHandle,
    build::{BuildCmd, derez},
};

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

    /// enableLinkObjects: a locked seat cannot be relinked. The regular
    /// RlvActions::canEdit policy remains enforced for every selected root.
    pub fn rlva_can_link(&self, roots: &[usize]) -> bool {
        if !self.rlva.enabled() {
            return true;
        }
        let locked_seat = self.rlva.has("unsit") && self.agent.seated;
        let seat = locked_seat.then(|| self.rlva_sit_id());
        roots
            .iter()
            .all(|&i| self.rlva_can_edit(i) && self.objects.get(i).is_some_and(|o| Some(o.full_id) != seat))
    }

    /// The root of `index`'s linkset when the agent sits on it under @unsit
    /// (RlvSelectIsSittingOn with RlvActions::canStand, rlvcommon.cpp).
    pub fn rlva_locked_seat(&self, index: usize) -> bool {
        self.rlva.has("unsit")
            && self.agent.seated
            && self
                .objects
                .get(self.objects.linkset_root(index))
                .is_some_and(|o| o.full_id == self.rlva_sit_id())
    }

    /// derenderObject (llviewermenu.cpp) asked of the seat held by @unsit:
    /// the prims that still go, or `None` when `index` is not of that seat.
    /// The prim sat on, the root of its linkset, stays in the world: the
    /// Legacy Viewer takes it alone out of the selection
    /// (deselectObjectOnly), and its loop then goes through the prims
    /// linked to it one by one (getFirstRootObject(true) falls back on
    /// them). `part`: one prim was asked, not its linkset.
    pub fn rlva_seat_derender(&self, index: usize, part: bool) -> Option<Vec<uuid::Uuid>> {
        if !self.rlva.enabled() || !self.rlva_locked_seat(index) {
            return None;
        }
        let root = self.objects.linkset_root(index);
        let prim = |i: usize| self.objects.get(i).filter(|o| !o.is_avatar()).map(|o| o.full_id);
        Some(if part {
            prim(index).filter(|_| index != root).into_iter().collect()
        } else {
            self.objects
                .subtree(root)
                .into_iter()
                .filter(|&i| i != root && self.objects.get(i).is_some_and(|o| self.objects.parent_of(o) == Some(root)))
                .filter_map(prim)
                .collect()
        })
    }

    /// RlvActions::canBuild (rlvactions.cpp), asked by LLToolMgr::canEdit
    /// (lltoolmgr.cpp): the build window opens as long as editing or rezzing
    /// is left to the agent; under both @edit and @rez its entries are greyed.
    pub fn rlva_can_build(&self) -> bool {
        !(self.rlva.has("edit") && self.rlva.has("rez"))
    }

    /// object_selected_and_point_valid (llviewermenu.cpp), "Wear", "Add" and
    /// "Attach to" on a rezzed object: attaching takes it from the world,
    /// which @rez forbids. On the default point (`point` 0) no attachment
    /// lock of any kind may hold (hasLockedAttachmentPoint(RLV_LOCK_ANY)); a
    /// chosen point must take attachments (canAttach).
    pub fn rlva_can_attach_object(&self, point: u8) -> bool {
        if !self.rlva.enabled() {
            return true;
        }
        if self.rlva.has("rez") {
            return false;
        }
        if point == 0 {
            !self.rlva_has_locked_attachment() && self.rlva.rules_for("addattach", uuid::Uuid::nil()).next().is_none()
        } else {
            self.rlva_point_addable(point)
        }
    }

    /// The @fartouch distance, which stands for MaxSelectDistance while the
    /// build tools are open (LLSelectMgr::deselectAllIfTooFar, llselectmgr.cpp).
    pub fn rlva_select_limit(&self) -> Option<f32> {
        let max = self.rlva.distance_limit("fartouch", 1.5, uuid::Uuid::nil());
        max.is_finite().then_some(max)
    }

    /// LLToolPlacer::raycastForNewObjPos (lltoolplacer.cpp): under @fartouch
    /// the Create tool places nothing farther than its distance.
    pub fn rlva_can_place_at(&self, point: glam::Vec3) -> bool {
        self.rlva_select_limit()
            .is_none_or(|max| point.distance(self.agent.position) <= max)
    }

    /// LLToolSelect::handleObjectSelection (lltoolselect.cpp): under
    /// @fartouch nothing farther than its distance is selected, what the
    /// agent wears aside. The distance is the object's own place; a right
    /// click on a surface within 1.5 m still gives the menu its selection
    /// (`ObjectMenu::near`), and such a click with the build tools closes
    /// them.
    pub fn rlva_within_fartouch(&self, index: usize) -> bool {
        let Some(max) = self.rlva_select_limit() else {
            return true;
        };
        if self
            .objects
            .get(index)
            .is_some_and(|o| self.rlva_own_attachment_root(o.full_id).is_some())
        {
            return true;
        }
        crate::scene::Scene::object_transform_unshifted(self, index, std::time::Instant::now())
            .is_some_and(|(position, _, _)| position.distance(self.agent.position) <= max)
    }

    /// LLSelectMgr::enableUnlinkObjects (llselectmgr.cpp): the linkset the
    /// agent cannot stand up from stays whole. Unlink is greyed and refused
    /// on the same answer.
    pub fn rlva_can_unlink(&self, index: usize) -> bool {
        !self.rlva.enabled() || !self.rlva_locked_seat(index)
    }

    /// Whether the agent sits on `index`'s linkset while @unsit or @sittp
    /// holds: the restraining seat. Its contents stay as they are, and it
    /// can be neither moved, rotated nor stretched
    /// (LLSelectMgr::selectGetEditMoveLinksetPermissions, llselectmgr.cpp:
    /// move = modify = false).
    pub fn rlva_restrained_seat(&self, index: usize) -> bool {
        if !self.rlva.enabled() || !self.agent.seated || !(self.rlva.has("unsit") || self.rlva.has("sittp")) {
            return false;
        }
        let seat = self.rlva_sit_id();
        !seat.is_nil()
            && self
                .objects
                .get(self.objects.linkset_root(index))
                .is_some_and(|o| o.full_id == seat)
    }

    /// gRlvAttachmentLocks.isLockedAttachment(getRootEdit()) (rlvlocks.h):
    /// `index` belongs to a worn object that may not be taken off (its own
    /// @detach, a locked point, a locked folder).
    pub fn rlva_locked_attachment(&self, index: usize) -> bool {
        if !self.rlva.enabled() {
            return false;
        }
        let root = self.objects.linkset_root(index);
        self.objects.get(root).is_some_and(|o| {
            self.objects
                .parent_of(o)
                .and_then(|i| self.objects.get(i))
                .is_some_and(|p| p.is_avatar())
                && self
                    .rlva_attachment_item(o.full_id)
                    .is_some_and(|item| !self.rlva_can_remove(item, uuid::Uuid::nil()))
        })
    }

    /// The contents of `index`'s object are locked: nothing is added to,
    /// removed from or moved out of a locked attachment or the restraining
    /// seat, whose script holds the restriction
    /// (LLTaskInvFVBridge::isItemRemovable and isItemMovable, and
    /// LLPanelContents::getState, llpanelobjectinventory.cpp and
    /// llpanelcontents.cpp; LLToolDragAndDrop::willObjectAcceptInventory,
    /// lltooldraganddrop.cpp). The items already inside are another rule:
    /// `BuildTool::contents_items_editable`.
    pub fn rlva_contents_locked(&self, index: usize) -> bool {
        self.rlva_locked_attachment(index) || self.rlva_restrained_seat(index)
    }

    /// rlvCanDeleteOrReturn (rlvcommon.cpp): deleting, returning, taking or
    /// duplicating an object (LLSelectMgr::selectDelete and selectDuplicate,
    /// handle_take, LLViewerObject::isReturnable). An edit lock on the object
    /// keeps it in place; under @edit or @rez so does whatever the agent
    /// owns, may modify or holds through a group, since it could not be put
    /// back; under @unsit so does the linkset the agent sits on. The menus
    /// grey their entries on the same answer. Nothing is asked unless
    /// @edit, @rez, @unsit or an @editobj lock is in force: @editworld and
    /// @editattach alone keep no object in place.
    pub fn rlva_can_delete_or_return(&self, index: usize) -> bool {
        use crate::ui::context::flags::{OBJECT_GROUP_OWNED, OBJECT_MODIFY, OBJECT_YOU_OWNER};
        if !self.rlva.enabled() {
            return true;
        }
        let asked = ["edit", "rez", "unsit"].iter().any(|name| self.rlva.has(name))
            || self.rlva.rules_for("editobj", uuid::Uuid::nil()).next().is_some();
        if !asked {
            return true;
        }
        let ours = self
            .objects
            .get(self.objects.linkset_root(index))
            .is_none_or(|o| o.update_flags & (OBJECT_YOU_OWNER | OBJECT_MODIFY | OBJECT_GROUP_OWNED) != 0);
        self.rlva_can_edit(index) && !((self.rlva.has("edit") || self.rlva.has("rez")) && ours) && !self.rlva_locked_seat(index)
    }

    /// handle_take_copy (llviewermenu.cpp): a copy leaves the object where it
    /// is, so only the seat the agent cannot leave refuses it.
    pub fn rlva_can_take_copy(&self, index: usize) -> bool {
        !self.rlva.enabled() || !self.rlva_locked_seat(index)
    }

    /// `index` is a prim of the viewer's own bridge as we wear it.
    fn rlva_viewer_bridge_object(&self, index: usize) -> bool {
        let root = self.objects.linkset_root(index);
        self.objects.get(root).is_some_and(|o| {
            self.objects
                .parent_of(o)
                .and_then(|i| self.objects.get(i))
                .is_some_and(|p| p.full_id == self.agent_id)
                && self
                    .rlva_attachment_item(o.full_id)
                    .is_some_and(|item| crate::world::bridge::viewer_bridge(self, item, o.attachment_point()))
        })
    }

    pub fn rlva_allows_build(&self, command: &BuildCmd) -> bool {
        if !self.rlva.enabled() {
            return true;
        }
        let index = |handle: RegionHandle, id| {
            self.objects.index_of(&ObjKey {
                region: handle,
                local_id: id,
            })
        };
        // The viewer shapes its own bridge and writes and starts its script
        // whatever holds Center 2 or forbids editing: FSLSLBridge sends
        // those messages itself, past RLVa (indra/newview/fslslbridge.cpp).
        let bridge = |i: usize| self.rlva_viewer_bridge_object(i);
        let upkeep = match command {
            BuildCmd::Transform { handle, updates } => {
                !updates.is_empty() && updates.iter().all(|u| index(*handle, u.local_id).is_some_and(bridge))
            }
            BuildCmd::SetShape { handle, local_ids, .. } => {
                !local_ids.is_empty() && local_ids.iter().all(|id| index(*handle, *id).is_some_and(bridge))
            }
            BuildCmd::SetTextures { handle, local_id, .. } | BuildCmd::RezScript { handle, local_id, .. } => {
                index(*handle, *local_id).is_some_and(bridge)
            }
            BuildCmd::SetScriptRunning { object_id, .. } => self.objects.index_of_uuid(object_id).is_some_and(bridge),
            _ => false,
        };
        if upkeep {
            return true;
        }
        let local = |handle: RegionHandle, id| index(handle, id).is_some_and(|i| self.rlva_can_edit(i));
        let uuid = |id: &uuid::Uuid| self.objects.index_of_uuid(id).is_some_and(|i| self.rlva_can_edit(i));
        // The contents of a locked attachment and of the restraining seat
        // keep the script that restrains (`rlva_contents_locked`).
        let contents = |i: usize| self.rlva_can_edit(i) && !self.rlva_contents_locked(i);
        match command {
            BuildCmd::Link { handle, local_ids } => {
                let roots: Option<Vec<_>> = local_ids
                    .iter()
                    .map(|&local_id| self.objects.index_of(&ObjKey { region: *handle, local_id }))
                    .collect();
                roots.is_some_and(|roots| self.rlva_can_link(&roots))
            }
            BuildCmd::Transform { handle, updates } => updates
                .iter()
                .all(|u| index(*handle, u.local_id).is_some_and(|i| self.rlva_can_edit(i) && !self.rlva_restrained_seat(i))),
            BuildCmd::SetDescription { handle, local_ids, .. } => local_ids.iter().all(|id| local(*handle, *id)),
            BuildCmd::SetFlags { handle, local_id, .. }
            | BuildCmd::SetPhysicsParams { handle, local_id, .. }
            | BuildCmd::SetTextures { handle, local_id, .. }
            | BuildCmd::SetExtraParam { handle, local_id, .. } => local(*handle, *local_id),
            BuildCmd::RemoveTaskInventory { handle, local_id, .. }
            | BuildCmd::MoveTaskInventory { handle, local_id, .. }
            | BuildCmd::RezScript { handle, local_id, .. } => index(*handle, *local_id).is_some_and(contents),
            // The same message puts an inventory item into the object (a
            // drop, refused like the rest) and changes one already inside:
            // its name, which LLTaskInvFVBridge::renameItem keeps in a
            // locked attachment only, or its properties, which
            // LLSidepanelItemInfo changes whatever holds the object. The
            // windows apply those two rules; here a change is told from a
            // drop by its item, which is not one of our inventory.
            BuildCmd::UpdateTaskInventory { handle, local_id, item } => index(*handle, *local_id).is_some_and(|i| {
                self.rlva_can_edit(i) && (!self.inventory.items.contains_key(&item.item_id) || !self.rlva_contents_locked(i))
            }),
            BuildCmd::SetName { handle, local_ids, .. } => local_ids.iter().all(|id| local(*handle, *id)),
            // LLSelectMgr::enableUnlinkObjects (llselectmgr.cpp): under
            // @unsit the linkset sat on stays whole.
            BuildCmd::Delink { handle, local_ids } => local_ids
                .iter()
                .all(|id| index(*handle, *id).is_some_and(|i| self.rlva_can_edit(i) && self.rlva_can_unlink(i))),
            // (a right click selects what it may not edit for the time of
            // its menu: LLToolSelect::handleObjectSelection, temp_select)
            BuildCmd::Select { .. } => true,
            BuildCmd::SetShape { handle, local_ids, .. }
            | BuildCmd::SetMaterial { handle, local_ids, .. }
            | BuildCmd::SetPermissions { handle, local_ids, .. }
            | BuildCmd::SetGroup { handle, local_ids, .. }
            | BuildCmd::SetOwner { handle, local_ids, .. }
            | BuildCmd::SetSaleInfo { handle, local_ids, .. }
            | BuildCmd::SetClickAction { handle, local_ids, .. }
            | BuildCmd::SetIncludeInSearch { handle, local_ids, .. } => local_ids.iter().all(|id| local(*handle, *id)),
            BuildCmd::DeRez {
                handle,
                local_ids,
                destination,
                ..
            } => local_ids.iter().all(|id| {
                index(*handle, *id).is_some_and(|i| match *destination {
                    derez::TRASH | derez::TAKE_INTO_AGENT_INVENTORY | derez::RETURN_TO_OWNER | derez::RETURN_TO_LAST_OWNER => {
                        self.rlva_can_delete_or_return(i)
                    }
                    // handle_take_copy and enable_object_take_copy
                    // (llviewermenu.cpp) keep the seat alone: an edit lock
                    // does not refuse a copy.
                    derez::ACQUIRE_TO_AGENT_INVENTORY => self.rlva_can_take_copy(i),
                    _ => self.rlva_can_edit(i),
                })
            }),
            // LLSelectMgr::selectDuplicate (Ctrl+D, a shift-drag): the
            // original answers like a delete, and nothing else is asked.
            BuildCmd::Duplicate { handle, local_ids, .. } => local_ids
                .iter()
                .all(|id| index(*handle, *id).is_some_and(|i| self.rlva_can_delete_or_return(i))),
            // A copy placed with the Create tool is LLToolPlacer::placeObject,
            // which @rez and @interact refuse and which asks nothing of the
            // original (LLSelectMgr::selectDuplicateOnRay).
            BuildCmd::DuplicateOnRay { .. } => self.rlva_can_rez(None),
            BuildCmd::SpinStart { object_id, .. } | BuildCmd::SpinUpdate { object_id, .. } => uuid(object_id),
            // handle_selected_script_action (llviewermenu.cpp): the scripts
            // of a locked attachment are neither reset nor stopped.
            BuildCmd::ScriptReset { object_id, .. } | BuildCmd::SetScriptRunning { object_id, .. } => self
                .objects
                .index_of_uuid(object_id)
                .is_some_and(|i| self.rlva_can_edit(i) && !self.rlva_locked_attachment(i)),
            BuildCmd::CreateTaskItem { item, .. } => self.objects.index_of_uuid(&item.parent_id).is_some_and(contents),
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
            // the point's ATTACHMENT_ADD bit is not part of its number
            BuildCmd::Attach { point, .. } => self.rlva_can_attach_object(*point & 0x7f),
            // Creation, terrain, buying, read requests and release messages
            // retain their own permissions; @edit is not a rez restriction.
            _ => true,
        }
    }

    /// Whether an object may be rezzed from the inventory or created by the
    /// build tools: on the ground or on an object (`into` is None), or put
    /// into the contents of `into` with Ctrl. LLToolPlacer::placeObject,
    /// LLToolDragAndDrop::dad3dRezObjectOnLand and dropObject refuse under
    /// @rez or @interact; dad3dRezObjectOnObject lets a Ctrl drop through
    /// @rez, and through @interact only onto one of our HUDs.
    pub fn rlva_can_rez(&self, into: Option<usize>) -> bool {
        match into {
            None => !self.rlva.has("rez") && !self.rlva.has("interact"),
            Some(index) => !self.rlva.has("interact") || self.objects.worn_on_hud(index),
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
