//! Temporary attachments have no inventory record. Their point and source
//! locks still apply, and detaching addresses the object in its simulator.
use super::super::{World, objects::ObjKey};
use uuid::Uuid;

impl World {
    /// RlvAttachmentLocks::hasLockedHUD / RlvActions::canViewWireframe:
    /// actual worn HUDs count, including temporary and folder-locked ones.
    pub fn rlva_has_locked_hud(&self) -> bool {
        let locks = super::inventory::FolderLocks::default();
        self.rlva.inventory_locked()
            && self.rlva_worn_attachment_points().into_iter().any(|(item, point)| {
                self.avatar_lib.attach_points.get(&point).is_some_and(|point| point.hud)
                    && (!self.rlva_attachment_remove_at(item, point, Uuid::nil())
                        || self
                            .inventory
                            .items
                            .get(&item)
                            .is_some_and(|item| !self.rlva_folder_allowed_in(&locks, item.parent, false, Uuid::nil())))
            })
    }

    /// Resolve only a real linkset worn by this agent, including child prims.
    pub(crate) fn rlva_own_attachment_root(&self, source: Uuid) -> Option<Uuid> {
        let mut index = self.objects.index_of_uuid(&source)?;
        for _ in 0..64 {
            let object = self.objects.get(index)?;
            let parent = self
                .objects
                .parent_of(object)
                .and_then(|index| self.objects.get(index).map(|parent| (index, parent)))?;
            if parent.1.is_avatar() {
                return (parent.1.full_id == self.agent_id && object.attachment_point() != 0).then_some(object.full_id);
            }
            index = parent.0;
        }
        None
    }

    pub(super) fn rlva_worn_attachment_points(&self) -> std::collections::HashMap<Uuid, u8> {
        let Some(agent) = self.objects.index_of_uuid(&self.agent_id).and_then(|i| self.objects.get(i)) else {
            return Default::default();
        };
        self.objects
            .children_of(&agent.key)
            .iter()
            .filter_map(|i| self.objects.get(*i))
            .filter(|o| o.attachment_point() != 0)
            .map(|o| {
                (
                    o.attachment_item_id().filter(|id| !id.is_nil()).unwrap_or(o.full_id),
                    o.attachment_point(),
                )
            })
            .collect()
    }

    pub fn rlva_temporary_attachment(&self, item: Uuid) -> Option<ObjKey> {
        if self.inventory.items.contains_key(&item) {
            return None;
        }
        let agent = self.objects.index_of_uuid(&self.agent_id).and_then(|i| self.objects.get(i))?;
        self.objects
            .children_of(&agent.key)
            .iter()
            .filter_map(|i| self.objects.get(*i))
            .find(|o| o.full_id == item && o.attachment_point() != 0 && o.attachment_item_id().is_none_or(|id| id.is_nil() || id == item))
            .map(|o| o.key)
    }

    pub(super) fn rlva_attachment_remove_at(&self, item: Uuid, point: u8, actor: Uuid) -> bool {
        !self.rlva_point_locked("remattach", point, actor)
            && !self.rlva.rules_for("detach", Uuid::nil()).any(|(source, option)| {
                if option.is_empty() {
                    self.rlva_attachment_item(source) == Some(item)
                } else {
                    super::inventory::point(option) == Some(point)
                }
            })
    }

    pub(super) fn rlva_temporary_removable(&self, item: Uuid, actor: Uuid) -> bool {
        let point = self
            .rlva_temporary_attachment(item)
            .and_then(|key| self.objects.index_of(&key))
            .and_then(|i| self.objects.get(i))
            .map(|o| o.attachment_point())
            .or_else(|| self.rlva.watchdog.known.get(&item).filter(|a| a.root == item).map(|a| a.point));
        point.is_some_and(|point| self.rlva_attachment_remove_at(item, point, actor))
    }

    pub(super) fn rlva_unloaded_attachment_removable(&self, item: Uuid, actor: Uuid) -> bool {
        // Manual recovery still honors source/point locks. Without the item
        // we cannot determine its folder locks or scripted nostrip state.
        if !actor.is_nil()
            || [
                "sharedunwear",
                "unsharedunwear",
                "detachthis",
                "detachallthis",
                "detachthis_except",
                "detachallthis_except",
            ]
            .iter()
            .any(|name| self.rlva.has(name))
        {
            return false;
        }
        self.worn_attachment(&item)
            .and_then(|i| self.objects.get(i))
            .is_some_and(|object| object.attachment_point() != 0 && self.rlva_attachment_remove_at(item, object.attachment_point(), actor))
    }

    pub(super) fn rlva_temporary_addable(&self, item: Uuid, point: u8, _actor: Uuid) -> bool {
        let Some(key) = self.rlva_temporary_attachment(item) else {
            return false;
        };
        let point = if point == 0 {
            self.objects
                .index_of(&key)
                .and_then(|i| self.objects.get(i))
                .map_or(0, |o| o.attachment_point())
        } else {
            point
        };
        self.rlva_point_addable(point)
    }
}
