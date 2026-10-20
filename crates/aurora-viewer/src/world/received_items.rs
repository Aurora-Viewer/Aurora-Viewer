//! Opt-in filing of new inventory offers. Direct offers identify the delivered
//! item/category (inventory_offer_handler, llimprocessing.cpp); scripted offers
//! choose their destination in the acceptance bucket (inventory_task_offer_callback,
//! llviewermessage.cpp). Existing inventory is never classified by ownership.

use super::World;
use super::inventory::{FetchState, Inventory, actions};
use super::notifications::{im, offered_object};
use aurora_net::inventory::operations::{Mutation, Operation};
use aurora_net::{InstantMessage, NetCommand, NetEvent};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const FOLDER: &str = "Received items";
const LIMIT: usize = 256;
const TIMEOUT: Duration = Duration::from_secs(120);

/// Find a real ordinary child, never an inventory link or a Library folder.
fn child(inv: &Inventory, parent: Uuid, name: &str) -> Option<Uuid> {
    inv.folders
        .get(&parent)
        .into_iter()
        .flat_map(|folder| folder.children.iter())
        .filter_map(|id| inv.folders.get(id))
        .filter(|f| !f.library && f.info.parent == parent && f.info.type_default == -1 && f.info.name == name)
        .map(|f| f.info.id)
        .min()
}

pub fn root(inv: &Inventory) -> Option<Uuid> {
    child(inv, child(inv, inv.root, super::bridge::FOLDER)?, FOLDER)
}

/// The independent server trees presented together by the Received tab.
pub fn roots(inv: &Inventory) -> Vec<Uuid> {
    actions::system(inv, 50).into_iter().chain(root(inv)).collect()
}

#[derive(Clone)]
struct Sender {
    id: Uuid,
    group: bool,
    /// Only direct avatar offers carry a legacy avatar name in their header.
    legacy: String,
}

enum Delivery {
    Item { id: Uuid, folder: bool },
    Accept(Box<NetCommand>),
}

struct Pending {
    offer: u64,
    sender: Sender,
    delivery: Delivery,
    since: Instant,
}

struct Moving {
    offer: u64,
    id: Uuid,
    asset_type: i8,
    declined: bool,
}

#[derive(Default)]
pub struct ReceivedItems {
    pub enabled: bool,
    pub enabled_since: i64,
    offers: HashMap<u64, Sender>,
    asked_groups: HashSet<Uuid>,
    pending: VecDeque<Pending>,
    seen: HashSet<Uuid>,
    /// Retained across a timeout too: a late move must still honor rejection.
    moves: HashMap<Uuid, Moving>,
    mutation: Option<(Uuid, Instant, Option<u64>)>,
}

impl ReceivedItems {
    pub fn offer(&mut self, im: &InstantMessage, notification: u64) {
        if !self.enabled || im.from_agent_id.is_nil() || (im.offline && im.timestamp != 0 && i64::from(im.timestamp) < self.enabled_since) {
            return;
        }
        let sender = Sender {
            id: im.from_agent_id,
            group: im.from_group,
            legacy: if im.dialog == im::INVENTORY_OFFERED {
                im.from_name.clone()
            } else {
                String::new()
            },
        };
        if im.dialog == im::TASK_INVENTORY_OFFERED {
            if self.offers.len() < LIMIT {
                self.offers.insert(notification, sender);
            }
        } else if let Some((kind, id)) = offered_object(&im.binary_bucket)
            && self.pending.len() < LIMIT
            && !self.seen.contains(&id)
        {
            if self.seen.len() >= 4096 {
                self.seen.clear();
            }
            self.seen.insert(id);
            self.pending.push_back(Pending {
                offer: notification,
                sender,
                delivery: Delivery::Item {
                    id,
                    folder: kind == super::notifications::AT_CATEGORY,
                },
                since: Instant::now(),
            });
        }
    }

