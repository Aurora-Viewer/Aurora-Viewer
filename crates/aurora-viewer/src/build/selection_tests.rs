//! Selection regressions for LLToolSelect::handleObjectSelection.

use super::*;
use aurora_net::NetEvent;
use std::sync::Arc;

fn fixture() -> (World, BuildTool, BuildSettings) {
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events().into_iter().chain(crate::demo::selection_cycle_events()) {
        world.apply(event);
    }
    let mut tool = BuildTool::default();
    tool.open_build(Tool::Edit);
    (world, tool, BuildSettings::default())
}

fn object(world: &World, local_id: u32) -> usize {
    world.objects.index_of_uuid(&crate::demo::action_id(local_id)).unwrap()
}

fn selected(tool: &BuildTool) -> Vec<u32> {
    tool.selection.iter().map(|key| key.local_id).collect()
}

#[test]
fn hud_edit_zoom_resets_on_world_body_empty_selection_and_close() {
    for target in [SelectType::World, SelectType::Attachment, SelectType::Hud] {
        let (mut world, mut tool, mut settings) = seated_fixture(false);
        let hud_id = crate::demo::hud::object(31, false).full_id;
        let hud = world.objects.index_of_uuid(&hud_id).unwrap();
        world.hud_scales = crate::world::hud_scale::HudScales::default();
        world.hud_scales.set(hud_id, 2.0);
        world.hud_scales.set_offset(hud_id, Vec3::Y * 0.2);
        let position = world.objects.get(hud).unwrap().position;
        tool.click_select(&mut world, &settings, Some(hud), false);
        tool.update(&mut world, &settings, (0.0, 0.0), true, Mods::default());
        world.hud_zoom = 0.4;
        let next = world
            .objects
            .iter()
            .find(|(idx, _)| root_of(&world, *idx) != hud && selectable(&world, *idx) && select_type(&world, *idx) == target)
            .unwrap()
            .0;
        tool.click_select(&mut world, &settings, Some(next), false);
        tool.update(&mut world, &settings, (0.0, 0.0), true, Mods::default());
        assert_eq!(world.hud_zoom, if target == SelectType::Hud { 0.4 } else { 1.0 });
        tool.click_select(&mut world, &settings, Some(hud), false);
        tool.update(&mut world, &settings, (0.0, 0.0), true, Mods::default());
        world.hud_zoom = 0.3;
        tool.deselect_all(&world);
        tool.update(&mut world, &settings, (0.0, 0.0), true, Mods::default());
        assert_eq!(world.hud_zoom, 1.0);
        tool.click_select(&mut world, &settings, Some(hud), false);
        tool.update(&mut world, &settings, (0.0, 0.0), true, Mods::default());
        world.hud_zoom = 0.2;
        tool.close(&mut world, &mut settings);
        assert_eq!(world.hud_zoom, 1.0);
        assert_eq!(world.hud_scale(hud), 2.0);
        assert_eq!(world.hud_scales.offset(hud_id), Vec3::Y * 0.2);
        assert_eq!(world.objects.get(hud).unwrap().position, position);
        // The explicit World > HUDs zoom remains usable outside HUD editing.
        world.hud_zoom = 0.5;
        tool.update(&mut world, &settings, (0.0, 0.0), true, Mods::default());
        assert_eq!(world.hud_zoom, 0.5);
    }
}

#[test]
fn removed_hud_selection_restores_view_zoom() {
    let (mut world, mut tool, settings) = seated_fixture(false);
    let hud = world.objects.index_of_uuid(&crate::demo::hud::object(31, false).full_id).unwrap();
    tool.click_select(&mut world, &settings, Some(hud), false);
    tool.update(&mut world, &settings, (0.0, 0.0), true, Mods::default());
    world.hud_zoom = 0.1;
    world.apply(NetEvent::ObjectsKilled {
        handle: world.objects.get(hud).unwrap().key.region,
        local_ids: vec![world.objects.get(hud).unwrap().key.local_id],
    });
    tool.update(&mut world, &settings, (0.0, 0.0), true, Mods::default());
    assert_eq!(world.hud_zoom, 1.0);
}

#[test]
fn creation_preview_follows_tool_state_and_defers_to_selection_modifiers() {
    let (_, mut tool, mut settings) = fixture();
    let mods = Mods::default();
    assert!(!tool.creation_preview_enabled(&settings, false, mods));
    tool.open_build(Tool::Create);
    assert!(tool.creation_preview_enabled(&settings, false, mods));
    assert!(!tool.creation_preview_enabled(&settings, true, mods));
    for mods in [
        Mods { shift: true, ..mods },
        Mods { ctrl: true, ..mods },
        Mods { alt: true, ..mods },
    ] {
        assert!(!tool.creation_preview_enabled(&settings, false, mods));
    }
    settings.copy_selection = true;
    assert!(!tool.creation_preview_enabled(&settings, false, mods));
    settings.copy_selection = false;
    settings.create_shape = shapes::shapes().len();
    assert!(!tool.creation_preview_enabled(&settings, false, mods));
    settings.create_shape = 8;
    assert!(tool.creation_preview_enabled(&settings, false, mods));
    tool.open = false;
    assert!(!tool.creation_preview_enabled(&settings, false, mods));
}

#[test]
fn transparent_selection_keeps_its_outline_during_a_real_move_drag() {
    let (mut world, mut tool, mut settings) = fixture();
    let idx = object(&world, 99614);
    let owner = world.agent_id;
    let o = world.objects.get_mut(idx).unwrap();
    o.owner_id = owner;
    o.update_flags = crate::ui::context::flags::OBJECT_MOVE | crate::ui::context::flags::OBJECT_MODIFY;
    let te = Arc::make_mut(o.te.as_mut().unwrap());
    for face in &mut te.faces {
        face.color[3] = 0.0;
    }
    let position = o.position;
    let eye = position + Vec3::new(8.0, -8.0, 6.0);
    let view = glam::camera::rh::view::look_at_mat4(eye, position, Vec3::Z);
    let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.0, 0.1);
    tool.cam = geom::Cam::new(eye, position, projection * view, 1.0, 800.0, 800.0, 1.0);
    world.agent.position = eye;
    settings.snap = false;
    tool.click_select(&mut world, &settings, Some(idx), false);
    let before = tool.outlined(&world, &settings);
    assert!(before.iter().any(|p| p.0 == idx));
    let handle = position + Vec3::Z * tool.cam.meters_for_pixels(position, 50.0) * 0.9;
    let cursor = tool.cam.project_px(handle).unwrap();
    assert!(tool.mouse_down(&mut world, &mut settings, None, None, None, cursor, Mods::default()));
    tool.update(&mut world, &settings, (cursor.0 + 30.0, cursor.1 - 30.0), false, Mods::default());
    assert!(tool.manip.dragging(), "The regression must exercise an active manipulation");
    assert_eq!(tool.outlined(&world, &settings), before);
    settings.show_highlight = false;
    assert!(tool.outlined(&world, &settings).is_empty());
}

