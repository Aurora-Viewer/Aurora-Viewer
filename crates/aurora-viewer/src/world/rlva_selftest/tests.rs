//! The runner against the offline simulator, on the demo world. The host
//! below stands for the application: it sends the runner's commands through
//! the same boundary functions as `App::send_from`
//! (`World::rlva_send_boundary`, `rlva_note_sent`) and plays the part of
//! `App::update_rlva` a World alone cannot (the forced-wear outfit update,
//! the watchdog's recovery, the replies leaving).
use super::report::Verdict;
use super::sim::Sim;
use super::{Boundary, Ctx, FOLDER, Mode, Out, Prim, Runner, SEAT_NAME, guard};
use crate::world::{World, appearance};
use aurora_net::build::BuildCmd;
use aurora_net::{AttachRequest, NetCommand, NetEvent};
use std::collections::HashSet;
use std::time::{Duration, Instant};
use uuid::Uuid;

struct Outfit {
    request: Uuid,
    sync: bool,
    actor: Uuid,
    attachments: Vec<AttachRequest>,
    worn_before: HashSet<Uuid>,
}

struct Host {
    world: World,
    runner: Runner,
    sim: Sim,
    now: Instant,
    /// Everything that passed the boundary, as it left.
    sent: Vec<NetCommand>,
    /// What the simulator never answers (a fault to inject).
    lost: Option<fn(&NetCommand) -> bool>,
    outfit: Option<Outfit>,
    online: bool,
    /// The inventory before the run (the outfit folder's links aside: the
    /// offline outfit folder writes them anew at each update).
    folders: HashSet<Uuid>,
    items: HashSet<Uuid>,
    /// What the outfit folder linked before the run.
    outfit_before: HashSet<Uuid>,
    /// Every object of the region before the run: none is the test's.
    objects_before: HashSet<(u64, u32)>,
    ids_before: HashSet<Uuid>,
    /// What the avatar wore before the run.
    worn_before: HashSet<Uuid>,
}

fn outfit_targets(world: &World) -> HashSet<Uuid> {
    appearance::cof(&world.inventory)
        .map(|cof| {
            appearance::folder_links(&world.inventory, cof)
                .into_iter()
                .map(|l| l.target)
                .collect()
        })
        .unwrap_or_default()
}

impl Host {
    fn new() -> Self {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        crate::world::bridge::demo::seed(&mut world);
        crate::ao::demo::inventory_services(&mut world, true);
        world.rlva_configure(true);
        // the avatar wears its outfit, as after a login
        let outfit = appearance::sync_with_attachments(
            &world.inventory,
            world.agent_id,
            &world.outfit_attachment_items(),
            &HashSet::new(),
            Uuid::nil(),
            Vec::new(),
        );
        for command in outfit.commands {
            if let NetCommand::RezAttachments(items) = &command {
                world.apply(NetEvent::ObjectUpdates {
                    handle: world.main_region.unwrap_or_default(),
                    objects: items.iter().map(crate::demo::outfit_attachment).collect(),
                });
            }
        }
        let worn_before = world.outfit_attachment_items();
        let folders = world.inventory.folders.keys().copied().collect();
        let cof = appearance::cof(&world.inventory);
        let items = world
            .inventory
            .items
            .values()
            .filter(|i| Some(i.parent) != cof)
            .map(|i| i.id)
            .collect();
        let outfit_before = outfit_targets(&world);
        let objects_before = world.objects.iter().map(|(_, o)| (o.key.region, o.key.local_id)).collect();
        let ids_before = world.objects.iter().map(|(_, o)| o.full_id).collect();
        Self {
            world,
            runner: Runner::default(),
            sim: Sim::default(),
            now: Instant::now(),
            sent: Vec::new(),
            lost: None,
            outfit: None,
            online: true,
            folders,
            items,
            outfit_before,
            objects_before,
            ids_before,
            worn_before,
        }
    }

