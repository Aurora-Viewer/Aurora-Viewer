use super::*;
use std::sync::Arc;

struct Simulator {
    world: World,
    bridge: Bridge,
    demo: demo::Demo,
    prefs: Preferences,
    now: Instant,
    copies: usize,
    uploads: usize,
    attaches: usize,
    unready_edits: usize,
}
impl Simulator {
    fn new() -> Self {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        demo::seed(&mut world);
        world.movement_complete = true;
        crate::ao::demo::inventory_services(&mut world, true);
        Self {
            world,
            bridge: Bridge::default(),
            demo: demo::Demo::default(),
            prefs: Preferences {
                enabled: true,
                ..Default::default()
            },
            now: Instant::now(),
            copies: 0,
            uploads: 0,
            attaches: 0,
            unready_edits: 0,
        }
    }
    fn event(&mut self, event: NetEvent) {
        self.bridge.receive(&mut self.world, event, self.now, self.now);
    }
    fn step(&mut self) {
        self.now += Duration::from_millis(250);
        self.bridge.update(&mut self.world, &self.prefs, true, self.now);
        for (folder, library) in std::mem::take(&mut self.world.inventory.queue) {
            let command = NetCommand::FetchInventory {
                folders: vec![folder],
                owner: if library {
                    self.world.inventory.lib_owner
                } else {
                    self.world.agent_id
                },
                library,
            };
            for event in self
                .demo
                .reply(&self.bridge, &self.world, &command, false, false, false)
                .expect("folder response")
            {
                self.event(event);
            }
        }
        for _ in 0..10 {
            let commands = std::mem::take(&mut self.bridge.commands);
            if commands.is_empty() {
                break;
            }
            for command in commands {
                if self.bridge.observe_command(&command, self.now) {
                    self.prefs.enabled = false;
                }
                match &command {
                    NetCommand::EditInventory { request, change } => {
                        let result = if self.world.inventory_writable() {
                            super::super::inventory::actions::demo_mutate(&mut self.world.inventory, self.world.agent_id, change.clone())
                        } else {
                            self.unready_edits += 1;
                            Err(aurora_i18n::tr!("session-the-inventory-server-is-not-available-yet").into())
                        };
                        self.event(NetEvent::InventoryEdited { request: *request, result });
                    }
                    NetCommand::FinishInventoryCopies { request, reply } => {
                        self.copies += reply.copies.len();
                        let events = self
                            .demo
                            .reply(
                                &self.bridge,
                                &self.world,
                                &NetCommand::FinishInventoryCopies {
                                    request: *request,
                                    reply: reply.clone(),
                                },
                                false,
                                false,
                                false,
                            )
                            .expect("copy response");
                        for event in events {
                            self.event(event);
                        }
                    }
                    NetCommand::RezAttachments(items) => {
                        self.attaches += items.len();
                        self.world.apply(NetEvent::ObjectUpdates {
                            handle: self.world.main_region.expect("region"),
                            objects: items.iter().map(crate::demo::outfit_attachment).collect(),
                        });
                    }
                    _ => {
                        if matches!(command, NetCommand::SaveScript { .. }) {
                            self.uploads += 1;
                        }
                        let events = self
                            .demo
                            .reply(&self.bridge, &self.world, &command, false, false, false)
                            .expect("bridge simulator command");
                        for event in events {
                            self.event(event);
                        }
                    }
                }
            }
        }
    }
    fn ready(&mut self) {
        for _ in 0..80 {
            self.step();
            if self.bridge.locked() {
                return;
            }
        }
        panic!("bridge did not become ready: {:?} {:?}", self.bridge.phase, self.bridge.error);
    }
    fn starting(&mut self) {
        for _ in 0..80 {
            self.step();
            if self.bridge.phase == Phase::Starting {
                return;
            }
        }
        panic!("bridge never started: {:?}", self.bridge.phase);
    }
}

#[test]
fn failed_http_command_preserves_its_reason_and_recovers_without_detaching_or_disabling_movelock() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.bridge.commands.clear();
    sim.bridge.motion = Motion::motor(1, 2.0, 1.7);
    sim.bridge.send_mode(&sim.prefs, sim.now);
    let (request, _, _) = sim.bridge.pending.expect("motor command");
    let reason = aurora_i18n::tr!("bridge-request-timeout").to_owned();
    sim.event(NetEvent::BridgeReply {
        request,
        result: Err(reason.clone()),
    });
    assert!(!sim.bridge.available());
    assert_eq!(sim.bridge.diagnostic(), Some(reason.as_str()));
    assert!(sim.prefs.enabled);
    sim.bridge.commands.clear();
    sim.now += Duration::from_secs(1);
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert!(!sim.bridge.commands.iter().any(|c| matches!(c, NetCommand::Bridge { .. })));
    assert!(!sim.bridge.commands.iter().any(|c| matches!(c, NetCommand::DetachAttachments(_))));
    sim.now += Duration::from_secs(5);
    sim.step();
    assert!(sim.bridge.available());
    assert!(sim.prefs.enabled);
    assert!(sim.bridge.diagnostic().is_none());
    assert_eq!(sim.attaches, 2, "HTTP recovery must not reattach the controller");
}

#[test]
fn discovery_simulator_rejects_negative_chatfromviewer_and_misaddressed_dialog_replies() {
    let mut sim = Simulator::new();
    sim.ready();
    let request = |object_id, label| NetCommand::ScriptDialogReply {
        object_id,
        channel: DISCOVERY_CHANNEL,
        index: 0,
        label,
    };
    for command in [
        NetCommand::Chat {
            message: format!("auroraHello|{}", sim.bridge.object),
            channel: DISCOVERY_CHANNEL,
            chat_type: ChatType::Whisper,
        },
        request(Uuid::new_v4(), format!("auroraHello|{}", sim.bridge.object)),
        request(sim.world.agent_id, format!("auroraHello|{}", Uuid::new_v4())),
    ] {
        assert!(sim.demo.reply(&sim.bridge, &sim.world, &command, false, false, false).is_none());
    }
    let valid = request(sim.world.agent_id, format!("auroraHello|{}", sim.bridge.object));
    assert_eq!(
        sim.demo
            .reply(&sim.bridge, &sim.world, &valid, false, false, false)
            .expect("valid transport")
            .len(),
        1
    );
}

#[test]
fn movement_diagnostic_preserves_connection_failure_and_reports_the_actual_startup_stage() {
    let mut sim = Simulator::new();
    sim.ready();
    assert!(sim.bridge.diagnostic().is_none());
    sim.bridge.fail("Synthetic HTTP failure".into());
    assert_eq!(sim.bridge.diagnostic(), Some("Synthetic HTTP failure"));
    sim.bridge.error = None;
    sim.bridge.enter(Phase::Handshake, sim.now);
    sim.bridge.boot = Some((false, true));
    assert_eq!(sim.bridge.diagnostic(), Some(aurora_i18n::tr!("bridge-waiting-permissions")));
    sim.bridge.boot = Some((true, false));
    assert_eq!(sim.bridge.diagnostic(), Some(aurora_i18n::tr!("bridge-waiting-url")));
    sim.bridge.boot = Some((true, true));
    assert_eq!(sim.bridge.diagnostic(), Some(aurora_i18n::tr!("bridge-waiting-handshake")));
}

