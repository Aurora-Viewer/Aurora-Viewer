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
    query_uploads: usize,
    ik_uploads: usize,
    ik_discoveries: usize,
    attaches: usize,
    detaches: usize,
    hello_versions: [Option<&'static str>; 3],
    unready_edits: usize,
    script_rez_groups: Vec<(String, Uuid)>,
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
            query_uploads: 0,
            ik_uploads: 0,
            ik_discoveries: 0,
            attaches: 0,
            detaches: 0,
            hello_versions: [None; 3],
            unready_edits: 0,
            script_rez_groups: Vec::new(),
        }
    }
    fn event(&mut self, mut event: NetEvent) {
        if let NetEvent::Chat(chat) = &mut event {
            for (index, tag) in ["<auroraBridge>", "<auroraBridgeQuery>", "<auroraBridgeIK>"].iter().enumerate() {
                if chat.message.starts_with(tag)
                    && let Some(version) = self.hello_versions[index]
                {
                    chat.message = chat.message.replacen(&format!(">{VERSION}|"), &format!(">{version}|"), 1);
                }
            }
        }
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
                        if let NetCommand::Build(BuildCmd::RezScript { group_id, item, .. }) = &command {
                            assert_eq!(*group_id, self.world.groups.active);
                            self.script_rez_groups.push((item.name.clone(), *group_id));
                        }
                        if let NetCommand::DetachAttachments(items) = &command {
                            self.detaches += usize::from(items.contains(&self.bridge.item));
                        }
                        if let NetCommand::ScriptDialogReply {
                            channel: ik::CONTROL,
                            label,
                            ..
                        } = &command
                            && label.ends_with("|discover")
                        {
                            self.ik_discoveries += 1;
                        }
                        if let NetCommand::SaveScript { target, .. } = &command {
                            if target.item == self.bridge.query_script {
                                self.query_uploads += 1;
                                self.hello_versions[1] = None;
                            } else if target.item == self.bridge.ik.script {
                                self.ik_uploads += 1;
                                self.hello_versions[2] = None;
                            } else {
                                self.uploads += 1;
                                self.hello_versions[0] = None;
                            }
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
        sender: None,
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
        3,
        "the controller, query and IK scripts answer the same probe"
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
        matches!(&commands[0], NetCommand::ScriptDialogReply { sender: None, object_id, channel, index, label }
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
fn login_reuses_an_up_to_date_worn_bridge_without_uploading_or_saving_it() {
    let mut sim = Simulator::new();
    sim.ready();
    let before = (
        sim.uploads,
        sim.query_uploads,
        sim.ik_uploads,
        sim.attaches,
        sim.detaches,
        sim.copies,
    );
    let item = sim.bridge.item;
    sim.bridge = Bridge::default();
    sim.world.hand_contacts.relay = None;
    sim.ready();
    assert_eq!(sim.bridge.item, item);
    assert_eq!(
        (
            sim.uploads,
            sim.query_uploads,
            sim.ik_uploads,
            sim.attaches,
            sim.detaches,
            sim.copies
        ),
        before,
        "a healthy existing bridge must not be rewritten, detached or reattached"
    );
    assert!(sim.bridge.query_endpoint().is_some());
    assert!(sim.world.hand_contacts.relay.is_some());
    assert!(!sim.bridge.save_pending);
}

#[test]
fn an_old_version_in_any_bridge_service_is_updated_then_saved_once() {
    for service in 0..3 {
        let mut sim = Simulator::new();
        sim.ready();
        let uploads = (sim.uploads, sim.query_uploads, sim.ik_uploads);
        let detaches = sim.detaches;
        let attaches = sim.attaches;
        sim.hello_versions[service] = Some("1.39");
        sim.bridge = Bridge::default();
        sim.world.hand_contacts.relay = None;
        for _ in 0..200 {
            sim.step();
            if sim.bridge.locked() {
                break;
            }
        }
        assert!(sim.bridge.locked(), "service {service} must recover after its update");
        assert_eq!(
            (sim.uploads, sim.query_uploads, sim.ik_uploads),
            (uploads.0 + 1, uploads.1 + 1, uploads.2 + 1)
        );
        assert_eq!(sim.detaches, detaches + 1, "save updated scripts exactly once");
        assert_eq!(sim.attaches, attaches + 1);
        assert!(!sim.bridge.save_pending);
    }
}

#[test]
fn failed_compilation_does_not_request_a_save_of_unchanged_scripts() {
    let mut sim = Simulator::new();
    sim.bridge.object = Uuid::new_v4();
    let request = Uuid::new_v4();
    sim.bridge.upload = Some(request);
    sim.bridge.enter(Phase::Upload, sim.now);
    sim.event(NetEvent::ScriptSaved {
        request,
        target: ScriptTarget {
            object: sim.bridge.object,
            item: Uuid::new_v4(),
        },
        result: Ok(aurora_net::scripts::ScriptSaved {
            asset: Uuid::nil(),
            compiled: false,
            errors: vec!["Synthetic compilation error".into()],
        }),
    });
    assert!(!sim.bridge.save_pending);
    assert_eq!(sim.bridge.phase, Phase::Failed);
    assert!(
        !sim.bridge
            .commands
            .iter()
            .any(|command| matches!(command, NetCommand::DetachAttachments(_)))
    );
}

#[test]
fn an_interrupted_update_is_saved_even_when_login_verification_can_reuse_it() {
    let mut sim = Simulator::new();
    sim.ready();
    let uploads = sim.uploads;
    let detaches = sim.detaches;
    // The running scripts are current, but this session wrote them before
    // losing the circuit. Verification cannot discard the pending save.
    sim.bridge = Bridge {
        agent: sim.world.agent_id,
        region: sim.world.main_region,
        since: Some(sim.now),
        save_pending: true,
        ..Default::default()
    };
    sim.world.hand_contacts.relay = None;
    sim.ready();
    assert_eq!(sim.uploads, uploads, "do not rewrite scripts already verified current");
    assert_eq!(sim.detaches, detaches + 1, "retain the outstanding save");
    assert!(!sim.bridge.save_pending);
}

#[test]
fn ready_bridge_retries_discovery_until_the_delayed_ik_script_answers() {
    let mut sim = Simulator::new();
    sim.ready();
    let copies = sim.copies;
    let uploads = sim.uploads;
    let relay = sim.world.hand_contacts.relay.take().expect("initial IK relay");
    sim.bridge.commands.clear();
    sim.bridge.probe = Some((sim.bridge.object, sim.now));
    sim.now += DISCOVERY_INTERVAL;
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    let commands = std::mem::take(&mut sim.bridge.commands);
    let probe = commands
        .iter()
        .find(|command| {
            matches!(
                command,
                NetCommand::ScriptDialogReply {
                    channel: DISCOVERY_CHANNEL,
                    ..
                }
            )
        })
        .expect("a ready controller still discovers a late IK relay");
    let replies = sim
        .demo
        .reply(&sim.bridge, &sim.world, probe, false, false, false)
        .expect("probe response");
    for reply in replies {
        sim.event(reply);
    }
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    let restored = sim.world.hand_contacts.relay.as_ref().expect("IK relay rediscovered");
    assert_eq!(restored.root, relay.root);
    assert_eq!(restored.region, relay.region);
    assert_eq!((sim.copies, sim.uploads), (copies, uploads));
    sim.bridge.commands.clear();
    sim.now += DISCOVERY_INTERVAL;
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert!(
        !sim.bridge.commands.iter().any(|command| matches!(
            command,
            NetCommand::ScriptDialogReply {
                channel: DISCOVERY_CHANNEL,
                ..
            }
        )),
        "discovery stops once all three services are present"
    );
}

#[test]
fn login_restores_the_bridge_detached_by_its_script_on_another_viewer() {
    let mut sim = Simulator::new();
    sim.ready();
    let item = sim.bridge.item;
    let before = (sim.uploads, sim.query_uploads, sim.ik_uploads, sim.attaches, sim.detaches);
    let command = NetCommand::DetachAttachments(vec![item]);
    // The script detaches on another viewer: Aurora never observes a user
    // command that would deliberately pause the bridge for this new session.
    for event in sim
        .demo
        .reply(&sim.bridge, &sim.world, &command, false, false, false)
        .expect("script detach")
    {
        sim.event(event);
    }
    sim.bridge = Bridge::default();
    sim.world.hand_contacts.relay = None;
    sim.ready();
    assert_eq!(sim.bridge.item, item);
    assert_eq!(sim.copies, 1, "reuse the existing inventory object at login");
    assert!(!sim.bridge.user_detached);
    assert!(sim.world.hand_contacts.relay.is_some());
    assert_eq!((sim.uploads, sim.query_uploads, sim.ik_uploads), (before.0, before.1, before.2));
    assert_eq!(sim.attaches, before.3 + 1, "only attach the object missing from the new login");
    assert_eq!(sim.detaches, before.4, "an existing current bridge needs no save detachment");
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
    let timeout = aurora_i18n::tr!("bridge-request-timeout");
    sim.bridge.query_reply(&sim.world, Some(timeout), sim.now);
    sim.bridge.query_reply(&sim.world, Some(timeout), sim.now + Duration::from_secs(1));
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

/// Run the simulator for `seconds`, with the ready bridge's requests failing
/// as the demo's halted script makes them fail.
fn silent_for(sim: &mut Simulator, seconds: u64) {
    for _ in 0..seconds * 4 {
        sim.step();
    }
}

#[test]
fn a_halted_script_is_detected_and_reinstalled_without_disabling_movelock() {
    let mut sim = Simulator::new();
    sim.ready();
    let uploads = sim.uploads;
    sim.demo.halt = true;
    sim.bridge.motion = Motion::motor(1, 2.0, 1.7);
    // Timeouts alone must not rebuild the bridge before the simulator says
    // the script stopped: a slow region is not a run-time error.
    silent_for(&mut sim, 8);
    assert_eq!(sim.uploads, uploads);
    assert!(sim.bridge.watch.silent_since.is_some());
    silent_for(&mut sim, 4);
    assert_eq!(sim.bridge.reinstalls.len(), 1, "a stopped script is found within the check delay");
    assert!(sim.prefs.enabled);
    sim.bridge.motion = Motion::default();
    sim.ready();
    assert_eq!(sim.uploads, uploads + 1, "the halted script is recompiled");
    assert!(sim.bridge.available() && sim.bridge.watch.silent_since.is_none());
    assert!(
        sim.world
            .chat
            .iter()
            .any(|line| line.text == aurora_i18n::tr!("bridge-reinstalling"))
    );
}

#[test]
fn http_replies_prove_the_script_alive_and_a_silent_running_script_is_reinstalled_late() {
    let mut sim = Simulator::new();
    sim.ready();
    let uploads = sim.uploads;
    let timeout = aurora_i18n::tr!("bridge-request-timeout");
    sim.bridge.query_reply(&sim.world, Some(timeout), sim.now);
    assert!(sim.bridge.query_watch.silent_since.is_some());
    // A 404 for a vanished object comes from the script itself.
    sim.bridge
        .query_reply(&sim.world, Some(aurora_i18n::tr!("bridge-script-info-missing")), sim.now);
    assert!(sim.bridge.query_watch.silent_since.is_none());
    sim.bridge.query_reply(&sim.world, Some(timeout), sim.now);
    sim.bridge.query_reply(&sim.world, None, sim.now);
    assert!(sim.bridge.query_watch.silent_since.is_none());
    // The script still reports running but nothing answers for 90 s.
    sim.bridge.watch.silent_since = Some(sim.now);
    sim.now += SILENCE_LIMIT;
    sim.bridge.commands.clear();
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert_eq!(sim.bridge.reinstalls.len(), 1);
    assert_eq!(sim.bridge.phase, Phase::Inventory);
    sim.ready();
    assert_eq!(sim.uploads, uploads + 1);
}

#[test]
fn queries_use_their_own_script_endpoint_installed_beside_the_controller() {
    let mut sim = Simulator::new();
    sim.ready();
    assert_eq!((sim.uploads, sim.query_uploads), (1, 1));
    assert!(!sim.bridge.query_script.is_nil() && sim.bridge.query_script != sim.bridge.script);
    // The query hello arrives with the controller's, from the same probe.
    for _ in 0..4 {
        sim.step();
    }
    let query = sim.bridge.query_endpoint().expect("query endpoint");
    let controller = sim.bridge.endpoint.clone().expect("controller endpoint");
    assert_ne!(query.token, controller.token);
    assert_ne!(query.url, controller.url);
    // A query hello from any other object is never accepted.
    let NetEvent::Chat(mut stranger) = sim.demo.query_handshake(&sim.bridge, &sim.world).expect("hello") else {
        panic!("query hello is chat");
    };
    stranger.source_id = Uuid::new_v4();
    stranger.message = stranger
        .message
        .replace(&Uuid::from_u128(43).to_string(), &Uuid::from_u128(44).to_string());
    sim.event(NetEvent::Chat(stranger));
    sim.step();
    assert_eq!(sim.bridge.query_endpoint().map(|e| e.token), Some(query.token));
}

#[test]
fn portal_relay_accepts_only_the_exact_ready_worn_bridge_and_never_enters_chat() {
    use aurora_net::bridge::portal::{Access, LIFETIME, Message, Portal};
    let mut sim = Simulator::new();
    sim.ready();
    let NetEvent::Chat(mut chat) = sim.demo.query_handshake(&sim.bridge, &sim.world).expect("query hello") else {
        panic!("chat");
    };
    let owner = Uuid::from_u128(0xA770);
    let portal = Portal {
        id: Uuid::from_u128(0xA771),
        region: sim.world.main_region.unwrap(),
        center: [10.0, 10.0, 25.0],
        yaw: 0.0,
        destination: "Demo destination".into(),
        position: [128.0, 128.0, 25.0],
        access: Access::Everyone,
        expires: super::super::portals::unix_time() + LIFETIME,
    };
    chat.message = format!(
        "<auroraPortal>{owner}|{}</auroraPortal>",
        Message::Open(portal.announcement()).encode().unwrap()
    );
    for (source, claimed_owner) in [(Uuid::new_v4(), sim.world.agent_id), (sim.bridge.object, Uuid::new_v4())] {
        let mut invalid = chat.clone();
        invalid.source_id = source;
        invalid.owner_id = claimed_owner;
        assert!(sim.bridge.event(&mut sim.world, &NetEvent::Chat(invalid), sim.now));
        assert!(sim.world.portals.entries.is_empty());
    }
    let before = sim.world.chat.len();
    assert!(sim.bridge.event(&mut sim.world, &NetEvent::Chat(chat.clone()), sim.now));
    assert_eq!(sim.world.portals.entries.len(), 1);
    assert_eq!(sim.world.chat.len(), before);
    sim.world.portals.entries.clear();
    sim.world.main_region = Some(123);
    assert!(sim.bridge.event(&mut sim.world, &NetEvent::Chat(chat), sim.now));
    assert!(sim.world.portals.entries.is_empty());
}

#[test]
fn an_older_bridge_gets_only_the_missing_query_script() {
    let mut sim = Simulator::new();
    sim.ready();
    let controller = sim.bridge.script;
    // A bridge made before the query service: only its controller remains.
    sim.demo.forget_script(QUERY_NAME);
    sim.bridge.retry(sim.now);
    sim.ready();
    assert_eq!(sim.bridge.script, controller, "the controller item is reused");
    assert!(!sim.bridge.query_script.is_nil());
    assert_eq!((sim.uploads, sim.query_uploads), (2, 2));
}

#[test]
fn ik_relay_is_a_third_script_discovers_once_and_upgrades_an_existing_attachment() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.step();
    assert_eq!((sim.uploads, sim.query_uploads, sim.ik_uploads), (1, 1, 1));
    assert_eq!(
        sim.ik_discoveries, 1,
        "saving the attachment cannot start premature regional discovery"
    );
    let relay = sim.world.hand_contacts.relay.clone().expect("authenticated relay");
    assert_eq!(relay.root, sim.bridge.object);
    assert_ne!(sim.bridge.ik.script, sim.bridge.script);
    assert_ne!(sim.bridge.ik.script, sim.bridge.query_script);
    for _ in 0..240 {
        sim.step();
    }
    assert_eq!(sim.ik_discoveries, 1, "no periodic regional queries");
    assert!(
        sim.world.chat.iter().all(|chat| !chat.text.contains("auroraBridge")),
        "bridge handshakes never enter chat"
    );
    let controller = sim.bridge.script;
    let query = sim.bridge.query_script;
    let attachment = sim.bridge.item;
    sim.demo.forget_script(ik::NAME);
    sim.bridge.retry(sim.now);
    sim.ready();
    sim.step();
    assert_eq!(
        (sim.bridge.script, sim.bridge.query_script, sim.bridge.item),
        (controller, query, attachment)
    );
    assert_eq!(sim.ik_uploads, 2);
    assert_eq!(sim.ik_discoveries, 2);
}

#[test]
fn a_stopped_query_script_is_reinstalled_but_never_costs_movelock_its_controller() {
    let mut sim = Simulator::new();
    sim.ready();
    for _ in 0..4 {
        sim.step();
    }
    assert!(sim.bridge.query_endpoint().is_some());
    let timeout = aurora_i18n::tr!("bridge-request-timeout");
    sim.bridge.query_reply(&sim.world, Some(timeout), sim.now);
    let stopped = NetEvent::ScriptRunning {
        object_id: sim.bridge.object,
        item_id: sim.bridge.query_script,
        running: false,
        mono: Some(true),
    };
    sim.event(stopped.clone());
    assert_eq!(sim.bridge.reinstalls.len(), 1);
    assert_eq!(sim.bridge.phase, Phase::Inventory);
    sim.ready();
    // At the limit, a stopped query script leaves the controller working.
    for _ in 1..REINSTALL_LIMIT {
        sim.bridge.reinstalls.push(sim.now);
    }
    sim.bridge.query_reply(&sim.world, Some(timeout), sim.now);
    let stopped = NetEvent::ScriptRunning {
        object_id: sim.bridge.object,
        item_id: sim.bridge.query_script,
        running: false,
        mono: Some(true),
    };
    sim.event(stopped);
    assert_eq!(sim.bridge.phase, Phase::Ready);
    assert!(sim.bridge.error.is_none() && sim.bridge.locked());
    // A silent but running query script is only probed, never reinstalled.
    sim.bridge.query_watch.silent_since = Some(sim.now);
    sim.now += SILENCE_LIMIT;
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert_eq!(sim.bridge.phase, Phase::Ready);
}

#[test]
fn query_state_tells_a_silent_query_script_from_a_missing_or_starting_bridge() {
    let mut sim = Simulator::new();
    assert_eq!(sim.bridge.query_state(sim.now), QueryState::Starting);
    sim.ready();
    for _ in 0..4 {
        sim.step();
    }
    assert_eq!(sim.bridge.query_state(sim.now), QueryState::Ready);
    // The controller answers, the query script never does (bridge 1.34's
    // query.lsl did not compile): unanswered probes, nothing processed.
    sim.bridge.query = None;
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert_eq!(
        sim.bridge.query_state(sim.now),
        QueryState::Starting,
        "a hello can still be on its way"
    );
    sim.now += SILENCE_CHECK;
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert!(sim.bridge.available());
    assert_eq!(sim.bridge.query_state(sim.now), QueryState::Silent);
    sim.bridge.error = Some(aurora_i18n::tr!("bridge-stopped").into());
    assert_eq!(sim.bridge.query_state(sim.now), QueryState::Failed);
    sim.bridge.user_detached = true;
    assert_eq!(sim.bridge.query_state(sim.now), QueryState::Detached);
    // Repair (retry) starts again from the inventory.
    sim.bridge.retry(sim.now);
    assert_eq!(sim.bridge.query_state(sim.now), QueryState::Starting);
}

#[test]
fn automatic_reinstalls_are_bounded_and_leave_repair_to_the_user() {
    let mut sim = Simulator::new();
    sim.ready();
    for _ in 0..REINSTALL_LIMIT {
        sim.bridge.reinstalls.push(sim.now);
    }
    sim.bridge.watch.silent_since = Some(sim.now);
    sim.now += SILENCE_LIMIT;
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert_eq!(sim.bridge.phase, Phase::Failed);
    assert_eq!(sim.bridge.error.as_deref(), Some(aurora_i18n::tr!("bridge-stopped")));
    assert!(sim.prefs.enabled);
    // Older reinstalls leave the window: a later stop is repaired again.
    sim.bridge.retry(sim.now);
    sim.ready();
    sim.bridge.watch.silent_since = Some(sim.now);
    sim.now += REINSTALL_WINDOW;
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert_eq!(sim.bridge.phase, Phase::Inventory);
    assert_eq!(sim.bridge.reinstalls.len(), 1);
}

#[test]
fn script_memory_low_marks_are_recorded_per_installed_script() {
    let mut bridge = Bridge::default();
    for (free, low) in [(20_000, 20_000), (19_500, 20_000), (18_900, 18_900), (30_000, 18_900)] {
        bridge.acknowledge(Ack {
            free_memory: Some(free),
            ..Ack::default()
        });
        assert_eq!(bridge.memory_low, Some(low));
    }
    bridge.enter(Phase::Upload, Instant::now());
    assert_eq!(bridge.memory_low, None);
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
    sim.bridge.motion = Motion::motor(13, 3.2, 1.7);
    assert!(
        sim.bridge.captures_motion(sim.now),
        "a recent compatible capture still owns the controls"
    );
    sim.step();
    assert!(sim.bridge.captures_motion(sim.now));
}

#[test]
fn first_dive_edge_is_forwarded_while_its_http_command_is_pending() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.bridge.motion = Motion::motor(3, 5.0, 1.7);
    sim.step();
    // The edge may arrive while a periodic surface refresh is awaiting HTTP.
    sim.now += Duration::from_millis(500);
    sim.bridge.send_mode(&sim.prefs, sim.now);
    let old_request = sim.bridge.pending.unwrap().0;
    sim.bridge.motion = Motion::motor(7, 5.0, 1.7);
    sim.bridge.motion.parameters[2] = -1.95;
    assert!(sim.bridge.captures_motion(sim.now));
    let flags = aurora_net::control::UP_NEG | aurora_net::control::FAST_UP;
    assert_eq!(
        crate::movement::controls(flags, crate::movement::State::Submerged, sim.bridge.captures_motion(sim.now)),
        aurora_net::control::UP_NEG
    );
    sim.bridge.send_mode(&sim.prefs, sim.now);
    assert!(sim.bridge.pending.is_some());
    assert_ne!(sim.bridge.pending.unwrap().0, old_request);
    assert!(sim.bridge.captures_motion(sim.now));
    sim.now += Duration::from_millis(1100);
    assert!(
        !sim.bridge.captures_motion(sim.now),
        "an unconfirmed transition cannot renew capture indefinitely"
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
fn water_walk_jump_and_landing_supersede_pending_support_without_waiting_for_an_ack() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.bridge.motion = Motion::motor(13, 3.2, 1.7);
    sim.bridge.send_mode(&sim.prefs, sim.now);
    let mut previous = sim.bridge.pending.expect("support in flight");
    for (ms, motion) in [(10, Motion::water_walk_jump(3.2, 1.7)), (20, Motion::motor(13, 3.2, 1.7))] {
        sim.bridge.motion = motion;
        sim.bridge.send_mode(&sim.prefs, sim.now + Duration::from_millis(ms));
        let next = sim.bridge.pending.expect("superseding action");
        assert_ne!(previous.0, next.0);
        assert!(next.1 > previous.1);
        assert_eq!(next.2.2, motion);
        previous = next;
    }
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
fn cancelling_an_unconfirmed_motor_sends_release_even_if_the_last_ack_was_idle() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.bridge.motion = Motion::motor(3, 5.0, 1.7);
    sim.bridge.send_mode(&sim.prefs, sim.now);
    let old = sim.bridge.pending.unwrap();
    sim.bridge.motion = Motion::default();
    sim.bridge.send_mode(&sim.prefs, sim.now + Duration::from_millis(10));
    let (request, sequence, mode) = sim.bridge.pending.unwrap();
    assert_ne!(request, old.0);
    assert!(sequence > old.1);
    assert_eq!(
        mode.2.mode, 0,
        "the unacknowledged surface command may already have captured controls"
    );
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
    assert_eq!(Preferences::default().mode, LockMode::Bridge);
    assert!(!Preferences::default().indicator, "the lock indicator is opt-in");
    let old: Preferences = serde_json::from_str(r#"{"enabled":true,"indicator":false}"#).expect("old account");
    assert!(old.enabled);
    assert_eq!(old.mode, LockMode::Bridge);
    let native = Preferences {
        mode: LockMode::NativeSit,
        ..prefs
    };
    assert_eq!(
        serde_json::from_str::<Preferences>(&serde_json::to_string(&native).expect("save")).expect("load"),
        native
    );
}

#[test]
fn native_movelock_disables_the_bridge_anchor_without_disabling_movement_services() {
    let mut sim = Simulator::new();
    sim.ready();
    assert!(sim.bridge.locked());
    sim.prefs.mode = LockMode::NativeSit;
    sim.bridge.motion = Motion::motor(2, 1.0, 1.7);
    sim.step();
    assert!(sim.prefs.enabled, "the user selection remains enabled");
    assert!(!sim.bridge.ack.enabled);
    assert!(!sim.bridge.locked());
    assert_eq!(sim.bridge.sent.expect("sent mode").2.mode, 2);
    sim.prefs.mode = LockMode::Bridge;
    sim.step();
    assert!(sim.bridge.ack.enabled);
}

#[test]
fn native_movelock_ao_fallback_routes_real_bridge_commands_and_recovers() {
    use crate::movement::movelock::{Controller, Frame, Travel};
    let mut sim = Simulator::new();
    sim.ready();
    let selected = Preferences {
        mode: LockMode::NativeSit,
        ..sim.prefs
    };
    let mut native = Controller::default();
    let mut frame = Frame {
        requested: true,
        region: sim.world.main_region,
        available: true,
        voluntary: false,
        moving: false,
        travel: Travel::Ready,
        pose_ready: false,
        pose_warm: false,
        ground_sit: false,
        on_object: false,
    };
    sim.bridge.motion = Motion::motor(2, 1.0, 1.7);
    for ready in [false, true, false, true] {
        frame.pose_ready = ready;
        assert!(native.update(frame, sim.now).is_none());
        sim.prefs = native.bridge_preferences(selected);
        sim.step();
        assert_eq!(sim.bridge.ack.enabled, !ready);
        assert_eq!(sim.bridge.locked(), !ready);
        assert!(sim.bridge.available(), "the bridge is reused without reinstallation");
        assert_eq!(sim.bridge.sent.expect("sent mode").2.mode, 2, "retain other movement services");
        assert_eq!(selected.mode, LockMode::NativeSit);
    }
    frame.requested = false;
    frame.pose_ready = false;
    native.update(frame, sim.now);
    sim.prefs = native.bridge_preferences(Preferences {
        enabled: false,
        ..selected
    });
    sim.step();
    assert!(!sim.bridge.ack.enabled && !sim.bridge.locked());
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

/// The worn bridge arrives in the next region, before any new hello.
fn move_to_next_region(sim: &mut Simulator) {
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
}

fn lock_again(sim: &mut Simulator) {
    for _ in 0..12 {
        sim.step();
        if sim.bridge.locked() {
            return;
        }
    }
    panic!("bridge did not lock again: {:?} {:?}", sim.bridge.phase, sim.bridge.error);
}

#[test]
fn a_moved_bridge_seen_before_its_new_hello_is_probed_instead_of_reinstalled() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.now += Duration::from_secs(30);
    let attaches = sim.attaches;
    move_to_next_region(&mut sim);
    sim.now += Duration::from_millis(250);
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert_eq!(sim.bridge.phase, Phase::Handshake, "wait for the new URL, never recompile");
    lock_again(&mut sim);
    assert_eq!(
        (sim.copies, sim.uploads, sim.attaches),
        (1, 1, attaches),
        "a region change keeps the installed scripts and the worn object"
    );
}

#[test]
fn a_region_change_right_after_the_handshake_never_reuses_the_old_region_url() {
    let mut sim = Simulator::new();
    sim.ready();
    assert!(sim.bridge.query_endpoint().is_some());
    let (_, at) = sim.bridge.accepted.clone().expect("accepted hello");
    assert!(
        sim.now.duration_since(at) < Duration::from_secs(3),
        "inside the hello replay window"
    );
    move_to_next_region(&mut sim);
    sim.now += Duration::from_millis(250);
    sim.bridge.update(&mut sim.world, &sim.prefs, true, sim.now);
    assert!(
        !sim.bridge.ready() && sim.bridge.endpoint.is_none(),
        "the hello that made the bridge ready came from the old region"
    );
    assert!(sim.bridge.query.is_none(), "the old region's query URL is invalid too");
    sim.bridge.query_candidate = None;
    lock_again(&mut sim);
    assert_eq!((sim.copies, sim.uploads), (1, 1));
}

#[test]
fn a_dead_query_url_is_hidden_until_an_authenticated_new_hello_without_touching_the_motor() {
    let mut sim = Simulator::new();
    sim.ready();
    assert!(sim.bridge.query_endpoint().is_some());
    sim.now += DISCOVERY_INTERVAL;
    let controller = sim.bridge.endpoint.as_ref().map(|endpoint| (endpoint.url.clone(), endpoint.token));
    sim.bridge
        .query_reply(&sim.world, Some(aurora_i18n::tr!("bridge-request-gone")), sim.now);
    assert!(sim.bridge.query_endpoint().is_none());
    assert_eq!(
        sim.bridge.endpoint.as_ref().map(|endpoint| (endpoint.url.clone(), endpoint.token)),
        controller
    );
    assert!(sim.bridge.ready());
    for _ in 0..4 {
        sim.step();
    }
    assert!(sim.bridge.query_endpoint().is_some());
    assert_eq!((sim.copies, sim.uploads, sim.query_uploads), (1, 1, 1));
}

#[test]
fn a_dead_url_waits_for_the_new_hello_without_an_error_or_a_reinstall() {
    let mut sim = Simulator::new();
    sim.ready();
    let request = Uuid::new_v4();
    sim.bridge.pending = Some((request, sim.bridge.sequence, (true, true, Motion::default())));
    sim.event(NetEvent::BridgeReply {
        request,
        result: Err(aurora_i18n::tr!("bridge-request-gone").into()),
    });
    assert_eq!(sim.bridge.phase, Phase::Handshake);
    assert!(sim.bridge.error.is_none() && sim.bridge.endpoint.is_none() && !sim.bridge.locked());
    lock_again(&mut sim);
    assert_eq!((sim.copies, sim.uploads), (1, 1));
}

#[test]
fn a_moved_bridge_that_never_says_hello_is_reinstalled_not_failed() {
    let mut sim = Simulator::new();
    sim.ready();
    sim.now += Duration::from_secs(30);
    sim.demo.startup_stalled = true;
    move_to_next_region(&mut sim);
    for _ in 0..32 * 4 {
        sim.step();
    }
    // The Repair path restarts from the inventory; this synthetic region has
    // no inventory capabilities, so it waits there instead of failing.
    assert_eq!(sim.bridge.reinstalls.len(), 1);
    assert_eq!(sim.bridge.phase, Phase::Inventory);
    assert!(sim.bridge.error.is_none());
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

#[test]
fn bridge_script_installation_and_repair_use_the_current_active_group() {
    let mut sim = Simulator::new();
    for event in crate::demo::demo_reply(&NetCommand::RequestGroups) {
        sim.world.apply(event);
    }
    let land = sim.world.groups.groups[0].id;
    sim.world.parcel = Some(Arc::new(aurora_net::ParcelInfo {
        group_id: land,
        ..Default::default()
    }));
    let active = Uuid::from_u128(8800);
    sim.world.groups.active = active;
    assert_eq!(sim.world.group_for_rez(true), land);
    sim.ready();
    let mut expected: Vec<_> = [NAME, QUERY_NAME, ik::NAME]
        .into_iter()
        .map(|name| (name.to_owned(), active))
        .collect();
    expected.sort();
    sim.script_rez_groups.sort();
    assert_eq!(sim.script_rez_groups, expected);
    for active in [Uuid::nil(), Uuid::from_u128(8801)] {
        sim.world.apply(NetEvent::ActiveGroup {
            id: active,
            name: "Active group".into(),
            title: "Builder".into(),
        });
        sim.script_rez_groups.clear();
        sim.demo.forget_script(QUERY_NAME);
        sim.bridge.retry(sim.now);
        sim.ready();
        assert_eq!(sim.script_rez_groups, vec![(QUERY_NAME.into(), active)]);
    }
}