#[test]
fn edit_pointing_tracks_selected_prims_and_clears_on_close_and_for_attachments() {
    let (mut world, mut tool, mut settings) = seated_fixture(false);
    let idx = world.objects.iter().find(|(_, o)| o.key.local_id == 950).unwrap().0;
    let child = family(&world, idx)[1];
    let now = Instant::now();
    tool.click_select(&mut world, &settings, Some(idx), false);
    let t = tool.pointing_target(&world, now).expect("selected object");
    assert_eq!(t.object, Some(world.objects.get(idx).unwrap().full_id));
    tool.point_pick = Some((world.objects.get(child).unwrap().key, glam::DVec3::new(0.2, -0.1, 0.3)));
    let picked = tool.pointing_target(&world, now).expect("picked child");
    assert_eq!(picked.object, Some(world.objects.get(child).unwrap().full_id));
    assert_eq!(picked.offset, tool.point_pick.unwrap().1);
    tool.close(&mut world, &mut settings);
    assert!(tool.pointing_target(&world, now).is_none());
    tool.open_build(Tool::Edit);
    let attachment = world
        .objects
        .iter()
        .find(|(i, _)| select_type(&world, *i) == SelectType::Attachment)
        .unwrap()
        .0;
    tool.click_select(&mut world, &settings, Some(attachment), false);
    assert!(tool.pointing_target(&world, now).is_none());
}

pub(super) fn seated_fixture(on_child: bool) -> (World, BuildTool, BuildSettings) {
    let (mut world, tool, settings) = fixture();
    for event in crate::demo::worn::events()
        .into_iter()
        .chain(crate::demo::hud::events())
        .chain(crate::demo::sit_events(250))
        .chain(crate::demo::sit_events(300))
    {
        let event = match event {
            NetEvent::ObjectUpdates { handle, mut objects } => {
                if on_child {
                    for object in &mut objects {
                        if object.full_id == world.agent_id {
                            object.parent_id = 951;
                        }
                    }
                }
                NetEvent::ObjectUpdates { handle, objects }
            }
            event => event,
        };
        world.apply(event);
    }
    (world, tool, settings)
}

#[test]
fn seated_linkset_selection_excludes_avatars_and_their_body_and_hud_attachments() {
    for on_child in [false, true] {
        let (mut world, mut tool, mut settings) = seated_fixture(on_child);
        let root = world.objects.iter().find(|(_, o)| o.key.local_id == 950).unwrap().0;
        let child = world.objects.index_of_uuid(&Uuid::from_u128(951)).unwrap();
        let root_key = world.objects.get(root).unwrap().key;
        let child_key = world.objects.get(child).unwrap().key;
        let now = Instant::now();
        assert_eq!(family(&world, root), [root, child]);
        assert_eq!(select_type(&world, root), SelectType::World);
        assert_eq!(select_type(&world, child), SelectType::World);
        for clicked in [root, child] {
            tool.click_select(&mut world, &settings, Some(clicked), false);
            tool.prune(&world, &settings, now);
            assert_eq!(tool.selection, [root_key]);
            assert_eq!(tool.highlighted(&world), [(root, true), (child, false)]);
            assert_eq!(tool.linkset_prims(&world), [root, child]);
            let bounds = tool.bounds(&world, now).unwrap();
            assert!(bounds.half.max_element() <= 0.51, "Avatar and HUD bounds cannot enlarge the seat");
            tool.set_edit_linked(&mut world, &mut settings, true);
            assert_eq!(tool.selection.len(), 2);
            assert!(tool.selection.contains(&root_key) && tool.selection.contains(&child_key));
            tool.click_select(&mut world, &settings, Some(child), false);
            assert_eq!(tool.selection, [child_key]);
            tool.set_edit_linked(&mut world, &mut settings, false);
            assert_eq!(tool.selection, [root_key]);
            tool.deselect_all(&world);
        }
        assert!(tool.select_for_menu(&mut world, &settings, root));
        tool.deselect_all(&world);
        for command in &tool.out {
            if let NetCommand::Build(BuildCmd::Select { local_ids, .. } | BuildCmd::Deselect { local_ids, .. }) = command {
                assert!(
                    local_ids.iter().all(|id| [950, 951].contains(id)),
                    "Unexpected selected occupant: {local_ids:?}"
                );
            }
        }
        let avatar = world.objects.index_of_uuid(&world.agent_id).unwrap();
        assert!(!selectable(&world, avatar));
        let backpack = world.objects.index_of_uuid(&crate::demo::worn::item_id()).unwrap();
        assert!(selectable(&world, backpack));
        assert_eq!(select_type(&world, backpack), SelectType::Attachment);
    }
}

#[test]
fn reflection_probe_selection_toggle_controls_rectangles_and_resets_at_startup() {
    let (mut world, mut tool, mut settings) = fixture();
    let probe = object(&world, 99614);
    let position = world.objects.get(probe).unwrap().position;
    world.objects.get_mut(probe).unwrap().extra.reflection_probe = Some(edits::PROBE_DEFAULT);
    let eye = position - Vec3::X * 10.0;
    let view = glam::camera::rh::view::look_at_mat4(eye, position, Vec3::Z);
    let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.0, 0.1);
    tool.cam = geom::Cam::new(eye, position, projection * view, 1.0, 800.0, 800.0, 1.0);
    world.agent.position = eye;
    assert!(!settings.select_probes);
    for enabled in [false, true, false] {
        settings.select_probes = enabled;
        tool.deselect_all(&world);
        tool.rect_select(&mut world, &settings, None, (0.0, 0.0), (800.0, 800.0), false);
        assert_eq!(selected(&tool).contains(&99614), enabled);
        assert!(!tool.selection.is_empty(), "Ordinary objects remain selectable");
    }
    settings.select_probes = true;
    let saved = serde_json::to_value(&settings).unwrap();
    assert!(saved.get("select_probes").is_none());
    let restored: BuildSettings = serde_json::from_value(saved).unwrap();
    assert!(!restored.select_probes);
    let restored: BuildSettings = serde_json::from_str(r#"{"select_probes":true}"#).unwrap();
    assert!(
        !restored.select_probes,
        "Old saved values must not enable probe selection at startup"
    );
}

