//! Editing what our avatar wears on its body: the selection space of
//! attachments and their coordinates relative to the attachment point
//! (demo_worn.rs fixture).
use super::*;
use crate::scene::Scene;

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

fn worn(world: &World) -> (usize, usize) {
    (local(world, crate::demo::worn::ROOT), local(world, crate::demo::worn::CHILD))
}

#[test]
fn what_we_wear_is_selectable_as_an_attachment_and_so_are_other_avatars_things_but_not_changeable() {
    let world = world();
    let (root, child) = worn(&world);
    assert_eq!(select_type(&world, root), SelectType::Attachment);
    assert_eq!(select_type(&world, child), SelectType::Attachment);
    assert!(selectable(&world, root) && selectable(&world, child));
    // a click on a linked prim takes the whole worn object
    assert_eq!(BuildTool::unit_of(&world, child, false), Some(world.objects.get(root).unwrap().key));
    // Violet Wolf's hat: an attachment too; it can be selected and looked at
    // (LLSelectMgr::canSelectObject), but never moved or modified
    let hat = local(&world, 9100);
    assert_eq!(select_type(&world, hat), SelectType::Attachment);
    assert!(selectable(&world, hat));
    assert!(worn_by_other(&world, hat));
    assert!(!worn_by_other(&world, root) && !worn_by_other(&world, child));
}

#[test]
fn another_avatars_attachment_stays_locked_whatever_rights_the_simulator_reports() {
    use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_MOVE};
    let mut world = world();
    let hat = local(&world, 9100);
    world.objects.get_mut(hat).unwrap().update_flags |= OBJECT_MODIFY | OBJECT_MOVE;
    let tool = BuildTool::default();
    assert!(!tool.can_modify(&world, hat));
    for mode in [EditMode::Move, EditMode::Stretch] {
        assert!(!can_transform(&world, hat, false, mode));
        assert!(!can_transform(&world, hat, true, mode));
    }
    // our own worn object is unchanged by the rule
    let (root, _) = worn(&world);
    world.objects.get_mut(root).unwrap().update_flags |= OBJECT_MODIFY | OBJECT_MOVE;
    assert!(tool.can_modify(&world, root));
    assert!(can_transform(&world, root, false, EditMode::Move));
}

/// Violet Wolf (local id 9001) also wears a two-prim object (root 9110, its
/// linked prim 9111) and a HUD (9112, top-left point 31), with no rights for
/// us, like the hat.
fn with_wolf_extras(world: &mut World) {
    let hat = crate::demo::wolf_attachments(false).remove(0);
    let attached = |local_id: u32, parent_id: u32, point: u8| {
        let mut o = hat.clone();
        o.local_id = local_id;
        o.full_id = Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | u128::from(local_id));
        o.parent_id = parent_id;
        o.state = ((point & 0x0F) << 4) | ((point & 0xF0) >> 4);
        o
    };
    let mut child = attached(9111, 9110, 0);
    child.state = 0;
    let handle = world.main_region.unwrap_or_default();
    world.apply(aurora_net::NetEvent::ObjectUpdates {
        handle,
        objects: vec![attached(9110, 9001, 2), child, attached(9112, 9001, 31)],
    });
}

#[test]
fn another_avatars_hud_stays_unselectable_but_their_linked_attachment_selects_and_stays_locked() {
    use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_MOVE};
    let mut world = world();
    with_wolf_extras(&mut world);
    let (root, child, hud) = (local(&world, 9110), local(&world, 9111), local(&world, 9112));
    // cursorIntersect only picks our own HUDs
    assert!(!selectable(&world, hud));
    assert!(selectable(&world, root) && selectable(&world, child));
    assert_eq!(root_of(&world, child), root);
    // the lock walks up the links, whatever the simulator reports
    for idx in [root, child] {
        world.objects.get_mut(idx).unwrap().update_flags |= OBJECT_MODIFY | OBJECT_MOVE;
    }
    let tool = BuildTool::default();
    for idx in [root, child] {
        assert!(worn_by_other(&world, idx));
        assert!(!tool.can_modify(&world, idx));
        assert!(!can_transform(&world, idx, true, EditMode::Move));
        assert!(!can_transform(&world, idx, false, EditMode::Rotate));
        assert!(!undo_allowed(&world, idx, false) && !undo_allowed(&world, idx, true));
    }
}