#[test]
fn quiet_bridge_discovery_is_addressed_throttled_and_requires_the_current_worn_root() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.bridge.commands.clear();
    sim.bridge.probe = None;
    sim.bridge.discover(&sim.world, sim.now);
    let commands = std::mem::take(&mut sim.bridge.commands);
    assert_eq!(commands.len(), 1);
    assert!(
        matches!(&commands[0], NetCommand::ScriptDialogReply { object_id, channel, index, label }
        if *object_id == sim.world.agent_id && *channel == DISCOVERY_CHANNEL && *channel < 0 && *index == 0
        && *label == format!("auroraHello|{}", sim.bridge.object))
    );
    for _ in 0..19 {
        sim.now += Duration::from_millis(250);
        sim.bridge.discover(&sim.world, sim.now);
    }
    assert!(sim.bridge.commands.is_empty());
    sim.now += Duration::from_millis(250);
    sim.bridge.discover(&sim.world, sim.now);
    assert_eq!(sim.bridge.commands.len(), 1);
    sim.bridge.commands.clear();
    sim.now += DISCOVERY_INTERVAL;
    sim.bridge.object = Uuid::new_v4();
    sim.bridge.discover(&sim.world, sim.now);
    assert!(sim.bridge.commands.is_empty(), "do not probe a stale or unrelated object");
    sim.bridge.object = sim
        .world
        .worn_attachment(&sim.bridge.item)
        .and_then(|i| sim.world.objects.get(i))
        .expect("worn")
        .full_id;
    sim.world.main_region = Some(sim.world.main_region.expect("region") + 65536);
    sim.bridge.discover(&sim.world, sim.now);
    assert!(sim.bridge.commands.is_empty(), "do not send into another region");
}

#[test]
fn discovery_recovers_a_quiet_bridge_after_a_lost_hello_and_stops_when_ready() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.bridge.commands.clear();
    sim.bridge.endpoint = None;
    sim.bridge.candidate = None;
    sim.bridge.probe = None;
    sim.bridge.enter(Phase::Handshake, sim.now);
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    let lost = std::mem::take(&mut sim.bridge.commands);
    assert!(lost.iter().any(|command| matches!(command, NetCommand::ScriptDialogReply { .. })));
    sim.now += DISCOVERY_INTERVAL;
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    let command = std::mem::take(&mut sim.bridge.commands)
        .into_iter()
        .find(|c| matches!(c, NetCommand::ScriptDialogReply { .. }))
        .expect("retry");
    for reply in sim
        .demo
        .reply(&sim.bridge, &sim.world, &command, false, false, false)
        .expect("quiet response")
    {
        sim.event(reply);
    }
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert!(sim.bridge.ready());
    sim.bridge.commands.clear();
    sim.now += DISCOVERY_INTERVAL;
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert!(
        !sim.bridge
            .commands
            .iter()
            .any(|c| matches!(c, NetCommand::ScriptDialogReply { .. }))
    );
}

#[test]
fn ready_bridge_rediscovers_a_reset_endpoint_without_changing_the_movelock_selection() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.bridge.commands.clear();
    sim.bridge.error = Some("Synthetic expired endpoint".into());
    sim.bridge.probe = None;
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    let commands = std::mem::take(&mut sim.bridge.commands);
    let probe = commands
        .iter()
        .find(|c| matches!(c, NetCommand::ScriptDialogReply { .. }))
        .expect("rediscovery");
    let mut replies = sim
        .demo
        .reply(&sim.bridge, &sim.world, probe, false, false, false)
        .expect("response");
    let NetEvent::Chat(chat) = &mut replies[0] else { panic!("hello") };
    chat.message = chat
        .message
        .replace("11111111-1111-4111-8111-111111111111", "22222222-2222-4222-8222-222222222222");
    sim.event(replies.remove(0));
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert!(sim.bridge.available());
    assert!(sim.prefs.enabled);
    assert!(
        sim.bridge
            .endpoint
            .as_ref()
            .expect("endpoint")
            .url
            .contains("22222222-2222-4222-8222-222222222222")
    );
    assert!(!sim.bridge.commands.iter().any(|c| matches!(c, NetCommand::DetachAttachments(_))));
}

#[test]
fn passage_completion_survives_a_late_startup_http_reply_but_not_a_new_action() {
    let mut sim = Simulator::new();
    sim.ready();
    let report = aurora_net::bridge::PassageReport {
        action: 123,
        state: 4,
        progress_cm: 10,
        remaining_cm: 140,
        speed_cm: 50,
        pushes: 2,
    };
    let ack = Ack {
        sequence: sim.bridge.sequence,
        passage: Some(report),
        ..Default::default()
    };
    sim.bridge.acknowledge(ack);
    sim.bridge.acknowledge(Ack {
        passage: Some(aurora_net::bridge::PassageReport { state: 2, ..report }),
        ..ack
    });
    assert_eq!(sim.bridge.passage_report(), Some(report));
    let new_action = aurora_net::bridge::PassageReport {
        action: 456,
        state: 2,
        ..report
    };
    sim.bridge.acknowledge(Ack {
        passage: Some(new_action),
        ..ack
    });
    assert_eq!(sim.bridge.passage_report(), Some(new_action));
    sim.bridge.retry(sim.now);
    assert_eq!(sim.bridge.passage_report(), None);
}

#[test]
fn motor_feedback_expires_without_renewing_capture_or_changing_requested_movelock() {
    let mut sim = Simulator::new();
    sim.ready();
    let report = aurora_net::bridge::MotorReport {
        mode: 1,
        run_state: 5,
        speed_cm: 540,
        target_cm: 2565,
        pushes: 17,
        regulation: None,
    };
    sim.bridge.ack.motor = Some(report);
    sim.bridge.capture_sent = Some(sim.now);
    assert_eq!(sim.bridge.motor_report(sim.now), Some(report));
    assert!(sim.prefs.enabled);
    let locked = sim.bridge.locked();
    sim.now += Duration::from_secs(2);
    assert!(sim.bridge.motor_report(sim.now).is_none());
    assert!(sim.prefs.enabled);
    assert_eq!(sim.bridge.locked(), locked);
    sim.bridge.retry(sim.now);
    assert!(sim.bridge.motor_report(sim.now).is_none());
}

