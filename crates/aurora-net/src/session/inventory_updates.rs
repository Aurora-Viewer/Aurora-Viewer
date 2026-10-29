//! LLDispatchBulkUpdateInventory and process_large_generic_message (Firestorm
//! indra/newview/llinventorymodel.cpp, llviewergenericmessage.cpp, LGPL 2.1).
//! Modern simulators send copy callbacks as notation in the HTTP event queue.

use super::*;
use crate::inventory::{InvFolder, InvItem, folder_from_llsd, item_from_llsd};

struct Update {
    transaction: Uuid,
    items: Vec<(u32, InvItem)>,
    folders: Vec<InvFolder>,
}

fn parse(agent: Uuid, method: &[u8], parameter: &[u8], expected: Uuid) -> Option<Result<Update, ()>> {
    if field_str(method) != "BulkUpdateInventory" {
        return None;
    }
    Some((|| {
        if agent != expected || expected.is_nil() || parameter.len() > 1024 * 1024 {
            return Err(());
        }
        let parameter = parameter.strip_prefix(b"<?llsd/notation?>").unwrap_or(parameter);
        let document = aurora_llsd::from_notation(parameter)
            .or_else(|_| aurora_llsd::from_xml(parameter))
            .or_else(|_| aurora_llsd::from_binary(parameter).map(|(document, _)| document))
            .map_err(|_| ())?;
        let mut items = Vec::new();
        for item in document["item_data"].as_array() {
            // Firestorm validates the recipient on each record, before firing
            // any callbacks. Validate the complete packet before applying it.
            if item["agent_id"].as_uuid() != expected {
                return Err(());
            }
            let value = item_from_llsd(item).unwrap_or_default();
            items.push((item["callback_id"].as_u32(), value));
        }
        let folders = document["folder_data"]
            .as_array()
            .iter()
            .filter_map(folder_from_llsd)
            .map(|mut f| {
                f.version = -1;
                f
            })
            .collect();
        Ok(Update {
            transaction: document["transaction_id"].as_uuid(),
            items,
            folders,
        })
    })())
}

fn parse_eq(event: &EqEvent, agent: Uuid) -> Option<Result<Update, ()>> {
    if matches!(event.message.as_str(), "UpdateCreateInventoryItem" | "BulkUpdateInventory") {
        return Some(parse_template(event, agent));
    }
    if !matches!(event.message.as_str(), "LargeGenericMessage" | "GenericMessage") {
        return None;
    }
    let body = &event.body;
    parse(
        body["AgentData"].at(0)["AgentID"].as_uuid(),
        body["MethodData"].at(0)["Method"].as_binary(),
        body["ParamList"].at(0)["Parameter"].as_binary(),
        agent,
    )
}

fn parse_template(event: &EqEvent, expected: Uuid) -> Result<Update, ()> {
    let b = &event.body;
    if expected.is_nil() || b["AgentData"].at(0)["AgentID"].as_uuid() != expected {
        return Err(());
    }
    let block = if event.message == "BulkUpdateInventory" {
        "ItemData"
    } else {
        "InventoryData"
    };
    let items = b[block]
        .as_array()
        .iter()
        .map(|it| {
            (
                it["CallbackID"].as_u32(),
                InvItem {
                    id: it["ItemID"].as_uuid(),
                    parent: it["FolderID"].as_uuid(),
                    asset_id: it["AssetID"].as_uuid(),
                    name: field_str(it["Name"].as_binary()),
                    desc: field_str(it["Description"].as_binary()),
                    asset_type: it["Type"].as_i32(),
                    inv_type: it["InvType"].as_i32(),
                    flags: it["Flags"].as_u32(),
                    creator: it["CreatorID"].as_uuid(),
                    owner: it["OwnerID"].as_uuid(),
                    group_id: it["GroupID"].as_uuid(),
                    group_owned: it["GroupOwned"].as_bool(),
                    base_mask: it["BaseMask"].as_u32(),
                    owner_mask: it["OwnerMask"].as_u32(),
                    group_mask: it["GroupMask"].as_u32(),
                    everyone_mask: it["EveryoneMask"].as_u32(),
                    next_owner_mask: it["NextOwnerMask"].as_u32(),
                    sale_type: it["SaleType"].as_i32() as u8,
                    sale_price: it["SalePrice"].as_i32(),
                    created_at: i64::from(it["CreationDate"].as_i32()),
                    ..Default::default()
                },
            )
        })
        .collect();
    let folders = b["FolderData"]
        .as_array()
        .iter()
        .filter(|f| !f["FolderID"].as_uuid().is_nil())
        .map(|f| InvFolder {
            id: f["FolderID"].as_uuid(),
            parent: f["ParentID"].as_uuid(),
            name: field_str(f["Name"].as_binary()),
            type_default: f["Type"].as_i32(),
            version: -1,
            ..Default::default()
        })
        .collect();
    Ok(Update {
        transaction: if event.message == "BulkUpdateInventory" {
            b["AgentData"].at(0)["TransactionID"].as_uuid()
        } else {
            Uuid::nil()
        },
        items,
        folders,
    })
}