#[test]
fn align_refuses_another_avatars_attachment_and_accepts_ours() {
    use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_MOVE};
    let mut world = world();
    let hat = local(&world, 9100);
    let (root, _) = worn(&world);
    for idx in [hat, root] {
        world.objects.get_mut(idx).unwrap().update_flags |= OBJECT_MODIFY | OBJECT_MOVE;
    }
    let s = BuildSettings::default();
    let mut tool = BuildTool::default();
    tool.click_select(&mut world, &s, Some(hat), false);
    assert_eq!(tool.selection, vec![world.objects.get(hat).unwrap().key]);
    assert!(!tool.align_allowed(&world));
    tool.click_select(&mut world, &s, Some(root), false);
    assert!(tool.align_allowed(&world));
}

#[test]
fn undo_and_redo_send_nothing_for_another_avatars_attachment() {
    use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_MOVE};
    let mut world = world();
    let hat = local(&world, 9100);
    let (root, _) = worn(&world);
    for idx in [hat, root] {
        world.objects.get_mut(idx).unwrap().update_flags |= OBJECT_MODIFY | OBJECT_MOVE;
    }
    let s = BuildSettings::default();
    let mut tool = BuildTool::default();
    tool.click_select(&mut world, &s, Some(hat), false);
    tool.out.clear();
    for redo in [false, true] {
        assert!(!tool.can_undo(&world, redo));
        tool.undo(&world, redo);
    }
    assert!(tool.out.is_empty(), "{:?}", tool.out);
    // our own worn object is still undone
    tool.click_select(&mut world, &s, Some(root), false);
    tool.out.clear();
    assert!(tool.can_undo(&world, false));
    tool.undo(&world, false);
    let id = world.objects.get(root).unwrap().full_id;
    assert!(matches!(&tool.out[..], [NetCommand::Build(BuildCmd::Undo { ids, .. })] if ids == &[id]));
}

#[test]
fn undo_follows_the_legacy_rights_on_our_own_object() {
    use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_MOVE};
    let mut world = world();
    let (root, _) = worn(&world);
    let set = |world: &mut World, flags: u32| {
        let o = world.objects.get_mut(root).unwrap();
        o.update_flags = o.update_flags & !(OBJECT_MODIFY | OBJECT_MOVE) | flags;
    };
    // getFirstUndoEnabledObject: Modify, or Move; getFirstEditableObject: Modify
    set(&mut world, 0);
    assert!(!undo_allowed(&world, root, false) && !undo_allowed(&world, root, true));
    set(&mut world, OBJECT_MOVE);
    assert!(undo_allowed(&world, root, false) && !undo_allowed(&world, root, true));
    set(&mut world, OBJECT_MODIFY);
    assert!(undo_allowed(&world, root, false) && undo_allowed(&world, root, true));
    // Move alone does not undo a permanent (pathfinding) object outside the
    // agent's region: isPermanentEnforced
    set(&mut world, OBJECT_MOVE | crate::ui::context::flags::AFFECTS_NAVMESH);
    world.main_region = Some(world.objects.get(root).unwrap().key.region);
    assert!(undo_allowed(&world, root, false));
    world.main_region = None;
    assert!(!undo_allowed(&world, root, false));
    set(&mut world, OBJECT_MODIFY | crate::ui::context::flags::AFFECTS_NAVMESH);
    assert!(undo_allowed(&world, root, false));
}