#[test]
fn compilation_does_not_detach_the_script_before_its_first_valid_hello() {
    let mut sim = Simulator::new();
    sim.starting();
    assert_eq!(sim.attaches, 1);
    sim.now += Duration::from_secs(3);
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert_eq!(sim.bridge.phase, Phase::Starting);
    assert!(
        !sim.bridge
            .commands
            .iter()
            .any(|command| matches!(command, NetCommand::DetachAttachments(_)))
    );
    let reply = NetEvent::ScriptRunning {
        object_id: sim.bridge.object,
        item_id: sim.bridge.script,
        running: false,
        mono: Some(true),
    };
    sim.event(reply.clone());
    sim.event(reply);
    assert_eq!(
        sim.bridge
            .commands
            .iter()
            .filter(|command| matches!(command, NetCommand::Build(BuildCmd::SetScriptRunning { running: true, .. })))
            .count(),
        1
    );
    let hello = sim.demo.handshake(&sim.bridge, &sim.world).expect("hello after startup");
    sim.event(hello);
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert_eq!(sim.bridge.phase, Phase::Store);
    assert!(!sim.bridge.ready(), "the initial endpoint must not bypass saving");
    assert!(
        sim.bridge
            .commands
            .iter()
            .any(|command| matches!(command, NetCommand::DetachAttachments(_)))
    );
}

#[test]
fn a_repeated_startup_hello_does_not_erase_the_lock_or_an_in_flight_command() {
    let mut sim = Simulator::new();
    sim.ready();
    let (hello, _) = sim.bridge.accepted.clone().expect("accepted hello");
    let sequence = sim.bridge.sequence;
    let pending = (Uuid::new_v4(), sequence, (true, true, Motion::default()));
    sim.bridge.pending = Some(pending);
    sim.event(NetEvent::Chat(hello));
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert!(sim.bridge.locked());
    assert_eq!(sim.bridge.sequence, sequence);
    assert_eq!(sim.bridge.pending, Some(pending));
}

#[test]
fn failed_read_queries_rediscover_quietly_without_resetting_physics_or_pending_motor_commands() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.now += DISCOVERY_INTERVAL;
    sim.bridge.commands.clear();
    let sequence = sim.bridge.sequence;
    let pending = (Uuid::new_v4(), sequence, (true, true, Motion::default()));
    sim.bridge.pending = Some(pending);
    let sent = sim.bridge.sent;
    let motion = sim.bridge.motion;
    let capture = sim.bridge.capture_sent;
    let copies = sim.copies;
    let attaches = sim.attaches;
    sim.bridge.refresh_queries(&sim.world, sim.now);
    sim.bridge.refresh_queries(&sim.world, sim.now + Duration::from_secs(1));
    assert_eq!(
        sim.bridge.commands.len(),
        1,
        "read failures share the five-second discovery throttle"
    );
    let command = sim.bridge.commands.pop().expect("quiet probe");
    assert!(matches!(&command, NetCommand::ScriptDialogReply { channel, .. } if *channel == DISCOVERY_CHANNEL));
    for event in sim
        .demo
        .reply(&sim.bridge, &sim.world, &command, false, false, false)
        .expect("hello")
    {
        sim.event(event);
    }
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert!(sim.bridge.locked() && sim.prefs.enabled);
    assert_eq!(sim.bridge.sequence, sequence);
    assert_eq!(sim.bridge.pending, Some(pending));
    assert_eq!(sim.bridge.sent, sent);
    assert_eq!(sim.bridge.motion, motion);
    assert_eq!(sim.bridge.capture_sent, capture);
    assert_eq!((sim.copies, sim.attaches), (copies, attaches));
}

#[test]
fn detachment_uses_firestorm_settle_delay_even_without_an_inventory_asset_update() {
    let mut sim = Simulator::new();
    sim.starting();
    sim.event(sim.demo.handshake(&sim.bridge, &sim.world).expect("first hello"));
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    let command = sim.bridge.commands.pop().expect("detach");
    let replies = sim
        .demo
        .reply(&sim.bridge, &sim.world, &command, false, false, false)
        .expect("save");
    for reply in replies {
        if !matches!(reply, NetEvent::InventoryUpdated { .. }) {
            sim.event(reply);
        }
    }
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    sim.now += STORE_DELAY - Duration::from_millis(250);
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert_eq!(sim.bridge.phase, Phase::Store, "the simulator must get its full save delay");
    assert!(
        !sim.bridge
            .commands
            .iter()
            .any(|command| matches!(command, NetCommand::RezAttachments(_)))
    );
    sim.now += Duration::from_millis(250);
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert_eq!(sim.bridge.phase, Phase::Restore);
    assert!(
        sim.bridge
            .commands
            .iter()
            .any(|command| matches!(command, NetCommand::RezAttachments(_)))
    );
}

#[test]
fn save_delay_starts_when_detachment_is_observed_not_when_it_is_requested() {
    let mut sim = Simulator::new();
    sim.starting();
    sim.event(sim.demo.handshake(&sim.bridge, &sim.world).expect("first hello"));
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    sim.bridge.commands.clear();
    sim.now += STORE_DELAY + Duration::from_secs(1);
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert_eq!(sim.bridge.phase, Phase::Store);
    assert!(sim.bridge.detached.is_none());
    assert!(
        sim.bridge.commands.is_empty(),
        "do not rez another copy while the bridge is still worn"
    );
}

