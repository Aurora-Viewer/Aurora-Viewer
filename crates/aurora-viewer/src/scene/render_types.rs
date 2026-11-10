//! Rendering types: the world's kinds of things one can stop drawing, to
//! see what is behind them or find what costs the frame (Advanced ›
//! Rendering types). Port of the render types of the Legacy Viewer's
//! Develop menu (LLPipeline::toggleRenderTypeControl, render_type_from_string
//! in indra/newview/llviewermenu.cpp): not saved, all drawn again at each
//! launch, as there.
//!
//! Each object falls in one kind, as the Legacy Viewer's drawables do:
//! an avatar and everything it wears are Avatars (LLAvatarBridge), an
//! animated object and its linkset Animesh (LLControlAVBridge), Linden
//! trees and grass their own kind (LLVOTree, LLVOGrass), the rest Objects
//! (LLVolumeBridge). A HUD is none of them: it stays. What is hidden is
//! left out of the view, the shadows, the reflections and the picking, the
//! way the photo's isolation leaves an object out (`ObjGpu::hidden`); name
//! tags stay over hidden avatars, as in the Legacy Viewer
//! (LLVOAvatar::idleUpdate still updates them).
//!
//! The terrain, the water, the sky and the clouds are left out of the
//! frame by the app (`app.rs`); the particles keep their own switch
//! (`world_overlays.hide_particles`, Ctrl+Alt+Shift+=), listed with them.

use crate::world::World;
use aurora_prim::params::{LL_PCODE_LEGACY_GRASS, LL_PCODE_LEGACY_TREE};

/// What is not drawn (all false: everything is).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Hidden {
    /// Surface Patch.
    pub terrain: bool,
    pub water: bool,
    pub sky: bool,
    pub clouds: bool,
    /// Volume: prims and meshes that are not worn nor animated.
    pub objects: bool,
    /// Character: avatars and what they wear.
    pub avatars: bool,
    /// Animated objects (controlAV).
    pub animesh: bool,
    pub trees: bool,
    pub grass: bool,
}

/// The kind of an object, for the rendering types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Object,
    Avatar,
    Animesh,
    Tree,
    Grass,
}

impl Hidden {
    /// One of the object kinds is hidden: the objects the sync visits are
    /// flagged again.
    pub fn any_object(&self) -> bool {
        self.objects || self.avatars || self.animesh || self.trees || self.grass
    }

    pub fn hides(&self, kind: Kind) -> bool {
        match kind {
            Kind::Object => self.objects,
            Kind::Avatar => self.avatars,
            Kind::Animesh => self.animesh,
            Kind::Tree => self.trees,
            Kind::Grass => self.grass,
        }
    }

    /// Whether object `index` is left out (never a HUD: the caller keeps
    /// those).
    pub fn hides_object(&self, world: &World, index: usize) -> bool {
        self.any_object() && kind(world, index).is_some_and(|k| self.hides(k))
    }

    /// `AURORA_DEMO_RENDER_TYPES`: the kinds hidden, separated by commas
    /// (`terrain,water,sky,clouds,objects,avatars,animesh,trees,grass`,
    /// `particles`), and whether the particles are among them; unknown
    /// words are left aside.
    pub fn parse(list: &str) -> (Hidden, bool) {
        let mut h = Hidden::default();
        let mut particles = false;
        for word in list.split(',').map(str::trim) {
            match word {
                "terrain" => h.terrain = true,
                "water" => h.water = true,
                "sky" => h.sky = true,
                "clouds" => h.clouds = true,
                "objects" => h.objects = true,
                "avatars" => h.avatars = true,
                "animesh" => h.animesh = true,
                "trees" => h.trees = true,
                "grass" => h.grass = true,
                "particles" => particles = true,
                _ => {}
            }
        }
        (h, particles)
    }
}