#[test]
fn undo_of_a_mixed_selection_sends_only_our_own_attachment() {
    use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_MOVE};
    let mut world = world();
    let hat = local(&world, 9100);
    let (root, _) = worn(&world);
    for idx in [hat, root] {
        world.objects.get_mut(idx).unwrap().update_flags |= OBJECT_MODIFY | OBJECT_MOVE;
    }
    let s = BuildSettings::default();
    let mut tool = BuildTool::default();
    tool.click_select(&mut world, &s, Some(root), false);
    tool.click_select(&mut world, &s, Some(hat), true);
    assert_eq!(tool.roots(&world).len(), 2, "both attachments are selected");
    tool.out.clear();
    for redo in [false, true] {
        assert!(tool.can_undo(&world, redo));
        tool.undo(&world, redo);
    }
    let id = world.objects.get(root).unwrap().full_id;
    assert_eq!(tool.out.len(), 2, "{:?}", tool.out);
    assert!(matches!(&tool.out[0], NetCommand::Build(BuildCmd::Undo { ids, .. }) if ids == &[id]));
    assert!(matches!(&tool.out[1], NetCommand::Build(BuildCmd::Redo { ids, .. }) if ids == &[id]));
}

#[test]
fn the_edit_frame_of_a_worn_object_is_its_attachment_point() {
    let world = world();
    let (root, _) = worn(&world);
    let now = Instant::now();
    let (point, frame) = edit_parent(&world, root, now).expect("attachment point");
    let (pos, rot, hud) = Scene::object_transform(&world, root, now, 0).unwrap();
    assert!(!hud);
    let o = world.objects.get(root).unwrap();
    assert!((point + frame * o.position).abs_diff_eq(pos, 1e-4));
    assert!((frame * o.rotation).abs_diff_eq(rot, 1e-4));
    // LLSelectMgr::getGrid: the world grid of a worn selection is that frame
    let f = manip::frame(&world, &BuildSettings::default(), &[o.key], false, now, None).unwrap();
    assert!(f.grid.world);
    assert!(f.grid.origin.abs_diff_eq(point, 1e-4));
    assert!(f.grid.rotation.abs_diff_eq(frame, 1e-4));
}

#[test]
fn typed_position_of_a_worn_object_is_local_to_its_point_and_within_reach() {
    let mut world = world();
    let (root, child) = worn(&world);
    let s = BuildSettings::default();
    let mut tool = BuildTool::default();
    tool.click_select(&mut world, &s, Some(root), false);
    tool.out.clear();
    let state = world.objects.get(root).unwrap().state;
    let child_before = world.objects.get(child).unwrap().position;
    let pos = Vec3::new(-0.3, 0.05, 0.2);
    let rotation = Quat::from_rotation_y(0.4);
    tool.set_transform(&mut world, &s, Some(pos), None, Some(rotation), true);
    let o = world.objects.get(root).unwrap();
    assert!(o.position.abs_diff_eq(pos, 1e-5));
    assert!(o.rotation.abs_diff_eq(rotation, 1e-5));
    // still worn on the same point, its linked prim where it was on it
    assert_eq!(o.state, state);
    assert_eq!(world.objects.get(child).unwrap().position, child_before);
    assert!(matches!(&tool.out[..], [NetCommand::Build(BuildCmd::Transform { updates, .. })]
        if updates[0].linked && updates[0].position.unwrap().abs_diff_eq(pos, 1e-5)));
    // MAX_ATTACHMENT_DIST from the point
    tool.set_transform(&mut world, &s, Some(Vec3::new(10.0, 0.0, 0.0)), None, None, true);
    let o = world.objects.get(root).unwrap();
    assert!(o.position.abs_diff_eq(Vec3::new(MAX_ATTACHMENT_DIST, 0.0, 0.0), 1e-4));
}

