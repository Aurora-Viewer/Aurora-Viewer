//! Restoring the outfit at login, as LLAppearanceMgr::updateAppearanceFromCOF
//! (callAfterCOFFetch in llstartup.cpp) does: fetch the Current Outfit
//! folder, wear the objects it links to that are not worn yet
//! (RezMultipleAttachmentsFromInv through LLAttachmentsMgr, attachment
//! point 0 = where each was last worn), then ask the server to bake the
//! outfit (UpdateAvatarAppearance with the COF version). Body parts and
//! clothing need no request: the server bakes them from the COF.

use super::inventory::{FetchState, Inventory};
use aurora_net::inventory::InvItem;
use aurora_net::{AttachRequest, NetCommand};
use std::collections::HashSet;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// LLFolderType::FT_CURRENT_OUTFIT.
pub const FT_CURRENT_OUTFIT: i32 = 46;
const ASSET_LINK: i32 = 24;
const ASSET_OBJECT: i32 = 6;
const INV_OBJECT: i32 = 6;
const INV_ATTACHMENT: i32 = 17;

/// Objects that arrive by themselves are given this long before being
/// requested (a quick relog can still have them on the region).
const SETTLE: Duration = Duration::from_secs(4);
/// Requests that produced nothing are sent once more after this.
const RETRY_AFTER: Duration = Duration::from_secs(30);
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const BAKE_RETRIES: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    /// Waiting for the region, its capabilities and the inventory skeleton.
    Waiting,
    FetchCof {
        since: Instant,
        attempts: u32,
    },
    FetchItems {
        since: Instant,
    },
    Settle {
        since: Instant,
    },
    Attaching {
        since: Instant,
        retried: bool,
    },
    Done,
}

/// What the restore needs from the world each tick.
pub struct OutfitInputs<'a> {
    pub inventory: &'a mut Inventory,
    pub agent: Uuid,
    /// In a region with its capabilities.
    pub ready: bool,
    pub first_login: bool,
    pub second_life: bool,
    /// Inventory items attached to our avatar now (AttachItemID).
    pub worn: HashSet<Uuid>,
    /// COF version of the last AvatarAppearance received for us.
    pub received_cof_version: i32,
}

pub struct OutfitRestore {
    stage: Stage,
    cof: Uuid,
    cof_version: i32,
    /// Objects the COF links to (target item ids).
    objects: Vec<Uuid>,
    requested: HashSet<Uuid>,
    bake_requested: Option<i32>,
    bake_retries: u32,
    pub commands: Vec<NetCommand>,
}

impl Default for OutfitRestore {
    fn default() -> Self {
        OutfitRestore {
            stage: Stage::Waiting,
            cof: Uuid::nil(),
            cof_version: -1,
            objects: Vec::new(),
            requested: HashSet::new(),
            bake_requested: None,
            bake_retries: 0,
            commands: Vec::new(),
        }
    }
}

/// Target ids of the object links in the COF (the links are sorted by name
/// in the folder; duplicates and broken links are skipped).
pub fn cof_objects(inv: &Inventory, cof: Uuid) -> Vec<Uuid> {
    let Some(f) = inv.folders.get(&cof) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for id in &f.items {
        let Some(link) = inv.items.get(id) else {
            continue;
        };
        if link.asset_type != ASSET_LINK || link.asset_id.is_nil() {
            continue;
        }
        let target = inv.items.get(&link.asset_id);
        let is_object = match target {
            Some(t) => t.asset_type == ASSET_OBJECT,
            None => matches!(link.inv_type, INV_OBJECT | INV_ATTACHMENT),
        };
        if is_object && !out.contains(&link.asset_id) {
            out.push(link.asset_id);
        }
    }
    out
}

fn attach_request(item: &InvItem, agent: Uuid) -> AttachRequest {
    AttachRequest {
        item_id: item.id,
        owner_id: if item.owner.is_nil() { agent } else { item.owner },
        point: 0,
        add: true,
        flags: item.flags,
        group_mask: item.group_mask,
        everyone_mask: item.everyone_mask,
        next_owner_mask: item.next_owner_mask,
        name: item.name.clone(),
        desc: item.desc.clone(),
    }
}

impl OutfitRestore {
    pub fn is_done(&self) -> bool {
        self.stage == Stage::Done
    }

    /// The server answered the bake request.
    pub fn on_bake_result(&mut self, cof_version: i32, success: bool, expected: Option<i32>) {
        if success {
            return;
        }
        // stale COF version: ask again with the one the server expects
        if let Some(v) = expected.filter(|v| *v != cof_version)
            && self.bake_retries < BAKE_RETRIES
        {
            self.bake_retries += 1;
            log::info!("outfit: server expects COF version {v}, asking again");
            self.cof_version = v;
            self.bake_requested = Some(v);
            self.commands.push(NetCommand::RequestServerAppearance { cof_version: v });
        }
    }