impl Session<'_> {
    pub(super) fn on_inventory_eq(&mut self, event: &EqEvent) -> bool {
        let Some(update) = parse_eq(event, self.agent_id()) else {
            return false;
        };
        self.apply_generic_inventory_update(update);
        true
    }

    pub(super) fn on_inventory_generic(&mut self, agent: Uuid, method: &[u8], parameter: &[u8]) {
        if let Some(update) = parse(agent, method, parameter, self.agent_id()) {
            self.apply_generic_inventory_update(update);
        }
    }

    fn apply_generic_inventory_update(&mut self, update: Result<Update, ()>) {
        let Ok(update) = update else {
            log::warn!("inventory generic update rejected: malformed or foreign recipient");
            return;
        };
        log::info!(
            "inventory server update: {} items, {} folders",
            update.items.len(),
            update.folders.len()
        );
        let mut items = Vec::new();
        let mut created = Vec::new();
        for (callback, item) in update.items {
            self.asset_created(callback, &item);
            let new = self.inventory_callbacks.remove(&callback).is_some_and(|(_, new)| new);
            self.accept_inventory_copy(callback, &item);
            if let Some((editor, notecard, open)) = self.notecard_copies.remove(&callback) {
                emit(
                    self.sh,
                    NetEvent::NotecardInventoryCopied {
                        editor,
                        notecard,
                        open,
                        result: if item.id.is_nil() {
                            Err(aurora_i18n::tr!("inventory-copy-unconfirmed").into())
                        } else {
                            Ok(item.clone())
                        },
                    },
                );
            }
            if item.id.is_nil() {
                continue;
            }
            if new {
                created.push(item.clone());
            }
            items.push(item);
        }
        let folders = update.folders.iter().map(|folder| folder.id).collect();
        emit(
            self.sh,
            NetEvent::InventoryUpdated {
                folders: update.folders,
                items,
            },
        );
        if !update.transaction.is_nil() {
            emit(
                self.sh,
                NetEvent::InventoryDelivery {
                    transaction: update.transaction,
                    folders,
                },
            );
        }
        for item in created {
            emit(self.sh, NetEvent::InventoryCreated(item));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    fn event(agent: Uuid) -> EqEvent {
        let document = llsd_map! { "transaction_id" => Uuid::from_u128(202), "item_data" => Llsd::Array(vec![llsd_map! {
            "agent_id" => agent, "callback_id" => 41,
            "item_id" => Uuid::from_u128(103), "parent_id" => Uuid::from_u128(100),
            "name" => "Copied item", "type" => 6,
            "permissions" => llsd_map! { "owner_id" => agent, "owner_mask" => 0x7fffffff }
        }]) };
        EqEvent {
            sim: "127.0.0.1:12000".parse().expect("local simulator"),
            message: "LargeGenericMessage".into(),
            body: llsd_map! {
                "AgentData" => Llsd::Array(vec![llsd_map! { "AgentID" => agent }]),
                "MethodData" => Llsd::Array(vec![llsd_map! { "Method" => Llsd::Binary(b"BulkUpdateInventory\0".to_vec()) }]),
                "ParamList" => Llsd::Array(vec![llsd_map! { "Parameter" => Llsd::Binary(aurora_llsd::to_notation(&document).into_bytes()) }])
            },
        }
    }

    fn template_event(agent: Uuid, name: &str) -> EqEvent {
        let mut body =
            llsd_map! { "AgentData" => Llsd::Array(vec![llsd_map! { "AgentID" => agent, "TransactionID" => Uuid::from_u128(202) }]) };
        let block = if name == "BulkUpdateInventory" {
            "ItemData"
        } else {
            "InventoryData"
        };
        body.insert(
            block,
            Llsd::Array(vec![llsd_map! {
                "ItemID" => Uuid::from_u128(103), "FolderID" => Uuid::from_u128(100), "CallbackID" => 41,
                "OwnerID" => agent, "OwnerMask" => 0x7fffffff, "Type" => 6, "InvType" => 6,
                "Name" => Llsd::Binary(b"Copied item\0".to_vec())
            }]),
        );
        EqEvent {
            sim: "127.0.0.1:12000".parse().expect("address"),
            message: name.into(),
            body,
        }
    }

    #[test]
    fn direct_http_inventory_messages_have_the_same_callbacks_as_udp() {
        let agent = Uuid::from_u128(2);
        for name in ["UpdateCreateInventoryItem", "BulkUpdateInventory"] {
            let update = parse_eq(&template_event(agent, name), agent)
                .expect("supported direct HTTP update")
                .expect("valid");
            assert_eq!(update.items[0].0, 41);
            assert_eq!(
                update.transaction,
                if name == "BulkUpdateInventory" {
                    Uuid::from_u128(202)
                } else {
                    Uuid::nil()
                }
            );
            assert_eq!(update.items[0].1.parent, Uuid::from_u128(100));
            assert_eq!(update.items[0].1.name, "Copied item");
            assert_eq!(update.items[0].1.owner_mask, 0x7fffffff);
            assert!(
                parse_eq(&template_event(Uuid::from_u128(3), name), agent)
                    .expect("supported")
                    .is_err()
            );
        }
    }

    #[test]
    fn event_queue_copy_confirmation_keeps_callback_parent_and_permissions() {
        let agent = Uuid::from_u128(2);
        let update = parse_eq(&event(agent), agent).expect("supported event").expect("valid update");
        assert_eq!(update.items.len(), 1);
        assert_eq!(update.transaction, Uuid::from_u128(202));
        let (callback, item) = &update.items[0];
        assert_eq!(*callback, 41);
        assert_eq!(item.id, Uuid::from_u128(103));
        assert_eq!(item.parent, Uuid::from_u128(100));
        assert_eq!(item.owner, agent);
        assert_eq!(item.owner_mask, 0x7fffffff);
        assert!(update.folders.is_empty());
    }

    #[test]
    fn foreign_and_malformed_generic_updates_do_not_fire_callbacks() {
        let agent = Uuid::from_u128(2);
        assert!(parse_eq(&event(Uuid::from_u128(3)), agent).expect("supported").is_err());
        let mut foreign = event(Uuid::from_u128(3));
        foreign
            .body
            .insert("AgentData", Llsd::Array(vec![llsd_map! { "AgentID" => agent }]));
        assert!(parse_eq(&foreign, agent).expect("supported").is_err());
        let mut bad = event(agent);
        bad.body.insert("ParamList", Llsd::Array(vec![llsd_map! { "Parameter" => "{" }]));
        assert!(parse_eq(&bad, agent).expect("supported").is_err());
        bad.message = "UnrelatedEvent".into();
        assert!(parse_eq(&bad, agent).is_none());
    }

    #[tokio::test]
    async fn a_real_event_queue_callback_completes_the_copy_without_the_sixty_second_timeout() {
        use crate::inventory::operations::{CopyItem, Reply};

        let agent = Uuid::from_u128(2);
        let folder = Uuid::from_u128(100);
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let simulator = UdpSocket::bind("127.0.0.1:0").await.expect("local simulator");
        let address = simulator.local_addr().expect("simulator address");
        let (events, received) = crossbeam_channel::unbounded();
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("HTTP client");
        let sh = Shared {
            http: http.clone(),
            caps_http: http,
            events: crate::EventSender(events),
            controls: Arc::new(Mutex::new(AgentControls::default())),
            stats: Arc::new(NetStats::default()),
            object_cache_dir: Arc::new(Mutex::new(None)),
            agent_language: Arc::new(Mutex::new(("en".into(), false))),
            bandwidth_kbps: Default::default(),
            viewer_stats: Default::default(),
            viewer_stats_posts: Default::default(),
        };
        let login = Arc::new(LoginResponse {
            agent_id: agent,
            session_id: Uuid::from_u128(5),
            secure_session_id: Uuid::nil(),
            first_name: String::new(),
            last_name: String::new(),
            sim_ip: Ipv4Addr::LOCALHOST,
            sim_port: address.port(),
            circuit_code: 0,
            region_x: 0,
            region_y: 0,
            region_size_x: 256,
            region_size_y: 256,
            seed_capability: String::new(),
            look_at: Vec3::ZERO,
            message: String::new(),
            agent_appearance_service: String::new(),
            inventory_root: Uuid::nil(),
            raw: Llsd::Undef,
            mfa_hash: None,
        });
        let (eq_tx, _) = mpsc::unbounded_channel();
        let (caps_tx, _) = mpsc::unbounded_channel();
        let (notecard_copy_tx, _) = mpsc::unbounded_channel();
        let (inventory_move_tx, _) = mpsc::unbounded_channel();
        let sim = Sim::new(address, 10);
        let mut session = Session {
            asset_imports: Default::default(),
            sh: &sh,
            sock: sock.clone(),
            login,
            sims: [(address, sim)].into(),
            main: Some(address),
            eq_tx,
            caps_tx,
            notecard_copy_tx,
            inventory_move_tx,
            notecard_copies: HashMap::new(),
            agent_update: AgentUpdateRule::new(Instant::now()),
            control_flags: 0,
            one_shot: 0,
            cam_warned: false,
            far: 128.0,
            refuse_neighbours: false,
            fov: DEFAULT_FOV,
            logout_started: None,
            finished: false,
            agent_moved: false,
            viewer_stats: Default::default(),
            regions_visited: Default::default(),
            cam_requested_at: None,
            agent_parcel: None,
            pending_named_tp: None,
            object_cache: super::object_cache::ObjectCaches::new(None).0,
            throttle: crate::throttle::Throttle::new(crate::throttle::DEFAULT_KBPS, Instant::now()),
            social: Default::default(),
            tasks: Default::default(),
            land: Default::default(),
            estate_invoice: uuid::Uuid::nil(),
            terrain_raw: Default::default(),
            decode_failures: HashMap::new(),
            inventory_uploads: Default::default(),
            inventory_previews: Default::default(),
            script_transfers: Default::default(),
            inventory_callbacks: HashMap::new(),
            inventory_copies: Default::default(),
            attach_queue: Default::default(),
            inventory_merchant_pending: false,
            next_inventory_callback: 40,
        };
        for (index, name) in ["LargeGenericMessage", "UpdateCreateInventoryItem", "BulkUpdateInventory"]
            .into_iter()
            .enumerate()
        {
            let request = Uuid::from_u128(index as u128 + 1);
            session.next_inventory_callback = 40;
            sock.writable().await.expect("writable socket");
            session.start_inventory_copies(
                request,
                Reply {
                    contents: Vec::new(),
                    folders: Vec::new(),
                    moves_queued: false,
                    refresh: vec![folder],
                    removed: Vec::new(),
                    created: HashMap::new(),
                    copies: vec![CopyItem {
                        item: Uuid::from_u128(3),
                        owner: agent,
                        parent: folder,
                        name: String::new(),
                    }],
                },
            );
            let mut bytes = [0; 2048];
            let (size, _) = simulator.recv_from(&mut bytes).await.expect("copy packet");
            let copy: CopyInventoryItem = parse_packet(&bytes[..size]).expect("packet").decode().expect("copy");
            assert_eq!(copy.inventory_data[0].callback_id, 41);
            assert!(copy.inventory_data[0].new_name.is_empty());
            // Exercise the production dispatcher, not only the notation parser.
            let response = if name == "LargeGenericMessage" {
                event(agent)
            } else {
                template_event(agent, name)
            };
            let started = Instant::now();
            session.on_eq_event(response);
            let result = tokio::time::timeout(Duration::from_secs(5), async {
                let mut updated = false;
                let mut delivered = false;
                let mut completion = None;
                loop {
                    while let Ok((_, event)) = received.try_recv() {
                        if let NetEvent::InventoryUpdated { ref items, .. } = event {
                            updated |= items.iter().any(|item| item.id == Uuid::from_u128(103));
                        }
                        if let NetEvent::InventoryDelivery { transaction, .. } = event {
                            assert!(updated, "delivery observers see the updated inventory model");
                            assert_eq!(transaction, Uuid::from_u128(202));
                            delivered = true;
                        }
                        if let NetEvent::InventoryEdited { request: done, result } = event {
                            assert!(updated, "server items are visible before completion");
                            assert_eq!(done, request);
                            completion = Some(result);
                        }
                    }
                    if (delivered || name == "UpdateCreateInventoryItem")
                        && let Some(result) = completion.take()
                    {
                        return result;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("copy must complete promptly after the HTTP callback")
            .expect("confirmed copy");
            assert!(result.copies.is_empty());
            assert!(result.contents.is_empty(), "server records already updated the model");
            println!(
                "{name}: copy confirmed in {} microseconds, without a folder readback",
                started.elapsed().as_micros()
            );
        }
        let request = Uuid::from_u128(10);
        let mut bytes = [0; 2048];
        session.on_command(NetCommand::EditInventory {
            request,
            change: crate::inventory::operations::Mutation {
                operations: vec![crate::inventory::operations::Operation::Patch {
                    id: Uuid::from_u128(3),
                    folder: false,
                    body: llsd_map! { "parent_id" => Uuid::from_u128(200) },
                }],
                refresh: vec![folder, Uuid::from_u128(200)],
                ..Default::default()
            },
        });
        let (size, _) = simulator.recv_from(&mut bytes).await.expect("move packet");
        let movement: MoveInventoryItem = parse_packet(&bytes[..size]).expect("packet").decode().expect("move");
        assert_eq!(movement.inventory_data[0].folder_id, Uuid::from_u128(200));
        assert!(received.try_iter().any(|(_, event)| matches!(event, NetEvent::InventoryEdited { request: done, result: Ok(reply) } if done == request && reply.moves_queued && reply.contents.is_empty())), "moves are queued without an HTTP capability or a readback gate");
        for message in ["GenericMessage", "LargeGenericMessage"] {
            session.on_eq_event(EqEvent {
                sim: address, message: message.into(), body: llsd_map! {
                    "AgentData" => Llsd::Array(vec![llsd_map! { "AgentID" => agent }]),
                    "MethodData" => Llsd::Array(vec![llsd_map! { "Method" => Llsd::Binary(b"ExperienceEvent\0".to_vec()), "Invoice" => Uuid::from_u128(700) }]),
                    "ParamList" => Llsd::Array(vec![llsd_map! { "Parameter" => Llsd::Binary(b"{'Permission':i4}".to_vec()) }, llsd_map! { "Parameter" => Llsd::Binary(b"Controller\0".to_vec()) }])
                }
            });
            assert!(received.try_iter().any(|(_, event)| matches!(event, NetEvent::ExperienceAttachment { experience, object_name } if experience == Uuid::from_u128(700) && object_name == "Controller")));
        }
        // Both inventory services preserve why a record was requested. A
        // late attachment read must use normal creation filing in the model,
        // rather than the separate AIS orphan-recovery classification.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (ais, attachment) in [(true, true), (false, true), (true, false), (false, false)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("local inventory service");
            let cap = format!("http://{}", listener.local_addr().expect("address"));
            session.sims.get_mut(&address).expect("sim").caps = Arc::new(HashMap::from([(
                if ais { "InventoryAPIv3" } else { "FetchInventory2" }.to_owned(),
                cap,
            )]));
            let record = llsd_map! {
                "item_id" => Uuid::from_u128(103), "parent_id" => Uuid::nil(),
                "name" => "Script-attached object", "type" => 6, "inv_type" => 6,
                "permissions" => llsd_map! { "owner_id" => agent }
            };
            let body = aurora_llsd::to_xml(&if ais {
                record
            } else {
                llsd_map! { "items" => Llsd::Array(vec![record]) }
            });
            let service = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.expect("HTTP read");
                let mut bytes = Vec::new();
                loop {
                    if let Some(header_end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..header_end]);
                        let length = header
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|n| n.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= header_end + 4 + length {
                            break;
                        }
                    }
                    let mut chunk = [0; 2048];
                    let n = socket.read(&mut chunk).await.expect("request");
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                }
                let request = String::from_utf8_lossy(&bytes);
                if ais {
                    assert!(request.starts_with(&format!("GET /item/{} ", Uuid::from_u128(103))));
                } else {
                    assert!(request.starts_with("POST / "));
                    assert!(request.contains(&agent.to_string()) && request.contains(&Uuid::from_u128(103).to_string()));
                }
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(headers.as_bytes()).await.expect("headers");
                socket.write_all(&body).await.expect("response");
            });
            session.on_command(if attachment {
                NetCommand::FetchAttachmentItems {
                    items: vec![Uuid::from_u128(103)],
                }
            } else {
                NetCommand::FetchItems {
                    items: vec![Uuid::from_u128(103)],
                    owner: agent,
                }
            });
            let (was_attachment, items) = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    while let Ok((_, event)) = received.try_recv() {
                        match event {
                            NetEvent::AttachmentInventoryItems(items) => return (true, items),
                            NetEvent::InventoryItems(items) => return (false, items),
                            NetEvent::InventoryItemsFailed { .. } => panic!("local item read failed"),
                            _ => {}
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("inventory reply");
            assert_eq!(was_attachment, attachment, "AIS {ais}");
            assert_eq!(items.len(), 1);
            assert!(items[0].parent.is_nil());
            assert_eq!(items[0].owner, agent);
            service.await.expect("service completed");
        }
    }
}