#[test]
fn a_worn_object_moves_without_the_modify_right_but_its_parts_need_it() {
    use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_MOVE};
    let mut world = world();
    let (root, child) = worn(&world);
    for idx in [root, child] {
        world.objects.get_mut(idx).unwrap().update_flags = OBJECT_MOVE;
    }
    let mut s = BuildSettings::default();
    let mut tool = BuildTool::default();
    tool.click_select(&mut world, &s, Some(root), false);
    tool.out.clear();
    let pos = Vec3::new(-0.2, 0.0, 0.3);
    tool.set_transform(&mut world, &s, Some(pos), None, None, true);
    assert!(world.objects.get(root).unwrap().position.abs_diff_eq(pos, 1e-5));
    assert_eq!(tool.out.len(), 1);
    s.edit_linked = true;
    tool.click_select(&mut world, &s, Some(child), false);
    tool.out.clear();
    let before = world.objects.get(child).unwrap().position;
    tool.set_transform(&mut world, &s, Some(Vec3::new(0.0, 0.1, 0.2)), None, None, true);
    assert_eq!(world.objects.get(child).unwrap().position, before);
    assert!(tool.out.is_empty());
    world.objects.get_mut(root).unwrap().update_flags |= OBJECT_MODIFY;
    tool.set_transform(&mut world, &s, Some(Vec3::new(0.0, 0.1, 0.2)), None, None, true);
    assert!(
        world
            .objects
            .get(child)
            .unwrap()
            .position
            .abs_diff_eq(Vec3::new(0.0, 0.1, 0.2), 1e-5)
    );
}

#[test]
fn a_worn_selection_is_kept_apart_and_cannot_be_deleted_taken_copied_or_linked() {
    let mut world = world();
    let (root, _) = worn(&world);
    let key = world.objects.get(root).unwrap().key;
    let ground = world
        .objects
        .iter()
        .find(|(idx, _)| selectable(&world, *idx) && select_type(&world, *idx) == SelectType::World)
        .map(|(i, _)| i)
        .unwrap();
    let s = BuildSettings::default();
    let mut tool = BuildTool::default();
    // Shift + click on a world object does not add it to a worn selection
    tool.click_select(&mut world, &s, Some(root), false);
    assert!(tool.attachment_selected(&world) && !tool.hud_selected(&world));
    tool.click_select(&mut world, &s, Some(ground), true);
    assert_eq!(tool.selection, vec![world.objects.get(ground).unwrap().key]);
    assert!(!tool.attachment_selected(&world));
    tool.click_select(&mut world, &s, Some(root), false);
    tool.out.clear();
    tool.delete(&mut world);
    tool.take(&mut world, false);
    tool.take(&mut world, true);
    tool.duplicate(&mut world, &s, Vec3::X, true);
    tool.link(&mut world);
    tool.unlink(&mut world);
    assert!(tool.out.is_empty(), "{:?}", tool.out);
    assert_eq!(tool.selection, vec![key]);
    // nor left behind when the avatar walks away with it
    tool.open_build(Tool::Edit);
    world.agent.position += Vec3::splat(1000.0);
    let limited = BuildSettings {
        limit_select_distance: true,
        ..Default::default()
    };
    tool.prune(&world, &limited, Instant::now());
    assert_eq!(tool.selection, vec![key]);
}

