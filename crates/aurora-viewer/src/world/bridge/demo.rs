//! Offline simulator of the attachment factory; no command reaches a grid.
use super::*;
use aurora_net::inventory::InvItem;

pub const SKY_VISITOR: Uuid = Uuid::from_u128(0xd0d0_b120_0000_0000_0000_0000_0000_0001);

pub fn seed_queries(world: &mut World) {
    if let Some(region) = world.main_region {
        world.coarse.entry(region).or_default().push((
            SKY_VISITOR,
            world.agent.position + glam::Vec3::new(16.0, 0.0, 1020.0 - world.agent.position.z),
        ));
        let _ = world.apply(NetEvent::DisplayNames {
            names: vec![aurora_net::AvatarNameData {
                id: SKY_VISITOR,
                username: "sky.visitor".into(),
                display_name: "Sky Visitor".into(),
                ..Default::default()
            }],
            bad_ids: Vec::new(),
            max_age: Some(3600),
        });
    }
}

#[derive(Default)]
pub struct Demo {
    script: Option<TaskItem>,
    pub installed: bool,
    pub startup_stalled: bool,
    pub invalid_endpoint: bool,
}

pub fn seed(world: &mut World) {
    super::super::inventory::demo::seed(&mut world.inventory, world.agent_id);
    let inv = &mut world.inventory;
    inv.add_items(vec![InvItem {
        id: Uuid::from_u128(0xA0_8100),
        parent: inv.lib_root,
        owner: inv.lib_owner,
        name: ROCK.into(),
        asset_type: 6,
        inv_type: 6,
        asset_id: Uuid::from_u128(0xA0_9000),
        base_mask: 0x7fff_ffff,
        owner_mask: 0x7fff_ffff,
        next_owner_mask: 0x7fff_ffff,
        ..Default::default()
    }]);
}

