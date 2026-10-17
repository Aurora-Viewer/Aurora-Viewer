//! Avatars that stand still while something they carry is being edited.
//!
//! Port of LLSelectMgr::pauseAssociatedAvatars (indra/newview/llselectmgr.cpp,
//! originally LGPL 2.1): as long as an attachment is in the selection — from
//! the right click that opens its menu, through the build floater, until the
//! selection is dropped — the avatar wearing it pauses its animations, and an
//! animated object pauses its own skeleton (an animated attachment pauses
//! both). The pause is local: nothing is sent, other viewers see the avatar
//! move as before. In synchronized mode, releasing the frozen pose catches up
//! to the receipt clock; Legacy suspends playback. HUD attachments do not pause anything
//! (LLSelectMgr::getSelectTypeForObject gives them SELECT_TYPE_HUD).
use std::collections::HashSet;

use aurora_prim::extra::EXTENDED_MESH_ANIMATED;
use uuid::Uuid;

use super::{SelectType, select_type};
use crate::world::World;
use crate::world::objects::ObjKey;

/// Skeleton owners (avatars, animated object roots) to pause for `selection`.
pub fn paused_skeletons(world: &World, selection: &[ObjKey]) -> HashSet<Uuid> {
    let mut paused = HashSet::new();
    for key in selection {
        let Some(idx) = world.objects.index_of(key) else {
            continue;
        };
        // LLViewerObject::getAvatarAncestor, for an attachment only
        if select_type(world, idx) == SelectType::Attachment
            && let Some(avatar) = avatar_ancestor(world, idx)
        {
            paused.insert(avatar);
        }
        // LLViewerObject::isAnimatedObject: the linkset root carries the flag
        let root = root(world, idx);
        if let Some(o) = world.objects.get(root)
            && o.extra.extended_mesh_flags.unwrap_or(0) & EXTENDED_MESH_ANIMATED != 0
        {
            paused.insert(o.full_id);
        }
    }
    paused
}

fn avatar_ancestor(world: &World, mut idx: usize) -> Option<Uuid> {
    for _ in 0..64 {
        let o = world.objects.get(idx)?;
        let parent = world.objects.parent_of(o)?;
        let p = world.objects.get(parent)?;
        if p.is_avatar() {
            return Some(p.full_id);
        }
        idx = parent;
    }
    None
}

/// The prim right below an avatar, or the topmost one.
fn root(world: &World, mut idx: usize) -> usize {
    for _ in 0..64 {
        let Some(o) = world.objects.get(idx) else { break };
        let Some(parent) = world.objects.parent_of(o) else { break };
        if world.objects.get(parent).is_none_or(|p| p.is_avatar()) {
            break;
        }
        idx = parent;
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{BuildSettings, BuildTool};

    fn world() -> World {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for ev in crate::demo::events().into_iter().chain(crate::demo::worn::events()) {
            world.apply(ev);
        }
        world
    }

    fn local(world: &World, local_id: u32) -> usize {
        world
            .objects
            .iter()
            .find(|(_, o)| o.key.local_id == local_id)
            .map(|(i, _)| i)
            .expect("demo object")
    }

    #[test]
    fn our_avatar_pauses_while_one_of_its_attachments_is_selected() {
        let mut world = world();
        let child = local(&world, crate::demo::worn::CHILD);
        let s = BuildSettings::default();
        let mut tool = BuildTool::default();
        assert!(paused_skeletons(&world, &tool.selection).is_empty());
        // the right click that opens the attachment's menu selects it
        assert!(tool.select_for_menu(&mut world, &s, child));
        assert_eq!(paused_skeletons(&world, &tool.selection), HashSet::from([world.agent_id]));
        // menu closed without editing: the selection goes, and the pause with it
        tool.release_menu_selection(&world);
        assert!(paused_skeletons(&world, &tool.selection).is_empty());
    }

    #[test]
    fn the_pause_lasts_while_the_build_floater_holds_the_selection() {
        let mut world = world();
        let root = local(&world, crate::demo::worn::ROOT);
        let s = BuildSettings::default();
        let mut tool = BuildTool::default();
        tool.select_for_menu(&mut world, &s, root);
        // "Edit": the build tools take the selection over
        tool.open = true;
        tool.release_menu_selection(&world);
        assert_eq!(paused_skeletons(&world, &tool.selection), HashSet::from([world.agent_id]));
        // the floater closes and drops it
        tool.open = false;
        tool.deselect_all(&world);
        assert!(paused_skeletons(&world, &tool.selection).is_empty());
    }

    #[test]
    fn edit_linked_parts_of_a_worn_object_pause_its_wearer_too() {
        let world = world();
        let child = local(&world, crate::demo::worn::CHILD);
        let key = world.objects.get(child).unwrap().key;
        assert_eq!(paused_skeletons(&world, &[key]), HashSet::from([world.agent_id]));
    }

    #[test]
    fn world_objects_pause_nothing_but_an_animated_object_pauses_its_skeleton() {
        let mut world = world();
        let (idx, o) = world
            .objects
            .iter()
            .find(|(i, o)| !o.is_avatar() && o.parent_id == 0 && select_type(&world, *i) == SelectType::World)
            .map(|(i, o)| (i, o.key))
            .expect("a world object of the demo");
        assert!(paused_skeletons(&world, &[o]).is_empty());
        // flagged as an animated object (LLVOVolume::isAnimatedObject)
        let full_id = {
            let ob = world.objects.get_mut(idx).unwrap();
            ob.extra.extended_mesh_flags = Some(EXTENDED_MESH_ANIMATED);
            ob.full_id
        };
        assert_eq!(paused_skeletons(&world, &[o]), HashSet::from([full_id]));
    }

    #[test]
    fn a_hud_attachment_pauses_nothing() {
        let mut world = world();
        let root = local(&world, crate::demo::worn::ROOT);
        // worn on HUD point 31 (the nibbles of the attachment point swapped)
        world.objects.get_mut(root).unwrap().state = 0xF1;
        assert_eq!(select_type(&world, root), SelectType::Hud);
        let key = world.objects.get(root).unwrap().key;
        assert!(paused_skeletons(&world, &[key]).is_empty());
    }
}
