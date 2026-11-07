use super::*;
use aurora_net::NetEvent;
use std::sync::Arc;

fn fixture(count: u32) -> (World, BuildTool, Vec<usize>) {
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    let handle = world.main_region.expect("demo region");
    let agent = world.agent_id;
    let updates = (0..count)
        .map(|number| {
            let mut update = crate::demo::action_avatar(false);
            update.pcode = aurora_prim::params::LL_PCODE_VOLUME;
            update.local_id = 980_000 + number;
            update.full_id = Uuid::from_u128(980_000 + u128::from(number));
            update.parent_id = 0;
            // Full ObjectUpdate need not carry the owner of a silent prim.
            update.owner_id = if number == 0 { agent } else { Uuid::nil() };
            update.update_flags = OBJECT_MODIFY | crate::ui::context::flags::OBJECT_MOVE;
            update.extra = Default::default();
            update
        })
        .collect();
    world.apply(NetEvent::ObjectUpdates { handle, objects: updates });
    let mut tool = BuildTool::default();
    tool.open_build(Tool::Edit);
    let indices: Vec<_> = (0..count)
        .map(|number| world.objects.index_of_uuid(&Uuid::from_u128(980_000 + u128::from(number))).unwrap())
        .collect();
    for &index in &indices {
        let object = world.objects.get(index).unwrap();
        tool.add(&world, object.key);
        tool.on_properties(
            &world,
            vec![ObjectProps {
                object_id: object.full_id,
                full: true,
                owner_id: agent,
                ..Default::default()
            }],
        );
    }
    tool.out.clear();
    (world, tool, indices)
}