#[test]
fn legacy_hard_lock_settings_are_ignored_and_can_never_disable_movement_relocking() {
    let mut sim = Simulator::new();
    sim.prefs = serde_json::from_str(r#"{"enabled":true,"relock_movement":false,"relock_region":false}"#).expect("legacy preferences");
    sim.ready();
    sim.demo.state(&mut sim.bridge, true, false, &sim.prefs);
    assert!(!sim.bridge.locked());
    sim.demo.state(&mut sim.bridge, false, false, &sim.prefs);
    assert!(sim.bridge.locked());
    assert!(sim.bridge.sent.expect("acknowledged mode").1);
    assert!(!serde_json::to_string(&sim.prefs).expect("preferences").contains("relock_movement"));
}

#[test]
fn explicit_detach_pauses_the_bridge_until_reactivation_and_does_not_duplicate_it() {
    let mut sim = Simulator::new();
    sim.ready();
    let item = sim.bridge.item;
    let original_region = sim.world.main_region;
    let command = NetCommand::DetachAttachments(vec![item]);
    assert!(sim.bridge.observe_command(&command, sim.now));
    sim.prefs.enabled = false;
    let replies = sim
        .demo
        .reply(&sim.bridge, &sim.world, &command, false, false, false)
        .expect("detach");
    for event in replies {
        sim.event(event);
    }
    sim.world.main_region = Some(sim.world.main_region.expect("region") + 65536);
    for _ in 0..80 {
        sim.step();
    }
    assert!(sim.bridge.user_detached && !sim.bridge.ready());
    assert_eq!(
        sim.attaches, 2,
        "an explicit removal must not trigger the normal lost-attachment recovery"
    );
    sim.world.main_region = original_region;
    sim.prefs.enabled = true;
    sim.bridge.retry(sim.now);
    sim.ready();
    assert!(!sim.bridge.user_detached);
    assert_eq!(sim.bridge.item, item);
    assert_eq!(sim.copies, 1);
}

#[test]
fn explicit_wear_resumes_the_paused_bridge_but_unrelated_attachments_do_not() {
    let mut sim = Simulator::new();
    sim.ready();
    assert!(
        !sim.bridge
            .observe_command(&NetCommand::DetachAttachments(vec![Uuid::new_v4()]), sim.now)
    );
    assert!(sim.bridge.ready());
    let request = AttachRequest {
        item_id: sim.bridge.item,
        owner_id: sim.world.agent_id,
        point: 31,
        add: true,
        flags: 0,
        group_mask: 0,
        everyone_mask: 0,
        next_owner_mask: 0,
        name: NAME.into(),
        desc: String::new(),
    };
    sim.bridge
        .observe_command(&NetCommand::DetachAttachments(vec![sim.bridge.item]), sim.now);
    assert!(sim.bridge.user_detached);
    sim.bridge.observe_command(&NetCommand::RezAttachments(vec![request]), sim.now);
    assert!(!sim.bridge.user_detached);
    assert_eq!(sim.bridge.phase, Phase::Inventory);
}

#[test]
fn an_early_hello_survives_the_upload_reply_and_boot_status_requires_the_exact_worn_bridge() {
    let mut sim = Simulator::new();
    sim.starting();
    let chat_count = sim.world.chat.len();
    let hello = sim.demo.handshake(&sim.bridge, &sim.world).expect("hello");
    let NetEvent::Chat(mut status) = hello.clone() else {
        panic!("chat")
    };
    status.message = format!("<auroraBridgeStatus>{VERSION}|{}|0|1</auroraBridgeStatus>", sim.bridge.folder);
    let source = status.source_id;
    status.source_id = Uuid::new_v4();
    sim.event(NetEvent::Chat(status.clone()));
    assert!(sim.bridge.boot.is_none());
    status.source_id = source;
    sim.event(NetEvent::Chat(status));
    assert_eq!(sim.bridge.boot, Some((false, true)));
    assert_eq!(sim.bridge.progress(), Some(aurora_i18n::tr!("bridge-waiting-permissions")));
    let request = Uuid::new_v4();
    sim.bridge.upload = Some(request);
    sim.bridge.enter(Phase::Upload, sim.now);
    sim.event(hello);
    sim.event(NetEvent::ScriptSaved {
        request,
        target: ScriptTarget {
            object: sim.bridge.object,
            item: sim.bridge.script,
        },
        result: Ok(aurora_net::scripts::ScriptSaved {
            asset: Uuid::new_v4(),
            compiled: true,
            errors: Vec::new(),
        }),
    });
    assert!(sim.bridge.candidate.is_some());
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert_eq!(sim.bridge.phase, Phase::Store);
    assert_eq!(sim.world.chat.len(), chat_count, "private diagnostics must never enter transcripts");
}

#[test]
fn bridge_factory_waits_for_copy_script_save_and_reattachment_without_duplicates() {
    let mut sim = Simulator::new();
    sim.ready();
    assert_eq!((sim.copies, sim.uploads, sim.attaches), (1, 1, 2));
    let folder = &sim.world.inventory.folders[&sim.bridge.folder];
    assert_eq!(folder.info.name, SUBFOLDER);
    let container = &sim.world.inventory.folders[&folder.info.parent];
    assert_eq!(container.info.name, FOLDER);
    assert_eq!(container.info.parent, sim.world.inventory.root);
    let item = &sim.world.inventory.items[&sim.bridge.item];
    assert_eq!(item.name, NAME);
    assert_eq!(item.owner, sim.world.agent_id);
    assert_eq!(
        sim.world
            .worn_attachment(&item.id)
            .and_then(|i| sim.world.objects.get(i))
            .expect("worn bridge")
            .attachment_point(),
        31
    );
    for _ in 0..120 {
        sim.step();
    }
    assert_eq!((sim.copies, sim.uploads, sim.attaches), (1, 1, 2));
    sim.prefs.enabled = false;
    sim.step();
    assert!(!sim.bridge.locked());
    assert!(!sim.bridge.ack.enabled);
}

#[test]
fn cached_old_bridge_waits_for_late_inventory_services_then_renames_its_original_folder() {
    let mut sim = Simulator::new();
    sim.ready();
    let (folder, item, object) = (sim.bridge.folder, sim.bridge.item, sim.bridge.object);
    let counts = (sim.copies, sim.uploads, sim.attaches);
    let hello = sim.demo.handshake(&sim.bridge, &sim.world).expect("existing script handshake");
    sim.world.inventory.folders.get_mut(&folder).expect("folder").info.name = OLD_SUBFOLDER.into();
    sim.bridge = Bridge::default();
    sim.event(hello);
    crate::ao::demo::inventory_services(&mut sim.world, false);
    for _ in 0..360 {
        sim.step();
    }
    assert_eq!(
        sim.bridge.phase,
        Phase::Inventory,
        "waiting longer than TIMEOUT must not fail an unsent rename"
    );
    assert_eq!(sim.unready_edits, 0, "Session would reject an edit before its services arrive");
    assert!(sim.bridge.mutation.is_none());
    assert_eq!(sim.world.inventory.folders[&folder].info.name, OLD_SUBFOLDER);
    assert!(sim.world.worn_attachment(&item).is_some());
    assert_eq!((sim.copies, sim.uploads, sim.attaches), counts);

    crate::ao::demo::inventory_services(&mut sim.world, true);
    sim.ready();
    assert_eq!((sim.bridge.folder, sim.bridge.item, sim.bridge.object), (folder, item, object));
    assert_eq!(sim.world.inventory.folders[&folder].info.name, SUBFOLDER);
    assert_eq!((sim.copies, sim.uploads, sim.attaches), counts);
    assert_eq!(sim.unready_edits, 0);
}

#[test]
fn new_bridge_waits_for_both_services_arrival_crossing_and_cache_release_before_creating() {
    let mut sim = Simulator::new();
    let generation = sim.world.inventory.generation;
    let region = sim.world.main_region.expect("region");
    for stage in 0..6 {
        crate::ao::demo::inventory_services(&mut sim.world, true);
        sim.world.movement_complete = true;
        sim.world.teleporting = false;
        sim.world.inventory.release_requests();
        match stage {
            0 => crate::ao::demo::inventory_services(&mut sim.world, false),
            1 => {
                Arc::make_mut(&mut sim.world.regions.get_mut(&region).expect("region").caps).remove("FetchInventoryDescendents2");
            }
            2 => {
                Arc::make_mut(&mut sim.world.regions.get_mut(&region).expect("region").caps).remove("InventoryAPIv3");
            }
            3 => sim.world.movement_complete = false,
            4 => sim.world.teleporting = true,
            _ => sim.world.inventory.hold_requests(),
        }
        for _ in 0..260 {
            sim.step();
        }
        assert_eq!(sim.bridge.phase, Phase::Inventory);
        assert!(sim.bridge.mutation.is_none());
        assert_eq!((sim.copies, sim.uploads, sim.attaches, sim.unready_edits), (0, 0, 0, 0));
        assert_eq!(
            sim.world.inventory.generation, generation,
            "waiting must leave cached inventory untouched"
        );
    }
    sim.world.inventory.release_requests();
    sim.ready();
    assert_eq!((sim.copies, sim.uploads, sim.attaches, sim.unready_edits), (1, 1, 2, 0));
}

#[test]
fn an_inflight_inventory_edit_keeps_its_timeout_when_services_disappear() {
    let mut sim = Simulator::new();
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert!(sim.bridge.mutation.is_some());
    crate::ao::demo::inventory_services(&mut sim.world, false);
    sim.now += TIMEOUT + Duration::from_secs(1);
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert_eq!(
        sim.bridge.phase,
        Phase::Failed,
        "only unsent operations may wait without spending their timeout"
    );
}

#[test]
fn inventory_services_disappearing_do_not_stop_a_ready_bridge_or_prevent_unlock() {
    let mut sim = Simulator::new();
    sim.ready();
    let counts = (sim.copies, sim.uploads, sim.attaches);
    crate::ao::demo::inventory_services(&mut sim.world, false);
    sim.prefs.enabled = false;
    sim.step();
    assert!(sim.bridge.available());
    assert!(!sim.bridge.locked());
    assert_eq!((sim.copies, sim.uploads, sim.attaches), counts);
}

#[test]
fn old_bridge_folder_is_renamed_without_replacing_its_worn_controller_or_script() {
    let mut sim = Simulator::new();
    sim.ready();
    let (folder, item, object) = (sim.bridge.folder, sim.bridge.item, sim.bridge.object);
    let counts = (sim.copies, sim.uploads, sim.attaches);
    let contents = sim.world.inventory.folders[&folder].items.clone();
    let hello = sim.demo.handshake(&sim.bridge, &sim.world).expect("existing script handshake");
    sim.world.inventory.folders.get_mut(&folder).expect("folder").info.name = OLD_SUBFOLDER.into();
    sim.bridge = Bridge::default();
    sim.event(hello);
    sim.ready();
    assert_eq!((sim.bridge.folder, sim.bridge.item, sim.bridge.object), (folder, item, object));
    assert_eq!(
        (sim.copies, sim.uploads, sim.attaches),
        counts,
        "no copy, upload, detach or reattach for a rename"
    );
    assert_eq!(sim.world.inventory.folders[&folder].items, contents);
    assert_eq!(sim.world.inventory.folders[&folder].info.name, SUBFOLDER);
    assert!(sim.world.worn_attachment(&item).is_some());
    for _ in 0..40 {
        sim.step();
    }
    assert_eq!((sim.copies, sim.uploads, sim.attaches), counts);
}

#[test]
fn bridge_folder_rename_waits_for_acknowledgement_and_does_not_create_after_failure() {
    let mut sim = Simulator::new();
    sim.ready();
    let folder = sim.bridge.folder;
    let counts = (sim.copies, sim.uploads, sim.attaches);
    sim.world.inventory.folders.get_mut(&folder).expect("folder").info.name = OLD_SUBFOLDER.into();
    sim.bridge = Bridge::default();
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    let request = sim.bridge.mutation.expect("rename pending");
    let edits: Vec<_> = sim
        .bridge
        .commands
        .iter()
        .filter_map(|command| {
            if let NetCommand::EditInventory { change, .. } = command {
                Some(change)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(edits.len(), 1);
    assert!(
        matches!(edits[0].operations.as_slice(), [Operation::Patch { id, folder: true, body }]
        if *id == folder && body["name"].as_str() == SUBFOLDER)
    );
    sim.bridge.commands.clear();
    sim.now += Duration::from_secs(1);
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert!(sim.bridge.commands.is_empty(), "wait rather than replay or create another folder");
    sim.event(NetEvent::InventoryEdited {
        request,
        result: Err("Synthetic failure".into()),
    });
    for _ in 0..40 {
        sim.step();
    }
    assert_eq!(sim.bridge.phase, Phase::Failed);
    assert_eq!(sim.world.inventory.folders[&folder].info.name, OLD_SUBFOLDER);
    assert_eq!((sim.copies, sim.uploads, sim.attaches), counts);
}

#[test]
fn existing_new_bridge_folder_wins_and_leaves_an_old_named_sibling_untouched() {
    let mut sim = Simulator::new();
    sim.ready();
    let (folder, item) = (sim.bridge.folder, sim.bridge.item);
    let counts = (sim.copies, sim.uploads, sim.attaches);
    let old = Uuid::new_v4();
    sim.world.inventory.created_folders(vec![aurora_net::inventory::InvFolder {
        id: old,
        parent: sim.bridge.container,
        name: OLD_SUBFOLDER.into(),
        type_default: -1,
        ..Default::default()
    }]);
    let hello = sim.demo.handshake(&sim.bridge, &sim.world).expect("existing script handshake");
    sim.bridge = Bridge::default();
    sim.event(hello);
    sim.ready();
    assert_eq!((sim.bridge.folder, sim.bridge.item), (folder, item));
    assert_eq!(sim.world.inventory.folders[&old].info.name, OLD_SUBFOLDER);
    assert_eq!((sim.copies, sim.uploads, sim.attaches), counts);
}

#[test]
fn startup_provisions_the_bridge_without_an_ao_or_enabled_movelock() {
    let mut sim = Simulator::new();
    sim.prefs.enabled = false;
    for _ in 0..80 {
        sim.step();
        if sim.bridge.ready() {
            break;
        }
    }
    assert!(sim.bridge.ready(), "startup must not depend on an enabled feature");
    assert!(!sim.bridge.locked() && !sim.bridge.ack.enabled);
    assert_eq!((sim.copies, sim.uploads, sim.attaches), (1, 1, 2));
    assert!(crate::ao::inventory::child(&sim.world.inventory, sim.bridge.container, crate::ao::inventory::FOLDER).is_none());
    for _ in 0..30 {
        sim.step();
    }
    assert_eq!((sim.copies, sim.uploads, sim.attaches), (1, 1, 2));
}

#[test]
fn bridge_accepts_only_the_owners_exact_worn_object_and_matching_folder() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.bridge.enter(Phase::Handshake, sim.now);
    let NetEvent::Chat(valid) = sim.demo.handshake(&sim.bridge, &sim.world).expect("handshake") else {
        panic!("chat")
    };
    for (source, owner) in [(Uuid::new_v4(), sim.world.agent_id), (sim.bridge.object, Uuid::new_v4())] {
        let mut invalid = valid.clone();
        invalid.source_id = source;
        invalid.owner_id = owner;
        assert!(sim.bridge.event(&mut sim.world, &NetEvent::Chat(invalid), sim.now));
        assert!(!sim.bridge.accept_candidate(&sim.world, sim.now));
    }
    let mut other_folder = valid.clone();
    other_folder.message = other_folder
        .message
        .replace(&sim.bridge.folder.to_string(), &Uuid::new_v4().to_string());
    sim.event(NetEvent::Chat(other_folder));
    assert!(!sim.bridge.accept_candidate(&sim.world, sim.now));
    sim.event(NetEvent::Chat(valid));
    assert!(sim.bridge.accept_candidate(&sim.world, sim.now));
}
#[test]
fn production_receive_path_hides_protocol_before_chat_and_applies_regular_world_updates() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.bridge.enter(Phase::Handshake, sim.now);
    let event = sim.demo.handshake(&sim.bridge, &sim.world).expect("private handshake");
    let before = sim.world.chat.len();
    assert!(sim.bridge.receive(&mut sim.world, event, sim.now, sim.now).is_none());
    assert_eq!(sim.world.chat.len(), before, "endpoint and token must never enter transcripts");
    assert!(sim.bridge.accept_candidate(&sim.world, sim.now));
    let count = sim.world.inventory.items.len();
    sim.event(NetEvent::InventoryUpdated {
        folders: Vec::new(),
        items: vec![aurora_net::inventory::InvItem {
            id: Uuid::from_u128(0xAB_CD),
            parent: sim.bridge.folder,
            name: "Synthetic receipt".into(),
            ..Default::default()
        }],
    });
    assert_eq!(sim.world.inventory.items.len(), count + 1);
}

#[test]
fn a_rejected_simulator_url_is_reported_as_a_response_failure_instead_of_a_stopped_script() {
    let mut sim = Simulator::new();
    sim.ready();
    let (valid, _) = sim.bridge.accepted.clone().expect("accepted modern simulator hello");
    sim.bridge.enter(Phase::Handshake, sim.now);
    sim.bridge.boot = Some((true, true));
    assert_eq!(sim.bridge.progress(), Some(aurora_i18n::tr!("bridge-waiting-handshake")));
    let mut invalid = valid.clone();
    invalid.message = invalid.message.replace("simhost-example.agni.secondlife.io", "untrusted.example");
    let before = sim.world.chat.len();
    sim.event(NetEvent::Chat(invalid));
    assert_eq!(sim.bridge.rejection, Some(HandshakeError::Endpoint));
    assert_eq!(sim.bridge.progress(), Some(aurora_i18n::tr!("bridge-response-endpoint")));
    assert!(sim.bridge.timeout_message().contains(aurora_i18n::tr!("bridge-response-endpoint")));
    assert!(sim.bridge.candidate.is_none());
    assert_eq!(sim.world.chat.len(), before);
    sim.event(NetEvent::Chat(valid));
    assert!(sim.bridge.accept_candidate(&sim.world, sim.now));
    assert!(sim.bridge.rejection.is_none());
}

#[test]
fn connection_response_errors_distinguish_envelope_version_endpoint_folder_and_token() {
    let folder = Uuid::from_u128(10);
    let token = Uuid::from_u128(20);
    let valid = format!(
        "<auroraBridge>{VERSION}|{folder}|https://simhost-example.aditi.secondlife.io:12043/cap/11111111-1111-4111-8111-111111111111/|{token}</auroraBridge>"
    );
    assert!(handshake(&valid).is_ok());
    for (message, error) in [
        (valid.replace("</auroraBridge>", ""), HandshakeError::Format),
        (valid.replace(VERSION, "0.0"), HandshakeError::Version),
        (valid.replace("https://", "http://"), HandshakeError::Endpoint),
        (valid.replace(&folder.to_string(), "invalid"), HandshakeError::Folder),
        (valid.replace(&token.to_string(), "invalid"), HandshakeError::Token),
    ] {
        assert_eq!(handshake(&message).err(), Some(error));
    }
}
#[test]
fn demo_echoes_native_animation_start_stop_without_dropping_other_signals() {
    let mut sim = Simulator::new();
    sim.ready();
    let id = Uuid::from_u128(0xAA_51);
    let before = sim.world.animations.get(&sim.world.agent_id).map_or(0, Vec::len);
    for start in [true, false] {
        let events = sim
            .demo
            .reply(
                &sim.bridge,
                &sim.world,
                &NetCommand::AgentAnimation { anim: id, start },
                false,
                false,
                false,
            )
            .expect("animation echo");
        for event in events {
            sim.event(event);
        }
        let signals = &sim.world.animations[&sim.world.agent_id];
        assert_eq!(signals.len(), before + usize::from(start));
        assert_eq!(signals.iter().any(|s| s.id == id), start);
    }
}
#[test]
fn repairing_an_uncertain_mutation_refetches_before_reusing_the_existing_attachment() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.bridge.mutation = Some(Uuid::new_v4());
    sim.bridge.enter(Phase::Copy, sim.now);
    sim.bridge.fail("Synthetic lost acknowledgement".into());
    sim.bridge.retry(sim.now);
    assert!(sim.bridge.mutation.is_none());
    sim.ready();
    assert_eq!(sim.copies, 1, "a repair must not duplicate the existing attachment");
}

#[test]
fn late_acknowledgements_cannot_restore_a_previous_connection_or_requested_mode() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.bridge.pending = Some((Uuid::from_u128(1), 7, (false, true, Motion::default())));
    sim.event(NetEvent::BridgeReply {
        request: Uuid::from_u128(2),
        result: Ok(Ack {
            sequence: 6,
            enabled: true,
            locked: true,
            ..Default::default()
        }),
    });
    assert!(sim.bridge.pending.is_some());
    sim.event(NetEvent::BridgeReply {
        request: Uuid::from_u128(1),
        result: Ok(Ack {
            sequence: 7,
            enabled: false,
            ..Default::default()
        }),
    });
    assert!(!sim.bridge.locked());
    sim.bridge.retry(sim.now);
    sim.event(NetEvent::BridgeReply {
        request: Uuid::from_u128(1),
        result: Ok(Ack {
            sequence: 7,
            enabled: true,
            locked: true,
            ..Default::default()
        }),
    });
    assert!(!sim.bridge.locked());
}