/// The kind of object `index`: the skeleton it belongs to decides
/// (`animesh::owner`: an avatar for its body and attachments, an animated
/// linkset root for its prims, a worn one included), then its pcode. None
/// when it is not in the store.
pub fn kind(world: &World, index: usize) -> Option<Kind> {
    let o = world.objects.get(index)?;
    Some(match super::animesh::owner(&world.objects, index) {
        Some((_, owner)) if world.objects.get(owner).is_some_and(|a| a.is_avatar()) => Kind::Avatar,
        Some(_) => Kind::Animesh,
        None if o.pcode == LL_PCODE_LEGACY_TREE => Kind::Tree,
        None if o.pcode == LL_PCODE_LEGACY_GRASS => Kind::Grass,
        None => Kind::Object,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use std::sync::Arc;
    use uuid::Uuid;

    fn fixture() -> World {
        let mut w = World::new(Arc::new(AvatarLibrary::load()));
        for e in crate::demo::events().into_iter().chain(crate::demo::animesh_events()) {
            w.apply(e);
        }
        w
    }

    fn index(w: &World, local: u32) -> usize {
        w.objects
            .iter()
            .find(|(_, o)| o.key.local_id == local)
            .map(|(i, _)| i)
            .expect("fixture object")
    }

    fn add(w: &mut World, local: u32, pcode: u8) -> usize {
        let region = w.objects.get(index(w, 9200)).expect("animesh").key.region;
        let mut u = crate::demo::action_avatar(false);
        u.pcode = pcode;
        u.local_id = local;
        u.full_id = Uuid::from_u128(local as u128);
        u.parent_id = 0;
        w.objects.upsert(region, u);
        index(w, local)
    }

    #[test]
    fn objects_fall_in_the_kind_of_their_skeleton_then_their_pcode() {
        let mut w = fixture();
        // an avatar, its attachment, a ground animesh, a linked prim of
        // another, a worn animated object
        for (local, expected) in [
            (9001, Kind::Avatar),
            (9100, Kind::Avatar),
            (9200, Kind::Animesh),
            (9202, Kind::Animesh),
            (9203, Kind::Animesh),
        ] {
            assert_eq!(kind(&w, index(&w, local)), Some(expected), "object {local}");
        }
        let tree = add(&mut w, 9300, LL_PCODE_LEGACY_TREE);
        let grass = add(&mut w, 9301, LL_PCODE_LEGACY_GRASS);
        let prim = add(&mut w, 9302, aurora_prim::params::LL_PCODE_VOLUME);
        assert_eq!(kind(&w, tree), Some(Kind::Tree));
        assert_eq!(kind(&w, grass), Some(Kind::Grass));
        assert_eq!(kind(&w, prim), Some(Kind::Object));
        assert_eq!(kind(&w, usize::MAX), None);
    }

    #[test]
    fn only_the_hidden_kinds_are_left_out() {
        let mut w = fixture();
        let prim = add(&mut w, 9302, aurora_prim::params::LL_PCODE_VOLUME);
        let worn = index(&w, 9100);
        let none = Hidden::default();
        assert!(!none.any_object() && !none.hides_object(&w, prim) && !none.hides_object(&w, worn));
        let avatars = Hidden {
            avatars: true,
            ..none
        };
        assert!(avatars.hides_object(&w, worn) && avatars.hides_object(&w, index(&w, 9001)));
        assert!(!avatars.hides_object(&w, prim) && !avatars.hides_object(&w, index(&w, 9200)));
        // the background kinds hide no object
        let background = Hidden {
            terrain: true,
            water: true,
            sky: true,
            clouds: true,
            ..none
        };
        assert!(!background.any_object() && !background.hides_object(&w, prim));
    }

    #[test]
    fn the_demo_list_names_the_hidden_kinds() {
        let (h, particles) = Hidden::parse("water, avatars,sky,particles,nope");
        assert!(h.water && h.avatars && h.sky && particles);
        assert!(!h.terrain && !h.clouds && !h.objects && !h.animesh && !h.trees && !h.grass);
        assert_eq!(Hidden::parse(""), (Hidden::default(), false));
    }
}
