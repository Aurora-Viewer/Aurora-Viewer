//! LLSelectMgr::enableLinkObjects / linkObjects (indra/newview/llselectmgr.cpp):
//! enabling needs one editable root; execution checks every root and uses the
//! selection's ObjectProperties, never the sound/particle owner in ObjectUpdate.
use super::*;
use crate::ui::context::flags::{AFFECTS_NAVMESH, OBJECT_MODIFY};

fn permanent(world: &World, index: usize) -> bool {
    // LLViewerObject::isPermanentEnforced. This viewer has no god mode.
    world
        .objects
        .get(index)
        .is_none_or(|o| o.update_flags & AFFECTS_NAVMESH != 0 && world.main_region != Some(o.key.region))
}

impl BuildTool {
    fn link_properties_ready(&self, world: &World, roots: &[usize]) -> bool {
        roots.iter().all(|&i| {
            world
                .objects
                .get(i)
                .is_some_and(|o| self.link_properties.contains(&o.full_id) && self.props.get(&o.full_id).is_some_and(|p| p.full))
        })
    }

    /// Scene metadata is read once before this frame's editor actions; rendering
    /// and linking use the same mesh headers, without asset reads or GPU waits.
    pub fn refresh_link_geometry(&mut self, world: &World, scene: &Scene) {
        self.link_triangles.clear();
        if !self
            .selection
            .iter()
            .filter_map(|key| world.objects.index_of(key))
            .any(|i| animated(world, root_of(world, i)))
        {
            return;
        }
        for index in self.roots(world).into_iter().flat_map(|i| members(world, i)) {
            if let Some(mesh) = world
                .objects
                .get(index)
                .and_then(|o| o.volume.sculpt.filter(|_| o.volume.is_mesh()))
                .map(|s| s.texture)
            {
                self.link_triangles
                    .entry(mesh)
                    .or_insert_with(|| scene.meshes.estimated_max_triangles(&mesh));
            }
        }
    }

    fn animated_link_allowed(&self, world: &World, roots: &[usize]) -> bool {
        if !roots.iter().any(|&i| animated(world, i)) {
            return true;
        }
        let triangles: f32 = roots
            .iter()
            .flat_map(|&i| members(world, i))
            .filter_map(|i| {
                let o = world.objects.get(i)?;
                o.volume
                    .sculpt
                    .filter(|_| o.volume.is_mesh())
                    .map(|s| self.link_triangles.get(&s.texture).copied().unwrap_or(0.0))
            })
            .sum();
        triangles <= world.main().map_or(0, |r| r.animated_object_max_tris) as f32
    }

    /// Same enable rule for both editor styles and the object menu.
    pub fn link_allowed(&self, world: &World, roots: &[usize]) -> bool {
        roots.len() >= 2
            && self.link_properties_ready(world, roots)
            && roots.iter().any(|&i| {
                world
                    .objects
                    .get(i)
                    .is_some_and(|o| o.update_flags & OBJECT_MODIFY != 0 && !permanent(world, i))
            })
            && self.animated_link_allowed(world, roots)
            && world.rlva_can_link(roots)
    }

    pub fn can_link(&self, world: &World) -> bool {
        !self.selection_linked && !self.attachment_selected(world) && self.link_allowed(world, &self.roots(world))
    }

    fn refuse_link(&mut self, message: String) {
        self.status.clone_from(&message);
        self.ui.link_error = Some(message);
    }