    fn start(&mut self, mode: Mode, phase2: bool) {
        self.runner.request();
        self.runner.phase2 = phase2;
        self.runner.confirm(&self.world, mode, Uuid::nil(), self.now);
    }

    fn send(&mut self, mut command: NetCommand, actor: Uuid) {
        let outcome = self.world.rlva_send_boundary(&mut command, actor);
        self.runner.boundary(outcome, &command);
        if outcome != Boundary::Send {
            return;
        }
        self.world.rlva_note_sent(&mut command, actor, self.now);
        self.sent.push(command.clone());
        if self.lost.is_some_and(|lost| lost(&command)) {
            return;
        }
        if let Some(events) = self.sim.reply(&mut self.world, &command, true) {
            for event in events {
                self.receive(event);
            }
        }
    }

    fn receive(&mut self, event: NetEvent) {
        if self.runner.event(&mut self.world, &event) {
            return;
        }
        if let NetEvent::OutfitUpdated { request, result } = &event
            && self.outfit.as_ref().is_some_and(|o| o.request == *request)
        {
            let Some(outfit) = self.outfit.take() else { return };
            if let Ok(contents) = result {
                self.world.inventory.apply(contents.clone());
                if outfit.sync {
                    let sync = appearance::sync_with_attachments(
                        &self.world.inventory,
                        self.world.agent_id,
                        &self.world.outfit_attachment_items(),
                        &outfit.worn_before,
                        Uuid::nil(),
                        outfit.attachments,
                    );
                    for command in sync.commands {
                        self.send(command, outfit.actor);
                    }
                }
            }
            return;
        }
        self.world.apply(event);
    }

    /// The tail of `App::apply_appearance_action_from`: a forced wear becomes
    /// an outfit update, then the attachment requests that follow it.
    fn apply_action(&mut self, action: appearance::Action, actor: Uuid) {
        let mut attachments = appearance::attachments_for_action(&self.world.inventory, self.world.agent_id, &action);
        let Ok((change, sync)) = appearance::plan(&self.world.inventory, action, &self.world.outfit_attachment_points()) else {
            return;
        };
        let request = Uuid::new_v4();
        let worn_before = self.world.outfit_attachment_items();
        let mut command = NetCommand::UpdateOutfit { request, change };
        let dropped = self.world.rlva_prepare_outfit(&mut command, actor);
        attachments.retain(|a| !dropped.contains(&a.item_id));
        if !self.world.rlva_allows_command(&command, actor) {
            return;
        }
        if let NetCommand::UpdateOutfit { change, .. } = &mut command {
            appearance::defer_attachment_links(&self.world.inventory, change, &worn_before, &mut attachments, self.world.agent_id);
        }
        self.outfit = Some(Outfit {
            request,
            sync,
            actor,
            attachments,
            worn_before,
        });
        self.send(command, actor);
    }

    /// What `App::update_rlva` does each frame, for what the cases need.
    fn rlva(&mut self) {
        use crate::world::rlva::Param;
        let now = self.now;
        self.world.rlva.appearance_busy = self.outfit.is_some() || self.world.rlva_detach_in_flight(now);
        self.world.rlva_watchdog_tick(now);
        for command in std::mem::take(&mut self.world.rlva.recovery) {
            self.send(command, crate::world::rlva::watchdog::ACTOR);
        }
        self.world.rlva.collect(now, |id| self.world.objects.index_of_uuid(&id).is_some());
        self.world.rlva_update_stand();
        for (actor, command) in self.world.rlva_sitting_tick() {
            self.send(command, actor);
        }
        let commands = self.world.rlva_pending();
        for (_, c) in std::mem::take(&mut self.world.rlva.viewer) {
            if let Param::Reply(channel) = c.param
                && let Some(text) = self.world.rlva.camera_reply(&c, 5.0, 0.8)
            {
                self.world.rlva.reply(channel, &c.behaviour, text);
            }
        }
        if !self.world.rlva.appearance_busy && self.outfit.is_none() && !self.world.rlva.appearance.is_empty() {
            let (actor, action) = self.world.rlva.appearance.remove(0);
            if !self.world.rlva_force_settled(&action, now) {
                self.apply_action(action, actor);
            }
        }
        for (actor, command) in commands {
            self.send(command, actor);
        }
        for command in std::mem::take(&mut self.world.rlva.out) {
            if let NetCommand::ChatReply { channel, message } = &command {
                self.runner.reply(*channel, message);
            }
        }
        if self.world.agent.flying && self.world.rlva.has("fly") {
            self.world.agent.flying = false;
        }
    }