#[test]
fn confirmed_link_reconciles_selection_colors_and_packet_order() {
    let (mut world, mut tool, settings) = fixture();
    let first = object(&world, 99610);
    let handle = world.objects.get(first).unwrap().key.region;
    let mut updates = Vec::new();
    for local_id in [99870, 99871] {
        let mut update = crate::demo::action_avatar(false);
        update.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        update.local_id = local_id;
        update.full_id = crate::demo::action_id(local_id);
        update.parent_id = 0;
        update.owner_id = world.agent_id;
        updates.push(update);
    }
    world.apply(NetEvent::ObjectUpdates { handle, objects: updates });
    for (id, extend) in [(99870, false), (99871, true)] {
        let idx = object(&world, id);
        tool.click_select(&mut world, &settings, Some(idx), extend);
    }
    tool.link(&mut world);
    assert_eq!(tool.status, aurora_i18n::tr!("build-link-pending"));
    assert_eq!(tool.highlighted(&world).iter().filter(|(_, root)| *root).count(), 2);
    let mut sim = demo_sim::DemoSim::default();
    for command in std::mem::take(&mut tool.out) {
        if let NetCommand::Build(command) = command {
            for mut event in sim.reply(&world, &command) {
                if let NetEvent::ObjectUpdates { objects, .. } = &mut event {
                    objects.reverse();
                }
                world.apply(event);
            }
        }
    }
    let highlighted = tool.highlighted(&world);
    assert_eq!(highlighted.len(), 2);
    assert_eq!(highlighted.iter().filter(|(_, root)| *root).count(), 1);
    tool.advance_link(&mut world, Instant::now());
    tool.prune(&world, &settings, Instant::now());
    assert_eq!(selected(&tool), [99871]);
    assert!(tool.pending_link.is_none());
    let root = object(&world, 99871);
    assert_eq!(family(&world, root), vec![root, object(&world, 99870)]);
    assert_eq!(tool.highlighted(&world), highlighted);
}

/// Three free prims of ours, linked through the demo simulator with
/// `99882` as root; the tool keeps the linkset selected.
fn linked_three(world: &mut World, tool: &mut BuildTool, settings: &BuildSettings, sim: &mut demo_sim::DemoSim) -> Vec<u32> {
    let handle = world.objects.get(object(world, 99610)).unwrap().key.region;
    let ids = [99880, 99881, 99882];
    let mut updates = Vec::new();
    for local_id in ids {
        let mut update = crate::demo::action_avatar(false);
        update.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        update.local_id = local_id;
        update.full_id = crate::demo::action_id(local_id);
        update.parent_id = 0;
        update.owner_id = world.agent_id;
        updates.push(update);
    }
    world.apply(NetEvent::ObjectUpdates { handle, objects: updates });
    for (i, id) in ids.iter().enumerate() {
        let idx = object(world, *id);
        tool.click_select(world, settings, Some(idx), i > 0);
    }
    tool.link(world);
    run_sim(world, tool, sim);
    tool.advance_link(world, Instant::now());
    tool.prune(world, settings, Instant::now());
    assert_eq!(selected(tool), [99882]);
    assert_eq!(family(world, object(world, 99882)).len(), 3);
    ids.to_vec()
}

fn run_sim(world: &mut World, tool: &mut BuildTool, sim: &mut demo_sim::DemoSim) {
    for command in std::mem::take(&mut tool.out) {
        if let NetCommand::Build(command) = command {
            for event in sim.reply(world, &command) {
                world.apply(event);
            }
        }
    }
}

#[test]
fn unlink_keeps_every_former_prim_selected() {
    let (mut world, mut tool, settings) = fixture();
    let mut sim = demo_sim::DemoSim::default();
    linked_three(&mut world, &mut tool, &settings, &mut sim);
    tool.unlink(&mut world);
    // nothing changes before the simulator answers
    tool.prune(&world, &settings, Instant::now());
    assert_eq!(selected(&tool), [99882]);
    run_sim(&mut world, &mut tool, &mut sim);
    tool.prune(&world, &settings, Instant::now());
    // LLSelectMgr::sendDelink: each prim stays selected as its own object,
    // the former root first
    let mut ids = selected(&tool);
    assert_eq!(ids[0], 99882);
    ids.sort_unstable();
    assert_eq!(ids, [99880, 99881, 99882]);
    assert!(tool.highlighted(&world).iter().all(|(_, root)| *root));
    assert_eq!(tool.highlighted(&world).len(), 3);
    assert!(tool.pending_unlink.is_none());
    // and once more: the selection stays as it is
    tool.prune(&world, &settings, Instant::now());
    assert_eq!(tool.selection.len(), 3);
}

#[test]
fn unlink_leaves_a_selection_changed_before_the_reply_alone() {
    let (mut world, mut tool, settings) = fixture();
    let mut sim = demo_sim::DemoSim::default();
    linked_three(&mut world, &mut tool, &settings, &mut sim);
    tool.unlink(&mut world);
    let delink = std::mem::take(&mut tool.out);
    tool.deselect_all(&world);
    tool.out = delink;
    run_sim(&mut world, &mut tool, &mut sim);
    tool.prune(&world, &settings, Instant::now());
    assert!(tool.selection.is_empty());
    assert!(tool.pending_unlink.is_none());
}

#[test]
fn typed_transforms_require_effective_move_rights_before_local_preview_or_commit() {
    use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_MOVE};
    let (mut world, mut tool, mut settings) = fixture();
    let idx = object(&world, 99610);
    tool.click_select(&mut world, &settings, Some(idx), false);
    for linked in [false, true] {
        settings.edit_linked = linked;
        for flags in [0, OBJECT_MODIFY, OBJECT_MOVE, OBJECT_MOVE | OBJECT_MODIFY] {
            world.objects.get_mut(idx).unwrap().update_flags = flags;
            for operation in 0..3 {
                for commit in [false, true] {
                    let object = world.objects.get(idx).unwrap();
                    let before = (object.position, object.scale, object.rotation);
                    let (pos, size, rot) = match operation {
                        0 => (Some(before.0 + Vec3::X), None, None),
                        1 => (None, Some(before.1 + Vec3::splat(0.1)), None),
                        _ => (None, None, Some(before.2 * Quat::from_rotation_z(0.1))),
                    };
                    tool.out.clear();
                    tool.set_transform(&mut world, &settings, pos, size, rot, commit);
                    let allowed = flags & OBJECT_MOVE != 0 && (!(linked || operation == 1) || flags & OBJECT_MODIFY != 0);
                    let object = world.objects.get(idx).unwrap();
                    assert_eq!((object.position, object.scale, object.rotation) != before, allowed);
                    assert_eq!(!tool.out.is_empty(), allowed && commit);
                }
            }
        }
    }
}