    pub fn update(&mut self, mut w: OutfitInputs, now: Instant) {
        match self.stage {
            Stage::Done => {}
            Stage::Waiting => {
                if !w.ready || w.inventory.folders.is_empty() {
                    return;
                }
                if w.first_login {
                    // the server dresses new residents (initial outfit)
                    log::info!("outfit: first login, nothing to restore");
                    self.stage = Stage::Done;
                    return;
                }
                let Some(cof) = w
                    .inventory
                    .folders
                    .values()
                    .find(|f| f.info.type_default == FT_CURRENT_OUTFIT && !f.library)
                    .map(|f| f.info.id)
                else {
                    log::warn!("outfit: no Current Outfit folder in the inventory");
                    self.stage = Stage::Done;
                    return;
                };
                if w.second_life {
                    self.commands.push(NetCommand::DummyWearablesUpdate);
                }
                self.cof = cof;
                self.refetch_cof(w.inventory);
                self.stage = Stage::FetchCof { since: now, attempts: 1 };
            }
            Stage::FetchCof { since, attempts } => {
                let state = w.inventory.folders.get(&self.cof).map(|f| f.state);
                match state {
                    Some(FetchState::Fetched) => {
                        self.cof_version = w.inventory.folders.get(&self.cof).map(|f| f.info.version).unwrap_or(-1);
                        self.objects = cof_objects(w.inventory, self.cof);
                        let missing: Vec<Uuid> = self
                            .objects
                            .iter()
                            .filter(|id| !w.inventory.items.contains_key(id))
                            .copied()
                            .collect();
                        log::info!(
                            "outfit: COF version {} links {} objects ({} to look up)",
                            self.cof_version,
                            self.objects.len(),
                            missing.len()
                        );
                        if missing.is_empty() {
                            self.stage = Stage::Settle { since: now };
                        } else {
                            self.commands.push(NetCommand::FetchItems {
                                items: missing,
                                owner: w.agent,
                            });
                            self.stage = Stage::FetchItems { since: now };
                        }
                    }
                    Some(FetchState::Failed) | Some(FetchState::Unknown) if attempts < 3 => {
                        if now.duration_since(since) > Duration::from_secs(3) {
                            self.refetch_cof(w.inventory);
                            self.stage = Stage::FetchCof {
                                since: now,
                                attempts: attempts + 1,
                            };
                        }
                    }
                    _ if now.duration_since(since) > FETCH_TIMEOUT * 2 => {
                        log::warn!("outfit: the Current Outfit folder could not be fetched");
                        self.stage = Stage::Done;
                    }
                    _ => {}
                }
            }
            Stage::FetchItems { since } => {
                let all = self.objects.iter().all(|id| w.inventory.items.contains_key(id));
                if all || now.duration_since(since) > FETCH_TIMEOUT {
                    // links to items that do not exist (deleted) are skipped
                    let before = self.objects.len();
                    self.objects.retain(|id| w.inventory.items.contains_key(id));
                    if self.objects.len() < before {
                        log::warn!("outfit: {} broken links in the Current Outfit folder", before - self.objects.len());
                    }
                    self.stage = Stage::Settle { since: now };
                }
            }
            Stage::Settle { since } => {
                if now.duration_since(since) < SETTLE {
                    return;
                }
                self.attach_missing(&mut w);
                self.request_bake(w.received_cof_version);
                self.stage = Stage::Attaching {
                    since: now,
                    retried: false,
                };
            }
            Stage::Attaching { since, retried } => {
                if now.duration_since(since) < RETRY_AFTER {
                    return;
                }
                let missing = self.objects.iter().filter(|id| !w.worn.contains(id)).count();
                if missing == 0 || retried {
                    log::info!(
                        "outfit: {} of {} attachments worn",
                        self.objects.len() - missing,
                        self.objects.len()
                    );
                    self.stage = Stage::Done;
                } else {
                    log::info!("outfit: {missing} attachments still missing, asking again");
                    self.requested.clear();
                    self.attach_missing(&mut w);
                    self.stage = Stage::Attaching { since: now, retried: true };
                }
            }
        }
    }

    fn refetch_cof(&self, inv: &mut Inventory) {
        if let Some(f) = inv.folders.get_mut(&self.cof) {
            f.state = FetchState::Unknown;
        }
        inv.request(self.cof);
    }

    fn attach_missing(&mut self, w: &mut OutfitInputs) {
        let list: Vec<AttachRequest> = self
            .objects
            .iter()
            .filter(|id| !w.worn.contains(id) && !self.requested.contains(id))
            .filter_map(|id| w.inventory.items.get(id))
            .map(|it| attach_request(it, w.agent))
            .collect();
        if list.is_empty() {
            return;
        }
        log::info!("outfit: wearing {} attachments", list.len());
        self.requested.extend(list.iter().map(|a| a.item_id));
        self.commands.push(NetCommand::RezAttachments(list));
    }