    fn frame(&mut self) {
        self.now += Duration::from_millis(50);
        self.rlva();
        self.runner.step(&mut self.world, self.online, self.now);
        for out in self.runner.take_out() {
            match out {
                Out::Send(command) => self.send(command, Uuid::nil()),
                Out::Probe(command) => {
                    self.runner.probe(true);
                    self.send(command, Uuid::nil());
                    self.runner.probe(false);
                }
                Out::Hear(event) => self.receive(event),
                Out::Fly(on) => {
                    if !on || crate::autopilot::can_takeoff(&self.world) {
                        self.world.agent.flying = on;
                    }
                }
            }
        }
    }

    /// Runs until the report is there.
    fn run(&mut self) -> super::Report {
        for _ in 0..200_000 {
            if !self.runner.running() {
                break;
            }
            self.frame();
        }
        assert!(!self.runner.running(), "the run never ended: {}", self.runner.view().current);
        self.runner.report().cloned().expect("a run ends with a report")
    }

    /// Runs until `when` holds.
    fn run_until(&mut self, when: impl Fn(&Host) -> bool) {
        for _ in 0..200_000 {
            if when(self) {
                return;
            }
            assert!(self.runner.running(), "the run ended first");
            self.frame();
        }
        panic!("never reached: {}", self.runner.view().current);
    }

    fn test_folder(&self) -> Uuid {
        let inv = &self.world.inventory;
        inv.folders
            .values()
            .find(|f| f.info.name == FOLDER)
            .map(|f| f.info.id)
            .expect("the test folder")
    }

    fn under_test_folder(&self, mut id: Uuid) -> bool {
        let root = self.test_folder();
        for _ in 0..16 {
            if id == root {
                return true;
            }
            match self.world.inventory.folders.get(&id) {
                Some(f) => id = f.info.parent,
                None => return false,
            }
        }
        false
    }

    fn worn_test_objects(&self) -> usize {
        self.world
            .inventory
            .items
            .values()
            .filter(|i| i.asset_type == 6 && i.name.starts_with(FOLDER) && self.world.worn_attachment(&i.id).is_some())
            .count()
    }

    fn seat_prims(&self) -> usize {
        // (the demo's prims have other sizes: the run's are 0.5 m boxes
        // created selected)
        self.world
            .objects
            .iter()
            .filter(|(_, o)| o.update_flags & crate::build::FLAGS_CREATE_SELECTED != 0)
            .count()
    }

