use super::*;
use crate::build::demo_sim::DemoSim;
use aurora_net::build::perm;
use aurora_net::{NetCommand, NetEvent};
use std::sync::Arc;
use std::time::Duration;

struct Harness {
    world: World,
    server: World,
    tool: BuildTool,
    sim: DemoSim,
    clock: Instant,
    deny_copy: bool,
    source_permissions: Option<(Uuid, u32)>,
    drop_link: bool,
    suppress_copy_confirmation: bool,
    shuffle_link_reply: bool,
    script_changes: Vec<(Uuid, bool)>,
    delay_script_writes: bool,
    pending_script_writes: Vec<BuildCmd>,
    drop_script_reply: bool,
    script_transfers: Vec<(Uuid, TaskItem)>,
    fail_task_listing_once: bool,
    fail_folder_creation: bool,
    shape_only_physics: bool,
    missing_physics: bool,
    physics_writes: Vec<(u32, aurora_net::build::PhysicsParams)>,
    keep_inventory_folders: bool,
}

impl Harness {
    fn new(target: u32) -> Self {
        let library = Arc::new(crate::scene::avatar::AvatarLibrary::load());
        let mut world = World::new(library.clone());
        let mut server = World::new(library);
        for event in crate::demo::events() {
            server.apply(event);
        }
        for event in crate::demo::events() {
            world.apply(event);
        }
        for event in demo::events(world.main_region.unwrap(), 25.0) {
            world.apply(event);
        }
        for event in demo::events(server.main_region.unwrap(), 25.0) {
            server.apply(event);
        }
        let mut tool = BuildTool::default();
        let settings = BuildSettings {
            edit_linked: true,
            ..Default::default()
        };
        let idx = world.objects.index_of_uuid(&demo::id(target)).unwrap();
        tool.open_build(Tool::Edit);
        tool.click_select(&mut world, &settings, Some(idx), false);
        tool.begin_replacement(&world, &settings);
        let source = world.objects.index_of_uuid(&demo::id(demo::SOURCE)).unwrap();
        tool.pick_replacement(&world, Some(source));
        tool.out.clear();
        Self {
            world,
            server,
            tool,
            sim: DemoSim::default(),
            clock: Instant::now(),
            deny_copy: false,
            source_permissions: None,
            drop_link: false,
            suppress_copy_confirmation: false,
            shuffle_link_reply: false,
            script_changes: Vec::new(),
            delay_script_writes: false,
            pending_script_writes: Vec::new(),
            drop_script_reply: false,
            script_transfers: Vec::new(),
            fail_task_listing_once: false,
            fail_folder_creation: false,
            shape_only_physics: false,
            missing_physics: false,
            physics_writes: Vec::new(),
            keep_inventory_folders: false,
        }
    }