fn sent(tool: &BuildTool) -> Vec<(aurora_net::RegionHandle, Vec<u32>)> {
    tool.out
        .iter()
        .filter_map(|command| match command {
            NetCommand::Build(BuildCmd::Link { handle, local_ids }) => Some((*handle, local_ids.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn linking_mesh_and_new_cubes_uses_selected_properties_and_keeps_last_root() {
    for count in [2, 7] {
        let (mut world, mut tool, indices) = fixture(count);
        for &index in &indices {
            world.objects.get_mut(index).unwrap().owner_id = Uuid::nil();
        }
        let mesh = world.objects.get_mut(*indices.last().unwrap()).unwrap();
        mesh.owner_id = world.agent_id;
        mesh.volume.sculpt = Some(aurora_prim::params::SculptParams {
            texture: Uuid::from_u128(2706),
            sculpt_type: 5,
        });
        assert!(tool.can_link(&world));
        assert!(world.objects.get(indices[0]).unwrap().owner_id.is_nil());
        tool.link(&mut world);
        assert_eq!(
            sent(&tool),
            [(world.main_region.unwrap(), (980_000..980_000 + count).rev().collect())]
        );
        assert!(tool.ui.link_error.is_none());
        assert_eq!(
            tool.pending_link.as_ref().unwrap().0[0],
            world.objects.get(*indices.last().unwrap()).unwrap().full_id
        );
    }
}

#[test]
fn linking_waits_for_full_properties_even_if_udp_owners_match() {
    let (mut world, mut tool, indices) = fixture(2);
    let id = world.objects.get(indices[0]).unwrap().full_id;
    tool.props.get_mut(&id).unwrap().full = false;
    assert!(!tool.can_link(&world));
    tool.link(&mut world);
    assert!(sent(&tool).is_empty());
    assert_eq!(tool.ui.link_error.as_deref(), Some(aurora_i18n::tr!("build-link-downloading")));
    tool.props.get_mut(&id).unwrap().full = true;
    assert!(tool.can_link(&world));
    tool.link(&mut world);
    assert_eq!(sent(&tool).len(), 1);
    assert!(tool.ui.link_error.is_none());
}

#[test]
fn linking_button_needs_one_editable_root_but_execution_needs_every_root() {
    let (mut world, mut tool, indices) = fixture(2);
    world.objects.get_mut(indices[0]).unwrap().update_flags &= !OBJECT_MODIFY;
    assert!(tool.can_link(&world));
    tool.link(&mut world);
    assert!(sent(&tool).is_empty());
    assert_eq!(tool.ui.link_error.as_deref(), Some(aurora_i18n::tr!("build-link-modify")));
    world.objects.get_mut(indices[1]).unwrap().update_flags &= !OBJECT_MODIFY;
    assert!(!tool.can_link(&world));
    // Effective simulator flags, rather than a hand-computed owner mask,
    // also allow a shared owner's objects when permission was granted.
    for &index in &indices {
        world.objects.get_mut(index).unwrap().update_flags |= OBJECT_MODIFY;
    }
    for props in tool.props.values_mut() {
        props.owner_id = Uuid::from_u128(91);
        props.owner_mask = 0;
    }
    tool.link(&mut world);
    assert_eq!(sent(&tool).len(), 1);
}

#[test]
fn linking_permanent_objects_matches_is_permanent_enforced() {
    let (mut world, mut tool, indices) = fixture(2);
    for &index in &indices {
        world.objects.get_mut(index).unwrap().update_flags |= AFFECTS_NAVMESH;
    }
    assert!(tool.can_link(&world), "permanent flags are editable in our region");
    tool.link(&mut world);
    assert_eq!(sent(&tool).len(), 1);
    tool.out.clear();
    world.main_region = Some(world.main_region.unwrap() + (256_u64 << 32));
    assert!(!tool.can_link(&world));
    world.objects.get_mut(indices[1]).unwrap().update_flags &= !AFFECTS_NAVMESH;
    assert!(tool.can_link(&world), "button needs only one non-permanent root");
    tool.link(&mut world);
    assert!(sent(&tool).is_empty());
    assert_eq!(tool.ui.link_error.as_deref(), Some(aurora_i18n::tr!("build-link-permanent")));
}

#[test]
fn linking_reports_distinct_owner_and_region_refusals_without_sending() {
    let (mut world, mut tool, indices) = fixture(2);
    let id = world.objects.get(indices[0]).unwrap().full_id;
    tool.props.get_mut(&id).unwrap().owner_id = Uuid::from_u128(92);
    tool.link(&mut world);
    assert!(sent(&tool).is_empty());
    assert_eq!(tool.ui.link_error.as_deref(), Some(aurora_i18n::tr!("build-link-owners")));
    tool.props.get_mut(&id).unwrap().owner_id = world.agent_id;
    let old_key = world.objects.get(indices[0]).unwrap().key;
    let mut update = crate::build::replace::demo_update(world.objects.get(indices[0]).unwrap());
    update.update_flags |= OBJECT_MODIFY;
    world.apply(NetEvent::ObjectUpdates {
        handle: old_key.region + (256_u64 << 32),
        objects: vec![update],
    });
    let new_key = world.objects.get(indices[0]).unwrap().key;
    for selected in &mut tool.selection {
        if *selected == old_key {
            *selected = new_key;
        }
    }
    tool.link(&mut world);
    assert!(sent(&tool).is_empty());
    assert_eq!(tool.ui.link_error.as_deref(), Some(aurora_i18n::tr!("build-link-regions")));
}

#[test]
fn linking_limits_count_prims_and_follow_grid_and_region_settings() {
    let (mut world, mut tool, _) = fixture(257);
    assert_eq!(world.link_limit(), 256);
    tool.link(&mut world);
    assert!(sent(&tool).is_empty());
    assert!(tool.ui.link_error.is_some());
    world.home_grid = Some("example.invalid:8002".into());
    assert_eq!(world.link_limit(), 10001);
    tool.link(&mut world);
    assert_eq!(sent(&tool).len(), 1);
    tool.out.clear();
    let handle = world.main_region.unwrap();
    world.apply(NetEvent::RegionLinkLimit {
        handle,
        max_children: Some(3),
    });
    assert_eq!(world.link_limit(), 4);
    tool.link(&mut world);
    assert!(sent(&tool).is_empty());
    world.apply(NetEvent::RegionLinkLimit {
        handle,
        max_children: None,
    });
    assert_eq!(world.link_limit(), 10001);
}

#[test]
fn linking_256_roots_confirms_every_member_in_the_servers_batch_order() {
    let (mut world, mut tool, _) = fixture(256);
    tool.link(&mut world);
    let ids = &sent(&tool)[0].1;
    assert_eq!(ids.len(), 256);
    let expected: Vec<_> = std::iter::once(ids[0])
        .chain(ids[254..].iter().copied())
        .chain(ids[1..254].iter().copied())
        .map(|id| Uuid::from_u128(u128::from(id)))
        .collect();
    assert_eq!(tool.pending_link.as_ref().unwrap().0, expected);
    let root = ids[0];
    let updates = ids
        .iter()
        .map(|id| {
            let object = world
                .objects
                .get(world.objects.index_of_uuid(&Uuid::from_u128(u128::from(*id))).unwrap())
                .unwrap();
            let mut update = crate::build::replace::demo_update(object);
            update.parent_id = if *id == root { 0 } else { root };
            update
        })
        .collect();
    world.apply(NetEvent::ObjectUpdates {
        handle: world.main_region.unwrap(),
        objects: updates,
    });
    tool.advance_link(&mut world, Instant::now());
    assert!(tool.pending_link.is_none());
    let root = world.objects.index_of_uuid(&expected[0]).unwrap();
    let children: Vec<_> = world
        .objects
        .children_of(&world.objects.get(root).unwrap().key)
        .iter()
        .map(|&i| world.objects.get(i).unwrap().full_id)
        .collect();
    assert_eq!(children, expected[1..]);
}

#[test]
fn linking_animesh_uses_the_whole_selections_mesh_header_estimates() {
    let (mut world, mut tool, indices) = fixture(2);
    world.regions.get_mut(&world.main_region.unwrap()).unwrap().animated_object_max_tris = 100;
    world.objects.get_mut(indices[0]).unwrap().extra.extended_mesh_flags = Some(aurora_prim::extra::EXTENDED_MESH_ANIMATED);
    for (&index, triangles) in indices.iter().zip([60.0, 50.0]) {
        let id = Uuid::from_u128(index as u128 + 1234);
        world.objects.get_mut(index).unwrap().volume.sculpt = Some(aurora_prim::params::SculptParams {
            texture: id,
            sculpt_type: 5,
        });
        tool.link_triangles.insert(id, triangles);
    }
    assert!(!tool.can_link(&world));
    tool.link(&mut world);
    assert!(sent(&tool).is_empty());
    assert_eq!(tool.ui.link_error.as_deref(), Some(aurora_i18n::tr!("build-link-animesh")));
    world.regions.get_mut(&world.main_region.unwrap()).unwrap().animated_object_max_tris = 110;
    assert!(tool.can_link(&world));
    tool.link(&mut world);
    assert_eq!(sent(&tool).len(), 1);
}

#[test]
fn linking_shortcut_does_not_bypass_edit_linked_mode() {
    let (mut world, mut tool, _) = fixture(2);
    tool.selection_linked = true;
    assert!(!tool.can_link(&world));
    tool.link(&mut world);
    assert!(sent(&tool).is_empty());
}

#[test]
fn linking_large_opensim_selections_retain_their_full_property_replies() {
    let (mut world, mut tool, _) = fixture(4098);
    world.home_grid = Some("example.invalid:8002".into());
    assert!(tool.can_link(&world));
    tool.link(&mut world);
    assert_eq!(sent(&tool)[0].1.len(), 4098);
}

#[test]
fn linking_reselection_waits_for_new_properties_instead_of_reusing_cached_owners() {
    let (mut world, mut tool, indices) = fixture(2);
    let cached = tool.props.values().cloned().collect();
    tool.deselect_all(&world);
    for index in indices {
        tool.add(&world, world.objects.get(index).unwrap().key);
    }
    tool.out.clear();
    assert!(!tool.can_link(&world));
    tool.link(&mut world);
    assert!(sent(&tool).is_empty());
    assert_eq!(tool.ui.link_error.as_deref(), Some(aurora_i18n::tr!("build-link-downloading")));
    tool.on_properties(&world, cached);
    assert!(tool.can_link(&world));
    tool.link(&mut world);
    assert_eq!(sent(&tool).len(), 1);
}

#[test]
fn linking_counts_every_prim_in_an_existing_large_linkset() {
    let (mut world, mut tool, indices) = fixture(4098);
    let parent = world.objects.get(indices[0]).unwrap().key.local_id;
    let updates = indices[1..4097]
        .iter()
        .map(|&index| {
            let mut update = crate::build::replace::demo_update(world.objects.get(index).unwrap());
            update.parent_id = parent;
            update
        })
        .collect();
    let handle = world.main_region.unwrap();
    world.apply(NetEvent::ObjectUpdates { handle, objects: updates });
    world.home_grid = Some("example.invalid:8002".into());
    world.apply(NetEvent::RegionLinkLimit {
        handle,
        max_children: Some(4096),
    });
    assert_eq!(tool.roots(&world).len(), 2);
    tool.link(&mut world);
    assert!(sent(&tool).is_empty(), "4,098 prims exceed the 4,097-prim regional limit");
    assert!(tool.ui.link_error.is_some());
}

#[test]
fn linking_group_owned_objects_uses_the_permissions_group_as_owner() {
    let (mut world, mut tool, indices) = fixture(2);
    let group = Uuid::from_u128(270600);
    for props in tool.props.values_mut() {
        props.owner_id = Uuid::nil();
        props.group_id = group;
    }
    assert!(tool.can_link(&world));
    tool.link(&mut world);
    assert_eq!(sent(&tool).len(), 1);
    tool.out.clear();
    let id = world.objects.get(indices[0]).unwrap().full_id;
    // Both wire representations identify the same owning group.
    tool.props.get_mut(&id).unwrap().owner_id = group;
    tool.link(&mut world);
    assert_eq!(sent(&tool).len(), 1);
    tool.out.clear();
    let props = tool.props.get_mut(&id).unwrap();
    props.owner_id = Uuid::nil();
    props.group_id = Uuid::from_u128(270601);
    tool.link(&mut world);
    assert!(sent(&tool).is_empty());
    assert_eq!(tool.ui.link_error.as_deref(), Some(aurora_i18n::tr!("build-link-owners")));
}