#[test]
fn linked_world_transforms_inherit_root_rights_even_when_child_flags_differ() {
    use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_MOVE};
    let (mut world, mut tool, mut settings) = fixture();
    let root = object(&world, 99610);
    let child = object(&world, 99611);
    settings.edit_linked = true;
    tool.click_select(&mut world, &settings, Some(child), false);
    world.objects.get_mut(child).unwrap().update_flags = OBJECT_MOVE | OBJECT_MODIFY;
    world.objects.get_mut(root).unwrap().update_flags = 0;
    tool.out.clear();
    let before = world.objects.get(child).unwrap().position;
    let key = world.objects.get(child).unwrap().key;
    let target = crate::scene::Scene::object_transform(&world, child, Instant::now(), 0).unwrap().0
        - world.region_offset(key.region).unwrap()
        + Vec3::X;
    tool.set_transform(&mut world, &settings, Some(target), None, None, true);
    assert_eq!(world.objects.get(child).unwrap().position, before);
    assert!(tool.out.is_empty());
    world.objects.get_mut(child).unwrap().update_flags = 0;
    world.objects.get_mut(root).unwrap().update_flags = OBJECT_MOVE | OBJECT_MODIFY;
    tool.set_transform(&mut world, &settings, Some(target), None, None, true);
    assert_ne!(world.objects.get(child).unwrap().position, before);
    assert_eq!(tool.out.len(), 1);
}

#[test]
fn first_plain_click_after_enabling_linked_edit_selects_only_the_clicked_part() {
    let (mut world, mut tool, mut settings) = fixture();
    let root = object(&world, 99610);
    let child = object(&world, 99611);
    tool.click_select(&mut world, &settings, Some(root), false);
    tool.set_edit_linked(&mut world, &mut settings, true);
    assert_eq!(tool.selection.len(), 2);
    tool.out.clear();
    tool.click_select(&mut world, &settings, Some(child), false);
    assert_eq!(selected(&tool), [99611]);
    assert!(tool.selection_linked);
    assert!(matches!(&tool.out[..],
        [NetCommand::Build(BuildCmd::Deselect { local_ids: old, .. }),
         NetCommand::Build(BuildCmd::Select { local_ids: new, .. })]
         if old.len() == 2 && old.contains(&99610) && old.contains(&99611) && new == &[99611]));
    tool.set_edit_linked(&mut world, &mut settings, false);
    assert_eq!(selected(&tool), [99610]);
    assert_eq!(tool.highlighted(&world).len(), 2);
}

#[test]
fn linked_extension_toggles_parts_and_plain_click_reduces_a_multiple_selection() {
    let (mut world, mut tool, mut settings) = fixture();
    let root = object(&world, 99610);
    let child = object(&world, 99611);
    tool.set_edit_linked(&mut world, &mut settings, true);
    tool.click_select(&mut world, &settings, Some(root), false);
    tool.click_select(&mut world, &settings, Some(child), true);
    assert_eq!(selected(&tool), [99611, 99610]);
    tool.click_select(&mut world, &settings, Some(child), true);
    assert_eq!(selected(&tool), [99610]);
    tool.click_select(&mut world, &settings, Some(child), true);
    tool.click_select(&mut world, &settings, Some(root), false);
    assert_eq!(selected(&tool), [99610]);
}

#[test]
fn plain_click_on_a_selected_whole_object_keeps_the_other_selected_objects() {
    let (mut world, mut tool, settings) = fixture();
    let first = object(&world, 99610);
    let other = object(&world, 99612);
    tool.click_select(&mut world, &settings, Some(first), false);
    tool.click_select(&mut world, &settings, Some(other), true);
    tool.out.clear();
    tool.click_select(&mut world, &settings, Some(first), false);
    assert_eq!(selected(&tool), [99610, 99612]);
    assert!(tool.out.is_empty());
    assert!(!tool.selection_linked);
}

#[test]
fn face_click_and_shift_toggle_faces_across_prims_and_objects() {
    let (mut world, mut tool, mut settings) = fixture();
    tool.set_edit_mode(&mut world, &mut settings, EditMode::Face);
    let a = object(&world, 99610);
    let child = object(&world, 99611);
    let b = object(&world, 99612);
    tool.face_click(&mut world, Some(a), Some(1), false);
    tool.face_click(&mut world, Some(a), Some(3), true);
    tool.face_click(&mut world, Some(child), Some(2), true);
    tool.face_click(&mut world, Some(b), Some(4), true);
    let faces = |tool: &BuildTool, idx| tool.sel_faces(&world, idx);
    assert_eq!(faces(&tool, a), [1, 3]);
    assert_eq!(faces(&tool, child), [2]);
    assert_eq!(faces(&tool, b), [4]);
    assert_eq!(tool.sel_te(&world).len(), 4);
    tool.face_click(&mut world, Some(a), Some(3), true);
    tool.out.clear();
    tool.face_click(&mut world, Some(child), Some(2), true);
    assert_eq!(tool.sel_faces(&world, a), [1]);
    assert_eq!(tool.sel_faces(&world, b), [4]);
    assert!(!tool.is_selected(&world.objects.get(child).unwrap().key));
    assert!(matches!(&tool.out[..], [NetCommand::Build(BuildCmd::Deselect { local_ids, .. })] if local_ids == &[99611]));
    tool.face_click(&mut world, Some(b), Some(4), false);
    assert_eq!(selected(&tool), [99612]);
    assert_eq!(tool.sel_te(&world), [(b, 4)]);
}

#[test]
fn first_shift_face_click_preserves_the_other_faces_of_whole_linksets() {
    let (mut world, mut tool, mut settings) = fixture();
    let root = object(&world, 99610);
    let child = object(&world, 99611);
    let other = object(&world, 99612);
    tool.click_select(&mut world, &settings, Some(root), false);
    tool.click_select(&mut world, &settings, Some(other), true);
    let prims = tool.sel_prims(&world);
    tool.set_edit_mode(&mut world, &mut settings, EditMode::Face);
    tool.face_click(&mut world, Some(child), Some(2), true);
    assert_eq!(tool.sel_prims(&world).len(), prims.len());
    assert!(!tool.sel_faces(&world, child).contains(&2));
    assert_eq!(tool.sel_faces(&world, root).len(), tool.num_faces(&world, root));
    assert_eq!(tool.sel_faces(&world, other).len(), tool.num_faces(&world, other));
    tool.face_click(&mut world, Some(child), Some(2), true);
    assert_eq!(tool.sel_faces(&world, child).len(), tool.num_faces(&world, child));
    assert!(!settings.edit_linked);
}

#[test]
fn removing_the_last_face_clears_the_selection_and_its_highlight() {
    let (mut world, mut tool, mut settings) = fixture();
    let idx = object(&world, 99610);
    tool.set_edit_mode(&mut world, &mut settings, EditMode::Face);
    tool.face_click(&mut world, Some(idx), Some(1), false);
    assert_eq!(tool.face_selection(&world, &settings)[&idx], 1 << 1);
    tool.face_click(&mut world, Some(idx), Some(1), true);
    assert!(tool.selection.is_empty());
    assert!(tool.faces.is_empty());
    assert!(tool.face_selection(&world, &settings).is_empty());
    assert!(tool.sel_te(&world).is_empty());
}

