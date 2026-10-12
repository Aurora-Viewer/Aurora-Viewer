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
    drop_link: bool,
    suppress_copy_confirmation: bool,
    shuffle_link_reply: bool,
    script_changes: Vec<(Uuid, bool)>,
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
            drop_link: false,
            suppress_copy_confirmation: false,
            shuffle_link_reply: false,
            script_changes: Vec::new(),
        }
    }

    fn deliver(&mut self, mut event: NetEvent) {
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
                    self.tool
                        .on_task_inventory(&self.world, object, serial, items.into_iter().filter(|i| !i.is_folder).collect());
                }
                NetEvent::CapReply { tag, result } => self.tool.on_cap_reply(&mut self.world, tag, result),
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
                        if let BuildCmd::SetScriptRunning { object_id, running, .. } = &command {
                            self.script_changes.push((*object_id, *running));
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
                        let result = crate::world::inventory::actions::demo_mutate(&mut self.server.inventory, self.world.agent_id, change);
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
    h.tool.replacement.options.contents = true;
    assert_eq!(h.run(), Stage::Done, "{:?}", h.tool.replacement.job);
    let source = &h.tool.contents.objects[&demo::id(demo::SOURCE)].items;
    let target = &h.tool.contents.objects[&demo::id(demo::TARGET)].items;
    assert_eq!(source.len(), target.len());
    assert!(source.iter().all(|i| !target.iter().any(|old| old.item_id == i.item_id)));
    assert!(h.script_changes.contains(&(demo::id(demo::SOURCE), true)));
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