#[test]
fn region_movement_and_sitting_never_change_the_requested_mode() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.demo.state(&mut sim.bridge, true, false, &sim.prefs);
    assert!(sim.bridge.ack.enabled && !sim.bridge.locked());
    sim.demo.state(&mut sim.bridge, false, false, &sim.prefs);
    assert!(sim.bridge.locked());
    sim.demo.state(&mut sim.bridge, false, true, &sim.prefs);
    assert!(!sim.bridge.locked() && sim.bridge.ack.seated);
    sim.demo.state(&mut sim.bridge, false, false, &sim.prefs);
    assert!(sim.bridge.locked());
    sim.world.main_region = Some(sim.world.main_region.expect("region") + 65536);
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert!(sim.prefs.enabled && !sim.bridge.locked() && sim.bridge.endpoint.is_none());
}

#[test]
fn selection_stays_enabled_during_motion_drift_background_requests_and_network_errors() {
    let mut sim = Simulator::new();
    sim.ready();
    for (moving, seated) in [(false, false), (true, false), (false, true)] {
        sim.demo.state(&mut sim.bridge, moving, seated, &sim.prefs);
        sim.bridge.ack.locked = false; // A push or small drift is a physical detail.
        sim.bridge.pending = Some((Uuid::new_v4(), sim.bridge.sequence, (true, true, Motion::default())));
        assert_eq!(sim.prefs.state(), aurora_i18n::tr!("movelock-on"));
        assert!(
            sim.bridge.progress().is_none(),
            "background acknowledgements must not flash Updating"
        );
    }
    sim.bridge.error = Some(aurora_i18n::tr!("bridge-request-failed").into());
    assert_eq!(sim.prefs.state(), aurora_i18n::tr!("movelock-on"));
    assert!(
        sim.bridge.progress().is_some(),
        "connection diagnostics remain available separately"
    );
    sim.prefs.enabled = false;
    assert_eq!(sim.prefs.state(), aurora_i18n::tr!("movelock-off"));
}

