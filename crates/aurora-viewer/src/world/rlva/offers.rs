//! Give-to-#RLV routing from RlvGiveToRLVOffer (rlvinventory.cpp, LGPL 2.1).
//! Acceptance is still a native inventory offer answer. Filing starts only
//! after that answer and, for scripted gifts, the matching delivery transaction.
use crate::world::{
    inventory::{Inventory, actions},
    notifications::{im, offered_object},
};
use aurora_llsd::llsd_map;
use aurora_net::{
    InstantMessage, NetCommand,
    inventory::operations::{Mutation, Operation},
};
use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};
use uuid::Uuid;

pub const ACTOR: Uuid = Uuid::from_u128(0x726c_7661_6f66_6665_7200_0000_0000_0001);
const LIMIT: usize = 256;

struct Offered {
    im: InstantMessage,
    name: String,
    task: bool,
    bypassed_filing: bool,
}

struct Delivery {
    transaction: Uuid,
    folder: Option<Uuid>,
    task: bool,
    offered_name: String,
    path: Option<Vec<String>>,
    since: Instant,
    asked: Option<Instant>,
}

struct Pending {
    request: Uuid,
    change: Mutation,
    since: Instant,
}

#[derive(Default)]
pub struct Offers {
    offered: BTreeMap<u64, Offered>,
    ready: VecDeque<Delivery>,
    pending: Option<Pending>,
    pub notices: Vec<String>,
}

fn path(name: &str) -> Option<Vec<String>> {
    if !name.starts_with("#RLV/~") {
        return None;
    }
    let parts = name.split('/').collect::<Vec<_>>();
    if !(2..=4).contains(&parts.len()) {
        return None;
    }
    parts[1..].iter().map(|part| actions::clean_name(part).ok()).collect()
}

fn name(im: &InstantMessage, task: bool) -> Option<String> {
    let name = if task {
        let end = [" ( http://", " ( https://"]
            .iter()
            .filter_map(|prefix| im.message.find(prefix))
            .min()
            .unwrap_or(im.message.len());
        let name = im.message[..end].trim_end().get(1..)?;
        name.strip_suffix('\'').unwrap_or(name)
    } else {
        im.message.as_str()
    };
    name.starts_with("#RLV/~").then(|| name.to_owned())
}

fn child(inv: &Inventory, parent: Uuid, name: &str) -> Option<Uuid> {
    inv.folders
        .get(&parent)?
        .children
        .iter()
        .filter_map(|id| inv.folders.get(id))
        .find(|folder| !folder.library && folder.info.name.eq_ignore_ascii_case(name))
        .map(|folder| folder.info.id)
}

fn same_operation(a: &Operation, b: &Operation) -> bool {
    match (a, b) {
        (
            Operation::Folder {
                id: a,
                parent: ap,
                name: an,
                type_default: at,
            },
            Operation::Folder {
                id: b,
                parent: bp,
                name: bn,
                type_default: bt,
            },
        ) => a == b && ap == bp && an == bn && at == bt,
        (
            Operation::Patch {
                id: a,
                folder: af,
                body: ab,
            },
            Operation::Patch {
                id: b,
                folder: bf,
                body: bb,
            },
        ) => a == b && af == bf && ab == bb,
        _ => false,
    }
}

impl Offers {
    /// Returns whether the generic Received-items filing must yield to RLVa.
    pub fn observe(&mut self, im: &InstantMessage, notification: u64, allow: bool) -> bool {
        let task = im.dialog == im::TASK_INVENTORY_OFFERED;
        if !matches!(im.dialog, im::TASK_INVENTORY_OFFERED | im::INVENTORY_OFFERED) || im.binary_bucket.first().copied() != Some(8) {
            return false;
        }
        let Some(name) = name(im, task) else {
            return false;
        };
        if self.offered.len() >= LIMIT {
            return false;
        }
        self.offered.insert(
            notification,
            Offered {
                im: im.clone(),
                name,
                task,
                bypassed_filing: allow,
            },
        );
        allow
    }

    pub fn retain_notifications(&mut self, mut exists: impl FnMut(u64) -> bool) {
        self.offered.retain(|id, _| exists(*id));
    }