#[test]
fn undo_puts_back_a_moved_object_of_ours_in_the_demo() {
    let mut world = world();
    let obj = world
        .objects
        .iter()
        .find(|(i, o)| {
            selectable(&world, *i)
                && select_type(&world, *i) == SelectType::World
                && o.parent_id == 0
                && o.update_flags & crate::ui::context::flags::OBJECT_YOU_OWNER != 0
        })
        .map(|(i, _)| i)
        .expect("an object of ours");
    let start = world.objects.get(obj).unwrap().position;
    let s = BuildSettings::default();
    let mut tool = BuildTool::default();
    tool.open_build(Tool::Edit);
    tool.click_select(&mut world, &s, Some(obj), false);
    let mut sim = super::demo_sim::DemoSim::default();
    let reply = |world: &mut World, tool: &mut BuildTool, sim: &mut super::demo_sim::DemoSim| {
        for c in std::mem::take(&mut tool.out) {
            if let NetCommand::Build(c) = c {
                for ev in sim.reply(world, &c) {
                    world.apply(ev);
                }
            }
        }
    };
    reply(&mut world, &mut tool, &mut sim);
    let at = |world: &World| world.objects.get(obj).unwrap().position;
    tool.set_transform(&mut world, &s, Some(start + Vec3::X), None, None, true);
    reply(&mut world, &mut tool, &mut sim);
    sim.end_edits();
    tool.set_transform(&mut world, &s, Some(start + Vec3::X * 2.0), None, None, true);
    reply(&mut world, &mut tool, &mut sim);
    assert!(at(&world).abs_diff_eq(start + Vec3::X * 2.0, 1e-4));
    // the simulator's answer to Undo waits out our last edit's (edit_echo.rs)
    let settle = |world: &mut World| world.objects.expire_sent_edits(Instant::now() + std::time::Duration::from_secs(3));
    assert!(tool.can_undo(&world, false));
    tool.undo(&world, false);
    reply(&mut world, &mut tool, &mut sim);
    settle(&mut world);
    assert!(at(&world).abs_diff_eq(start + Vec3::X, 1e-4), "{:?}", at(&world));
    tool.undo(&world, false);
    reply(&mut world, &mut tool, &mut sim);
    settle(&mut world);
    assert!(at(&world).abs_diff_eq(start, 1e-4), "{:?}", at(&world));
    tool.undo(&world, true);
    reply(&mut world, &mut tool, &mut sim);
    settle(&mut world);
    assert!(at(&world).abs_diff_eq(start + Vec3::X, 1e-4), "{:?}", at(&world));
}

#[test]
fn the_demo_reports_another_avatars_attachment_contents_as_theirs() {
    let world = world();
    let mut sim = super::demo_sim::DemoSim::default();
    let owners = |sim: &mut super::demo_sim::DemoSim, idx: usize| {
        let o = world.objects.get(idx).unwrap();
        let events = sim.reply(
            &world,
            &BuildCmd::RequestTaskInventory {
                handle: o.key.region,
                local_id: o.key.local_id,
                object_id: o.full_id,
            },
        );
        let Some(aurora_net::NetEvent::TaskInventory { result: Ok(items), .. }) = events.into_iter().next() else {
            panic!("contents");
        };
        items.into_iter().filter(|i| !i.is_folder).map(|i| i.owner_id).collect::<Vec<_>>()
    };
    let hat = local(&world, 9100);
    let wolf = world
        .objects
        .get(world.objects.parent_of(world.objects.get(hat).unwrap()).unwrap())
        .unwrap()
        .full_id;
    let theirs = owners(&mut sim, hat);
    assert!(!theirs.is_empty() && theirs.iter().all(|&o| o == wolf), "{theirs:?}");
    let (root, _) = worn(&world);
    let ours = owners(&mut sim, root);
    assert!(!ours.is_empty() && ours.iter().all(|&o| o == crate::demo::DEMO_AGENT), "{ours:?}");
}

#[test]
fn a_prim_we_create_in_the_demo_is_our_own_work() {
    let mut world = world();
    let s = BuildSettings::default();
    let mut tool = BuildTool::default();
    tool.open_build(Tool::Create);
    let point = world.agent.position + Vec3::X * 5.0;
    tool.place(&mut world, &s, Some(point), None, (0.0, 0.0));
    let Some(NetCommand::Build(add @ BuildCmd::Add(prim))) = tool.out.iter().find(|c| matches!(c, NetCommand::Build(BuildCmd::Add(_))))
    else {
        panic!("ObjectAdd");
    };
    let handle = prim.handle;
    let mut sim = super::demo_sim::DemoSim::default();
    let events = sim.reply(&world, add);
    let Some(aurora_net::NetEvent::ObjectUpdates { objects, .. }) = events.first() else {
        panic!("created object");
    };
    let (id, local_id) = (objects[0].full_id, objects[0].local_id);
    for ev in events {
        world.apply(ev);
    }
    let replies = sim.reply(
        &world,
        &BuildCmd::Select {
            handle,
            local_ids: vec![local_id],
        },
    );
    let props = replies
        .iter()
        .find_map(|ev| match ev {
            aurora_net::NetEvent::ObjectProperties(props) => props.iter().find(|p| p.object_id == id),
            _ => None,
        })
        .expect("properties");
    let me = crate::demo::DEMO_AGENT;
    assert_eq!((props.creator_id, props.owner_id, props.last_owner_id), (me, me, me));
}