    /// Nothing the run sent names an inventory item or folder that existed
    /// before it, nor an object it did not create.
    fn assert_only_own(&self) {
        use aurora_net::inventory::operations::Operation;
        let root = self.test_folder();
        let rlv = self.world.inventory.folders[&root].info.parent;
        let mine = |id: &Uuid| self.world.inventory.items.get(id).is_some_and(|i| i.name.starts_with(FOLDER));
        // (an object that was there before the run is not the run's)
        let own = |handle: &u64, ids: &[u32]| !ids.is_empty() && ids.iter().all(|id| !self.objects_before.contains(&(*handle, *id)));
        for command in &self.sent {
            let ok = match command {
                NetCommand::EditInventory { change, .. } => {
                    change.removed.is_empty()
                        && change
                            .copies
                            .iter()
                            .all(|c| self.under_test_folder(c.parent) && c.name.starts_with(FOLDER))
                        && change.operations.iter().all(|op| match op {
                            Operation::Folder { parent, name, .. } => {
                                self.under_test_folder(*parent) || (*parent == rlv && name == FOLDER) || name == "#RLV"
                            }
                            Operation::Links { parent, links } => self.under_test_folder(*parent) && links.iter().all(|l| mine(&l.target)),
                            // never a patch (rename, move), a delete or a purge
                            _ => false,
                        })
                }
                NetCommand::FinishInventoryCopies { reply, .. } => reply.copies.iter().all(|c| self.under_test_folder(c.parent)),
                NetCommand::RezAttachments(items) => items.iter().all(|i| mine(&i.item_id)),
                NetCommand::DetachAttachments(items) => items.iter().all(mine),
                // the forced wear's outfit update: the test's objects only
                NetCommand::UpdateOutfit { change, .. } => change
                    .links
                    .iter()
                    .chain(change.cof.iter().flat_map(|(_, links)| links.iter()))
                    .all(|l| mine(&l.target) || self.outfit_before.contains(&l.target)),
                NetCommand::RequestTaskInventory { .. } | NetCommand::SaveScript { .. } => true,
                NetCommand::ScriptDialogReply { channel, .. } => *channel < 0,
                NetCommand::RequestSit { target, .. } => !self.ids_before.contains(target),
                NetCommand::OneShotControl(flags) => *flags == aurora_net::control::STAND_UP,
                NetCommand::RequestServerAppearance { .. } => true,
                NetCommand::Build(build) => match build {
                    BuildCmd::Add(prim) => prim.scale.max_element() <= 0.5 && prim.ray_end.distance(prim.ray_start) <= 3.0,
                    BuildCmd::SetName { handle, local_ids, name } => name == SEAT_NAME && own(handle, local_ids),
                    BuildCmd::Deselect { handle, local_ids } | BuildCmd::Link { handle, local_ids } => own(handle, local_ids),
                    // the one deletion: its own prims, to the Trash
                    BuildCmd::DeRez {
                        handle,
                        local_ids,
                        destination,
                        ..
                    } => *destination == aurora_net::build::derez::TRASH && own(handle, local_ids),
                    BuildCmd::RezScript { handle, local_id, .. } => own(handle, &[*local_id]),
                    // never a transform, a duplicate, an unlink nor a change
                    // of contents: those are only ever refused probes
                    _ => false,
                },
                _ => false,
            };
            assert!(ok, "a command outside the test's own objects left: {command:?}");
        }
        assert!(
            self.sent
                .iter()
                .filter(|c| matches!(c, NetCommand::Build(BuildCmd::Add(_))))
                .count()
                <= 2,
            "more than two prims were rezzed"
        );
        // nothing that was in the inventory before changed place, name or went away
        for id in &self.folders {
            assert!(self.world.inventory.folders.contains_key(id), "a folder went away");
        }
        for id in &self.items {
            assert!(self.world.inventory.items.contains_key(id), "an item went away");
        }
        // what the avatar wore is still worn
        for id in &self.worn_before {
            assert!(
                self.world.worn_attachment(id).is_some(),
                "an attachment of the avatar was taken off"
            );
        }
        // and the avatar's outfit is what it was
        if !self.runner.running() {
            assert_eq!(outfit_targets(&self.world), self.outfit_before, "the outfit changed");
        }
    }
}

fn verdict_of<'a>(report: &'a super::Report, id: &str) -> &'a Verdict {
    &report
        .lines
        .iter()
        .find(|l| l.id == id)
        .unwrap_or_else(|| panic!("no line {id}"))
        .verdict
}

fn assert_all_pass(report: &super::Report) {
    let bad: Vec<String> = report
        .lines
        .iter()
        .filter(|l| !matches!(l.verdict, Verdict::Pass | Verdict::Skipped(_) | Verdict::Known(_)))
        .map(super::report::Line::text)
        .collect();
    assert!(bad.is_empty(), "failing cases:\n{}\n\n{}", bad.join("\n"), report.text());
}