    /// Keep ordinary offer semantics: a declined item can never be filed back
    /// out of the Trash. Accepted scripted deliveries wait for their destination.
    pub fn answer(&mut self, offer: u64, commands: &mut Vec<NetCommand>) {
        let sender = self.offers.remove(&offer);
        if commands.iter().any(|c| {
            matches!(
                c,
                NetCommand::SendImDialog {
                    dialog: im::INVENTORY_DECLINED,
                    ..
                }
            )
        }) {
            self.pending.retain(|p| p.offer != offer);
            for movement in self.moves.values_mut().filter(|m| m.offer == offer) {
                movement.declined = true;
            }
        }
        if !self.enabled || self.pending.len() >= LIMIT {
            return;
        }
        if let Some(sender) = sender
            && let Some(index) = commands.iter().position(|c| {
                matches!(
                    c,
                    NetCommand::SendImDialog {
                        dialog: im::TASK_INVENTORY_ACCEPTED,
                        ..
                    }
                )
            })
        {
            self.pending.push_back(Pending {
                offer,
                sender,
                delivery: Delivery::Accept(Box::new(commands.remove(index))),
                since: Instant::now(),
            });
        }
    }

    fn fallback(&mut self, out: &mut Vec<NetCommand>) {
        if let Some(pending) = self.pending.pop_front()
            && let Delivery::Accept(command) = pending.delivery
        {
            // A folder/name failure must never swallow the user's acceptance.
            out.push(*command);
        }
    }

    fn create(&mut self, parent: Uuid, name: &str, now: Instant, out: &mut Vec<NetCommand>) {
        let request = Uuid::new_v4();
        self.mutation = Some((request, now, self.pending.front().map(|p| p.offer)));
        out.push(NetCommand::EditInventory {
            request,
            change: Mutation {
                operations: vec![Operation::Folder {
                    id: Uuid::new_v4(),
                    parent,
                    name: name.into(),
                    type_default: -1,
                }],
                refresh: vec![parent],
                defer_folder_contents: true,
                ..Default::default()
            },
        });
    }