#[test]
fn undo_after_link_or_unlink_in_the_demo_does_not_throw_a_prim_away() {
    use aurora_net::build::TransformUpdate;
    let mut world = world();
    let ours: Vec<usize> = world
        .objects
        .iter()
        .filter(|(i, o)| {
            selectable(&world, *i)
                && select_type(&world, *i) == SelectType::World
                && o.parent_id == 0
                && o.update_flags & crate::ui::context::flags::OBJECT_YOU_OWNER != 0
                && world.objects.iter().all(|(_, c)| c.parent_id != o.key.local_id)
        })
        .map(|(i, _)| i)
        .take(2)
        .collect();
    let [root, child] = ours[..] else {
        panic!("two single prims of ours")
    };
    let handle = world.objects.get(root).unwrap().key.region;
    assert_eq!(handle, world.objects.get(child).unwrap().key.region);
    let (root_id, child_id) = (
        world.objects.get(root).unwrap().key.local_id,
        world.objects.get(child).unwrap().key.local_id,
    );
    let uuid = world.objects.get(child).unwrap().full_id;
    let mut sim = super::demo_sim::DemoSim::default();
    let send = |world: &mut World, sim: &mut super::demo_sim::DemoSim, c: BuildCmd| {
        for ev in sim.reply(world, &c) {
            world.apply(ev);
        }
    };
    let nudge = |world: &World| BuildCmd::Transform {
        handle,
        updates: vec![TransformUpdate {
            local_id: child_id,
            position: Some(world.objects.get(child).unwrap().position + Vec3::X * 0.5),
            rotation: None,
            scale: None,
            linked: false,
            uniform: false,
        }],
    };
    let undo = BuildCmd::Undo {
        handle,
        ids: vec![uuid],
        group_id: Uuid::nil(),
    };
    let settle = |world: &mut World| world.objects.expire_sent_edits(Instant::now() + std::time::Duration::from_secs(3));
    let region_pos = |world: &World| super::replace::region_transform(world, child).unwrap().0;
    // a move in the region, then Link: the old region position must not
    // come back as an offset from the root
    send(
        &mut world,
        &mut sim,
        BuildCmd::Select {
            handle,
            local_ids: vec![root_id, child_id],
        },
    );
    let c = nudge(&world);
    send(&mut world, &mut sim, c);
    sim.end_edits();
    send(
        &mut world,
        &mut sim,
        BuildCmd::Link {
            handle,
            local_ids: vec![root_id, child_id],
        },
    );
    assert_eq!(world.objects.get(child).unwrap().parent_id, root_id);
    let linked_at = region_pos(&world);
    send(&mut world, &mut sim, undo.clone());
    settle(&mut world);
    assert!(
        region_pos(&world).abs_diff_eq(linked_at, 1e-3),
        "{:?} vs {linked_at:?}",
        region_pos(&world)
    );
    // a move as a linked part (an offset), then Unlink: the offset must not
    // come back as a region position
    let c = nudge(&world);
    send(&mut world, &mut sim, c);
    sim.end_edits();
    send(
        &mut world,
        &mut sim,
        BuildCmd::Delink {
            handle,
            local_ids: vec![child_id],
        },
    );
    assert_eq!(world.objects.get(child).unwrap().parent_id, 0);
    let unlinked_at = region_pos(&world);
    send(&mut world, &mut sim, undo);
    settle(&mut world);
    assert!(
        region_pos(&world).abs_diff_eq(unlinked_at, 1e-3),
        "{:?} vs {unlinked_at:?}",
        region_pos(&world)
    );
}