#[test]
fn selftest_is_a_development_build_tool() {
    assert!(super::available_in("debug"));
    assert!(super::available_in("ci"));
    assert!(super::available_in("debugging"));
    assert!(!super::available_in("release"));
}

#[test]
fn selftest_guard_lets_only_the_runs_own_objects_through() {
    use aurora_net::build::{TransformUpdate, derez};
    let mut host = Host::new();
    let world = &mut host.world;
    let handle = world.main_region.unwrap_or_default();
    let mut ctx = Ctx::new(world, Mode::Simulated, Uuid::nil());
    // one object of the test, one of the avatar's own
    let own_item = Uuid::new_v4();
    ctx.items[0] = own_item;
    let foreign_item = *host.worn_before.iter().next().expect("the demo avatar wears something");
    // a prim the run rezzed, and a prim that was there before it
    let rezzed = crate::demo::build_rez(&super::cases_world::new_prim(world, world.agent.position + glam::Vec3::X));
    let prim = Prim {
        id: rezzed.full_id,
        handle,
        local_id: rezzed.local_id,
    };
    let other = crate::demo::build_rez(&super::cases_world::new_prim(world, world.agent.position - glam::Vec3::X));
    let (other_id, other_local) = (other.full_id, other.local_id);
    world.apply(NetEvent::ObjectUpdates {
        handle,
        objects: vec![rezzed, other],
    });
    ctx.prims.push(prim);
    let world = &host.world;
    let attach = |item: Uuid| {
        NetCommand::RezAttachments(vec![AttachRequest {
            item_id: item,
            owner_id: world.agent_id,
            point: super::POINT_B,
            add: true,
            flags: 0,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            name: String::new(),
            desc: String::new(),
        }])
    };
    let to = |local_id: u32, destination: u8| {
        NetCommand::Build(BuildCmd::DeRez {
            handle,
            local_ids: vec![local_id],
            destination,
            destination_id: Uuid::nil(),
            group_id: Uuid::nil(),
        })
    };
    let moved = |local_id: u32| {
        NetCommand::Build(BuildCmd::Transform {
            handle,
            updates: vec![TransformUpdate {
                local_id,
                position: Some(glam::Vec3::ONE),
                rotation: None,
                scale: None,
                linked: true,
                uniform: false,
            }],
        })
    };
    let sit = |target: Uuid| NetCommand::RequestSit {
        handle,
        target,
        offset: glam::Vec3::ZERO,
    };
    let named = |local_id: u32| {
        NetCommand::Build(BuildCmd::SetName {
            handle,
            local_ids: vec![local_id],
            name: SEAT_NAME.to_owned(),
        })
    };
    let add = |scale: f32, away: f32| {
        let mut prim = super::cases_world::new_prim(world, world.agent.position + glam::Vec3::X * away);
        prim.scale = glam::Vec3::splat(scale);
        NetCommand::Build(BuildCmd::Add(Box::new(prim)))
    };
    for refused in [false, true] {
        let allows = |command: &NetCommand| guard::allows(command, &ctx, world, refused);
        // the test's own
        assert!(allows(&attach(own_item)));
        assert!(allows(&NetCommand::DetachAttachments(vec![own_item])));
        assert!(allows(&named(prim.local_id)));
        assert!(allows(&to(prim.local_id, derez::TRASH)));
        assert!(allows(&add(0.5, 1.5)));
        // what changes an object passes as a refused probe only
        assert_eq!(allows(&moved(prim.local_id)), refused);
        assert_eq!(allows(&sit(prim.id)), refused);
        assert_eq!(allows(&to(prim.local_id, derez::TAKE_INTO_AGENT_INVENTORY)), refused);
        assert_eq!(allows(&to(prim.local_id, derez::RETURN_TO_OWNER)), refused);
        // never anything else
        assert!(!allows(&attach(foreign_item)));
        assert!(!allows(&NetCommand::DetachAttachments(vec![foreign_item])));
        assert!(!allows(&NetCommand::DetachAttachments(vec![own_item, foreign_item])));
        assert!(!allows(&NetCommand::DetachAttachments(Vec::new())));
        for command in [
            named(other_local),
            moved(other_local),
            to(other_local, derez::TRASH),
            to(other_local, derez::RETURN_TO_OWNER),
            sit(other_id),
            add(1.0, 1.5),
            add(0.5, 6.0),
            NetCommand::OneShotControl(aurora_net::control::STAND_UP | 1),
            NetCommand::TeleportHome,
            NetCommand::Chat {
                message: "hello".to_owned(),
                channel: 0,
                chat_type: aurora_net::ChatType::Normal,
            },
        ] {
            assert!(!allows(&command), "{command:?}");
        }
    }
    // a third prim is never asked for
    ctx.adds = 2;
    assert!(!guard::allows(&add(0.5, 1.5), &ctx, world, false));
    // a prim that is no longer the one the run rezzed is not the run's
    ctx.prims[0].local_id += 1;
    assert!(!guard::allows(&named(prim.local_id), &ctx, world, false));
    assert!(!guard::allows(&to(prim.local_id, derez::TRASH), &ctx, world, false));
}