#[test]
fn idle_movelock_needs_no_keepalive_but_explicit_disable_is_sent_immediately() {
    let mut sim = Simulator::new();
    sim.ready();
    let sequence = sim.bridge.sequence;
    sim.now += Duration::from_secs(3600);
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert!(sim.bridge.commands.is_empty(), "the LSL lock runs autonomously after activation");
    assert_eq!(sim.bridge.sequence, sequence);
    sim.prefs.enabled = false;
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert!(
        sim.bridge
            .commands
            .iter()
            .any(|cmd| matches!(cmd, NetCommand::Bridge { command, .. } if !command.enabled))
    );
}

#[test]
fn swim_control_forwarding_requires_a_recent_confirmed_motor_and_ignores_chat_refreshes() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.bridge.motion = Motion::motor(3, 2.0, 1.7);
    assert!(!sim.bridge.captures_motion(sim.now));
    sim.step();
    assert!(sim.bridge.captures_motion(sim.now));
    let sent_at = sim.bridge.capture_sent.expect("confirmed send time");
    let (mut chat, _) = sim.bridge.accepted.clone().expect("bridge source");
    sim.now = sent_at + Duration::from_millis(1100);
    chat.message = format!("<auroraMoveLock>{}|1|0|1|0</auroraMoveLock>", sim.bridge.sequence);
    sim.event(NetEvent::Chat(chat));
    assert!(
        !sim.bridge.captures_motion(sim.now),
        "a control-status report does not renew the motor watchdog"
    );
    sim.step();
    assert!(sim.bridge.captures_motion(sim.now));
    sim.bridge.motion.mode = 4;
    assert!(
        !sim.bridge.captures_motion(sim.now),
        "the short exit climb is not a persistent capture"
    );
}