    fn deliver(&mut self, mut event: NetEvent) {
        if let NetEvent::CapReply {
            result: Ok(aurora_llsd::Llsd::Map(objects)),
            ..
        } = &mut event
        {
            for values in objects.values_mut() {
                if values.has("PhysicsShapeType") {
                    if self.shape_only_physics {
                        *values = aurora_llsd::llsd_map! { "PhysicsShapeType" => values["PhysicsShapeType"].clone() };
                    }
                    if self.missing_physics {
                        *values = aurora_llsd::Llsd::Undef;
                    }
                }
            }
        }
        if self.fail_task_listing_once
            && let NetEvent::TaskInventory { object, result, .. } = &mut event
            && *object == demo::id(demo::TARGET)
        {
            self.fail_task_listing_once = false;
            *result = Err("test inventory failure".into());
        }
        if self.drop_script_reply
            && matches!(&event, NetEvent::ScriptRunning { object_id, .. } if *object_id == demo::id(demo::TARGET))
            && self.tool.replacement.job.as_ref().is_some_and(|j| j.stage == Stage::ReadScripts)
        {
            self.drop_script_reply = false;
            return;
        }
        if let Some((owner, mask)) = self.source_permissions
            && let NetEvent::ObjectProperties(props) = &mut event
            && let Some(props) = props.iter_mut().find(|props| props.object_id == demo::id(demo::SOURCE))
        {
            props.owner_id = owner;
            props.owner_mask = mask;
        }
        if self.suppress_copy_confirmation
            && matches!(&event, NetEvent::TaskInventory { object, .. } if *object == demo::id(demo::SOURCE))
            && self
                .tool
                .replacement
                .job
                .as_ref()
                .is_some_and(|j| matches!(j.stage, Stage::CopyIn(_)))
        {
            return;
        }
        if self.deny_copy
            && let NetEvent::TaskInventory {
                object, result: Ok(items), ..
            } = &mut event
            && *object == demo::id(demo::TARGET)
            && let Some(item) = items.iter_mut().find(|i| !i.is_folder)
        {
            item.owner_mask &= !perm::COPY;
        }
        if let Some(event) = self.world.apply(event) {
            match event {
                NetEvent::ObjectProperties(props) => self.tool.on_properties(props),
                NetEvent::TaskInventory {
                    object,
                    serial,
                    result: Ok(items),
                } => {
                    self.tool.on_task_inventory(
                        &self.world,
                        object,
                        serial,
                        items.into_iter().filter(|i| self.keep_inventory_folders || !i.is_folder).collect(),
                    );
                }
                NetEvent::CapReply { tag, result } => self.tool.on_cap_reply(&mut self.world, tag, result),
                NetEvent::TaskInventory {
                    object,
                    result: Err(reason),
                    ..
                } => {
                    self.tool.replacement_task_inventory_error(object, reason);
                }
                NetEvent::ScriptRunning {
                    object_id,
                    item_id,
                    running,
                    mono,
                } => {
                    self.tool.contents.running.insert((object_id, item_id), (running, mono));
                }
                _ => {}
            }
        }
    }

    fn step(&mut self) {
        self.clock += Duration::from_secs(1);
        // The script engine commits state changes after the first status reply.
        for command in std::mem::take(&mut self.pending_script_writes) {
            self.sim.reply(&self.server, &command);
        }
        self.tool.update_replacement(&mut self.world, self.clock);
        // Drain commands produced by replies too, as the app does over frames.
        for _ in 0..4 {
            let commands = std::mem::take(&mut self.tool.out);
            if commands.is_empty() {
                break;
            }
            for command in commands {
                match command {
                    NetCommand::Build(command) => {
                        if let BuildCmd::SetPhysicsParams { local_id, params, .. } = &command {
                            self.physics_writes.push((*local_id, *params));
                        }
                        if let BuildCmd::SetScriptRunning { object_id, running, .. } = &command {
                            self.script_changes.push((*object_id, *running));
                            if self.delay_script_writes {
                                self.pending_script_writes.push(command);
                                continue;
                            }
                        }
                        if let BuildCmd::RezScript { group_id, item, .. } = &command {
                            self.script_transfers.push((*group_id, (**item).clone()));
                        }
                        let is_link = matches!(command, BuildCmd::Link { .. });
                        let previous_children: Vec<_> = if let BuildCmd::Link { handle, local_ids } = &command {
                            let key = crate::world::objects::ObjKey {
                                region: *handle,
                                local_id: local_ids[0],
                            };
                            self.server
                                .objects
                                .children_of(&key)
                                .iter()
                                .filter_map(|i| self.server.objects.get(*i).map(|o| o.full_id))
                                .collect()
                        } else {
                            Vec::new()
                        };
                        for mut event in self.sim.reply(&self.server, &command) {
                            if let NetEvent::ObjectUpdates { handle, objects } = &event {
                                self.server.apply(NetEvent::ObjectUpdates {
                                    handle: *handle,
                                    objects: objects.clone(),
                                });
                            }
                            if let NetEvent::InventoryContents(contents) = &event {
                                self.server.inventory.apply(contents.clone());
                            }
                            if self.drop_link && is_link {
                                self.drop_link = false;
                                continue;
                            }
                            if self.shuffle_link_reply
                                && is_link
                                && let NetEvent::ObjectUpdates { objects, .. } = &mut event
                            {
                                objects[1..].reverse();
                            }
                            self.deliver(event);
                        }
                        if let BuildCmd::Link { handle, local_ids } = &command {
                            let linked: Vec<_> = local_ids
                                .iter()
                                .filter_map(|local_id| {
                                    self.server
                                        .objects
                                        .index_of(&crate::world::objects::ObjKey {
                                            region: *handle,
                                            local_id: *local_id,
                                        })
                                        .and_then(|i| self.server.objects.get(i))
                                        .map(|o| o.full_id)
                                })
                                .collect();
                            let children: Vec<_> = linked[1..]
                                .iter()
                                .copied()
                                .chain(previous_children.into_iter().filter(|id| !linked.contains(id)))
                                .collect();
                            assert!(self.server.objects.remember_link_order(linked[0], &children));
                        }
                    }
                    NetCommand::RequestObjects { handle, local_ids } => {
                        let objects = local_ids
                            .iter()
                            .filter_map(|local_id| {
                                self.server
                                    .objects
                                    .index_of(&crate::world::objects::ObjKey {
                                        region: handle,
                                        local_id: *local_id,
                                    })
                                    .and_then(|i| self.server.objects.get(i))
                                    .map(super::demo_update)
                            })
                            .collect();
                        self.deliver(NetEvent::ObjectUpdates { handle, objects });
                    }
                    NetCommand::EditInventory { request, change } => {
                        let result = if self.fail_folder_creation {
                            Err(aurora_i18n::tr!("inventory-operations-the-server-did-not-confirm-every-change-the-folders").into())
                        } else {
                            crate::world::inventory::actions::demo_mutate(&mut self.server.inventory, self.world.agent_id, change)
                        };
                        if let Ok(reply) = &result {
                            self.world.inventory.apply(reply.contents.clone());
                        }
                        assert!(self.tool.replacement_inventory_result(request, &result));
                    }
                    _ => {}
                }
            }
        }
    }