#[test]
fn selftest_needs_the_tool_then_the_confirmation() {
    let mut host = Host::new();
    // without the tool's press, a confirmation starts nothing
    host.runner.confirm(&host.world, Mode::Simulated, Uuid::nil(), host.now);
    assert!(!host.runner.running());
    for _ in 0..20 {
        host.frame();
    }
    assert!(host.sent.is_empty(), "nothing is sent before the confirmation");
    // the tool opens the dialog, with the in-world cases off; cancelled, nothing runs
    host.runner.phase2 = true;
    host.runner.request();
    assert!(host.runner.confirming() && !host.runner.phase2);
    for _ in 0..20 {
        host.frame();
    }
    assert!(!host.runner.running() && host.sent.is_empty());
    host.runner.cancel();
    host.runner.confirm(&host.world, Mode::Simulated, Uuid::nil(), host.now);
    assert!(!host.runner.running());
}

#[test]
fn selftest_phase_1_passes_on_the_demo_world() {
    let mut host = Host::new();
    host.start(Mode::Simulated, false);
    let report = host.run();
    assert_all_pass(&report);
    assert!(report.count(|v| matches!(v, Verdict::Pass)) >= 40, "{}", report.text());
    assert!(report.notes.is_empty(), "{}", report.text());
    // the link case (#2677): held by the folder of its link
    assert_eq!(verdict_of(&report, "C4"), &Verdict::Pass, "{}", report.text());
    assert_eq!(report.count(|v| matches!(v, Verdict::Known(_))), 0, "{}", report.text());
    assert!(matches!(verdict_of(&report, "B05"), Verdict::Skipped(_)));
    assert!(!report.lines.iter().any(|l| l.id.starts_with('W')), "phase 2 was not chosen");
    // cleaned up: nothing of the test is worn, no rule is left
    assert_eq!(host.worn_test_objects(), 0);
    assert!(host.world.rlva.restrictions().is_empty());
    host.assert_only_own();
    assert!(
        !host
            .sent
            .iter()
            .any(|c| matches!(c, NetCommand::Build(_) | NetCommand::RequestSit { .. }))
    );
    // the report names no id
    let text = report.text();
    assert!(!text.contains(&host.world.agent_id.to_string()));
    for item in host.world.inventory.items.values().filter(|i| i.name.starts_with(FOLDER)) {
        assert!(!text.contains(&item.asset_id.to_string()) && !text.contains(&item.id.to_string()));
    }
}

#[test]
fn selftest_reuses_what_an_earlier_run_made() {
    let mut host = Host::new();
    host.start(Mode::Simulated, false);
    assert_all_pass(&host.run());
    let made = host.world.inventory.items.len();
    let folders = host.world.inventory.folders.len();
    host.sent.clear();
    host.start(Mode::Simulated, false);
    assert_all_pass(&host.run());
    assert_eq!(host.world.inventory.folders.len(), folders, "no folder is made twice");
    assert_eq!(
        host.world
            .inventory
            .items
            .values()
            .filter(|i| i.asset_type == 6 && i.name.starts_with(FOLDER))
            .count(),
        6,
        "no object is copied twice"
    );
    assert!(host.world.inventory.items.len() <= made);
    assert!(!host.sent.iter().any(|c| matches!(c, NetCommand::EditInventory { .. })));
}