#[test]
fn a_delayed_motor_acknowledgement_cannot_extend_the_script_capture_watchdog() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.bridge.motion = Motion::motor(3, 2.0, 1.7);
    sim.bridge.send_mode(&sim.prefs, sim.now);
    let (request, sequence, _) = sim.bridge.pending.expect("motion request");
    sim.now += Duration::from_secs(2);
    sim.event(NetEvent::BridgeReply {
        request,
        result: Ok(Ack {
            sequence,
            enabled: true,
            ..Default::default()
        }),
    });
    assert!(!sim.bridge.captures_motion(sim.now));
    assert_eq!(sim.prefs.state(), aurora_i18n::tr!("movelock-on"));
}

#[test]
fn provisioning_times_out_and_retry_keeps_existing_inventory_objects() {
    let mut sim = Simulator::new();
    sim.ready();
    let item = sim.bridge.item;
    sim.bridge.enter(Phase::Upload, sim.now);
    sim.now += TIMEOUT + Duration::from_secs(1);
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert_eq!(sim.bridge.phase, Phase::Failed);
    sim.bridge.retry(sim.now);
    sim.ready();
    assert_eq!(sim.bridge.item, item);
    assert_eq!(sim.copies, 1);
}

#[test]
fn motion_release_supersedes_a_delayed_request_without_waiting_for_its_ack() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.prefs.enabled = false;
    sim.bridge.sent = Some((false, true, Motion::motor(3, 2.0, 1.7)));
    sim.bridge.last_send = Some(sim.now);
    let old = Uuid::new_v4();
    let old_sequence = sim.bridge.sequence;
    sim.bridge.pending = Some((old, sim.bridge.sequence, sim.bridge.sent.expect("mode")));
    sim.bridge.motion = Motion::default();
    sim.bridge.send_mode(&sim.prefs, sim.now + Duration::from_millis(10));
    let command = sim.bridge.commands.last().expect("immediate release");
    let NetCommand::Bridge { request, command } = command else {
        panic!("bridge command")
    };
    assert_ne!(*request, old);
    assert_eq!(command.motion, Motion::default());
    assert!(command.sequence > old_sequence);
}

#[test]
fn existing_root_bridge_is_moved_into_the_subfolder_and_reused() {
    let mut sim = Simulator::new();
    sim.ready();
    let item = sim.bridge.item;
    let container = sim.bridge.container;
    let folder = sim.bridge.folder;
    sim.world.inventory.items.get_mut(&item).expect("bridge").parent = container;
    sim.world
        .inventory
        .folders
        .get_mut(&folder)
        .expect("folder")
        .items
        .retain(|id| *id != item);
    sim.world.inventory.folders.get_mut(&container).expect("container").items.push(item);
    sim.world.inventory.generation += 1;
    sim.bridge.retry(sim.now);
    sim.ready();
    assert_eq!(sim.bridge.item, item);
    assert_eq!(sim.copies, 1, "moving a worn bridge never creates a second controller");
    assert_eq!(sim.world.inventory.items[&item].parent, folder);
}

#[test]
fn account_preferences_roundtrip_and_old_settings_default_to_unlocked() {
    let prefs = Preferences {
        enabled: true,
        ..Default::default()
    };
    let saved = serde_json::to_string(&prefs).expect("save");
    assert_eq!(serde_json::from_str::<Preferences>(&saved).expect("load"), prefs);
    let old: crate::settings::Settings = serde_json::from_str("{}").expect("old settings");
    assert!(old.bridge.is_empty());
    assert!(!Preferences::default().enabled);
}

#[test]
fn release_supersedes_in_flight_lock_commands_and_does_not_change_saved_preferences() {
    let mut sim = Simulator::new();
    sim.ready();
    let previous_sequence = sim.bridge.sequence;
    let command = sim.bridge.release().expect("release command");
    let NetCommand::Bridge { command, .. } = command else {
        panic!("bridge command")
    };
    assert!(!command.enabled && command.sequence > previous_sequence);
    assert!(sim.prefs.enabled && !sim.bridge.locked());
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert!(sim.bridge.commands.is_empty(), "a closing viewer must not renew the lock");
}