    /// Global disable restores ordinary filing for still-unanswered gifts;
    /// already accepted gifts keep their native server delivery semantics.
    pub fn ordinary_offers(&mut self) -> Vec<(u64, InstantMessage)> {
        self.offered
            .iter_mut()
            .filter_map(|(id, offer)| std::mem::take(&mut offer.bypassed_filing).then(|| (*id, offer.im.clone())))
            .collect()
    }

    pub fn answer(&mut self, notification: u64, commands: &mut [NetCommand], allow: bool, root: Option<Uuid>, now: Instant) {
        let Some(offer) = self.offered.get(&notification) else {
            return;
        };
        let Some(accepted) = commands.iter().find_map(|command| match command {
            NetCommand::SendImDialog { to, id, dialog, .. } if *to == offer.im.from_agent_id && *id == offer.im.session_id => match *dialog
            {
                im::INVENTORY_ACCEPTED | im::TASK_INVENTORY_ACCEPTED => Some(true),
                im::INVENTORY_DECLINED | im::TASK_INVENTORY_DECLINED => Some(false),
                _ => None,
            },
            _ => None,
        }) else {
            return;
        };
        let Some(offer) = self.offered.remove(&notification) else {
            return;
        };
        if !accepted {
            if offer.task {
                self.notices.push(format!("declined inv_offer {}", offer.name));
            }
            return;
        }
        if !allow {
            if offer.task {
                self.notices.push(format!("accepted_in_inv inv_offer {}", offer.name));
            }
            return;
        }
        if offer.task
            && let Some(root) = root
        {
            for command in commands {
                if let NetCommand::SendImDialog {
                    dialog: im::TASK_INVENTORY_ACCEPTED,
                    bucket,
                    ..
                } = command
                {
                    *bucket = root.as_bytes().to_vec();
                }
            }
        }
        if self.ready.len() < LIMIT {
            self.ready.push_back(Delivery {
                transaction: offer.im.session_id,
                folder: if offer.task {
                    None
                } else {
                    offered_object(&offer.im.binary_bucket).map(|(_, folder)| folder)
                },
                task: offer.task,
                offered_name: offer.name,
                path: None,
                since: now,
                asked: None,
            });
        }
    }

    pub fn delivered(&mut self, transaction: Uuid, folders: &[Uuid], inv: &Inventory) {
        let Some(delivery) = self
            .ready
            .iter_mut()
            .find(|delivery| delivery.task && delivery.transaction == transaction && delivery.folder.is_none())
        else {
            return;
        };
        delivery.folder = folders
            .iter()
            .copied()
            .find(|id| inv.folders.get(id).is_some_and(|folder| folder.info.name == delivery.offered_name))
            .or_else(|| {
                folders
                    .iter()
                    .copied()
                    .find(|id| inv.folders.get(id).is_some_and(|folder| folder.info.name.starts_with("#RLV/~")))
            });
    }

    pub fn owns(&self, request: Uuid) -> bool {
        self.pending.as_ref().is_some_and(|pending| pending.request == request)
    }

    /// Only the exact accepted-gift mutation can bypass folder locks. No
    /// arbitrary command becomes privileged by using the internal actor.
    pub fn permits(&self, request: Uuid, change: &Mutation) -> bool {
        self.pending.as_ref().is_some_and(|pending| {
            pending.request == request
                && change.copies.is_empty()
                && change.removed.is_empty()
                && change.refresh == pending.change.refresh
                && change.defer_folder_contents == pending.change.defer_folder_contents
                && matches!((&pending.change.operations[..], &change.operations[..]), ([a], [b]) if same_operation(a, b))
        })
    }

    pub fn failed(&mut self, request: Uuid) {
        if self.owns(request) {
            self.pending = None;
            self.ready.pop_front();
        }
    }

    fn command(&mut self, operation: Operation, refresh: Vec<Uuid>, now: Instant) -> NetCommand {
        let request = Uuid::new_v4();
        let change = Mutation {
            operations: vec![operation],
            refresh,
            defer_folder_contents: true,
            ..Default::default()
        };
        self.pending = Some(Pending {
            request,
            change: change.clone(),
            since: now,
        });
        NetCommand::EditInventory { request, change }
    }