#[test]
fn selftest_with_the_relay_script_passes() {
    // the worn controller: the script is listed, added, written, and
    // answers on its private channel
    let mut host = Host::new();
    host.start(Mode::Worn, false);
    let report = host.run();
    assert_all_pass(&report);
    assert!(
        host.sent
            .iter()
            .any(|c| matches!(c, NetCommand::SaveScript { source, .. } if source.contains("llOwnerSay")))
    );
    assert!(
        host.sent
            .iter()
            .all(|c| !matches!(c, NetCommand::ScriptDialogReply { channel, .. } if *channel >= 0)),
        "the relay is never spoken to on a public channel"
    );
    host.assert_only_own();
}

#[test]
fn selftest_both_phases_pass_on_the_demo_world() {
    let mut host = Host::new();
    host.start(Mode::Simulated, true);
    let report = host.run();
    assert_all_pass(&report);
    for id in ["W00", "W01", "W02", "W03", "W04", "W05", "W05b", "W06", "W07"] {
        assert_eq!(verdict_of(&report, id), &Verdict::Pass, "{id}\n{}", report.text());
    }
    assert!(report.notes.is_empty(), "{}", report.text());
    assert!(!host.world.agent.seated && !host.world.agent.flying);
    assert_eq!(host.seat_prims(), 0, "its prims are removed");
    assert_eq!(host.worn_test_objects(), 0);
    host.assert_only_own();
}

#[test]
fn selftest_stop_ends_in_the_cleanup_with_a_report() {
    let mut host = Host::new();
    host.start(Mode::Simulated, false);
    host.run_until(|h| h.runner.view().done >= 12);
    host.runner.stop();
    let report = host.run();
    assert!(report.notes.iter().any(|n| n.contains("stopped")), "{}", report.text());
    assert!(report.count(|v| matches!(v, Verdict::Skipped(_))) > 10);
    assert!(report.count(|v| matches!(v, Verdict::Pass)) >= 12);
    assert_eq!(host.worn_test_objects(), 0);
    assert!(host.world.rlva.restrictions().is_empty());
    host.assert_only_own();
}

#[test]
fn selftest_timeout_ends_in_the_cleanup_with_a_report() {
    // the simulator never wears the test object B: the cases that need it
    // time out and fail, the others go on, and the run still cleans up
    let mut host = Host::new();
    host.lost = Some(|c| matches!(c, NetCommand::RezAttachments(items) if items.iter().any(|i| i.name.ends_with(" b"))));
    host.start(Mode::Simulated, false);
    let report = host.run();
    assert_eq!(verdict_of(&report, "C1"), &Verdict::Fail, "{}", report.text());
    assert_eq!(verdict_of(&report, "A04"), &Verdict::Pass);
    assert_eq!(verdict_of(&report, "C5"), &Verdict::Pass, "{}", report.text());
    assert_eq!(host.worn_test_objects(), 0);
    assert!(host.world.rlva.restrictions().is_empty());
    host.assert_only_own();
}

#[test]
fn selftest_setup_timeout_skips_everything() {
    // the copies never arrive
    let mut host = Host::new();
    host.lost = Some(|c| matches!(c, NetCommand::FinishInventoryCopies { .. }));
    host.start(Mode::Simulated, false);
    let report = host.run();
    assert_eq!(report.count(|v| matches!(v, Verdict::Pass | Verdict::Fail)), 0);
    assert!(
        report.notes.iter().any(|n| n.contains("could not be prepared")),
        "{}",
        report.text()
    );
    assert!(!report.lines.is_empty());
}