#[test]
fn face_highlights_replace_whole_object_outlines_only_in_face_mode() {
    let (mut world, mut tool, mut settings) = fixture();
    let idx = object(&world, 99610);
    tool.set_edit_mode(&mut world, &mut settings, EditMode::Face);
    tool.face_click(&mut world, Some(idx), Some(1), false);
    tool.face_click(&mut world, Some(idx), Some(3), true);
    assert_eq!(tool.face_selection(&world, &settings)[&idx], (1 << 1) | (1 << 3));
    assert!(tool.wire_selection(&world, &settings).is_empty());
    assert!(tool.silhouette_selection(&world, &settings).is_empty());
    settings.show_highlight = false;
    assert!(tool.face_selection(&world, &settings).is_empty());
    settings.show_highlight = true;
    tool.set_edit_mode(&mut world, &mut settings, EditMode::Move);
    assert!(tool.face_selection(&world, &settings).is_empty());
    assert!(tool.faces.is_empty());
}

#[test]
fn face_masks_keep_texture_entry_indices_across_the_32_bit_boundary() {
    let (mut world, mut tool, mut settings) = fixture();
    let idx = object(&world, 99610);
    let id = world.objects.get(idx).unwrap().full_id;
    tool.face_counts.insert(id, aurora_prim::te::MAX_TES);
    tool.set_edit_mode(&mut world, &mut settings, EditMode::Face);
    for (n, face) in [0, 31, 32, 44].into_iter().enumerate() {
        tool.face_click(&mut world, Some(idx), Some(face), n != 0);
    }
    assert_eq!(
        tool.face_selection(&world, &settings)[&idx],
        1 | (1u64 << 31) | (1u64 << 32) | (1u64 << 44)
    );
    tool.face_click(&mut world, Some(idx), Some(32), true);
    assert_eq!(tool.face_selection(&world, &settings)[&idx], 1 | (1u64 << 31) | (1u64 << 44));
}

#[test]
fn face_stepping_reaches_every_face_of_a_local_mesh_preview() {
    // #1353: the preview replaces the volume (VJLocalMeshObject), so ◄ ►
    // walks the local mesh's faces, not the fewer ones of the worn prim.
    let (mut world, mut tool, mut settings) = fixture();
    let idx = object(&world, 99610);
    let prim_faces = tool.num_faces(&world, idx);
    let o = world.objects.get_mut(idx).unwrap();
    let mut volume = o.volume;
    volume.sculpt = Some(aurora_prim::params::SculptParams {
        texture: uuid::Uuid::from_u128(0x1353),
        sculpt_type: aurora_prim::params::LL_SCULPT_TYPE_MESH,
    });
    o.render.local_volume = Some(Box::new(volume));
    // before the local asset is counted: the mesh default
    assert!(prim_faces < 8);
    assert_eq!(tool.num_faces(&world, idx), 8);
    tool.set_edit_mode(&mut world, &mut settings, EditMode::Face);
    tool.face_click(&mut world, Some(idx), Some(0), false);
    for face in 1..8u8 {
        tool.select_next_part(&mut world, &mut settings, true, false);
        assert_eq!(tool.sel_te(&world), [(idx, face)]);
    }
    tool.select_next_part(&mut world, &mut settings, true, false);
    assert_eq!(tool.sel_te(&world), [(idx, 0)]);
    tool.select_next_part(&mut world, &mut settings, false, false);
    assert_eq!(tool.sel_te(&world), [(idx, 7)]);
}

#[test]
fn invalid_face_and_shift_click_on_empty_space_preserve_selection() {
    let (mut world, mut tool, mut settings) = fixture();
    let idx = object(&world, 99610);
    tool.set_edit_mode(&mut world, &mut settings, EditMode::Face);
    tool.face_click(&mut world, Some(idx), Some(1), false);
    tool.face_click(&mut world, Some(idx), Some(255), false);
    tool.face_click(&mut world, None, None, true);
    assert_eq!(tool.sel_te(&world), [(idx, 1)]);
    tool.face_click(&mut world, None, None, false);
    assert!(tool.sel_te(&world).is_empty());
}

#[test]
fn new_prim_is_selected_once_without_update_owner_id_and_deleted_without_false_warning() {
    let (mut world, mut tool, settings) = fixture();
    tool.expect_created(&world);
    let mut update = crate::demo::action_avatar(false);
    update.pcode = aurora_prim::params::LL_PCODE_VOLUME;
    update.local_id = 99940;
    update.full_id = crate::demo::action_id(99940);
    update.parent_id = 0;
    update.owner_id = Uuid::nil();
    update.update_flags = FLAGS_CREATE_SELECTED | crate::ui::context::flags::OBJECT_YOU_OWNER;
    let handle = world.objects.get(object(&world, 99610)).unwrap().key.region;
    world.apply(NetEvent::ObjectUpdates {
        handle,
        objects: vec![update],
    });
    tool.pick_up_created(&world, &settings);
    assert_eq!(selected(&tool), [99940]);
    let sends = tool.out.len();
    tool.pick_up_created(&world, &settings);
    assert_eq!(tool.out.len(), sends);
    tool.delete(&mut world);
    assert_eq!(tool.status, aurora_i18n::tr!("build-mod-object-s-sent-to-trash", value0 = "1"));
    assert!(tool.selection.is_empty());
}

#[test]
fn preexisting_create_flags_are_never_selected_after_ownership_arrives() {
    let (mut world, mut tool, settings) = fixture();
    let idx = object(&world, 99610);
    let o = world.objects.get_mut(idx).unwrap();
    o.owner_id = Uuid::nil();
    o.update_flags |= FLAGS_CREATE_SELECTED;
    tool.expect_created(&world);
    world.objects.get_mut(idx).unwrap().owner_id = world.agent_id;
    tool.pick_up_created(&world, &settings);
    assert!(tool.selection.is_empty());
}