    pub fn update(&mut self, inv: &mut Inventory, shared: Option<Uuid>, owner: Uuid, now: Instant) -> Option<NetCommand> {
        if let Some(pending) = &self.pending {
            let confirmed = match pending.change.operations.first()? {
                Operation::Folder { parent, name, .. } => child(inv, *parent, name).is_some(),
                Operation::Patch { id, body, .. } => inv.folders.get(id).is_some_and(|folder| {
                    if body.as_map().is_some_and(|map| map.contains_key("parent_id")) {
                        folder.info.parent == body["parent_id"].as_uuid()
                    } else {
                        folder.info.name == body["name"].as_str()
                    }
                }),
                _ => false,
            };
            if confirmed {
                self.pending = None;
            } else {
                if now.saturating_duration_since(pending.since) < Duration::from_secs(30) {
                    return None;
                }
                self.pending = None;
                self.ready.pop_front();
            }
        }
        // An undelivered scripted gift must not delay an unrelated ready one.
        for _ in 0..self.ready.len() {
            if self.ready.front().is_none_or(|delivery| delivery.folder.is_some()) {
                break;
            }
            let delivery = self.ready.pop_front()?;
            self.ready.push_back(delivery);
        }
        while let Some(delivery) = self.ready.front_mut() {
            if now.saturating_duration_since(delivery.since) >= Duration::from_secs(120) {
                self.ready.pop_front();
                continue;
            }
            let id = delivery.folder?;
            let Some(folder) = inv.folders.get(&id) else {
                if delivery
                    .asked
                    .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(10))
                {
                    delivery.asked = Some(now);
                    return Some(NetCommand::FetchInventory {
                        folders: vec![inv.root],
                        owner,
                        library: false,
                    });
                }
                let delivery = self.ready.pop_front()?;
                self.ready.push_back(delivery);
                return None;
            };
            if folder.library || id == inv.root || id == inv.lib_root || Some(id) == shared {
                self.ready.pop_front();
                continue;
            }
            // The actual delivered name is authoritative, as in the native
            // observers. A late server correction must not use the IM's path.
            if delivery.path.is_none() {
                delivery.path = path(&folder.info.name);
            }
            let Some(parts) = delivery.path.clone() else {
                self.ready.pop_front();
                continue;
            };
            let task = delivery.task;
            let expected = parts.last()?.clone();
            let mut parent = if let Some(shared) = shared {
                shared
            } else {
                return Some(self.command(
                    Operation::Folder {
                        id: Uuid::new_v4(),
                        parent: inv.root,
                        name: "#RLV".into(),
                        type_default: -1,
                    },
                    vec![inv.root],
                    now,
                ));
            };
            for component in &parts[..parts.len() - 1] {
                if !crate::world::appearance::loaded(inv, parent) {
                    inv.request(parent);
                    return None;
                }
                if let Some(child) = child(inv, parent, component) {
                    parent = child;
                } else {
                    return Some(self.command(
                        Operation::Folder {
                            id: Uuid::new_v4(),
                            parent,
                            name: component.clone(),
                            type_default: -1,
                        },
                        vec![parent],
                        now,
                    ));
                }
            }
            if actions::under(inv, parent, id) {
                self.ready.pop_front();
                continue;
            }
            if folder.info.parent != parent {
                return Some(self.command(
                    Operation::Patch {
                        id,
                        folder: true,
                        body: llsd_map! { "parent_id" => parent },
                    },
                    vec![folder.info.parent, parent],
                    now,
                ));
            }
            if folder.info.name != expected {
                return Some(self.command(
                    Operation::Patch {
                        id,
                        folder: true,
                        body: llsd_map! { "name" => expected },
                    },
                    vec![parent],
                    now,
                ));
            }
            if task && let Some(path) = super::inventory::path(inv, id) {
                self.notices.push(format!("accepted_in_rlv inv_offer {path}"));
            }
            self.ready.pop_front();
        }
        None
    }
}

impl crate::world::World {
    pub(crate) fn rlva_demo_shared_offer(&mut self, deliver: bool) {
        let gift = Uuid::from_u128(0x726c76616f66666572);
        let transaction = Uuid::from_u128(0x726c76616f66667478);
        if deliver {
            self.inventory.server_updated(
                vec![aurora_net::inventory::InvFolder {
                    id: gift,
                    parent: self.inventory.root,
                    name: "#RLV/~Demo/Cuffs".into(),
                    type_default: -1,
                    ..Default::default()
                }],
                vec![],
            );
            self.apply(aurora_net::NetEvent::InventoryDelivery {
                transaction,
                folders: vec![gift],
            });
        } else {
            self.on_task_inventory_offer(
                &InstantMessage {
                    from_agent_id: crate::demo::DEMO_TESS,
                    from_name: "Shared inventory gift".into(),
                    to_agent_id: self.agent_id,
                    dialog: im::TASK_INVENTORY_OFFERED,
                    session_id: transaction,
                    message: "'#RLV/~Demo/Cuffs'".into(),
                    offline: false,
                    from_group: false,
                    binary_bucket: vec![8],
                    timestamp: 0,
                },
                "Shared inventory gift",
            );
        }
    }