#[test]
fn selftest_rlva_switched_off_ends_the_run() {
    let mut host = Host::new();
    host.start(Mode::Simulated, false);
    host.run_until(|h| h.runner.view().done >= 5);
    host.world.rlva_configure(false);
    let report = host.run();
    assert!(report.notes.iter().any(|n| n.contains("switched off")), "{}", report.text());
    assert_eq!(host.worn_test_objects(), 0);
}

#[test]
fn selftest_failure_in_phase_2_still_stands_up_and_removes_its_prims() {
    // seated on its prim under @unsit, the relay scripts stop answering: the
    // case fails, the cleanup's own @clear is not heard either, and the
    // cleanup still releases the rules (the relays come off), stands the
    // avatar up and removes the prims
    let mut host = Host::new();
    host.start(Mode::Worn, true);
    host.run_until(|h| h.world.agent.seated && h.world.rlva.has("unsit"));
    assert_eq!(host.seat_prims(), 2);
    host.lost = Some(|c| matches!(c, NetCommand::ScriptDialogReply { .. }));
    let report = host.run();
    assert_eq!(verdict_of(&report, "W01"), &Verdict::Fail, "{}", report.text());
    assert!(report.notes.iter().any(|n| n.contains("stopped answering")), "{}", report.text());
    assert!(!host.world.agent.seated, "{}", report.text());
    assert_eq!(host.seat_prims(), 0, "{}", report.text());
    assert_eq!(host.worn_test_objects(), 0);
    assert!(host.world.rlva.restrictions().is_empty());
    host.assert_only_own();
}

#[test]
fn selftest_stop_in_phase_2_stands_up_and_removes_its_prims() {
    let mut host = Host::new();
    host.start(Mode::Simulated, true);
    host.run_until(|h| h.world.agent.seated && h.world.rlva.has("unsit"));
    host.runner.stop();
    let report = host.run();
    assert!(report.notes.iter().any(|n| n.contains("stopped")), "{}", report.text());
    assert!(!host.world.agent.seated, "{}", report.text());
    assert_eq!(host.seat_prims(), 0, "{}", report.text());
    assert_eq!(host.worn_test_objects(), 0);
    assert!(host.world.rlva.restrictions().is_empty());
    host.assert_only_own();
}

#[test]
fn selftest_moves_the_avatar_once_each_way() {
    // one sit, on its own prim, and one take-off
    let mut host = Host::new();
    host.start(Mode::Simulated, true);
    let (mut sits, mut flights, mut seated, mut flying) = (0, 0, false, false);
    while host.runner.running() {
        host.frame();
        sits += usize::from(host.world.agent.seated && !seated);
        flights += usize::from(host.world.agent.flying && !flying);
        (seated, flying) = (host.world.agent.seated, host.world.agent.flying);
    }
    assert_eq!((sits, flights), (1, 1));
    let sit = host.sent.iter().filter(|c| matches!(c, NetCommand::RequestSit { .. })).count();
    assert_eq!(sit, 1, "one sit request leaves");
}

#[test]
fn selftest_teleport_in_phase_2_ends_the_run() {
    let mut host = Host::new();
    host.start(Mode::Simulated, true);
    host.run_until(|h| h.seat_prims() == 2);
    host.world.agent.position += glam::Vec3::X * 60.0;
    let report = host.run();
    assert!(report.notes.iter().any(|n| n.contains("teleported")), "{}", report.text());
    assert_eq!(host.seat_prims(), 0, "its prims are removed even from afar");
    assert_eq!(host.worn_test_objects(), 0);
}

#[test]
fn selftest_logout_ends_the_run_at_once() {
    let mut host = Host::new();
    host.start(Mode::Simulated, false);
    host.run_until(|h| h.runner.view().done >= 3);
    host.online = false;
    let before = host.sent.len();
    let report = host.run();
    assert_eq!(host.sent.len(), before, "nothing is sent once the session is gone");
    assert!(report.notes.iter().any(|n| n.contains("logged out")), "{}", report.text());
}