#[test]
fn creation_rez_group_reaches_prim_plant_and_demo_properties() {
    let (mut world, mut tool, mut settings) = fixture();
    for event in crate::demo::demo_reply(&NetCommand::RequestGroups) {
        world.apply(event);
    }
    let land = world.groups.groups[0].id;
    let active = Uuid::from_u128(8800);
    let other = Uuid::from_u128(8899);
    let nil = Uuid::nil();
    for (active, parcel, under_land, expected) in [
        (active, Some(land), true, land),
        (active, Some(land), false, active),
        (active, Some(other), true, active),
        (active, Some(nil), true, active),
        (active, None, true, active),
        (nil, Some(land), true, land),
        (nil, Some(land), false, nil),
        (nil, Some(other), true, nil),
        (nil, None, true, nil),
    ] {
        world.apply(NetEvent::ActiveGroup {
            id: active,
            name: "Active group".into(),
            title: "Builder".into(),
        });
        world.parcel = parcel.map(|group_id| {
            Arc::new(aurora_net::ParcelInfo {
                group_id,
                ..Default::default()
            })
        });
        settings.rez_under_land_group = under_land;
        for shape in [0, shapes::shapes().len(), shapes::shapes().len() + 1] {
            settings.create_shape = shape;
            let point = world.agent.position + Vec3::X * 5.0;
            tool.place(&mut world, &settings, Some(point), None, (0.0, 0.0));
            let commands = std::mem::take(&mut tool.out);
            let prims: Vec<_> = commands
                .iter()
                .filter_map(|command| match command {
                    NetCommand::Build(BuildCmd::Add(prim)) => Some(prim),
                    _ => None,
                })
                .collect();
            assert_eq!(prims.len(), 1);
            let prim = prims[0];
            assert_eq!(prim.group_id, expected, "shape {shape}, land mode {under_land}");
            let mut sim = demo_sim::DemoSim::default();
            let command = BuildCmd::Add(prim.clone());
            let events = sim.reply(&world, &command);
            let NetEvent::ObjectUpdates { objects, .. } = &events[0] else {
                panic!("expected created object");
            };
            let id = objects[0].full_id;
            let local_id = objects[0].local_id;
            for event in events {
                world.apply(event);
            }
            let replies = sim.reply(
                &world,
                &BuildCmd::Select {
                    handle: prim.handle,
                    local_ids: vec![local_id],
                },
            );
            let props = replies
                .iter()
                .find_map(|event| match event {
                    NetEvent::ObjectProperties(props) => props.iter().find(|props| props.object_id == id),
                    _ => None,
                })
                .expect("created object properties");
            assert_eq!(props.group_id, expected, "demo must preserve even an empty group");
        }
    }
}