    fn update(&mut self, world: &mut World, now: Instant) -> Vec<NetCommand> {
        let mut out = Vec::new();
        // The list of notifications is bounded too; expired scripted offers
        // must not keep a sender alive indefinitely.
        self.offers.retain(|id, _| world.notifications.list.iter().any(|n| n.id == *id));
        if !self.enabled {
            self.offers.clear();
            self.asked_groups.clear();
            while !self.pending.is_empty() {
                self.fallback(&mut out);
            }
            return out;
        }
        if let Some((_, since, offer)) = self.mutation {
            if now.duration_since(since) < TIMEOUT {
                return out;
            }
            self.mutation = None;
            log::warn!("received item filing: inventory operation timed out");
            if self.pending.front().is_some_and(|p| Some(p.offer) == offer) {
                self.fallback(&mut out);
            }
        }
        let Some(pending) = self.pending.front() else { return out };
        if now.duration_since(pending.since) >= TIMEOUT {
            log::warn!("received item filing: name or inventory unavailable");
            self.fallback(&mut out);
            return out;
        }
        if let Delivery::Item { id, .. } = pending.delivery {
            if actions::in_type(&world.inventory, id, 14) || world.mutes.is_muted_id(&pending.sender.id) {
                self.pending.pop_front();
                return out;
            }
            if actions::parent(&world.inventory, id).is_none() {
                self.pending.rotate_left(1);
                return out;
            }
        }
        let sender = &pending.sender;
        let name = if sender.group {
            let name = world
                .groups
                .groups
                .iter()
                .find(|g| g.id == sender.id)
                .map(|g| g.name.clone())
                .or_else(|| world.search.groups.get(&sender.id).map(|g| g.name.clone()));
            if name.is_none() && self.asked_groups.insert(sender.id) {
                world.search.request_group(sender.id);
            }
            name
        } else {
            world
                .social
                .avatar_names
                .get(&sender.id)
                .filter(|n| !n.legacy_first.is_empty())
                .map(|n| {
                    format!(
                        "{} {}",
                        n.legacy_first,
                        if n.legacy_last.is_empty() { "Resident" } else { &n.legacy_last }
                    )
                })
                .or_else(|| world.social.names.get(&sender.id).cloned())
                .or_else(|| (!sender.legacy.is_empty()).then(|| sender.legacy.clone()))
        };
        let Some(mut name) = name else {
            self.pending.rotate_left(1);
            return out;
        };
        if !sender.group && !name.contains(' ') {
            name.push_str(" Resident");
        }
        if name.trim().is_empty() || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\')) {
            log::warn!("received item filing: invalid sender name");
            self.fallback(&mut out);
            return out;
        }
        let inv = &mut world.inventory;
        let mut parent = inv.root;
        for part in [super::bridge::FOLDER, FOLDER, &name] {
            inv.request(parent);
            if !inv.folders.get(&parent).is_some_and(|f| f.state == FetchState::Fetched) {
                return out;
            }
            let Some(id) = child(inv, parent, part) else {
                self.create(parent, part, now, &mut out);
                return out;
            };
            parent = id;
        }
        let Some(pending) = self.pending.pop_front() else { return out };
        match pending.delivery {
            Delivery::Accept(mut command) => {
                if let NetCommand::SendImDialog { bucket, .. } = command.as_mut() {
                    *bucket = parent.as_bytes().to_vec();
                }
                out.push(*command);
            }
            Delivery::Item { id, folder } => {
                let Some(source) = actions::parent(inv, id) else { return out };
                // A folder stays intact. No copy, re-upload or permission change.
                if source != parent && !actions::under(inv, parent, id) && !actions::in_type(inv, id, 14) {
                    if self.moves.len() >= LIMIT {
                        log::warn!("received item filing: too many unacknowledged moves");
                        return out;
                    }
                    let request = Uuid::new_v4();
                    self.moves.insert(
                        request,
                        Moving {
                            offer: pending.offer,
                            id,
                            asset_type: if folder {
                                super::notifications::AT_CATEGORY
                            } else {
                                inv.items.get(&id).map_or(-1, |item| item.asset_type as i8)
                            },
                            declined: false,
                        },
                    );
                    self.mutation = Some((request, now, None));
                    out.push(NetCommand::EditInventory {
                        request,
                        change: Mutation {
                            operations: vec![Operation::Patch {
                                id,
                                folder,
                                body: aurora_llsd::llsd_map! { "parent_id" => parent },
                            }],
                            refresh: vec![source, parent],
                            ..Default::default()
                        },
                    });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;

impl World {
    pub fn file_received_items(&mut self, now: Instant) -> Vec<NetCommand> {
        let mut filing = std::mem::take(&mut self.received_items);
        let out = filing.update(self, now);
        self.received_items = filing;
        out
    }

    pub fn received_items_event(&mut self, event: &NetEvent) -> bool {
        let NetEvent::InventoryEdited { request, result } = event else {
            return false;
        };
        let owns_mutation = self.received_items.mutation.is_some_and(|(id, _, _)| id == *request);
        let movement = self.received_items.moves.remove(request);
        if !owns_mutation && movement.is_none() {
            return false;
        }
        let offer = owns_mutation
            .then(|| self.received_items.mutation.and_then(|(_, _, offer)| offer))
            .flatten();
        if owns_mutation {
            self.received_items.mutation = None;
        }
        match result {
            Ok(reply) => {
                self.inventory.apply(reply.contents.clone());
                self.inventory.created_folders(reply.folders.clone());
            }
            Err(reason) => {
                log::warn!("received item filing failed: {reason}");
                let mut fallback = Vec::new();
                if self.received_items.pending.front().is_some_and(|p| Some(p.offer) == offer) {
                    self.received_items.fallback(&mut fallback);
                }
                self.groups.out.extend(fallback);
            }
        }
        if let Some(movement) = movement.filter(|m| m.declined) {
            self.discard_offer(movement.asset_type, movement.id);
        }
        true
    }
}

/// Offline fixture using the actual offer, delayed-fetch and filing paths.
pub fn demo(world: &mut World) {
    use aurora_net::inventory::{InvFolder, InvItem};
    super::inventory::demo::seed_received(&mut world.inventory, world.agent_id);
    world.received_items.enabled = true;
    for (number, kind, first, last, offline) in [
        (91_001, 0, "Example", "Resident", false),
        (91_002, 6, "Example", "Resident", true),
        (91_003, 8, "Alice", "Example", false),
    ] {
        let id = Uuid::from_u128(number);
        let sender = Uuid::from_u128(if first == "Example" { 91_100 } else { 91_101 });
        world.social.avatar_names.on_legacy(sender, first, last);
        let name = aurora_i18n::tr!("received-filing-demo-gift").to_owned();
        let mut bucket = vec![kind];
        bucket.extend_from_slice(id.as_bytes());
        world.apply(NetEvent::InstantMessage(InstantMessage {
            from_agent_id: sender,
            from_name: format!("{first} {last}"),
            to_agent_id: world.agent_id,
            dialog: im::INVENTORY_OFFERED,
            session_id: Uuid::from_u128(number + 1000),
            message: name.clone(),
            offline,
            from_group: false,
            binary_bucket: bucket,
            timestamp: 0,
        }));
        let parent = actions::system(&world.inventory, i32::from(kind)).unwrap_or(world.inventory.root);
        if kind == 8 {
            world.inventory.created_folders(vec![InvFolder {
                id,
                parent: world.inventory.root,
                name,
                type_default: -1,
                ..Default::default()
            }]);
            world.inventory.add_items(vec![InvItem {
                id: Uuid::from_u128(91_004),
                parent: id,
                name: aurora_i18n::tr!("received-filing-demo-photo").into(),
                asset_type: 0,
                inv_type: 0,
                owner: world.agent_id,
                ..Default::default()
            }]);
        } else {
            world.inventory.add_items(vec![InvItem {
                id,
                parent,
                name,
                asset_type: i32::from(kind),
                inv_type: i32::from(kind),
                owner: world.agent_id,
                ..Default::default()
            }]);
        }
    }
    let owner = Uuid::from_u128(91_102);
    world.social.avatar_names.on_legacy(owner, "Owner", "Resident");
    world.apply(NetEvent::InstantMessage(InstantMessage {
        from_agent_id: owner,
        from_name: "Gift dispenser".into(),
        to_agent_id: world.agent_id,
        dialog: im::TASK_INVENTORY_OFFERED,
        session_id: Uuid::from_u128(91_200),
        message: aurora_i18n::tr!("received-filing-demo-gift").into(),
        offline: true,
        from_group: false,
        binary_bucket: vec![6],
        timestamp: 0,
    }));
    if let Some(id) = world.notifications.list.last().map(|n| n.id) {
        let (commands, _) = world.respond_notification(id, super::notifications::Response::Accept);
        world.groups.out.extend(commands);
    }
}

pub fn demo_accept(world: &mut World, command: &NetCommand) -> bool {
    let NetCommand::SendImDialog {
        dialog: im::TASK_INVENTORY_ACCEPTED,
        id,
        bucket,
        ..
    } = command
    else {
        return false;
    };
    if *id != Uuid::from_u128(91_200) {
        return false;
    }
    if let Ok(parent) = Uuid::from_slice(bucket) {
        world.inventory.add_items(vec![aurora_net::inventory::InvItem {
            id: Uuid::from_u128(91_005),
            parent,
            name: aurora_i18n::tr!("received-filing-demo-object").into(),
            asset_type: 6,
            inv_type: 6,
            owner: world.agent_id,
            ..Default::default()
        }]);
    }
    true
}