    pub(crate) fn rlva_demo_shared_offer_report(&self) {
        log::info!(
            "RLVa shared gift demo: path={}",
            super::inventory::path(&self.inventory, Uuid::from_u128(0x726c76616f66666572)).unwrap_or_default()
        );
    }
    pub(crate) fn rlva_configure(&mut self, enabled: bool) {
        if !enabled {
            for (id, im) in self.rlva.offers.ordinary_offers() {
                self.received_items.offer(&im, id);
            }
        }
        self.rlva.configure(enabled);
    }

    pub(crate) fn rlva_inventory_offer(&mut self, im: &InstantMessage, notification: u64) -> bool {
        self.rlva.enabled()
            && self
                .rlva
                .offers
                .observe(im, notification, self.rlva.preferences.allow_shared_offers)
    }

    pub(crate) fn rlva_shared_offer_answer(&mut self, notification: u64, commands: &mut [NetCommand]) {
        let allow = self.rlva.enabled() && self.rlva.preferences.allow_shared_offers;
        self.rlva.offers.answer(
            notification,
            commands,
            allow,
            super::inventory::shared_root(&self.inventory),
            Instant::now(),
        );
    }

    pub(crate) fn rlva_offer_tick(&mut self, now: Instant) -> Option<NetCommand> {
        if !self.rlva.enabled() {
            return None;
        }
        self.rlva
            .offers
            .retain_notifications(|id| self.notifications.list.iter().any(|notification| notification.id == id));
        if !self.rlva.preferences.allow_shared_offers {
            for (id, im) in self.rlva.offers.ordinary_offers() {
                self.received_items.offer(&im, id);
            }
        }
        for notice in std::mem::take(&mut self.rlva.offers.notices) {
            self.rlva.notify(&notice);
        }
        let shared = super::inventory::shared_root(&self.inventory);
        self.rlva.offers.update(&mut self.inventory, shared, self.agent_id, now)
    }

    pub(crate) fn rlva_offer_event(&mut self, event: &aurora_net::NetEvent) -> bool {
        let aurora_net::NetEvent::InventoryEdited { request, result } = event else {
            return false;
        };
        if !self.rlva.offers.owns(*request) {
            return false;
        }
        match result {
            Ok(reply) => {
                self.inventory.apply(reply.contents.clone());
                self.inventory.created_folders(reply.folders.clone());
            }
            Err(_) => {
                self.rlva.offers.failed(*request);
                log::warn!("RLVa shared gift filing failed");
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{
        World,
        inventory::{FetchState, Folder},
        notifications::{Response, inventory_answer},
    };
    use aurora_net::inventory::InvFolder;

    fn folder(inv: &mut Inventory, number: u128, parent: Uuid, name: &str) -> Uuid {
        let id = Uuid::from_u128(number);
        inv.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent,
                    name: name.into(),
                    type_default: -1,
                    ..Default::default()
                },
                children: vec![],
                items: vec![],
                state: FetchState::Fetched,
                library: false,
            },
        );
        if let Some(parent) = inv.folders.get_mut(&parent) {
            parent.children.push(id);
        }
        id
    }

    fn fixture() -> (Inventory, Uuid, Uuid) {
        let mut inv = Inventory::default();
        inv.root = folder(&mut inv, 1, Uuid::nil(), "Inventory");
        let root = inv.root;
        let shared = folder(&mut inv, 2, root, "#RLV");
        let gift = folder(&mut inv, 3, root, "#RLV/~Wardrobe/Cuffs");
        (inv, shared, gift)
    }