#[test]
fn copy_selection_uses_the_current_rez_group_on_land_and_objects() {
    let (mut world, mut tool, mut settings) = fixture();
    for event in crate::demo::demo_reply(&NetCommand::RequestGroups) {
        world.apply(event);
    }
    let land = world.groups.groups[0].id;
    let active = Uuid::from_u128(8800);
    let nil = Uuid::nil();
    let source = object(&world, 99610);
    let source_key = world.objects.get(source).unwrap().key;
    let target = object(&world, 99614);
    settings.copy_selection = true;
    settings.copy_centers = true;
    settings.copy_rotates = false;
    for (active, parcel, under_land, expected) in [
        (active, Some(land), true, land),
        (active, Some(land), false, active),
        (active, Some(Uuid::from_u128(8899)), true, active),
        (active, Some(nil), true, active),
        (active, None, true, active),
        (nil, Some(land), true, land),
        (nil, Some(land), false, nil),
        (nil, None, true, nil),
    ] {
        world.apply(NetEvent::ActiveGroup {
            id: active,
            name: "Active group".into(),
            title: "Builder".into(),
        });
        world.parcel = parcel.map(|group_id| {
            Arc::new(aurora_net::ParcelInfo {
                group_id,
                ..Default::default()
            })
        });
        settings.rez_under_land_group = under_land;
        for target in [None, Some(target)] {
            tool.click_select(&mut world, &settings, Some(source), false);
            tool.open_build(Tool::Create);
            tool.out.clear();
            let point = world.agent.position + Vec3::X * 5.0;
            assert!(tool.mouse_down(&mut world, &mut settings, Some(point), target, None, (0.0, 0.0), Mods::default()));
            let copies: Vec<_> = tool
                .out
                .iter()
                .filter_map(|command| match command {
                    NetCommand::Build(BuildCmd::DuplicateOnRay {
                        group_id,
                        local_ids,
                        bypass_raycast,
                        ray_target,
                        copy_centers,
                        copy_rotates,
                        ..
                    }) => {
                        assert_eq!(local_ids, &[99610]);
                        assert_eq!(*bypass_raycast, target.is_none());
                        assert_eq!(*ray_target, target.map_or(nil, |idx| world.objects.get(idx).unwrap().full_id));
                        assert!(*copy_centers && !*copy_rotates);
                        Some(*group_id)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(copies, [expected]);
            assert_eq!(tool.selection, [source_key], "Copy selection retains the original selection");
        }
    }
}

#[test]
fn primitive_placement_and_deletion_round_trip_through_the_demo_simulator() {
    let (mut world, mut tool, mut settings) = fixture();
    tool.open_build(Tool::Create);
    settings.copy_selection = false;
    let x = world.agent.position.x + 5.0;
    let y = world.agent.position.y + 3.0;
    let point = Vec3::new(x, y, crate::demo::floor_at(x, y));
    tool.mouse_down(&mut world, &mut settings, Some(point), None, None, (0.0, 0.0), Mods::default());
    let mut sim = demo_sim::DemoSim::default();
    for cmd in std::mem::take(&mut tool.out) {
        if let NetCommand::Build(cmd) = cmd {
            for event in sim.reply(&world, &cmd) {
                world.apply(event);
            }
        }
    }
    tool.pick_up_created(&world, &settings);
    assert_eq!(tool.selection.len(), 1);
    let key = tool.selection[0];
    assert!(world.objects.get(world.objects.index_of(&key).unwrap()).unwrap().owner_id.is_nil());
    tool.out.clear();
    tool.delete(&mut world);
    assert_eq!(tool.status, aurora_i18n::tr!("build-mod-object-s-sent-to-trash", value0 = "1"));
    for cmd in std::mem::take(&mut tool.out) {
        if let NetCommand::Build(cmd) = cmd {
            for event in sim.reply(&world, &cmd) {
                world.apply(event);
            }
        }
    }
    assert!(world.objects.index_of(&key).is_none());
}

#[test]
fn select_many_takes_each_linkset_once_in_one_message_and_leaves_attachments_out() {
    let (mut world, mut tool, settings) = seated_fixture(false);
    let root = world.objects.iter().find(|(_, o)| o.key.local_id == 950).unwrap().0;
    let child = world.objects.index_of_uuid(&Uuid::from_u128(951)).unwrap();
    let other = object(&world, 99614);
    let attachment = world
        .objects
        .iter()
        .find(|(i, _)| select_type(&world, *i) == SelectType::Attachment)
        .unwrap()
        .0;
    tool.click_select(&mut world, &settings, Some(other), false);
    tool.out.clear();
    tool.select_many(&mut world, &settings, &[child, other, root, attachment]);
    let keys = |idxs: &[usize]| idxs.iter().map(|&i| world.objects.get(i).unwrap().key).collect::<Vec<_>>();
    assert_eq!(
        tool.selection,
        keys(&[root, other]),
        "the first row is primary, a linkset counts once"
    );
    let selects: Vec<_> = tool
        .out
        .iter()
        .filter_map(|c| match c {
            NetCommand::Build(BuildCmd::Select { local_ids, .. }) => Some(local_ids.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(selects.len(), 1, "one ObjectSelect for the region: {selects:?}");
    let mut sent = selects[0].clone();
    sent.sort_unstable();
    assert_eq!(sent, [950, 951, 99614]);
}

#[test]
fn far_selection_stays_unless_the_select_distance_is_limited() {
    let (mut world, mut tool, mut settings) = fixture();
    let idx = object(&world, 99610);
    tool.click_select(&mut world, &settings, Some(idx), false);
    let key = tool.selection.clone();
    assert_eq!(key.len(), 1);
    let home = world.agent.position;
    world.agent.position = home + Vec3::X * (MAX_SELECT_DISTANCE + 50.0);
    // LimitSelectDistance off: nothing is deselected
    settings.limit_select_distance = false;
    tool.prune(&world, &settings, Instant::now());
    assert_eq!(tool.selection, key);
    // on, LLSelectMgr::updateSelectionCenter deselects past MaxSelectDistance
    settings.limit_select_distance = true;
    settings.max_select_distance = MAX_SELECT_DISTANCE + 100.0;
    tool.prune(&world, &settings, Instant::now());
    assert_eq!(tool.selection, key);
    settings.max_select_distance = 20.0;
    tool.prune(&world, &settings, Instant::now());
    assert!(tool.selection.is_empty());
    assert_eq!(tool.status, aurora_i18n::tr!("build-mod-selection-too-far-away-deselected"));
}

#[test]
fn distance_limits_follow_their_switches_and_stay_in_range() {
    let mut s = BuildSettings::default();
    assert_eq!(s.select_limit(), Some(MAX_SELECT_DISTANCE));
    assert_eq!(s.drag_limit(), Some(MAX_DRAG_DISTANCE));
    s.limit_select_distance = false;
    assert_eq!(s.select_limit(), None);
    s.limit_select_distance = true;
    s.max_select_distance = 5000.0;
    assert_eq!(s.select_limit(), Some(1024.0));
    s.max_select_distance = f32::NAN;
    assert_eq!(s.select_limit(), Some(MAX_SELECT_DISTANCE));
    s.limit_drag_distance = false;
    assert_eq!(s.drag_limit(), None);
    // older settings files read the new fields' defaults
    let old: BuildSettings = serde_json::from_str("{}").unwrap();
    assert!(old.limit_select_distance && old.limit_drag_distance);
}

/// Gives `idx`'s root the update flags `flags`.
fn set_root_flags(world: &mut World, idx: usize, flags: u32) {
    let root = root_of(world, idx);
    world.objects.get_mut(root).unwrap().update_flags = flags;
}

#[test]
fn select_only_my_objects_filters_clicks_lists_and_wheel_but_not_the_menu() {
    use crate::ui::context::flags::{OBJECT_MOVE, OBJECT_YOU_OWNER};
    let (mut world, mut tool, mut settings) = fixture();
    let mine = object(&world, 99610);
    let theirs = object(&world, 99614);
    set_root_flags(&mut world, mine, OBJECT_YOU_OWNER | OBJECT_MOVE);
    set_root_flags(&mut world, theirs, OBJECT_MOVE);
    let id = |world: &World, idx: usize| world.objects.get(root_of(world, idx)).unwrap().key.local_id;
    let (mine_id, theirs_id) = (id(&world, mine), id(&world, theirs));
    let click = |tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, idx: usize, shift: bool| {
        let mods = Mods { shift, ..Mods::default() };
        tool.mouse_down(world, s, None, Some(idx), None, (0.0, 0.0), mods);
        tool.mouse_up(world, s, None);
    };
    // off by default: anything editable is selected
    assert!(!settings.select_owned_only);
    click(&mut tool, &mut world, &mut settings, theirs, false);
    assert_eq!(selected(&tool), [theirs_id]);
    settings.select_owned_only = true;
    // a click on someone else's object selects nothing and drops the rest
    click(&mut tool, &mut world, &mut settings, theirs, false);
    assert!(tool.selection.is_empty());
    click(&mut tool, &mut world, &mut settings, mine, false);
    assert_eq!(selected(&tool), [mine_id]);
    click(&mut tool, &mut world, &mut settings, theirs, true);
    assert_eq!(selected(&tool), [mine_id], "Shift does not add it either");
    tool.select_many(&mut world, &settings, &[theirs, mine]);
    assert_eq!(selected(&tool), [mine_id], "Area Search keeps only our own");
    tool.deselect_all(&world);
    tool.cycle_under_cursor(&mut world, &settings, &[theirs, mine], -1);
    assert_eq!(selected(&tool), [mine_id], "the wheel skips what may not be selected");
    // the right-click menu's temporary selection ignores the filter (temp_select)
    assert!(tool.select_for_menu(&mut world, &settings, theirs));
    assert_eq!(selected(&tool), [theirs_id]);
}

#[test]
fn select_only_my_objects_filters_rectangles() {
    use crate::ui::context::flags::{OBJECT_MOVE, OBJECT_YOU_OWNER};
    let (mut world, mut tool, mut settings) = fixture();
    let mine = object(&world, 99614);
    let position = world.objects.get(mine).unwrap().position;
    let eye = position - Vec3::X * 10.0;
    let view = glam::camera::rh::view::look_at_mat4(eye, position, Vec3::Z);
    let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.0, 0.1);
    tool.cam = geom::Cam::new(eye, position, projection * view, 1.0, 800.0, 800.0, 1.0);
    world.agent.position = eye;
    let all: Vec<usize> = world.objects.iter().map(|(i, _)| i).collect();
    for idx in all {
        set_root_flags(&mut world, idx, OBJECT_MOVE);
    }
    set_root_flags(&mut world, mine, OBJECT_YOU_OWNER | OBJECT_MOVE);
    tool.rect_select(&mut world, &settings, None, (0.0, 0.0), (800.0, 800.0), false);
    assert!(tool.selection.len() > 1, "without the filter the rectangle takes several objects");
    tool.deselect_all(&world);
    settings.select_owned_only = true;
    tool.rect_select(&mut world, &settings, None, (0.0, 0.0), (800.0, 800.0), false);
    let root = world.objects.get(root_of(&world, mine)).unwrap().key;
    assert_eq!(tool.selection, [root]);
}

#[test]
fn select_only_movable_objects_follows_root_rights_and_permanent_objects() {
    use crate::ui::context::flags::{AFFECTS_NAVMESH, OBJECT_MOVE, OBJECT_YOU_OWNER};
    let (mut world, _, mut settings) = seated_fixture(false);
    let root = world.objects.iter().find(|(_, o)| o.key.local_id == 950).unwrap().0;
    let child = world.objects.index_of_uuid(&Uuid::from_u128(951)).unwrap();
    world.objects.get_mut(child).unwrap().update_flags = 0;
    world.objects.get_mut(root).unwrap().update_flags = OBJECT_YOU_OWNER;
    assert!(passes_select_filters(&world, &settings, child), "no filter on");
    settings.select_owned_only = true;
    assert!(passes_select_filters(&world, &settings, child), "a part follows its root's owner");
    settings.select_owned_only = false;
    settings.select_movable_only = true;
    assert!(!passes_select_filters(&world, &settings, child));
    world.objects.get_mut(root).unwrap().update_flags = OBJECT_MOVE;
    assert!(passes_select_filters(&world, &settings, child));
    // a permanent (pathfinding) object only moves in the agent's region
    world.objects.get_mut(root).unwrap().update_flags = OBJECT_MOVE | AFFECTS_NAVMESH;
    world.main_region = Some(world.objects.get(root).unwrap().key.region);
    assert!(passes_select_filters(&world, &settings, root));
    world.main_region = None;
    assert!(!passes_select_filters(&world, &settings, root));
}

#[test]
fn selection_filters_persist_and_sit_under_the_reflection_probes() {
    use layout::{Item, Layout, Panel};
    let mut settings = BuildSettings::default();
    assert!(!settings.select_owned_only && !settings.select_movable_only);
    settings.select_owned_only = true;
    settings.select_movable_only = true;
    let restored: BuildSettings = serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap();
    assert!(restored.select_owned_only && restored.select_movable_only);
    let old: BuildSettings = serde_json::from_str("{}").unwrap();
    assert!(!old.select_owned_only && !old.select_movable_only);
    // a layout saved before the filters existed gets them right under the probes
    let mut saved = Layout::default();
    saved.order.retain(|item| !matches!(item, Item::OwnedOnly | Item::MovableOnly));
    saved.locations.remove(&Item::OwnedOnly);
    saved.locations.remove(&Item::MovableOnly);
    saved.place(Item::Probes, Panel::Settings, Some(Item::FollowEdit), true);
    saved.normalize();
    let panel = saved.in_panel(Panel::Settings);
    let probes = panel.iter().position(|item| *item == Item::Probes).unwrap();
    assert_eq!(panel[probes + 1..probes + 3], [Item::OwnedOnly, Item::MovableOnly]);
    assert_eq!(
        Layout::default().in_panel(Panel::Settings)[..5],
        [Item::Snap, Item::Highlight, Item::Probes, Item::OwnedOnly, Item::MovableOnly]
    );
    // notecards exported before them still load, with the filters off
    let (_, tool, _) = fixture();
    let mut profile = serde_json::to_value(layout::Profile::capture(&tool, &settings)).unwrap();
    let options = profile["options"].as_object_mut().unwrap();
    assert_eq!(options["ownedOnly"], true);
    options.remove("ownedOnly");
    options.remove("movableOnly");
    let read = layout::Profile::read(&profile.to_string()).unwrap();
    assert!(!read.options.owned_only && !read.options.movable_only);
}

/// A camera 10 m in front of `target`, on an 800 × 800 viewport.
fn camera_on(target: Vec3) -> geom::Cam {
    let eye = target - Vec3::X * 10.0;
    let view = glam::camera::rh::view::look_at_mat4(eye, target, Vec3::Z);
    let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.0, 0.1);
    geom::Cam::new(eye, target, projection * view, 1.0, 800.0, 800.0, 1.0)
}

#[test]
fn rectangle_cover_of_a_box_and_its_vertices() {
    use geom::RectCover;
    let cam = camera_on(Vec3::ZERO);
    let model = glam::Mat4::IDENTITY;
    let (min, max) = (Vec3::splat(-0.5), Vec3::splat(0.5));
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1));
    assert_eq!(cam.box_in_rect(model, min, max, rect(0.0, 0.0, 800.0, 800.0)), RectCover::Inside);
    assert_eq!(cam.box_in_rect(model, min, max, rect(0.0, 0.0, 100.0, 100.0)), RectCover::Outside);
    assert_eq!(cam.box_in_rect(model, min, max, rect(0.0, 0.0, 400.0, 400.0)), RectCover::Across);
    // a rectangle inside the box's footprint touches it without a corner in it
    assert_eq!(
        cam.box_in_rect(model, min, max, rect(395.0, 395.0, 405.0, 405.0)),
        RectCover::Across
    );
    let behind = glam::Mat4::from_translation(-Vec3::X * 20.0);
    assert_eq!(cam.box_in_rect(behind, min, max, rect(0.0, 0.0, 800.0, 800.0)), RectCover::Outside);
    // areVertsVisible: all of them with Select by surrounding, one without
    assert!(geom::verts_in_rect([true, true], true));
    assert!(!geom::verts_in_rect([true, false], true));
    assert!(geom::verts_in_rect([false, true], false));
    assert!(!geom::verts_in_rect([false, false], false));
}

#[test]
fn select_by_surrounding_takes_only_what_the_rectangle_surrounds() {
    let (mut world, mut tool, mut settings) = fixture();
    assert!(settings.select_surrounding, "on by default, as RectangleSelectInclusive");
    let target = object(&world, 99614);
    let now = Instant::now();
    let (position, rot, _) = Scene::object_transform(&world, target, now, 0).unwrap();
    tool.cam = camera_on(position);
    world.agent.position = tool.cam.eye;
    // the prim's footprint on screen
    let model = glam::Mat4::from_scale_rotation_translation(world.visual_scale_at(target, now), rot, position);
    let mut footprint = egui::Rect::NOTHING;
    for i in 0..8 {
        let corner = Vec3::new([-0.5, 0.5][i & 1], [-0.5, 0.5][(i >> 1) & 1], [-0.5, 0.5][(i >> 2) & 1]);
        let (x, y) = tool.cam.project_px(model.transform_point3(corner)).unwrap();
        footprint.extend_with(egui::pos2(x, y));
    }
    let around = footprint.expand(2.0);
    let half = egui::Rect::from_min_max(around.min, footprint.center());
    for (surround, rect, taken) in [
        (true, around, true),
        (true, half, false),
        (false, around, true),
        (false, half, true),
    ] {
        settings.select_surrounding = surround;
        tool.deselect_all(&world);
        tool.rect_select(
            &mut world,
            &settings,
            None,
            (rect.min.x, rect.min.y),
            (rect.max.x, rect.max.y),
            false,
        );
        let root = world.objects.get(root_of(&world, target)).unwrap().key;
        assert_eq!(tool.selection.contains(&root), taken, "surround {surround}, rect {rect:?}");
    }
}

#[test]
fn select_by_surrounding_is_saved_and_on_when_a_profile_lacks_it() {
    let settings = BuildSettings {
        select_surrounding: false,
        ..BuildSettings::default()
    };
    let restored: BuildSettings = serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap();
    assert!(!restored.select_surrounding, "persisted, as RectangleSelectInclusive");
    let restored: BuildSettings = serde_json::from_str("{}").unwrap();
    assert!(restored.select_surrounding);
    let tool = BuildTool::default();
    let mut text = serde_json::to_value(layout::Profile::capture(&tool, &settings)).unwrap();
    assert_eq!(text["options"]["surrounding"], false);
    text["options"].as_object_mut().unwrap().remove("surrounding");
    let profile = layout::Profile::read(&text.to_string()).unwrap();
    assert!(profile.options.surrounding, "a notecard from before the option keeps the default");
}
