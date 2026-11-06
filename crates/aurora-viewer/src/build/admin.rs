//! Advanced › Admin › Object on the selection (llviewermenu.cpp:
//! force_take_copy, handle_object_lock; the god requests themselves are
//! sent by `app/admin_status.rs`).

use super::BuildTool;
use crate::world::World;
use aurora_net::RegionHandle;
use aurora_net::build::{BuildCmd, derez, perm, perm_field};

impl BuildTool {
    /// The selected roots' local ids, by region (SEND_ONLY_ROOTS).
    pub fn admin_selection(&self, world: &World) -> Vec<(RegionHandle, Vec<u32>)> {
        let roots = self.roots(world);
        let mut out: Vec<_> = self.group_by_region(world, &roots).into_iter().collect();
        out.sort_by_key(|(handle, _)| *handle);
        out
    }

    /// Take Copy (force_take_copy): into our Objects folder whatever its
    /// permissions (DRD_FORCE_TO_GOD_INVENTORY).
    pub fn force_take_copy(&mut self, world: &World) {
        let objects = Self::folder_of_type(world, 6);
        for (handle, ids) in self.admin_selection(world) {
            self.send(BuildCmd::DeRez {
                handle,
                local_ids: ids,
                destination: derez::FORCE_TO_GOD_INVENTORY,
                destination_id: objects,
                group_id: world.groups.active,
            });
        }
    }

    /// Lock (handle_object_lock): the owner's modify right taken away.
    pub fn lock_selection(&mut self, world: &World) {
        for (handle, ids) in self.admin_selection(world) {
            self.send(BuildCmd::SetPermissions {
                handle,
                local_ids: ids,
                field: perm_field::OWNER,
                set: false,
                mask: perm::MODIFY,
            });
        }
    }
}