    fn run(&mut self) -> Stage {
        self.tool.apply_replacement(&self.world);
        assert!(self.tool.replacement.job.is_some(), "{}", self.tool.replacement.message);
        for _ in 0..180 {
            self.step();
            let job = self.tool.replacement.job.as_ref().unwrap();
            if job.stage.terminal() {
                return job.stage;
            }
        }
        panic!("replacement never finished: {:?}", self.tool.replacement.job);
    }

    fn ids(&self, root: u32) -> Vec<Uuid> {
        let root = self.world.objects.index_of_uuid(&demo::id(root)).unwrap();
        family(&self.world, root)
            .iter()
            .map(|i| self.world.objects.get(*i).unwrap().full_id)
            .collect()
    }

    fn only_target_script(&mut self) {
        for local in (demo::ROOT..demo::ROOT + 15).chain(std::iter::once(demo::SOURCE)) {
            let object_id = demo::id(local);
            let handle = self.server.main_region.unwrap();
            for event in self.sim.reply(
                &self.server,
                &BuildCmd::RequestTaskInventory {
                    handle,
                    local_id: local,
                    object_id,
                },
            ) {
                if let NetEvent::TaskInventory { result: Ok(items), .. } = event {
                    for item in items.into_iter().filter(|i| !i.is_folder) {
                        if local != demo::TARGET || item.asset_type != crate::build::contents::AT_LSL_TEXT {
                            self.sim.reply(
                                &self.server,
                                &BuildCmd::RemoveTaskInventory {
                                    handle,
                                    local_id: local,
                                    item_id: item.item_id,
                                },
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn replaces_ninth_part_preserving_other_numbers_and_world_transforms() {
    let mut h = Harness::new(demo::TARGET);
    let original = h.ids(demo::ROOT);
    let captured: Vec<_> = original
        .iter()
        .map(|id| {
            let idx = h.world.objects.index_of_uuid(id).unwrap();
            Prim::capture(&h.tool, &h.world, idx).unwrap()
        })
        .collect();
    let source = h.world.objects.index_of_uuid(&demo::id(demo::SOURCE)).unwrap();
    let source_before = Prim::capture(&h.tool, &h.world, source).unwrap();
    h.tool.replacement.options.size = true;
    assert_eq!(h.run(), Stage::Done, "{:?}", h.tool.replacement.job);
    let mut expected = original;
    expected[8] = demo::id(demo::SOURCE);
    assert_eq!(h.ids(demo::ROOT), expected);
    for p in &captured {
        if p.id != demo::id(demo::TARGET) {
            assert!(p.matches(&h.world));
        }
    }
    let replacement = h.world.objects.index_of_uuid(&demo::id(demo::SOURCE)).unwrap();
    let (position, rotation) = region_transform(&h.world, replacement).unwrap();
    assert!(position.abs_diff_eq(captured[8].position, 1e-5));
    assert!(rotation.abs_diff_eq(captured[8].rotation, 1e-5));
    assert_eq!(h.world.objects.get(replacement).unwrap().scale, captured[8].size);
    let old = h.world.objects.index_of_uuid(&demo::id(demo::TARGET)).unwrap();
    let (position, rotation) = region_transform(&h.world, old).unwrap();
    assert!(position.abs_diff_eq(source_before.position, 1e-5));
    assert!(rotation.abs_diff_eq(captured[8].rotation, 1e-5));
    let old = h.world.objects.get(old).unwrap();
    assert_eq!(old.parent_id, 0);
    assert_eq!(old.scale, captured[8].size);
    assert!(!h.script_changes.contains(&(demo::id(demo::ROOT + 6), true)));
}

#[test]
fn position_rotation_size_and_pause_options_are_independent() {
    let mut h = Harness::new(demo::TARGET);
    let source = h.world.objects.index_of_uuid(&demo::id(demo::SOURCE)).unwrap();
    let before = Prim::capture(&h.tool, &h.world, source).unwrap();
    h.tool.replacement.options = Options {
        position: false,
        rotation: true,
        size: false,
        contents: false,
        pause_scripts: false,
    };
    assert_eq!(h.run(), Stage::Done);
    let (position, rotation) = region_transform(&h.world, source).unwrap();
    assert!(position.abs_diff_eq(before.position, 1e-5));
    assert!(rotation.abs_diff_eq(Quat::IDENTITY, 1e-5));
    assert_eq!(h.world.objects.get(source).unwrap().scale, before.size);
    let old = h.world.objects.index_of_uuid(&demo::id(demo::TARGET)).unwrap();
    assert!(region_transform(&h.world, old).unwrap().0.abs_diff_eq(before.position, 1e-5));
    assert!(h.script_changes.is_empty());
}

#[test]
fn contents_copy_uses_confirmed_agent_items_and_restores_copied_scripts() {
    let mut h = Harness::new(demo::TARGET);
    h.world.groups.active = crate::demo::DEMO_GROUP1;
    h.tool.replacement.options.contents = true;
    assert_eq!(h.run(), Stage::Done, "{:?}", h.tool.replacement.job);
    let source = &h.tool.contents.objects[&demo::id(demo::SOURCE)].items;
    let target = &h.tool.contents.objects[&demo::id(demo::TARGET)].items;
    assert_eq!(source.len(), target.len());
    assert!(source.iter().all(|i| !target.iter().any(|old| old.item_id == i.item_id)));
    assert!(h.script_changes.contains(&(demo::id(demo::SOURCE), true)));
    let (group, packet) = &h.script_transfers[0];
    assert_eq!(*group, crate::demo::DEMO_GROUP1);
    let staged = &h.world.inventory.items[&packet.item_id];
    assert!(!staged.last_owner.is_nil());
    assert_eq!(packet.last_owner_id, staged.last_owner);
    assert_eq!(packet.parent_id, demo::id(demo::SOURCE));
}

#[test]
fn script_only_content_copy_confirms_delayed_script_state_changes() {
    for (pause, lost_reply) in [(true, false), (true, true), (false, false)] {
        let mut h = Harness::new(demo::TARGET);
        h.only_target_script();
        h.tool.replacement.options.contents = true;
        h.tool.replacement.options.pause_scripts = pause;
        h.delay_script_writes = true;
        h.drop_script_reply = lost_reply;
        assert_eq!(h.run(), Stage::Done, "{:?}", h.tool.replacement.job);
        let copied = &h.tool.contents.objects[&demo::id(demo::SOURCE)].items;
        assert_eq!(copied.len(), 1);
        assert_eq!(copied[0].asset_type, crate::build::contents::AT_LSL_TEXT);
        assert_eq!(
            h.script_changes
                .iter()
                .filter(|(id, running)| *id == demo::id(demo::TARGET) && !running)
                .count(),
            usize::from(pause)
        );
        assert_eq!(
            h.script_changes
                .iter()
                .filter(|(id, running)| *id == demo::id(demo::SOURCE) && *running)
                .count(),
            1
        );
    }
}

#[test]
fn task_inventory_error_is_reported_before_any_unlink_or_script_stop() {
    let mut h = Harness::new(demo::TARGET);
    h.tool.replacement.options.contents = true;
    h.fail_task_listing_once = true;
    let original = h.ids(demo::ROOT);
    assert_eq!(h.run(), Stage::Failed);
    assert!(h.tool.replacement.job.as_ref().unwrap().message.contains("test inventory failure"));
    assert_eq!(h.ids(demo::ROOT), original);
    assert!(h.script_changes.is_empty());
}

#[test]
fn a_successful_listing_supersedes_an_older_inventory_request_error() {
    let mut h = Harness::new(demo::TARGET);
    h.tool.apply_replacement(&h.world);
    h.step();
    h.tool
        .replacement_task_inventory_error(demo::id(demo::TARGET), "test inventory failure".into());
    let items = h.tool.contents.objects[&demo::id(demo::TARGET)].items.clone();
    h.tool.on_task_inventory(&h.world, demo::id(demo::TARGET), Some(1), items);
    assert!(h.tool.replacement.job.as_ref().unwrap().inventory_error.is_none());
    for _ in 0..180 {
        h.step();
        if h.tool.replacement.job.as_ref().unwrap().stage.terminal() {
            assert_eq!(h.tool.replacement.job.as_ref().unwrap().stage, Stage::Done);
            return;
        }
    }
    panic!("replacement never completed");
}

#[test]
fn no_copy_contents_are_rejected_before_any_unlink_or_script_stop() {
    let mut h = Harness::new(demo::TARGET);
    h.tool.replacement.options.contents = true;
    h.deny_copy = true;
    let original = h.ids(demo::ROOT);
    assert_eq!(h.run(), Stage::Failed);
    assert_eq!(h.ids(demo::ROOT), original);
    assert!(h.script_changes.is_empty());
}

#[test]
fn a_link_timeout_recovers_original_order_transforms_and_script_states() {
    let mut h = Harness::new(demo::TARGET);
    let original = h.ids(demo::ROOT);
    let source = h.world.objects.index_of_uuid(&demo::id(demo::SOURCE)).unwrap();
    let before = Prim::capture(&h.tool, &h.world, source).unwrap();
    h.drop_link = true;
    assert_eq!(h.run(), Stage::Failed, "{:?}", h.tool.replacement.job);
    assert_eq!(h.ids(demo::ROOT), original);
    assert!(before.matches(&h.world));
    assert_eq!(h.world.objects.get(source).unwrap().parent_id, 0);
    assert!(!h.script_changes.contains(&(demo::id(demo::ROOT + 6), true)));
}

#[test]
fn failed_link_after_contents_copy_removes_only_added_contents() {
    let mut h = Harness::new(demo::TARGET);
    h.tool.replacement.options.contents = true;
    h.drop_link = true;
    let original = h.ids(demo::ROOT);
    assert_eq!(h.run(), Stage::Failed);
    assert_eq!(h.ids(demo::ROOT), original);
    assert!(h.tool.contents.objects[&demo::id(demo::SOURCE)].items.is_empty());
    assert_eq!(h.tool.contents.objects[&demo::id(demo::TARGET)].items.len(), 3);
}

#[test]
fn child_with_no_physics_shape_regains_its_shape_after_relinking() {
    let mut h = Harness::new(demo::TARGET);
    let local_id = demo::ROOT + 2;
    h.sim.reply(
        &h.world,
        &BuildCmd::SetPhysicsParams {
            handle: h.world.main_region.unwrap(),
            local_id,
            physics: false,
            temporary: false,
            phantom: false,
            params: aurora_net::build::PhysicsParams {
                shape_type: 1,
                ..Default::default()
            },
        },
    );
    assert_eq!(h.run(), Stage::Done);
    assert_eq!(h.tool.physics[&local_id].shape_type, 1);
}

#[test]
fn shape_only_physics_replies_keep_materials_and_do_not_send_unchanged_shapes() {
    let mut h = Harness::new(demo::TARGET);
    h.shape_only_physics = true;
    let params = aurora_net::build::PhysicsParams {
        density: 1200.0,
        friction: 0.8,
        restitution: 0.2,
        gravity_multiplier: 0.5,
        ..Default::default()
    };
    for local in (demo::ROOT..demo::ROOT + 15).chain(std::iter::once(demo::SOURCE)) {
        h.tool.physics.insert(local, params);
    }
    assert_eq!(h.run(), Stage::Done);
    assert!(h.physics_writes.is_empty());
    assert!(h.tool.physics.values().all(|actual| *actual == params));
}

#[test]
fn temporarily_enabled_collision_shape_preserves_custom_materials_with_shape_only_replies() {
    let mut h = Harness::new(demo::TARGET);
    h.shape_only_physics = true;
    let local_id = demo::ROOT + 2;
    let params = aurora_net::build::PhysicsParams {
        shape_type: 1,
        density: 1200.0,
        friction: 0.8,
        restitution: 0.2,
        gravity_multiplier: 0.5,
    };
    h.tool.physics.insert(local_id, params);
    h.tool.physics_materials_known.insert(local_id);
    h.sim.reply(
        &h.server,
        &BuildCmd::SetPhysicsParams {
            handle: h.server.main_region.unwrap(),
            local_id,
            physics: false,
            temporary: false,
            phantom: false,
            params,
        },
    );
    assert_eq!(h.run(), Stage::Done);
    let mut standalone = params;
    standalone.shape_type = 2;
    assert_eq!(h.physics_writes, vec![(local_id, standalone), (local_id, params)]);
    assert_eq!(h.tool.physics[&local_id], params);
}

#[test]
fn cached_physics_does_not_confirm_a_missing_fresh_reply() {
    let mut h = Harness::new(demo::TARGET);
    h.missing_physics = true;
    for local in (demo::ROOT..demo::ROOT + 15).chain(std::iter::once(demo::SOURCE)) {
        h.tool.physics.insert(local, Default::default());
    }
    let original = h.ids(demo::ROOT);
    assert_eq!(h.run(), Stage::Failed);
    assert_eq!(h.ids(demo::ROOT), original);
    assert!(h.physics_writes.is_empty());
}

#[test]
fn no_collision_material_is_replaced_with_defaults_when_the_server_omits_it() {
    let mut h = Harness::new(demo::TARGET);
    h.shape_only_physics = true;
    let original = h.ids(demo::ROOT);
    h.sim.reply(
        &h.server,
        &BuildCmd::SetPhysicsParams {
            handle: h.server.main_region.unwrap(),
            local_id: demo::ROOT + 2,
            physics: false,
            temporary: false,
            phantom: false,
            params: aurora_net::build::PhysicsParams {
                shape_type: 1,
                density: 1200.0,
                ..Default::default()
            },
        },
    );
    assert_eq!(h.run(), Stage::Failed);
    assert_eq!(h.ids(demo::ROOT), original);
    assert!(h.physics_writes.is_empty());
    assert!(h.script_changes.is_empty());
}

#[test]
fn unconfirmed_content_copy_is_read_back_and_removed_during_recovery() {
    let mut h = Harness::new(demo::TARGET);
    h.tool.replacement.options.contents = true;
    h.suppress_copy_confirmation = true;
    assert_eq!(h.run(), Stage::Failed);
    let idx = h.world.objects.index_of_uuid(&demo::id(demo::SOURCE)).unwrap();
    let local_id = h.world.objects.get(idx).unwrap().key.local_id;
    for event in h.sim.reply(
        &h.world,
        &BuildCmd::RequestTaskInventory {
            handle: h.world.main_region.unwrap(),
            local_id,
            object_id: demo::id(demo::SOURCE),
        },
    ) {
        h.deliver(event);
    }
    assert!(h.tool.contents.objects[&demo::id(demo::SOURCE)].items.is_empty());
}

#[test]
fn replacing_root_retains_child_numbers_and_world_placement() {
    let mut h = Harness::new(demo::ROOT);
    let mut expected = h.ids(demo::ROOT);
    expected[0] = demo::id(demo::SOURCE);
    assert_eq!(h.run(), Stage::Done);
    assert_eq!(h.ids(demo::SOURCE), expected);
}

#[test]
fn button_requires_one_selected_linked_part_and_picker_keeps_target_selection() {
    let mut h = Harness::new(demo::TARGET);
    assert_eq!(h.tool.selection.len(), 1);
    assert_eq!(
        h.world
            .objects
            .get(h.world.objects.index_of(&h.tool.selection[0]).unwrap())
            .unwrap()
            .full_id,
        demo::id(demo::TARGET)
    );
    let settings = BuildSettings {
        edit_linked: true,
        ..Default::default()
    };
    assert!(h.tool.replacement_visible(&h.world, &settings));
    assert!(!h.tool.replacement_visible(&h.world, &BuildSettings::default()));
    h.tool.selection.push(
        h.world
            .objects
            .get(h.world.objects.index_of_uuid(&demo::id(demo::ROOT)).unwrap())
            .unwrap()
            .key,
    );
    assert!(!h.tool.replacement_visible(&h.world, &settings));
}

#[test]
fn owned_prims_without_update_owner_ids_can_be_picked_and_replaced() {
    let mut h = Harness::new(demo::TARGET);
    for local in (demo::ROOT..demo::ROOT + 15).chain(std::iter::once(demo::SOURCE)) {
        for world in [&mut h.world, &mut h.server] {
            let idx = world.objects.index_of_uuid(&demo::id(local)).unwrap();
            let object = world.objects.get_mut(idx).unwrap();
            object.owner_id = Uuid::nil();
            if local != demo::ROOT && local != demo::SOURCE {
                // As in Firestorm, children inherit ownership flags from their root.
                object.update_flags &= !crate::ui::context::flags::OBJECT_YOU_OWNER;
            }
        }
    }
    let draft = h.tool.replacement.draft.as_mut().unwrap();
    draft.source = None;
    draft.picking = true;
    let source = h.world.objects.index_of_uuid(&demo::id(demo::SOURCE)).unwrap();
    h.tool.pick_replacement(&h.world, Some(source));
    assert_eq!(h.tool.replacement.draft.as_ref().unwrap().source, Some(demo::id(demo::SOURCE)));
    assert_eq!(h.run(), Stage::Done, "{:?}", h.tool.replacement.job);
    assert_eq!(h.ids(demo::ROOT)[8], demo::id(demo::SOURCE));
}

#[test]
fn full_candidate_permissions_override_missing_update_ownership_and_keep_real_refusals() {
    for (mine, modifiable, accepted) in [(true, true, true), (false, true, false), (true, false, false)] {
        let mut h = Harness::new(demo::TARGET);
        let source = h.world.objects.index_of_uuid(&demo::id(demo::SOURCE)).unwrap();
        let object = h.world.objects.get_mut(source).unwrap();
        object.owner_id = Uuid::nil();
        object.update_flags &= !crate::ui::context::flags::OBJECT_YOU_OWNER;
        h.tool.on_properties(vec![aurora_net::build::ObjectProps {
            object_id: demo::id(demo::SOURCE),
            full: true,
            owner_id: if mine { h.world.agent_id } else { Uuid::from_u128(123) },
            owner_mask: if modifiable { u32::MAX } else { !perm::MODIFY },
            ..Default::default()
        }]);
        let draft = h.tool.replacement.draft.as_mut().unwrap();
        draft.source = None;
        draft.picking = true;
        h.tool.pick_replacement(&h.world, Some(source));
        assert_eq!(h.tool.replacement.draft.as_ref().unwrap().source.is_some(), accepted);
        assert_eq!(h.tool.selection.len(), 1);
    }
}

#[test]
fn fresh_server_permissions_refuse_replacement_before_unlinking_or_stopping_scripts() {
    for mine in [true, false] {
        let mut h = Harness::new(demo::TARGET);
        let original = h.ids(demo::ROOT);
        let source = h.world.objects.index_of_uuid(&demo::id(demo::SOURCE)).unwrap();
        let before = Prim::capture(&h.tool, &h.world, source).unwrap();
        h.source_permissions = Some((
            if mine { h.world.agent_id } else { Uuid::from_u128(123) },
            if mine { !perm::MODIFY } else { u32::MAX },
        ));
        assert_eq!(h.run(), Stage::Failed, "{:?}", h.tool.replacement.job);
        assert_eq!(h.ids(demo::ROOT), original);
        assert!(before.matches(&h.server));
        assert!(h.script_changes.is_empty());
    }
}

#[test]
fn unconfirmed_copy_folder_reports_the_stage_and_restores_the_original_script() {
    let mut h = Harness::new(demo::TARGET);
    h.only_target_script();
    h.tool.replacement.options.contents = true;
    h.fail_folder_creation = true;
    let original = h.ids(demo::ROOT);
    let source = h.world.objects.index_of_uuid(&demo::id(demo::SOURCE)).unwrap();
    let before = Prim::capture(&h.tool, &h.world, source).unwrap();
    assert_eq!(h.run(), Stage::Failed);
    let message = &h.tool.replacement.job.as_ref().unwrap().message;
    assert!(message.contains(&aurora_i18n::tr!("replace-folder-failed", reason = "")));
    assert_eq!(h.ids(demo::ROOT), original);
    assert!(before.matches(&h.server));
    assert!(h.script_transfers.is_empty());
    assert_eq!(
        h.script_changes,
        vec![(demo::id(demo::TARGET), false), (demo::id(demo::TARGET), true)]
    );
}

#[test]
fn contents_folder_metadata_does_not_conflict_with_a_script_named_contents() {
    let mut h = Harness::new(demo::TARGET);
    h.only_target_script();
    h.keep_inventory_folders = true;
    h.tool.replacement.options.contents = true;
    let handle = h.server.main_region.unwrap();
    for event in h.sim.reply(
        &h.server,
        &BuildCmd::RequestTaskInventory {
            handle,
            local_id: demo::TARGET,
            object_id: demo::id(demo::TARGET),
        },
    ) {
        if let NetEvent::TaskInventory { result: Ok(items), .. } = event {
            let mut item = items.into_iter().find(|i| !i.is_folder).unwrap();
            item.name = "Contents".into();
            h.sim.reply(
                &h.server,
                &BuildCmd::UpdateTaskInventory {
                    handle,
                    local_id: demo::TARGET,
                    item: Box::new(item),
                },
            );
        }
    }
    assert_eq!(h.run(), Stage::Done, "{:?}", h.tool.replacement.job);
    let inventory = &h.tool.contents.objects[&demo::id(demo::SOURCE)];
    assert!(inventory.items.iter().any(|i| i.is_folder && i.name == "Contents"));
    assert!(inventory.items.iter().any(|i| !i.is_folder && i.name == "Contents"));
    assert_eq!(h.script_transfers.len(), 1);
}

#[test]
fn reordered_update_packets_do_not_scramble_link_numbers() {
    let mut h = Harness::new(demo::TARGET);
    h.shuffle_link_reply = true;
    let mut expected = h.ids(demo::ROOT);
    expected[8] = demo::id(demo::SOURCE);
    assert_eq!(h.run(), Stage::Done);
    assert_eq!(h.ids(demo::ROOT), expected);
}

#[test]
fn a_full_256_prim_set_links_in_confirmed_tail_then_head_batches() {
    let mut h = Harness::new(demo::TARGET);
    let template = super::demo_update(
        h.world
            .objects
            .get(h.world.objects.index_of_uuid(&demo::id(demo::ROOT + 1)).unwrap())
            .unwrap(),
    );
    let mut extra = Vec::new();
    for n in 0..241 {
        let mut prim = template.clone();
        prim.local_id = 100000 + n;
        prim.full_id = demo::id(prim.local_id);
        prim.position = Vec3::new((n % 16) as f32 * 0.03, (n / 16) as f32 * 0.03, 0.4);
        extra.push(prim);
    }
    let handle = h.world.main_region.unwrap();
    h.server.apply(NetEvent::ObjectUpdates {
        handle,
        objects: extra.clone(),
    });
    h.world.apply(NetEvent::ObjectUpdates { handle, objects: extra });
    let mut expected = h.ids(demo::ROOT);
    assert_eq!(expected.len(), 256);
    expected[8] = demo::id(demo::SOURCE);
    let batches = super::transaction::link_batches(&expected);
    assert_eq!(batches.iter().map(Vec::len).collect::<Vec<_>>(), vec![2, 255]);
    assert_eq!(h.run(), Stage::Done);
    assert_eq!(h.ids(demo::ROOT), expected);
}