    fn offer(task: bool, gift: Uuid) -> InstantMessage {
        InstantMessage {
            from_agent_id: Uuid::from_u128(4),
            from_name: "Gift giver".into(),
            to_agent_id: Uuid::from_u128(7),
            session_id: Uuid::from_u128(5),
            dialog: if task { im::TASK_INVENTORY_OFFERED } else { im::INVENTORY_OFFERED },
            message: if task {
                "'#RLV/~Wardrobe/Cuffs' ( https://maps.secondlife.com/secondlife/Example/128/128/0 )".into()
            } else {
                "#RLV/~Wardrobe/Cuffs".into()
            },
            binary_bucket: if task {
                vec![8]
            } else {
                [vec![8], gift.as_bytes().to_vec()].concat()
            },
            offline: false,
            from_group: false,
            timestamp: 0,
        }
    }

    fn accepted(planner: &mut Offers, message: &InstantMessage, shared: Uuid, now: Instant) -> Vec<NetCommand> {
        assert!(planner.observe(message, 10, true));
        let mut commands = vec![inventory_answer(
            message.from_agent_id,
            message.session_id,
            message.dialog == im::TASK_INVENTORY_OFFERED,
            true,
            Uuid::from_u128(99),
        )];
        planner.answer(10, &mut commands, true, Some(shared), now);
        commands
    }

    #[test]
    fn rlva_shared_task_gift_waits_for_its_transaction_and_each_confirmed_inventory_edit() {
        let (mut inv, shared, gift) = fixture();
        let now = Instant::now();
        let mut planner = Offers::default();
        let message = offer(true, gift);
        let commands = accepted(&mut planner, &message, shared, now);
        assert!(matches!(&commands[0], NetCommand::SendImDialog { bucket, .. } if bucket == shared.as_bytes()));
        planner.delivered(Uuid::from_u128(999), &[gift], &inv);
        assert!(planner.update(&mut inv, Some(shared), Uuid::nil(), now).is_none());
        planner.delivered(message.session_id, &[gift], &inv);
        let Some(NetCommand::EditInventory { request, change }) = planner.update(&mut inv, Some(shared), Uuid::nil(), now) else {
            panic!("create destination");
        };
        assert!(planner.permits(request, &change));
        assert!(!planner.permits(Uuid::new_v4(), &change));
        let mut forged = change.clone();
        forged.removed.push(gift);
        assert!(!planner.permits(request, &forged));
        assert!(
            planner.update(&mut inv, Some(shared), Uuid::nil(), now).is_none(),
            "an acknowledgement without metadata does not advance"
        );
        let parent = folder(&mut inv, 6, shared, "~wardrobe");
        let Some(NetCommand::EditInventory { change, .. }) = planner.update(&mut inv, Some(shared), Uuid::nil(), now) else {
            panic!("move");
        };
        assert!(
            matches!(&change.operations[..], [Operation::Patch { id, body, .. }] if *id == gift && body["parent_id"].as_uuid() == parent)
        );
        assert!(planner.update(&mut inv, Some(shared), Uuid::nil(), now).is_none());
        assert!(planner.notices.is_empty());
        inv.folders.get_mut(&gift).unwrap().info.parent = parent;
        let Some(NetCommand::EditInventory { change, .. }) = planner.update(&mut inv, Some(shared), Uuid::nil(), now) else {
            panic!("rename");
        };
        assert!(matches!(&change.operations[..], [Operation::Patch { id, body, .. }] if *id == gift && body["name"].as_str() == "Cuffs"));
        assert!(planner.notices.is_empty());
        inv.folders.get_mut(&gift).unwrap().info.name = "Cuffs".into();
        assert!(planner.update(&mut inv, Some(shared), Uuid::nil(), now).is_none());
        assert_eq!(planner.notices, ["accepted_in_rlv inv_offer ~wardrobe/Cuffs"]);
        assert!(planner.ready.is_empty());
    }

