//! The pathfinding console's View tab in the scene: which prims are drawn
//! again as a pathfinding shape (and in which colour), what World off and
//! Movables only leave out, and X-ray. Port of the shapes and world options
//! of LLFloaterPathfindingConsole (isRenderWorld, isRenderWorldMovablesOnly,
//! getRenderShapeFlags, isRenderXRay; the pipeline.cpp pathfinding pass;
//! gPipeline.hidePermanentObjects), indra/newview, originally LGPL 2.1.
//!
//! Without Havok, the shapes are not the baked navmesh's: they are each
//! prim's collision shape (GetObjectPhysicsData's PhysicsShapeType, built
//! by `physics_shapes`), which is what the navmesh is made from, tinted by
//! the linkset's current use (RegionObjects). A prim whose shape type is
//! not known yet, or that has no shape of its own (a sculpt, a flexible
//! prim), is drawn with its visible faces meanwhile.
use crate::world::World;
use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PathfindingView {
    /// World unticked: only the shapes (and the terrain they lie on, and
    /// HUDs) are drawn.
    pub world_hidden: bool,
    /// Movables only: the permanent objects (the navmesh's) are not drawn.
    pub movables_only: bool,
    /// Prim index → its shape list (`aurora_render::PATHFINDING_*`).
    pub overlay: HashMap<usize, usize>,
    /// Prim index → its known physics shape type (0 prim, 1 none, 2
    /// convex hull): drawn as that shape, not with its visible faces.
    pub physics: HashMap<usize, u8>,
    pub xray: bool,
    /// Changed whenever the above changes: the hidden flags are applied
    /// again (`Scene::sync_photo_isolation`).
    pub version: u64,
}

impl PathfindingView {
    pub fn active(&self) -> bool {
        self.world_hidden || self.movables_only || !self.overlay.is_empty()
    }

    /// Whether the View tab leaves this object out of the drawing (never a
    /// HUD: the caller keeps those).
    pub fn hides(&self, world: &World, idx: usize) -> bool {
        if self.world_hidden {
            // a shape drawn from its physics leaves its object hidden too
            return !self.overlay.contains_key(&idx) || self.physics.contains_key(&idx);
        }
        self.movables_only && permanent(world, idx)
    }
}

/// flagObjectPermanent: its linkset shapes the navmesh (FLAGS_AFFECTS_NAVMESH
/// on the root). Avatars and what they wear never are.
fn permanent(world: &World, idx: usize) -> bool {
    let mut root = idx;
    for _ in 0..64 {
        let Some(object) = world.objects.get(root) else { return false };
        if object.is_avatar() {
            return false;
        }
        match world.objects.parent_of(object) {
            Some(parent) if world.objects.get(parent).is_some_and(|p| p.is_avatar()) => return false,
            Some(parent) => root = parent,
            None => break,
        }
    }
    world
        .objects
        .get(root)
        .is_some_and(|o| o.update_flags & crate::ui::context::flags::AFFECTS_NAVMESH != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::context::flags;

    fn world() -> World {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for ev in crate::demo::events() {
            w.apply(ev);
        }
        w
    }

    #[test]
    fn world_off_keeps_only_the_shapes_and_movables_only_hides_the_permanent() {
        let mut w = world();
        let roots: Vec<usize> = w
            .objects
            .iter()
            .filter(|(_, o)| !o.is_avatar() && w.objects.parent_of(o).is_none())
            .map(|(i, _)| i)
            .take(2)
            .collect();
        let [shape, other] = roots[..] else { panic!("two rezzed objects") };
        let avatar = w.objects.iter().find(|(_, o)| o.is_avatar()).map(|(i, _)| i).expect("an avatar");

        let mut view = PathfindingView::default();
        assert!(!view.active() && !view.hides(&w, shape));
        view.world_hidden = true;
        view.overlay.insert(shape, aurora_render::PATHFINDING_WALKABLE);
        assert!(view.active());
        assert!(!view.hides(&w, shape), "a shape stays");
        assert!(view.hides(&w, other) && view.hides(&w, avatar), "the rest of the world goes");
        view.physics.insert(shape, 0);
        assert!(view.hides(&w, shape), "drawn as its physics shape: its own faces go");

        let mut view = PathfindingView {
            movables_only: true,
            ..Default::default()
        };
        w.objects.get_mut(other).unwrap().update_flags = flags::AFFECTS_NAVMESH;
        w.objects.get_mut(shape).unwrap().update_flags = 0;
        assert!(view.hides(&w, other), "permanent: part of the navmesh");
        assert!(!view.hides(&w, shape), "movable");
        assert!(!view.hides(&w, avatar), "avatars are never permanent");
        view.movables_only = false;
        assert!(!view.hides(&w, other));
    }
}