impl Demo {
    /// Commands use the same callbacks and confirmations as a simulator.
    pub fn reply(
        &mut self,
        bridge: &Bridge,
        world: &World,
        command: &NetCommand,
        moving: bool,
        seated: bool,
        failed: bool,
    ) -> Option<Vec<NetEvent>> {
        match command {
            NetCommand::BridgeQuery { request, query, .. } => {
                use aurora_net::bridge::query::{Altitude, Query, Reply, ScriptInfo};
                let result = if std::env::var("AURORA_DEMO_BRIDGE_QUERY").is_ok_and(|m| m == "error") {
                    Err(aurora_i18n::tr!("bridge-script-info-missing").into())
                } else {
                    Ok(match query {
                        Query::Altitudes(ids) => Reply::Altitudes(
                            ids.iter()
                                .filter(|id| **id == SKY_VISITOR)
                                .map(|id| Altitude { avatar: *id, z: 2048.0 })
                                .collect(),
                        ),
                        Query::ScriptInfo(object) => Reply::ScriptInfo(ScriptInfo {
                            object: *object,
                            name: "Demo scripted object".into(),
                            running: 12,
                            total: 17,
                            memory: 17 * 65536,
                            cpu_seconds: 0.00125,
                            character_seconds: 0.0002,
                        }),
                    })
                };
                Some(vec![NetEvent::BridgeQueryReply { request: *request, result }])
            }
            NetCommand::ScriptDialogReply {
                object_id,
                channel,
                index,
                label,
            } if *object_id == world.agent_id
                && *channel == DISCOVERY_CHANNEL
                && *index == 0
                && *label == format!("auroraHello|{}", bridge.object) =>
            {
                Some(self.handshake(bridge, world).into_iter().collect())
            }
            NetCommand::AgentAnimation { anim, start } if !bridge.agent.is_nil() => {
                let mut anims: Vec<_> = world
                    .animations
                    .get(&world.agent_id)
                    .into_iter()
                    .flatten()
                    .filter(|a| a.id != *anim)
                    .map(|a| (a.id, a.sequence))
                    .collect();
                if *start {
                    let sequence = anims.iter().map(|a| a.1).max().unwrap_or(0).saturating_add(1);
                    anims.push((*anim, sequence));
                }
                Some(vec![NetEvent::AvatarAnimations {
                    avatar: world.agent_id,
                    anims,
                    sources: Vec::new(),
                }])
            }
            NetCommand::FetchInventory { folders, owner, .. }
                if !bridge.agent.is_nil() && folders.iter().all(|id| world.inventory.folders.contains_key(id)) =>
            {
                let contents = folders
                    .iter()
                    .filter_map(|id| world.inventory.folders.get(id))
                    .map(|folder| aurora_net::inventory::FolderContents {
                        folder_id: folder.info.id,
                        owner_id: *owner,
                        version: folder.info.version.saturating_add(1),
                        folders: folder
                            .children
                            .iter()
                            .filter_map(|id| world.inventory.folders.get(id).map(|f| f.info.clone()))
                            .collect(),
                        items: folder
                            .items
                            .iter()
                            .filter_map(|id| world.inventory.items.get(id).cloned())
                            .collect(),
                    })
                    .collect();
                Some(vec![NetEvent::InventoryContents(contents)])
            }
            NetCommand::Build(BuildCmd::Transform { updates, .. })
                if world
                    .objects
                    .index_of_uuid(&bridge.object)
                    .and_then(|i| world.objects.get(i))
                    .is_some_and(|object| updates.iter().all(|update| update.local_id == object.key.local_id)) =>
            {
                Some(Vec::new())
            }
            NetCommand::FinishInventoryCopies { request, reply } if bridge.mutation == Some(*request) => {
                let mut items = Vec::new();
                for copy in &reply.copies {
                    let Some(mut item) = world.inventory.items.get(&copy.item).cloned() else {
                        return Some(vec![NetEvent::InventoryEdited {
                            request: *request,
                            result: Err(aurora_i18n::tr!("bridge-inventory-failed").into()),
                        }]);
                    };
                    item.id = Uuid::new_v4();
                    item.parent = copy.parent;
                    item.name.clone_from(&copy.name);
                    item.owner = world.agent_id;
                    items.push(item);
                }
                let mut completed = reply.clone();
                completed.copies.clear();
                completed.contents.clear();
                completed.folders.clear();
                Some(vec![
                    NetEvent::InventoryUpdated {
                        folders: Vec::new(),
                        items,
                    },
                    NetEvent::InventoryEdited {
                        request: *request,
                        result: Ok(completed),
                    },
                ])
            }
            NetCommand::RequestTaskInventory { object, .. } if *object == bridge.object => Some(vec![NetEvent::TaskInventory {
                object: *object,
                serial: Some(1),
                result: Ok(self.script.iter().cloned().collect()),
            }]),
            NetCommand::Build(BuildCmd::RezScript { item, .. }) if item.name == NAME => {
                let mut script = (**item).clone();
                script.item_id = Uuid::new_v4();
                self.script = Some(script);
                Some(Vec::new())
            }
            NetCommand::SaveScript { request, target, .. } if bridge.upload == Some(*request) => {
                self.installed = !failed;
                Some(vec![NetEvent::ScriptSaved {
                    request: *request,
                    target: *target,
                    result: Ok(aurora_net::scripts::ScriptSaved {
                        asset: Uuid::new_v4(),
                        compiled: !failed,
                        errors: if failed {
                            vec!["Synthetic compilation error".into()]
                        } else {
                            Vec::new()
                        },
                    }),
                }])
            }
            NetCommand::Bridge { request, command } => {
                let ack = Ack {
                    sequence: command.sequence,
                    enabled: command.enabled,
                    locked: command.enabled && !seated && !moving,
                    moving,
                    seated,
                    passage: None,
                    // Synthetic protocol state only; demo does not run Havok.
                    motor: Some(aurora_net::bridge::MotorReport {
                        mode: command.motion.mode,
                        run_state: if seated {
                            9
                        } else if command.motion.mode == 1 && moving {
                            5
                        } else {
                            1
                        },
                        speed_cm: (world.agent.velocity.with_z(0.0).length() * 100.0) as u32,
                        target_cm: if command.motion.mode == 1 {
                            (command.motion.parameters[0] * 513.0).round() as u32
                        } else {
                            0
                        },
                        pushes: u32::from(command.motion.mode == 1 && moving),
                        regulation: Some(aurora_net::bridge::RunRegulation::default()),
                    }),
                };
                Some(vec![NetEvent::BridgeReply {
                    request: *request,
                    result: if failed {
                        Err(aurora_i18n::tr!("bridge-request-failed").into())
                    } else {
                        Ok(ack)
                    },
                }])
            }
            NetCommand::DetachAttachments(items) if items.contains(&bridge.item) => {
                let local_ids = world
                    .worn_attachment(&bridge.item)
                    .and_then(|i| world.objects.get(i))
                    .map(|o| vec![o.key.local_id])
                    .unwrap_or_default();
                let mut item = world.inventory.items.get(&bridge.item)?.clone();
                item.asset_id = Uuid::new_v4();
                Some(vec![
                    NetEvent::InventoryUpdated {
                        folders: Vec::new(),
                        items: vec![item],
                    },
                    NetEvent::ObjectsKilled {
                        handle: world.main_region.unwrap_or_default(),
                        local_ids,
                    },
                ])
            }
            NetCommand::FetchItems { items, .. } if items.contains(&bridge.item) => Some(vec![NetEvent::InventoryUpdated {
                folders: Vec::new(),
                items: items.iter().filter_map(|id| world.inventory.items.get(id).cloned()).collect(),
            }]),
            NetCommand::Build(BuildCmd::GetScriptRunning { object_id, item_id, .. }) if *object_id == bridge.object => {
                Some(vec![NetEvent::ScriptRunning {
                    object_id: *object_id,
                    item_id: *item_id,
                    running: self.installed,
                    mono: Some(true),
                }])
            }
            _ => None,
        }
    }
    pub fn handshake(&self, bridge: &Bridge, world: &World) -> Option<NetEvent> {
        if !self.installed || self.startup_stalled || !matches!(bridge.phase, Phase::Starting | Phase::Handshake | Phase::Ready) {
            return None;
        }
        let host = if self.invalid_endpoint {
            "untrusted.example"
        } else {
            "simhost-example.agni.secondlife.io"
        };
        Some(NetEvent::Chat(ChatMessage {
            from_name: NAME.into(),
            source_id: bridge.object,
            owner_id: world.agent_id,
            source_type: ChatSourceType::Object,
            chat_type: ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: format!(
                "<auroraBridge>{VERSION}|{}|https://{host}:12043/cap/11111111-1111-4111-8111-111111111111|{}</auroraBridge>",
                bridge.folder,
                Uuid::from_u128(42)
            ),
        }))
    }
    pub fn state(&self, bridge: &mut Bridge, moving: bool, seated: bool, _prefs: &Preferences) {
        if bridge.ready() && bridge.sent.is_some() {
            bridge.ack.moving = moving;
            bridge.ack.seated = seated;
            bridge.ack.locked = bridge.ack.enabled && !seated && !moving;
        }
    }
}