    #[test]
    fn rlva_direct_shared_gifts_create_a_missing_root_and_fail_without_unrelated_moves() {
        let (mut inv, shared, gift) = fixture();
        inv.folders.get_mut(&shared).unwrap().info.name = "Ordinary folder".into();
        let now = Instant::now();
        let mut planner = Offers::default();
        accepted(&mut planner, &offer(false, gift), shared, now);
        let Some(NetCommand::EditInventory { request, change }) = planner.update(&mut inv, None, Uuid::nil(), now) else {
            panic!("root");
        };
        assert!(matches!(&change.operations[..], [Operation::Folder { parent, name, .. }] if *parent == inv.root && name == "#RLV"));
        planner.failed(Uuid::new_v4());
        assert!(planner.owns(request));
        planner.failed(request);
        assert!(planner.update(&mut inv, None, Uuid::nil(), now).is_none());
        assert!(planner.notices.is_empty());
        accepted(&mut planner, &offer(false, gift), shared, now);
        planner.update(&mut inv, None, Uuid::nil(), now);
        assert!(planner.update(&mut inv, None, Uuid::nil(), now + Duration::from_secs(31)).is_none());
        assert!(planner.ready.is_empty(), "failed metadata confirmation is bounded");
    }

    #[test]
    fn rlva_shared_offer_paths_and_native_accept_decline_notifications_match_the_reference() {
        assert_eq!(path("#RLV/~Parent/Child/Last").unwrap(), ["~Parent", "Child", "Last"]);
        for invalid in [
            "#RLV/Ordinary",
            "#rlv/~Name",
            "#RLV/~Parent/Child/Last/Too deep",
            "#RLV/~Parent//Empty",
        ] {
            assert!(path(invalid).is_none());
        }
        let mut planner = Offers::default();
        let message = offer(true, Uuid::from_u128(3));
        assert!(!planner.observe(&message, 10, false));
        planner.answer(10, &mut [], false, None, Instant::now());
        assert!(planner.notices.is_empty(), "opening a name link is not an answer");
        let mut commands = [inventory_answer(message.from_agent_id, message.session_id, true, true, Uuid::nil())];
        planner.answer(10, &mut commands, false, None, Instant::now());
        assert_eq!(planner.notices, ["accepted_in_inv inv_offer #RLV/~Wardrobe/Cuffs"]);
        planner.observe(&message, 11, true);
        commands[0] = inventory_answer(message.from_agent_id, message.session_id, true, false, Uuid::nil());
        planner.answer(11, &mut commands, true, None, Instant::now());
        assert_eq!(planner.notices[1], "declined inv_offer #RLV/~Wardrobe/Cuffs");
        assert!(planner.ready.is_empty());
        let mut unrelated = message.clone();
        unrelated.message = "Ordinary gift".into();
        assert!(!planner.observe(&unrelated, 12, true));
        unrelated = message.clone();
        unrelated.binary_bucket = vec![6];
        assert!(!planner.observe(&unrelated, 13, true));
    }

    #[test]
    fn rlva_shared_offer_world_routing_preserves_consent_global_disable_and_exact_internal_authority() {
        let (inv, shared, gift) = fixture();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.inventory = inv;
        world.agent_id = Uuid::from_u128(7);
        world.rlva_configure(true);
        let message = offer(true, gift);
        world.on_task_inventory_offer(&message, "Gift giver");
        let notification = world.notifications.list.last().unwrap().id;
        assert!(world.rlva.offers.ready.is_empty(), "receiving a gift never accepts it");
        let (commands, _) = world.respond_notification(notification, Response::Accept);
        assert!(matches!(&commands[..], [NetCommand::SendImDialog { bucket, .. }] if bucket == shared.as_bytes()));
        world.apply(aurora_net::NetEvent::InventoryDelivery {
            transaction: message.session_id,
            folders: vec![gift],
        });
        let command = world.rlva_offer_tick(Instant::now()).unwrap();
        assert!(world.rlva_allows_command(&command, ACTOR));
        if let NetCommand::EditInventory { request, change } = command {
            assert!(!world.rlva_allows_command(
                &NetCommand::EditInventory {
                    request: Uuid::new_v4(),
                    change: change.clone()
                },
                ACTOR
            ));
            world.rlva_configure(false);
            assert!(!world.rlva.offers.owns(request));
        }
        assert!(world.rlva_offer_tick(Instant::now()).is_none());
        world.rlva_configure(true);
        world.received_items.enabled = true;
        world.on_task_inventory_offer(&message, "Gift giver");
        let notification = world.notifications.list.last().unwrap().id;
        world.rlva_configure(false);
        assert!(world.rlva.offers.offered.is_empty());
        let (commands, _) = world.respond_notification(notification, Response::Accept);
        assert!(
            commands.is_empty(),
            "ordinary Received-items filing resumes its deferred acceptance"
        );
    }
}