#[test]
fn region_handoff_preserves_an_early_handshake_and_monotonic_command_sequence() {
    let mut sim = Simulator::new();
    sim.ready();
    let (mut handshake, _) = sim.bridge.accepted.clone().expect("accepted handshake");
    let token = Uuid::from_u128(99);
    handshake.message = handshake.message.replace(&Uuid::from_u128(42).to_string(), &token.to_string());
    sim.event(NetEvent::Chat(handshake));
    sim.step();
    let sequence = sim.bridge.sequence;
    let new_region = sim.world.main_region.expect("region") + 65536;
    let attachment = crate::demo::outfit_attachment(&AttachRequest {
        item_id: sim.bridge.item,
        owner_id: sim.world.agent_id,
        point: 31,
        add: true,
        flags: 0,
        group_mask: 0,
        everyone_mask: 0,
        next_owner_mask: 0,
        name: NAME.into(),
        desc: String::new(),
    });
    sim.world.apply(NetEvent::ObjectUpdates {
        handle: new_region,
        objects: vec![crate::demo::action_avatar(false), attachment],
    });
    sim.world.main_region = Some(new_region);
    sim.step();
    assert!(sim.prefs.enabled && sim.bridge.locked());
    assert!(sim.bridge.ready() && sim.bridge.sequence > sequence);
    assert_eq!(sim.bridge.endpoint.as_ref().expect("endpoint").token, token);
    assert_eq!((sim.copies, sim.uploads), (1, 1), "crossing must not recreate an existing bridge");
}

#[test]
fn worn_firestorm_bridge_is_only_detached_once_without_any_inventory_or_script_edit() {
    use aurora_net::inventory::{InvFolder, InvItem};
    let mut sim = Simulator::new();
    let fs = Uuid::from_u128(0xF500);
    let folder = Uuid::from_u128(0xF501);
    let item = Uuid::from_u128(0xF502);
    sim.world.apply(NetEvent::InventoryUpdated {
        folders: vec![
            InvFolder {
                id: fs,
                parent: sim.world.inventory.root,
                name: "#Firestorm".into(),
                ..Default::default()
            },
            InvFolder {
                id: folder,
                parent: fs,
                name: "#LSL Bridge".into(),
                ..Default::default()
            },
        ],
        items: vec![InvItem {
            id: item,
            parent: folder,
            owner: sim.world.agent_id,
            name: "#Firestorm LSL Bridge v2.29".into(),
            asset_type: 6,
            inv_type: 6,
            ..Default::default()
        }],
    });
    let request = AttachRequest {
        item_id: item,
        owner_id: sim.world.agent_id,
        point: 31,
        add: true,
        flags: 0,
        group_mask: 0,
        everyone_mask: 0,
        next_owner_mask: 0,
        name: String::new(),
        desc: String::new(),
    };
    let attachment = crate::demo::outfit_attachment(&request);
    let handle = sim.world.main_region.expect("region");
    sim.world.apply(NetEvent::ObjectUpdates {
        handle,
        objects: vec![attachment.clone()],
    });
    let before = format!(
        "{:?}|{:?}|{:?}",
        sim.world.inventory.folders[&fs], sim.world.inventory.folders[&folder], sim.world.inventory.items[&item]
    );
    sim.bridge.agent = sim.world.agent_id;
    sim.bridge.user_detached = true;
    sim.bridge.phase = Phase::Failed;
    let valid = sim.world.inventory.items[&item].clone();
    for case in 0..6 {
        let mut other = valid.clone();
        match case {
            0 => other.name = "Firestorm HUD".into(),
            1 => other.name.push_str(" extra"),
            2 => other.name = "#Firestorm LSL Bridge v2.".into(),
            3 => other.owner = Uuid::new_v4(),
            4 => other.asset_type = 24,
            _ => other.parent = sim.world.inventory.root,
        }
        sim.world.inventory.items.insert(item, other);
        sim.now += Duration::from_secs(1);
        sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
        assert!(
            sim.bridge.commands.is_empty(),
            "an unrelated or unresolved attachment must remain worn"
        );
    }
    sim.world.inventory.items.insert(item, valid);
    sim.now += Duration::from_secs(1);
    sim.bridge.update(&mut sim.world, &sim.prefs, false, sim.now);
    assert!(sim.bridge.commands.is_empty());
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert!(matches!(sim.bridge.commands.as_slice(), [NetCommand::DetachAttachments(items)] if *items == [item]));
    let command = sim.bridge.commands.pop().expect("detach");
    assert!(
        !sim.bridge.observe_command(&command, sim.now),
        "legacy detach cannot disable Aurora Movelock"
    );
    for _ in 0..20 {
        sim.now += Duration::from_secs(1);
        sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
        assert!(
            sim.bridge.commands.is_empty(),
            "no repeated detach while the reliable command is in flight"
        );
    }
    assert_eq!(
        format!(
            "{:?}|{:?}|{:?}",
            sim.world.inventory.folders[&fs], sim.world.inventory.folders[&folder], sim.world.inventory.items[&item]
        ),
        before
    );
    sim.world.apply(NetEvent::ObjectsKilled {
        handle,
        local_ids: vec![attachment.local_id],
    });
    let mut reattached = attachment;
    reattached.full_id = Uuid::new_v4();
    sim.world.apply(NetEvent::ObjectUpdates {
        handle,
        objects: vec![reattached],
    });
    sim.now += Duration::from_secs(1);
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert!(matches!(sim.bridge.commands.as_slice(), [NetCommand::DetachAttachments(items)] if *items == [item]));
}

#[test]
fn ordinary_object_updates_keep_the_network_receive_time_through_the_bridge() {
    let mut sim = Simulator::new();
    let (slot, object) = sim.world.objects.iter().find(|(_, o)| !o.is_avatar()).expect("demo object");
    let (key, position, rotation) = (object.key, object.position, object.rotation);
    let received = sim.now - Duration::from_millis(200);
    sim.world.objects.advance_motion(key.region, 10.0);
    sim.world.objects.note_motion_tick(sim.now, key.region, 1.0);
    let expected = sim.world.objects.report_clock(key.region, Some(received));
    sim.bridge.receive(
        &mut sim.world,
        NetEvent::TerseUpdates {
            handle: key.region,
            updates: vec![aurora_net::objects::TerseUpdate {
                local_id: key.local_id,
                state: 0,
                is_avatar: false,
                foot_plane: None,
                position,
                velocity: glam::Vec3::X,
                acceleration: glam::Vec3::ZERO,
                rotation,
                angular_velocity: glam::Vec3::ZERO,
                texture_entry: None,
            }],
        },
        sim.now,
        received,
    );
    let object = sim.world.objects.get(slot).expect("updated object");
    let clock = sim.world.objects.motion_time(key.region);
    let (predicted, _) = object.predicted(sim.now + Duration::from_secs(5), clock);
    assert!((predicted.x - position.x - (clock - expected) as f32).abs() < 1e-4);
    assert!(predicted.x > position.x + 0.19, "the buffered 200 ms must be retained");
}