    /// Last selected root first, as LLObjectSelection::addNode / sendLink.
    pub fn link(&mut self, world: &mut World) {
        if self.selection_linked || self.attachment_selected(world) {
            return;
        }
        let roots = self.roots(world);
        let error = if !self.link_properties_ready(world, &roots) {
            Some(aurora_i18n::tr!("build-link-downloading").into())
        } else {
            let total: usize = roots.iter().map(|&r| members(world, r).len()).sum();
            let limit = world.link_limit();
            if total > limit {
                Some(aurora_i18n::tr!(
                    "build-link-too-many",
                    count = total.to_string(),
                    max = limit.to_string()
                ))
            } else if roots.len() < 2 {
                Some(aurora_i18n::tr!("build-mod-select-at-least-two-objects-to-link").into())
            } else if roots
                .iter()
                .any(|&i| world.objects.get(i).is_none_or(|o| o.update_flags & OBJECT_MODIFY == 0))
            {
                Some(aurora_i18n::tr!("build-link-modify").into())
            } else if roots.iter().any(|&i| permanent(world, i)) {
                Some(aurora_i18n::tr!("build-link-permanent").into())
            } else if !world.rlva_can_link(&roots) {
                Some(aurora_i18n::tr!("build-link-restricted").into())
            } else {
                let owners: Vec<_> = roots
                    .iter()
                    .filter_map(|&i| world.objects.get(i).and_then(|o| self.props.get(&o.full_id)))
                    // LLSelectGetFirstOwner: a group-owned object's OwnerID
                    // is null; its permissions identify the group as owner.
                    .map(|p| if p.owner_id.is_nil() { p.group_id } else { p.owner_id })
                    .collect();
                let regions: Vec<_> = roots.iter().filter_map(|&i| world.objects.get(i).map(|o| o.key.region)).collect();
                if owners.first().is_none_or(Uuid::is_nil) || owners.iter().any(|owner| Some(owner) != owners.first()) {
                    Some(aurora_i18n::tr!("build-link-owners").into())
                } else if regions.iter().any(|region| Some(region) != regions.first()) {
                    Some(aurora_i18n::tr!("build-link-regions").into())
                } else if !self.animated_link_allowed(world, &roots) {
                    Some(aurora_i18n::tr!("build-link-animesh").into())
                } else {
                    None
                }
            }
        };
        if let Some(error) = error {
            self.refuse_link(error);
            return;
        }
        let root = roots[0];
        let Some(handle) = world.objects.get(root).map(|o| o.key.region) else {
            return;
        };
        let ids = roots.iter().filter_map(|&i| world.objects.get(i).map(|o| o.key.local_id)).collect();
        // Each subsequent packet inserts its children before those already on
        // the root. Mirror sendListToRegions' packet order in our confirmation.
        let batches = aurora_net::build::link_batches(&roots);
        let mut expected = vec![root];
        for batch in batches.iter().rev() {
            expected.extend(batch.iter().skip(1).flat_map(|&r| members(world, r)));
        }
        expected.extend(members(world, root).into_iter().skip(1));
        self.pending_link = Some((
            expected
                .into_iter()
                .filter_map(|i| world.objects.get(i).map(|o| o.full_id))
                .collect(),
            Instant::now(),
        ));
        self.ui.link_error = None;
        self.send(BuildCmd::Link { handle, local_ids: ids });
        self.status = aurora_i18n::tr!("build-link-pending").into();
        self.costs_stale();
    }
}

fn animated(world: &World, index: usize) -> bool {
    world
        .objects
        .get(index)
        .is_some_and(|o| o.extra.extended_mesh_flags.unwrap_or(0) & aurora_prim::extra::EXTENDED_MESH_ANIMATED != 0)
}

/// Link counts include every descendant, even on grids allowing more than
/// the editor's ordinary 4,096-member traversal. A malformed cycle ends once.
pub(super) fn members(world: &World, root: usize) -> Vec<usize> {
    let mut result = vec![root];
    let mut seen = HashSet::from([root]);
    let mut cursor = 0;
    while cursor < result.len() {
        if let Some(object) = world.objects.get(result[cursor]) {
            result.extend(
                world
                    .objects
                    .children_of(&object.key)
                    .iter()
                    .copied()
                    .filter(|&child| world.objects.get(child).is_some_and(|o| !o.is_avatar()) && seen.insert(child)),
            );
        }
        cursor += 1;
    }
    result
}

#[cfg(test)]
mod tests;
