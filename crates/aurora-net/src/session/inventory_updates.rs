//! LLDispatchBulkUpdateInventory and process_large_generic_message (Firestorm
//! indra/newview/llinventorymodel.cpp, llviewergenericmessage.cpp, LGPL 2.1).
//! Modern simulators send copy callbacks as notation in the HTTP event queue.

use super::*;
use crate::inventory::{InvItem, folder_from_llsd, item_from_llsd};

struct Update {
    items: Vec<(u32, InvItem)>,
    parents: Vec<Uuid>,
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
        let mut parents: Vec<_> = items.iter().map(|(_, item)| item.parent).collect();
        parents.extend(
            document["folder_data"]
                .as_array()
                .iter()
                .filter_map(folder_from_llsd)
                .map(|f| f.parent),
        );
        parents.retain(|id| !id.is_nil());
        parents.sort();
        parents.dedup();
        Ok(Update { items, parents })
    })())
}

fn parse_eq(event: &EqEvent, agent: Uuid) -> Option<Result<Update, ()>> {
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
        let Ok(mut update) = update else {
            log::warn!("inventory generic update rejected: malformed or foreign recipient");
            return;
        };
        log::info!(
            "inventory generic update: {} items, {} parent folders",
            update.items.len(),
            update.parents.len()
        );
        // The tracked copy batch re-reads these parents once after all callbacks.
        // Avoid one extra HTTP snapshot per item while the copy is still filling.
        let copied_parents: HashSet<_> = update
            .items
            .iter()
            .filter(|(callback, _)| self.inventory_copies.request_for(*callback).is_some())
            .map(|(_, item)| item.parent)
            .collect();
        update.parents.retain(|parent| !copied_parents.contains(parent));
        let mut items = Vec::new();
        let mut created = Vec::new();
        for (callback, item) in update.items {
            let new = self.inventory_callbacks.remove(&callback).is_some_and(|(_, new)| new);
            self.accept_inventory_copy(callback, &item);
            if item.id.is_nil() {
                continue;
            }
            if let Some((editor, notecard, open)) = self.notecard_copies.remove(&callback) {
                emit(
                    self.sh,
                    NetEvent::NotecardInventoryCopied {
                        editor,
                        notecard,
                        open,
                        result: Ok(item.clone()),
                    },
                );
            }
            if new {
                created.push(item.clone());
            }
            items.push(item);
        }
        emit(self.sh, NetEvent::InventoryItems(items));
        for item in created {
            emit(self.sh, NetEvent::InventoryCreated(item));
        }
        if !update.parents.is_empty() {
            self.fetch_inventory(update.parents, self.agent_id(), false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    fn event(agent: Uuid) -> EqEvent {
        let document = llsd_map! { "item_data" => Llsd::Array(vec![llsd_map! {
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

    #[test]
    fn event_queue_copy_confirmation_keeps_callback_parent_and_permissions() {
        let agent = Uuid::from_u128(2);
        let update = parse_eq(&event(agent), agent).expect("supported event").expect("valid update");
        assert_eq!(update.items.len(), 1);
        let (callback, item) = &update.items[0];
        assert_eq!(*callback, 41);
        assert_eq!(item.id, Uuid::from_u128(103));
        assert_eq!(item.parent, Uuid::from_u128(100));
        assert_eq!(item.owner, agent);
        assert_eq!(item.owner_mask, 0x7fffffff);
        assert_eq!(update.parents, vec![item.parent]);
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
        use crate::outfits::tests::server;
        let agent = Uuid::from_u128(2);
        let folder = Uuid::from_u128(100);
        let snapshot = llsd_map! { "folders" => Llsd::Array(vec![llsd_map! {
            "folder_id" => folder, "version" => 2,
            "items" => Llsd::Array(vec![llsd_map! { "item_id" => Uuid::from_u128(103), "parent_id" => folder, "type" => 6 }])
        }]) };
        let (fetch, server_task) = server(vec![(200, snapshot)]).await;
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
            events,
            controls: Arc::new(Mutex::new(AgentControls::default())),
            stats: Arc::new(NetStats::default()),
            object_cache_dir: Arc::new(Mutex::new(None)),
            agent_language: Arc::new(Mutex::new(("en".into(), false))),
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
        let mut sim = Sim::new(address, 10);
        sim.caps = Arc::new(HashMap::from([("FetchInventoryDescendents2".into(), fetch)]));
        let mut session = Session {
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
            last_controls: AgentControls::default(),
            last_agent_update: Instant::now(),
            one_shot: 0,
            cam_warned: false,
            far: 128.0,
            logout_started: None,
            finished: false,
            agent_moved: false,
            cam_requested_at: None,
            agent_parcel: None,
            pending_named_tp: None,
            obj_cache_dir: None,
            obj_caches: HashMap::new(),
            last_cache_save: Instant::now(),
            social: Default::default(),
            tasks: Default::default(),
            land: Default::default(),
            decode_failures: HashMap::new(),
            inventory_uploads: Default::default(),
            inventory_previews: Default::default(),
            script_transfers: Default::default(),
            inventory_callbacks: HashMap::new(),
            inventory_copies: Default::default(),
            inventory_merchant_pending: false,
            next_inventory_callback: 40,
        };
        let request = Uuid::from_u128(1);
        sock.writable().await.expect("writable socket");
        session.start_inventory_copies(
            request,
            Reply {
                contents: Vec::new(),
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
        // Exercise the production dispatcher, not only the notation parser.
        session.on_eq_event(event(agent));
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                while let Ok(event) = received.try_recv() {
                    if let NetEvent::InventoryEdited { request: done, result } = event {
                        assert_eq!(done, request);
                        return result;
                    }
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("copy must complete promptly after the HTTP callback")
        .expect("confirmed copy");
        assert!(result.copies.is_empty());
        assert_eq!(result.contents[0].items[0].id, Uuid::from_u128(103));
        let requests = server_task.await.expect("readbacks");
        assert_eq!(requests.len(), 1, "tracked callbacks share the final readback");
    }
}