    /// LLAppearanceMgr::requestServerAppearanceUpdate (skipped when the
    /// server already sent an appearance for this COF version).
    fn request_bake(&mut self, received: i32) {
        if self.cof_version < 0 || self.cof_version <= received || self.bake_requested.is_some_and(|v| v >= self.cof_version) {
            return;
        }
        self.bake_requested = Some(self.cof_version);
        self.commands.push(NetCommand::RequestServerAppearance {
            cof_version: self.cof_version,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::inventory::{FolderContents, InvFolder};

    fn item(id: u128, parent: Uuid, asset_type: i32, inv_type: i32, target: u128) -> InvItem {
        InvItem {
            id: Uuid::from_u128(id),
            parent,
            name: format!("item {id}"),
            desc: String::new(),
            asset_type,
            inv_type,
            asset_id: Uuid::from_u128(target),
            flags: 0,
            favorite: false,
            creator: Uuid::nil(),
            created_at: 0,
            owner: Uuid::nil(),
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            thumbnail: Uuid::nil(),
            base_mask: 0x7fffffff,
            owner_mask: 0x7fffffff,
            last_owner: uuid::Uuid::nil(),
            group_id: uuid::Uuid::nil(),
            group_owned: false,
            sale_type: 0,
            sale_price: 0,
        }
    }

    #[test]
    fn restores_missing_attachments_and_requests_bake() {
        let agent = Uuid::from_u128(1);
        let root = Uuid::from_u128(10);
        let cof = Uuid::from_u128(11);
        let mut inv = Inventory {
            root,
            ..Default::default()
        };
        inv.folders.insert(
            cof,
            super::super::inventory::Folder {
                info: InvFolder {
                    id: cof,
                    parent: root,
                    name: "Current Outfit".into(),
                    type_default: FT_CURRENT_OUTFIT,
                    version: 5,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Unknown,
                library: false,
            },
        );
        let mut r = OutfitRestore::default();
        let t0 = Instant::now();
        fn inputs<'a>(inv: &'a mut Inventory, worn: &[u128]) -> OutfitInputs<'a> {
            OutfitInputs {
                inventory: inv,
                agent: Uuid::from_u128(1),
                ready: true,
                first_login: false,
                second_life: true,
                worn: worn.iter().map(|w| Uuid::from_u128(*w)).collect(),
                received_cof_version: 3,
            }
        }
        r.update(inputs(&mut inv, &[]), t0);
        assert!(matches!(r.commands[0], NetCommand::DummyWearablesUpdate));
        assert_eq!(inv.queue.len(), 1, "COF fetch queued");
        // COF: links to a body (object, already known), hair (object, unknown), a shape (body part)
        inv.items.insert(Uuid::from_u128(200), item(200, root, ASSET_OBJECT, INV_OBJECT, 0));
        inv.apply(vec![FolderContents {
            folder_id: cof,
            owner_id: agent,
            version: 7,
            folders: Vec::new(),
            items: vec![
                item(100, cof, ASSET_LINK, INV_OBJECT, 200),
                item(101, cof, ASSET_LINK, INV_ATTACHMENT, 201),
                item(102, cof, ASSET_LINK, 18, 202),
            ],
        }]);
        r.commands.clear();
        r.update(inputs(&mut inv, &[]), t0);
        match &r.commands[..] {
            [NetCommand::FetchItems { items, .. }] => assert_eq!(items, &vec![Uuid::from_u128(201)]),
            other => panic!("unexpected {other:?}"),
        }
        inv.items.insert(Uuid::from_u128(201), item(201, root, ASSET_OBJECT, INV_OBJECT, 0));
        r.commands.clear();
        r.update(inputs(&mut inv, &[]), t0);
        // settle, then wear what is not worn yet (200 is already on)
        r.update(inputs(&mut inv, &[200]), t0 + SETTLE + Duration::from_millis(1));
        match &r.commands[..] {
            [
                NetCommand::RezAttachments(list),
                NetCommand::RequestServerAppearance { cof_version: 7 },
            ] => {
                assert_eq!(list.len(), 1);
                assert_eq!(list[0].item_id, Uuid::from_u128(201));
                assert_eq!(list[0].owner_id, agent);
                assert!(list[0].add);
            }
            other => panic!("unexpected {other:?}"),
        }
        // everything worn after a while: done
        r.commands.clear();
        r.update(inputs(&mut inv, &[200, 201]), t0 + SETTLE + RETRY_AFTER * 2);
        assert!(r.is_done());
        assert!(r.commands.is_empty());
    }

    #[test]
    fn first_login_does_nothing() {
        let mut inv = Inventory::default();
        inv.folders.insert(
            Uuid::from_u128(11),
            super::super::inventory::Folder {
                info: InvFolder {
                    id: Uuid::from_u128(11),
                    parent: Uuid::nil(),
                    name: "COF".into(),
                    type_default: FT_CURRENT_OUTFIT,
                    version: 1,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Unknown,
                library: false,
            },
        );
        let mut r = OutfitRestore::default();
        r.update(
            OutfitInputs {
                inventory: &mut inv,
                agent: Uuid::from_u128(1),
                ready: true,
                first_login: true,
                second_life: true,
                worn: HashSet::new(),
                received_cof_version: 0,
            },
            Instant::now(),
        );
        assert!(r.is_done());
        assert!(r.commands.is_empty());
    }
}
