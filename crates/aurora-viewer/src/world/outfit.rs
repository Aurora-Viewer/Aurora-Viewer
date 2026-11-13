//! Restoring the outfit at login, as LLAppearanceMgr::updateAppearanceFromCOF
//! (callAfterCOFFetch in llstartup.cpp) does: fetch the Current Outfit
//! folder, wear the objects it links to that are not worn yet
//! (RezMultipleAttachmentsFromInv through LLAttachmentsMgr, attachment
//! point 0 = where each was last worn), then ask the server to bake the
//! outfit (UpdateAvatarAppearance with the COF version). Body parts and
//! clothing need no request: the server bakes them from the COF.
//!
//! At login the region puts back by itself part of the outfit (one object
//! per attachment point, the ones it remembers); the objects added on top
//! of them are the viewer's to request. Requests sent while the region is
//! still putting its own back race with it: it answers "Attempt to rez an
//! object failed" and the objects added on those points do not stay (a
//! busy region, 2026-10-27: 18 of 38 objects missing, exactly one left per
//! point). So the requests wait until the region has stopped bringing
//! objects back (`restore_settled`), then leave one at a time (aurora-net's
//! attachment queue); an object the region still takes off afterwards is
//! asked for again as soon as it is quiet, and keeps its COF link. The
//! Legacy Viewer requests at once and loses objects the same way now and
//! then (they have to be worn again by hand); the wait is the visible
//! difference (the added objects appear a few seconds later).

use super::inventory::{FetchState, Inventory};
use aurora_net::inventory::InvItem;
use aurora_net::{AttachRequest, NetCommand};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// LLFolderType::FT_CURRENT_OUTFIT.
pub const FT_CURRENT_OUTFIT: i32 = 46;
const ASSET_LINK: i32 = 24;
const ASSET_OBJECT: i32 = 6;
const INV_OBJECT: i32 = 6;
const INV_ATTACHMENT: i32 = 17;

/// Nothing is requested sooner than this after arriving in the region
/// (a quick relog can still have the objects on the region).
const SETTLE: Duration = Duration::from_secs(4);
/// The region's own restore is over once no object of the outfit has
/// arrived for this long.
const QUIET: Duration = Duration::from_secs(4);
/// No object arrived at all by then: the region restores nothing.
const NO_RESTORE: Duration = Duration::from_secs(12);
/// The requests leave by then whatever the region does.
const MAX_SETTLE: Duration = Duration::from_secs(30);
/// Requests that produced nothing are sent once more after this.
const RETRY_AFTER: Duration = Duration::from_secs(30);
/// LLAttachmentsMgr MIN_RETRY_REQUEST_TIME: an item requested this
/// recently is not requested again.
const MIN_RETRY: Duration = Duration::from_secs(5);
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const BAKE_RETRIES: u32 = 3;
/// LLFolderType::FT_TRASH. Descendants of trash are not an initial outfit
/// (LLInventoryModel::findDescendentCategoryIDByName, EXCLUDE_TRASH).
const FT_TRASH: i32 = 14;
/// LLFolderType::FT_CLOTHING, the copy destination for a library outfit.
const FT_CLOTHING: i32 = 5;
/// LLAgentWearables::createStandardWearables (llagentwearables.cpp):
/// shape, skin, hair, eyes, shirt, pants, shoes, socks, undershirt,
/// underpants. Jacket, gloves and skirt are not created.
const STANDARD_WEARABLES: [u8; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 10, 11];
/// Times the objects the region took off are asked for again.
const REWEAR_ROUNDS: u32 = 3;

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
    /// Waiting for the region to put back what it remembers.
    Settle,
    Attaching {
        since: Instant,
        retried: bool,
    },
    Done,
}

/// First login (LLStartUp::loadInitialOutfit, llstartup.cpp). Kept beside
/// `Stage` so the returning-resident restore is not entered, and so a later
/// tick does not start the search again.
#[derive(Debug, Clone)]
enum Initial {
    Decide,
    /// Folders asked for before the named outfit can be worn or given up.
    Wait {
        ids: Vec<Uuid>,
        cof_creating: bool,
    },
    /// A library copy was sent. Wear the non-library folder of this name under `dest`.
    Copy {
        name: String,
        dest: Uuid,
    },
    /// The standard wearables were created. Wear them once they are listed.
    Made {
        kinds: Vec<(u8, String)>,
    },
    Finished,
}

/// What the restore needs from the world each tick.
pub struct OutfitInputs<'a> {
    pub inventory: &'a mut Inventory,
    pub agent: Uuid,
    /// In a region with its capabilities.
    pub ready: bool,
    pub first_login: bool,
    pub second_life: bool,
    /// `initial-outfit[0].folder_name` from the login reply. Empty when
    /// the grid did not name one.
    pub initial_folder: &'a str,
    /// `initial-outfit[0].gender`. Only `"male"` selects a male shape;
    /// anything else, including empty, is female.
    pub initial_gender: &'a str,
    /// Inventory items attached to our avatar now (AttachItemID).
    pub worn: HashSet<Uuid>,
    /// COF version of the last AvatarAppearance received for us.
    pub received_cof_version: i32,
}

pub struct OutfitRestore {
    stage: Stage,
    cof: Uuid,
    cof_version: i32,
    /// When the region was ready: the clock of the login restore.
    ready_at: Option<Instant>,
    /// Objects the COF links to (target item ids).
    objects: Vec<Uuid>,
    /// Those of them worn at the last update, and when one last arrived.
    worn_objects: HashSet<Uuid>,
    last_arrival: Option<Instant>,
    /// Items requested and when (LLAttachmentsMgr::mAttachmentRequests).
    requested: HashMap<Uuid, Instant>,
    /// Requested objects that came off without being asked to (the
    /// region's own restore replacing them), to ask for again, and how many
    /// times that was done.
    knocked_off: Vec<Uuid>,
    rewears: u32,
    /// Items the resident (or a script, RLVa) asked to take off.
    user_removed: HashSet<Uuid>,
    /// Objects a COF change linked before the inventory knew their items:
    /// worn once FetchItems brings them, given up after FETCH_TIMEOUT.
    pending_wear: HashMap<Uuid, Instant>,
    bake_requested: Option<i32>,
    bake_retries: u32,
    /// Rebake: the COF version the server answered last, taken as received
    /// (syncCofVersionAndRefreshCoro slams mLastUpdateReceivedCOFVersion),
    /// and an IncrementCOFVersion request waiting for its answer.
    stepped_cof_version: i32,
    stepping: bool,
    pub commands: Vec<NetCommand>,
    initial: Initial,
    initial_since: Option<Instant>,
    /// `DummyWearablesUpdate` already queued for this first login.
    initial_dummy: bool,
    /// Current Outfit creation already sent.
    cof_create_sent: bool,
    /// Sync flag of the last `UpdateOutfit` this restore queued (`plan`'s bool).
    pub pending_sync: bool,
}

impl Default for OutfitRestore {
    fn default() -> Self {
        OutfitRestore {
            stage: Stage::Waiting,
            cof: Uuid::nil(),
            cof_version: -1,
            ready_at: None,
            objects: Vec::new(),
            worn_objects: HashSet::new(),
            last_arrival: None,
            requested: HashMap::new(),
            knocked_off: Vec::new(),
            rewears: 0,
            user_removed: HashSet::new(),
            pending_wear: HashMap::new(),
            bake_requested: None,
            bake_retries: 0,
            stepped_cof_version: -1,
            stepping: false,
            commands: Vec::new(),
            initial: Initial::Decide,
            initial_since: None,
            initial_dummy: false,
            cof_create_sent: false,
            pending_sync: false,
        }
    }
}

/// Target ids of the object links in the COF (the links are sorted by name
/// in the folder; duplicates, broken links and coalesced objects are
/// skipped). A link whose
/// target the inventory does not know yet counts when the link says object.
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
            // a link left to a coalesced object is never worn (#1668)
            Some(t) => t.asset_type == ASSET_OBJECT && !super::appearance::coalesced(t),
            None => matches!(link.inv_type, INV_OBJECT | INV_ATTACHMENT),
        };
        if is_object && !out.contains(&link.asset_id) {
            out.push(link.asset_id);
        }
    }
    out
}

/// The Current Outfit folder of the agent's inventory.
fn find_cof(inv: &Inventory) -> Option<Uuid> {
    inv.folders
        .values()
        .find(|f| f.info.type_default == FT_CURRENT_OUTFIT && !f.library)
        .map(|f| f.info.id)
}

/// RezMultipleAttachmentsFromInv for `item` where it was last worn, added
/// to what the point holds (LLAttachmentsMgr::addAttachmentRequest(id, 0, true)).
pub fn attach_request(item: &InvItem, agent: Uuid) -> AttachRequest {
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

/// Whether the region is done putting back the outfit by itself, from when
/// the region was ready and when an outfit object last arrived.
fn restore_settled(now: Instant, ready_at: Instant, last_arrival: Option<Instant>) -> bool {
    let waited = now.saturating_duration_since(ready_at);
    if waited < SETTLE {
        return false;
    }
    if waited >= MAX_SETTLE {
        return true;
    }
    match last_arrival {
        Some(at) => now.saturating_duration_since(at) >= QUIET,
        None => waited >= NO_RESTORE,
    }
}

/// Item names for the log, quoted and comma separated.
pub(super) fn names(inv: &Inventory, ids: &[Uuid]) -> String {
    ids.iter()
        .map(|id| format!("'{}'", inv.items.get(id).map(|it| it.name.as_str()).unwrap_or("?")))
        .collect::<Vec<_>>()
        .join(", ")
}

impl OutfitRestore {
    pub fn is_done(&self) -> bool {
        self.stage == Stage::Done
    }

    /// Nothing left to do: the login restore is over and no worn object
    /// waits for its item.
    pub fn is_idle(&self) -> bool {
        self.is_done() && self.pending_wear.is_empty()
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

    /// A restore that is already over, as after the login (tests of what
    /// happens later in the session).
    #[cfg(test)]
    pub fn finished() -> Self {
        OutfitRestore {
            stage: Stage::Done,
            ..Default::default()
        }
    }

    /// A detach was sent for these items: their coming off is wanted.
    pub fn on_detach_requested(&mut self, items: &[Uuid]) {
        self.user_removed.extend(items.iter().copied());
    }

    /// Whether the COF link of `item` stays when the region takes it off
    /// without the viewer asking for it (a detach the viewer sent, for the
    /// resident or RLVa, always removes it).
    ///
    /// Until the login restore has read the Current Outfit folder, no
    /// unrequested detach removes a link: LLAppearanceMgr::unregisterAttachment
    /// (indra/newview/llappearancemgr.cpp) leaves the COF alone while
    /// mAttachmentInvLinkEnabled is false, and only set_flags_and_update_appearance
    /// (indra/newview/llstartup.cpp) turns it on, after the COF fetch at login.
    /// A region that clears what it had kept from the last session (a relog
    /// after a crash) would otherwise unlink those objects for good, and the
    /// restore would not wear them again. Then, while the restore puts the
    /// outfit on, an outfit object the region replaced keeps its link and
    /// is worn again.
    pub fn keeps_cof_link(&self, item: Uuid) -> bool {
        if self.user_removed.contains(&item) {
            return false;
        }
        match self.stage {
            Stage::Waiting | Stage::FetchCof { .. } | Stage::FetchItems { .. } => true,
            Stage::Settle | Stage::Attaching { .. } => self.objects.contains(&item),
            Stage::Done => false,
        }
    }

    /// A COF change (Replace outfit, Add…) linked objects whose items the
    /// inventory does not know yet: they are looked up, then worn if they
    /// are still in the COF (callAfterCOFFetch fetches the links' items
    /// before updateAppearanceFromCOF wears them).
    pub fn wear_when_known(&mut self, items: Vec<Uuid>, agent: Uuid, now: Instant) {
        let new: Vec<Uuid> = items
            .into_iter()
            .filter(|id| self.pending_wear.insert(*id, now).is_none())
            .collect();
        if new.is_empty() {
            return;
        }
        log::info!("outfit: {} worn objects to look up before wearing them", new.len());
        self.commands.push(NetCommand::FetchItems { items: new, owner: agent });
    }

    pub fn update(&mut self, mut w: OutfitInputs, now: Instant) {
        self.wear_known(&mut w, now);
        match self.stage {
            Stage::Done => {}
            Stage::Waiting => {
                if !w.ready || w.inventory.folders.is_empty() {
                    return;
                }
                if w.first_login {
                    // LLStartUp::loadInitialOutfit (llstartup.cpp). Stays on
                    // Waiting until that outfit is worn or given up, so a
                    // later tick does not fall into the returning-resident path.
                    self.step_initial(&mut w, now);
                    return;
                }
                let Some(cof) = find_cof(w.inventory) else {
                    log::warn!("outfit: no Current Outfit folder in the inventory");
                    self.stage = Stage::Done;
                    return;
                };
                if w.second_life {
                    self.commands.push(NetCommand::DummyWearablesUpdate);
                }
                self.cof = cof;
                self.ready_at = Some(now);
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
                            "outfit: COF version {} links {} objects ({} to look up, {} already worn)",
                            self.cof_version,
                            self.objects.len(),
                            missing.len(),
                            self.objects.iter().filter(|id| w.worn.contains(id)).count()
                        );
                        if missing.is_empty() {
                            self.enter_settle(&w, now);
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
                    self.enter_settle(&w, now);
                }
            }
            Stage::Settle => {
                let _ = self.watch_worn(&w, now);
                let ready_at = self.ready_at.unwrap_or(now);
                let all_worn = self.worn_objects.len() == self.objects.len();
                if !all_worn && !restore_settled(now, ready_at, self.last_arrival) {
                    return;
                }
                log::info!(
                    "outfit: the region put back {} of {} objects by itself, {:.1} s after arriving",
                    self.worn_objects.len(),
                    self.objects.len(),
                    now.saturating_duration_since(ready_at).as_secs_f32()
                );
                self.attach_missing(&mut w, now);
                self.request_bake(w.received_cof_version);
                self.stage = Stage::Attaching {
                    since: now,
                    retried: false,
                };
            }
            Stage::Attaching { since, retried } => {
                let lost = self.watch_worn(&w, now);
                self.knocked_off.extend(
                    lost.into_iter()
                        .filter(|id| self.requested.contains_key(id) && !self.user_removed.contains(id)),
                );
                let quiet = self.last_arrival.is_none_or(|at| now.saturating_duration_since(at) >= QUIET);
                if !self.knocked_off.is_empty() && quiet && self.rewears < REWEAR_ROUNDS {
                    self.rewears += 1;
                    let list: Vec<AttachRequest> = std::mem::take(&mut self.knocked_off)
                        .iter()
                        .filter(|id| !w.worn.contains(id))
                        .filter_map(|id| w.inventory.items.get(id))
                        .map(|it| attach_request(it, w.agent))
                        .collect();
                    self.send_attach(w.inventory, list, now, "taken-off ");
                }
                if now.duration_since(since) < RETRY_AFTER {
                    return;
                }
                let missing: Vec<Uuid> = self
                    .objects
                    .iter()
                    .filter(|id| !w.worn.contains(id) && !self.user_removed.contains(id))
                    .copied()
                    .collect();
                if missing.is_empty() || retried {
                    let detail = if missing.is_empty() {
                        String::new()
                    } else {
                        format!(", missing {}", names(w.inventory, &missing))
                    };
                    log::info!(
                        "outfit: {} of {} attachments worn{detail}",
                        self.objects.len() - missing.len(),
                        self.objects.len()
                    );
                    self.stage = Stage::Done;
                } else {
                    log::info!(
                        "outfit: {} attachments still missing, asking again: {}",
                        missing.len(),
                        names(w.inventory, &missing)
                    );
                    self.attach_missing(&mut w, now);
                    self.stage = Stage::Attaching { since: now, retried: true };
                }
            }
        }
    }

    fn enter_settle(&mut self, w: &OutfitInputs, now: Instant) {
        self.worn_objects = self.objects.iter().filter(|id| w.worn.contains(id)).copied().collect();
        // objects already back: the region's restore is under way
        self.last_arrival = (!self.worn_objects.is_empty()).then_some(now);
        self.stage = Stage::Settle;
    }

    /// Notes when the outfit objects worn last changed (an object arriving
    /// or replaced: the region's restore goes on) and returns (and logs)
    /// those that came off.
    fn watch_worn(&mut self, w: &OutfitInputs, now: Instant) -> Vec<Uuid> {
        let worn: HashSet<Uuid> = self.objects.iter().filter(|id| w.worn.contains(id)).copied().collect();
        let lost: Vec<Uuid> = self.worn_objects.difference(&worn).copied().collect();
        if !lost.is_empty() || worn.difference(&self.worn_objects).next().is_some() {
            self.last_arrival = Some(now);
        }
        if !lost.is_empty() {
            log::warn!("outfit: {} came off while the outfit was put on", names(w.inventory, &lost));
        }
        self.worn_objects = worn;
        lost
    }

    fn refetch_cof(&self, inv: &mut Inventory) {
        if let Some(f) = inv.folders.get_mut(&self.cof) {
            f.state = FetchState::Unknown;
        }
        inv.request(self.cof);
    }

    /// Requests the outfit objects not worn yet, except those asked for
    /// less than MIN_RETRY ago (LLAttachmentsMgr::addAttachmentRequest).
    fn attach_missing(&mut self, w: &mut OutfitInputs, now: Instant) {
        let list: Vec<AttachRequest> = self
            .objects
            .iter()
            .filter(|id| !w.worn.contains(id) && !self.user_removed.contains(id))
            .filter(|id| {
                !self
                    .requested
                    .get(id)
                    .is_some_and(|at| now.saturating_duration_since(*at) < MIN_RETRY)
            })
            .filter_map(|id| w.inventory.items.get(id))
            .map(|it| attach_request(it, w.agent))
            .collect();
        self.send_attach(w.inventory, list, now, "");
    }

    /// Wears the objects of `wear_when_known` whose items have arrived.
    fn wear_known(&mut self, w: &mut OutfitInputs, now: Instant) {
        if self.pending_wear.is_empty() {
            return;
        }
        let cof = if self.cof.is_nil() { find_cof(w.inventory) } else { Some(self.cof) };
        let linked: HashSet<Uuid> = cof
            .map(|cof| cof_objects(w.inventory, cof))
            .unwrap_or_default()
            .into_iter()
            .collect();
        let mut list = Vec::new();
        let mut lost = 0;
        self.pending_wear.retain(|id, since| {
            // taken out of the outfit, or worn meanwhile
            if !linked.contains(id) || w.worn.contains(id) {
                return false;
            }
            if let Some(item) = w.inventory.items.get(id) {
                list.push(attach_request(item, w.agent));
                return false;
            }
            if now.saturating_duration_since(*since) > FETCH_TIMEOUT {
                lost += 1;
                return false;
            }
            true
        });
        if lost > 0 {
            log::warn!("outfit: {lost} worn objects could not be looked up, not worn");
        }
        self.send_attach(w.inventory, list, now, "looked-up ");
    }

    fn send_attach(&mut self, inv: &Inventory, list: Vec<AttachRequest>, now: Instant, kind: &str) {
        if list.is_empty() {
            return;
        }
        let ids: Vec<Uuid> = list.iter().map(|a| a.item_id).collect();
        log::info!("outfit: wearing {} {kind}attachments: {}", list.len(), names(inv, &ids));
        self.requested.extend(ids.iter().map(|id| (*id, now)));
        self.commands.push(NetCommand::RezAttachments(list));
    }

    /// "Force appearance update (rebake)", the server side of
    /// handle_rebake_textures (llviewermenu.cpp): with the IncrementCOFVersion
    /// capability, the server steps the Current Outfit folder's version
    /// (syncCofVersionAndRefresh) and the bake request follows its answer
    /// (`on_cof_stepped`); without it, a bake request for the folder's
    /// version as it is (requestServerAppearanceUpdate), skipped when the
    /// server already sent an appearance for it, as there.
    pub fn rebake(&mut self, inv: &Inventory, step_capability: bool, received: i32) {
        if step_capability {
            if !self.stepping {
                self.stepping = true;
                self.commands.push(NetCommand::IncrementCofVersion);
            }
        } else {
            self.follow_cof(inv);
            self.request_bake(received);
        }
    }

    /// The server answered the IncrementCOFVersion request: its version is
    /// the folder's and counts as received (the viewer's own tends to be
    /// ahead), then a bake is requested even when it failed, as
    /// syncCofVersionAndRefreshCoro does.
    pub fn on_cof_stepped(&mut self, version: Option<i32>, inv: &mut Inventory, received: i32) {
        self.stepping = false;
        self.follow_cof(inv);
        if let Some(v) = version {
            if let Some(f) = inv.folders.get_mut(&self.cof) {
                f.info.version = v;
            }
            self.cof_version = v;
            self.stepped_cof_version = v;
            self.bake_requested = Some(v);
        }
        self.request_bake(received);
    }

    /// The COF version the last IncrementCOFVersion answer gave, -1 before.
    pub fn stepped_cof_version(&self) -> i32 {
        self.stepped_cof_version
    }

    /// The Current Outfit folder and its version as the inventory has them
    /// now: the outfit may have changed since the login restore.
    fn follow_cof(&mut self, inv: &Inventory) {
        if self.cof.is_nil()
            && let Some(f) = inv
                .folders
                .values()
                .find(|f| f.info.type_default == FT_CURRENT_OUTFIT && !f.library)
        {
            self.cof = f.info.id;
        }
        if let Some(f) = inv.folders.get(&self.cof) {
            self.cof_version = f.info.version;
        }
    }

    /// LLAppearanceMgr::requestServerAppearanceUpdate (skipped when the
    /// server already sent an appearance for this COF version).
    fn request_bake(&mut self, received: i32) {
        let received = received.max(self.stepped_cof_version);
        if self.cof_version < 0 || self.cof_version <= received || self.bake_requested.is_some_and(|v| v >= self.cof_version) {
            return;
        }
        self.bake_requested = Some(self.cof_version);
        self.commands.push(NetCommand::RequestServerAppearance {
            cof_version: self.cof_version,
        });
    }

    /// LLStartUp::loadInitialOutfit (llstartup.cpp): wear the named folder
    /// under My Outfits, else copy it from the library and wear the copy
    /// (LLAppearanceMgr::wearInventoryCategory), else
    /// LLAgentWearables::createStandardWearables. The dummy wearables update
    /// goes out once, on Second Life only, when that choice is made.
    ///
    /// An empty folder name takes the standard wearables. The Legacy Viewer
    /// skips `loadInitialOutfit` instead; the reported bug is a naked
    /// avatar, and the standard wearables are what dress one when no folder
    /// was named. A named outfit keeps the gender of its own shape: Legacy
    /// `setSex` lasts only until that shape loads, and this module has no
    /// separate avatar sex. Only a generated shape sets param 80.
    fn step_initial(&mut self, w: &mut OutfitInputs<'_>, now: Instant) {
        let since = *self.initial_since.get_or_insert(now);
        let timed_out = now.saturating_duration_since(since) > FETCH_TIMEOUT;
        match self.initial.clone() {
            Initial::Finished => self.stage = Stage::Done,
            Initial::Wait { ids, cof_creating } => {
                let pending = ids.iter().any(|id| folder_pending(w.inventory, *id));
                let cof_ready = !cof_creating || find_cof(w.inventory).is_some_and(|id| folder_fetched(w.inventory, id));
                if (pending || !cof_ready) && !timed_out {
                    return;
                }
                if cof_creating && find_cof(w.inventory).is_none() {
                    log::warn!("outfit: the Current Outfit folder was not created");
                    self.finish_initial();
                    return;
                }
                if find_cof(w.inventory).is_some_and(|id| !folder_fetched(w.inventory, id)) && timed_out {
                    log::warn!("outfit: the Current Outfit folder could not be fetched");
                    self.finish_initial();
                    return;
                }
                self.decide_initial(w, !timed_out);
            }
            Initial::Copy { name, dest } => self.poll_copy(w, &name, dest, timed_out),
            Initial::Made { kinds } => self.poll_made(w, &kinds, timed_out),
            Initial::Decide => self.decide_initial(w, !timed_out),
        }
    }

    fn decide_initial(&mut self, w: &mut OutfitInputs<'_>, allow_wait: bool) {
        let Some(cof) = find_cof(w.inventory) else {
            if allow_wait && !self.cof_create_sent {
                self.create_cof(w);
                self.cof_create_sent = true;
                self.initial = Initial::Wait {
                    ids: Vec::new(),
                    cof_creating: true,
                };
                return;
            }
            log::warn!("outfit: no Current Outfit folder for the initial outfit");
            self.finish_initial();
            return;
        };
        if !folder_fetched(w.inventory, cof) {
            if allow_wait {
                self.wait_for(w, vec![cof]);
            } else {
                log::warn!("outfit: the Current Outfit folder could not be fetched");
                self.finish_initial();
            }
            return;
        }
        let name = w.initial_folder.trim();
        if name.is_empty() {
            log::info!("outfit: first login with no initial folder name; creating the standard wearables");
            self.create_standard(w);
            return;
        }
        let my = super::appearance::system_folder(w.inventory, super::appearance::FT_MY_OUTFITS);
        if let Some(id) = find_named(w.inventory, my, name) {
            self.wear_named(w, id, allow_wait);
            return;
        }
        if allow_wait && listing_pending(w.inventory, my) {
            self.wait_for(w, vec![my.expect("listing pending")]);
            return;
        }
        let lib = (!w.inventory.lib_root.is_nil()).then_some(w.inventory.lib_root);
        if let Some(id) = find_named(w.inventory, lib, name) {
            self.copy_named(w, id, name, allow_wait);
            return;
        }
        if allow_wait && listing_pending(w.inventory, lib) {
            self.wait_for(w, vec![lib.expect("listing pending")]);
            return;
        }
        log::info!("outfit: initial folder '{name}' was not found; creating the standard wearables");
        self.create_standard(w);
    }

    fn wear_named(&mut self, w: &mut OutfitInputs<'_>, folder: Uuid, allow_wait: bool) {
        let missing = unfetched_tree(w.inventory, folder);
        if !missing.is_empty() {
            if allow_wait {
                self.wait_for(w, missing);
            } else {
                log::info!("outfit: the initial outfit folder did not finish loading; creating the standard wearables");
                self.create_standard(w);
            }
            return;
        }
        match wearable_nonempty(w.inventory, folder) {
            Some(true) => {
                if self.wear_folder(w, folder) {
                    self.finish_initial();
                } else if allow_wait {
                    self.wait_for(w, vec![folder]);
                } else {
                    self.create_standard(w);
                }
            }
            Some(false) => {
                log::info!("outfit: the initial outfit folder has nothing to wear; creating the standard wearables");
                self.create_standard(w);
            }
            None if allow_wait => self.wait_for(w, vec![folder]),
            None => self.create_standard(w),
        }
    }

    fn copy_named(&mut self, w: &mut OutfitInputs<'_>, folder: Uuid, name: &str, allow_wait: bool) {
        let missing = unfetched_tree(w.inventory, folder);
        if !missing.is_empty() {
            if allow_wait {
                self.wait_for(w, missing);
            } else {
                log::info!("outfit: the library outfit did not finish loading; creating the standard wearables");
                self.create_standard(w);
            }
            return;
        }
        let dest = super::appearance::system_folder(w.inventory, FT_CLOTHING).unwrap_or(w.inventory.root);
        match super::inventory::actions::copy_tree(w.inventory, folder, dest) {
            Ok(change) => {
                log::info!("outfit: copying the library outfit '{name}'");
                self.commands.push(NetCommand::EditInventory {
                    request: Uuid::new_v4(),
                    change,
                });
                self.note_dummy(w.second_life);
                self.initial = Initial::Copy {
                    name: name.to_owned(),
                    dest,
                };
            }
            Err(reason) => {
                log::warn!("outfit: the library outfit was not copied: {reason}");
                self.create_standard(w);
            }
        }
    }

    fn poll_copy(&mut self, w: &mut OutfitInputs<'_>, name: &str, dest: Uuid, timed_out: bool) {
        if let Some(folder) = copied_folder(w.inventory, name, dest) {
            let missing = unfetched_tree(w.inventory, folder);
            if missing.is_empty() && wearable_nonempty(w.inventory, folder) == Some(true) {
                if !self.wear_folder(w, folder) {
                    log::warn!("outfit: the copied initial outfit was not worn");
                }
                self.finish_initial();
                return;
            }
            if !timed_out {
                let ids = if missing.is_empty() { vec![folder] } else { missing };
                for id in ids {
                    w.inventory.request(id);
                }
                return;
            }
        } else if !timed_out {
            if !dest.is_nil() {
                w.inventory.request(dest);
            }
            return;
        }
        log::warn!("outfit: the library outfit copy did not arrive; creating the standard wearables");
        self.create_standard(w);
    }

    fn poll_made(&mut self, w: &mut OutfitInputs<'_>, kinds: &[(u8, String)], timed_out: bool) {
        let found: Vec<Uuid> = kinds
            .iter()
            .filter_map(|(kind, name)| find_created(w.inventory, *kind, name))
            .collect();
        if found.len() == kinds.len() {
            if !self.wear_items(w, found) {
                log::warn!("outfit: the standard wearables were not worn");
            }
            self.finish_initial();
            return;
        }
        if timed_out {
            log::warn!("outfit: the standard wearables did not all arrive");
            if !found.is_empty() && !self.wear_items(w, found) {
                log::warn!("outfit: the standard wearables that arrived were not worn");
            }
            self.finish_initial();
        }
    }

    fn wait_for(&mut self, w: &mut OutfitInputs<'_>, ids: Vec<Uuid>) {
        for id in &ids {
            w.inventory.request(*id);
        }
        self.initial = Initial::Wait { ids, cof_creating: false };
    }

    fn wear_folder(&mut self, w: &OutfitInputs<'_>, folder: Uuid) -> bool {
        let worn = HashMap::new();
        match super::appearance::plan(
            w.inventory,
            super::appearance::Action::WearContents {
                folder,
                mode: super::appearance::WearMode::ReplaceOutfit,
            },
            &worn,
        ) {
            Ok((change, sync)) => {
                self.pending_sync = sync;
                self.commands.push(NetCommand::UpdateOutfit {
                    request: Uuid::new_v4(),
                    change,
                });
                self.note_dummy(w.second_life);
                log::info!("outfit: wearing the initial outfit folder {folder}");
                true
            }
            Err(reason) => {
                log::warn!("outfit: the initial outfit was not worn: {reason}");
                false
            }
        }
    }

    fn wear_items(&mut self, w: &OutfitInputs<'_>, items: Vec<Uuid>) -> bool {
        let worn = HashMap::new();
        match super::appearance::plan(
            w.inventory,
            super::appearance::Action::WearItems {
                items,
                replace: true,
                point: 0,
            },
            &worn,
        ) {
            Ok((change, sync)) => {
                self.pending_sync = sync;
                self.commands.push(NetCommand::UpdateOutfit {
                    request: Uuid::new_v4(),
                    change,
                });
                true
            }
            Err(reason) => {
                log::warn!("outfit: the standard wearables were not worn: {reason}");
                false
            }
        }
    }

    fn create_cof(&mut self, w: &OutfitInputs<'_>) {
        let id = Uuid::new_v4();
        let parent = w.inventory.root;
        log::info!("outfit: creating the Current Outfit folder");
        self.commands.push(NetCommand::EditInventory {
            request: Uuid::new_v4(),
            change: aurora_net::inventory::operations::Mutation {
                operations: vec![aurora_net::inventory::operations::Operation::Folder {
                    id,
                    parent,
                    name: aurora_i18n::tr!("common-current-outfit").to_owned(),
                    type_default: FT_CURRENT_OUTFIT,
                }],
                refresh: vec![parent],
                ..Default::default()
            },
        });
    }

    fn create_standard(&mut self, w: &OutfitInputs<'_>) {
        let names = super::appearance::wearables();
        let male = w.initial_gender == "male";
        let mut pending = Vec::new();
        for kind in STANDARD_WEARABLES {
            let name = names.get(usize::from(kind)).copied().unwrap_or("Clothing").to_owned();
            match super::inventory::wearable::create(kind, &name, w.agent) {
                Ok(data) => {
                    let data = if kind == 0 && male { male_shape(data) } else { data };
                    self.commands.push(NetCommand::CreateInventoryWearable {
                        parent: Uuid::nil(),
                        kind,
                        name: name.clone(),
                        data,
                        permissions: None,
                    });
                    pending.push((kind, name));
                }
                Err(reason) => log::warn!("outfit: standard wearable {kind} was not created: {reason}"),
            }
        }
        self.note_dummy(w.second_life);
        if pending.is_empty() {
            self.finish_initial();
        } else {
            self.initial = Initial::Made { kinds: pending };
        }
    }

    fn note_dummy(&mut self, second_life: bool) {
        if second_life && !self.initial_dummy {
            self.commands.push(NetCommand::DummyWearablesUpdate);
            self.initial_dummy = true;
        }
    }

    fn finish_initial(&mut self) {
        self.initial = Initial::Finished;
        self.stage = Stage::Done;
    }
}

fn folder_fetched(inv: &Inventory, id: Uuid) -> bool {
    inv.folders.get(&id).is_some_and(|f| f.state == FetchState::Fetched)
}

fn folder_pending(inv: &Inventory, id: Uuid) -> bool {
    matches!(
        inv.folders.get(&id).map(|f| f.state),
        Some(FetchState::Unknown | FetchState::Fetching)
    )
}

/// A skeleton can list children before the folder itself is fetched. An
/// empty folder that is still unread may gain children, so the name is not
/// treated as missing yet.
fn listing_pending(inv: &Inventory, id: Option<Uuid>) -> bool {
    id.is_some_and(|id| {
        inv.folders
            .get(&id)
            .is_some_and(|f| matches!(f.state, FetchState::Unknown | FetchState::Fetching) && f.children.is_empty() && f.items.is_empty())
    })
}

/// First descendant with this name, left to right, not inside trash.
/// LLNameCategoryCollector compares without case (llinventoryfunctions.cpp).
fn find_named(inv: &Inventory, root: Option<Uuid>, name: &str) -> Option<Uuid> {
    let mut stack = Vec::new();
    if let Some(f) = root.and_then(|id| inv.folders.get(&id)) {
        stack.extend(f.children.iter().rev().copied());
    }
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(f) = inv.folders.get(&id) else { continue };
        if f.info.type_default == FT_TRASH {
            continue;
        }
        if f.info.name.eq_ignore_ascii_case(name) {
            return Some(id);
        }
        stack.extend(f.children.iter().rev().copied());
    }
    None
}

fn unfetched_tree(inv: &Inventory, root: Uuid) -> Vec<Uuid> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(f) = inv.folders.get(&id) else { continue };
        if f.info.type_default == FT_TRASH {
            continue;
        }
        if f.state != FetchState::Fetched {
            out.push(id);
        }
        stack.extend(f.children.iter().copied());
    }
    out
}

fn wearable_nonempty(inv: &Inventory, folder: Uuid) -> Option<bool> {
    match super::appearance::wearable_contents(inv, folder) {
        Ok(links) => Some(!links.is_empty()),
        Err(_) => None,
    }
}

fn copied_folder(inv: &Inventory, name: &str, dest: Uuid) -> Option<Uuid> {
    let mut stack = inv.folders.get(&dest).map(|f| f.children.clone()).unwrap_or_default();
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(f) = inv.folders.get(&id) else { continue };
        if f.library || f.info.type_default == FT_TRASH {
            continue;
        }
        if f.info.name.eq_ignore_ascii_case(name) {
            return Some(id);
        }
        stack.extend(f.children.iter().copied());
    }
    None
}

fn find_created(inv: &Inventory, kind: u8, name: &str) -> Option<Uuid> {
    let asset = if kind < 4 { 13 } else { 5 };
    let ids: Vec<Uuid> = inv
        .items
        .values()
        .filter(|it| it.name.eq_ignore_ascii_case(name) && it.asset_type == asset && (it.flags & 0xff) as u8 == kind)
        .map(|it| it.id)
        .collect();
    ids.into_iter().find(|id| !super::inventory::actions::library(inv, *id))
}

/// Param 80 is "male" on the shape (avatar_lad.xml). `create` writes the
/// default 0. Legacy `setSex` uses gender == "male"; everything else is female.
fn male_shape(data: Vec<u8>) -> Vec<u8> {
    let Ok(text) = String::from_utf8(data) else {
        return Vec::new();
    };
    let mut out = String::new();
    let mut saw = false;
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        if bare.starts_with("80 ") {
            out.push_str("80 1\n");
            saw = true;
        } else {
            out.push_str(line);
        }
    }
    if !saw {
        out.push_str("80 1\n");
    }
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::inventory::{FolderContents, InvFolder, InvItem};

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
            favorite_known: false,
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

    fn inputs<'a>(inv: &'a mut Inventory, worn: &[u128]) -> OutfitInputs<'a> {
        OutfitInputs {
            inventory: inv,
            agent: Uuid::from_u128(1),
            ready: true,
            first_login: false,
            second_life: true,
            initial_folder: "",
            initial_gender: "",
            worn: worn.iter().map(|w| Uuid::from_u128(*w)).collect(),
            received_cof_version: 3,
        }
    }

    /// An inventory whose Current Outfit folder (11, version 5) is not
    /// fetched yet, under the root (10).
    fn unfetched_cof() -> Inventory {
        let root = Uuid::from_u128(10);
        let mut inv = Inventory {
            root,
            ..Default::default()
        };
        inv.folders.insert(
            Uuid::from_u128(11),
            super::super::inventory::Folder {
                info: InvFolder {
                    id: Uuid::from_u128(11),
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
        inv
    }

    /// The COF fetched at version 7 with links (100 + n) to the objects
    /// `objects` (known items).
    fn fetch_cof(inv: &mut Inventory, objects: &[u128]) {
        let root = Uuid::from_u128(10);
        let cof = Uuid::from_u128(11);
        for id in objects {
            inv.items.insert(Uuid::from_u128(*id), item(*id, root, ASSET_OBJECT, INV_OBJECT, 0));
        }
        inv.apply(vec![FolderContents {
            folder_id: cof,
            owner_id: Uuid::from_u128(1),
            version: 7,
            folders: Vec::new(),
            items: objects.iter().map(|id| item(id - 100, cof, ASSET_LINK, INV_OBJECT, *id)).collect(),
        }]);
    }

    fn requested(commands: &[NetCommand]) -> Vec<u128> {
        let mut ids: Vec<u128> = commands
            .iter()
            .filter_map(|c| match c {
                NetCommand::RezAttachments(list) => Some(list.iter().map(|a| a.item_id.as_u128())),
                _ => None,
            })
            .flatten()
            .collect();
        ids.sort();
        ids
    }

    #[test]
    fn restores_missing_attachments_and_requests_bake() {
        let agent = Uuid::from_u128(1);
        let root = Uuid::from_u128(10);
        let cof = Uuid::from_u128(11);
        let mut inv = unfetched_cof();
        let mut r = OutfitRestore::default();
        let t0 = Instant::now();
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
        // the region puts the body back by itself after 2 s: the hair waits
        // until nothing has arrived for QUIET
        let back = t0 + Duration::from_secs(2);
        r.update(inputs(&mut inv, &[200]), back);
        r.update(inputs(&mut inv, &[200]), back + QUIET - Duration::from_millis(1));
        assert!(r.commands.is_empty(), "{:?}", r.commands);
        r.update(inputs(&mut inv, &[200]), back + QUIET);
        match &r.commands[..] {
            [
                NetCommand::RezAttachments(list),
                NetCommand::RequestServerAppearance { cof_version: 7 },
            ] => {
                assert_eq!(list.len(), 1);
                assert_eq!(list[0].item_id, Uuid::from_u128(201));
                assert_eq!(list[0].owner_id, agent);
                assert_eq!(list[0].point, 0);
                assert!(list[0].add, "added to what the point holds");
            }
            other => panic!("unexpected {other:?}"),
        }
        // everything worn after a while: done
        r.commands.clear();
        r.update(inputs(&mut inv, &[200, 201]), back + QUIET + RETRY_AFTER);
        assert!(r.is_done());
        assert!(r.commands.is_empty());
    }

    #[test]
    fn the_region_restore_settles_on_quiet_or_after_the_limits() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        // never before SETTLE, even with every object quiet
        assert!(!restore_settled(t0 + s(3), t0, Some(t0)));
        // objects arriving: QUIET after the last one
        assert!(!restore_settled(t0 + s(7), t0, Some(t0 + s(5))));
        assert!(restore_settled(t0 + s(9), t0, Some(t0 + s(5))));
        // nothing back yet: the region may not have started its restore
        assert!(!restore_settled(t0 + s(8), t0, None));
        assert!(restore_settled(t0 + NO_RESTORE, t0, None));
        // a restore that keeps going is not waited for past MAX_SETTLE
        assert!(restore_settled(t0 + MAX_SETTLE, t0, Some(t0 + MAX_SETTLE - s(1))));
    }

    /// A busy region, cold inventory cache (2026-10-27): the outfit is known
    /// before the region has put anything back. Nothing is requested then;
    /// once the region has put its objects back, only the others are.
    #[test]
    fn nothing_is_requested_while_the_region_restores_its_own() {
        let mut inv = unfetched_cof();
        let mut r = OutfitRestore::default();
        let t0 = Instant::now();
        r.update(inputs(&mut inv, &[]), t0);
        fetch_cof(&mut inv, &[200, 201, 202, 203]);
        r.update(inputs(&mut inv, &[]), t0 + Duration::from_secs(2));
        r.commands.clear();
        let mut at = t0 + Duration::from_secs(2);
        let step = Duration::from_millis(500);
        // the region starts bringing its objects back at 8 s, one every 3 s
        let mut worn: Vec<u128> = Vec::new();
        for (when, id) in [(8, 200), (11, 201)] {
            while at < t0 + Duration::from_secs(when) {
                r.update(inputs(&mut inv, &worn), at);
                at += step;
            }
            worn.push(id);
        }
        assert!(r.commands.is_empty(), "requested during the region's restore: {:?}", r.commands);
        while r.commands.is_empty() && at < t0 + MAX_SETTLE {
            r.update(inputs(&mut inv, &worn), at);
            at += step;
        }
        assert_eq!(requested(&r.commands), vec![202, 203]);
        assert!(at >= t0 + Duration::from_secs(11) + QUIET, "quiet after the last arrival");
    }

    #[test]
    fn missing_objects_are_asked_again_once_then_given_up() {
        let mut inv = unfetched_cof();
        let mut r = OutfitRestore::default();
        let t0 = Instant::now();
        r.update(inputs(&mut inv, &[]), t0);
        fetch_cof(&mut inv, &[200, 201, 202]);
        r.update(inputs(&mut inv, &[200]), t0);
        r.commands.clear();
        let asked = t0 + SETTLE.max(QUIET);
        r.update(inputs(&mut inv, &[200]), asked);
        assert_eq!(requested(&r.commands), vec![201, 202]);
        r.commands.clear();
        // 201 arrived; 202 did not: asked again once
        r.update(inputs(&mut inv, &[200, 201]), asked + RETRY_AFTER);
        assert_eq!(requested(&r.commands), vec![202]);
        r.commands.clear();
        r.update(inputs(&mut inv, &[200, 201]), asked + RETRY_AFTER * 2);
        assert!(r.is_done());
        assert!(r.commands.is_empty());
    }

    /// The region puts its own object back on a point after the requests
    /// left (a pause longer than QUIET): the object added there comes off.
    /// It keeps its COF link and is asked for again once the region is
    /// quiet; what the resident takes off is left off.
    #[test]
    fn objects_the_region_takes_off_are_worn_again_but_not_those_taken_off_on_purpose() {
        let mut inv = unfetched_cof();
        let mut r = OutfitRestore::default();
        let t0 = Instant::now();
        r.update(inputs(&mut inv, &[]), t0);
        fetch_cof(&mut inv, &[200, 201, 202, 203]);
        r.update(inputs(&mut inv, &[200]), t0);
        r.commands.clear();
        let asked = t0 + SETTLE;
        r.update(inputs(&mut inv, &[200]), asked);
        assert_eq!(requested(&r.commands), vec![201, 202, 203]);
        r.commands.clear();
        r.update(inputs(&mut inv, &[200, 201, 202, 203]), asked + Duration::from_secs(1));
        // the resident takes 203 off; the region's late object replaces 201
        r.on_detach_requested(&[Uuid::from_u128(203)]);
        assert!(r.keeps_cof_link(Uuid::from_u128(201)));
        assert!(!r.keeps_cof_link(Uuid::from_u128(203)));
        let late = asked + Duration::from_secs(2);
        r.update(inputs(&mut inv, &[200, 202]), late);
        r.update(inputs(&mut inv, &[200, 202]), late + QUIET - Duration::from_millis(1));
        assert!(r.commands.is_empty(), "not while the region is busy: {:?}", r.commands);
        r.update(inputs(&mut inv, &[200, 202]), late + QUIET);
        assert_eq!(requested(&r.commands), vec![201]);
        r.commands.clear();
        r.update(inputs(&mut inv, &[200, 201, 202]), asked + RETRY_AFTER);
        assert!(r.is_done(), "203 is not missing: it was taken off on purpose");
        assert!(r.commands.is_empty());
        assert!(!r.keeps_cof_link(Uuid::from_u128(201)), "the restore is over");
    }

    /// Relog after a crash: the region clears what it kept of the last
    /// session before the restore has read the COF. No link is removed then
    /// (mAttachmentInvLinkEnabled), and the object is worn again; once the
    /// restore is over, an unrequested detach removes the link as usual.
    #[test]
    fn no_link_is_removed_before_the_cof_is_read_and_the_object_is_worn_again() {
        let mut inv = unfetched_cof();
        let mut r = OutfitRestore::default();
        let (body, ring, taken_off, unlinked) = (
            Uuid::from_u128(200),
            Uuid::from_u128(201),
            Uuid::from_u128(203),
            Uuid::from_u128(250),
        );
        assert!(r.keeps_cof_link(ring), "waiting for the region");
        let t0 = Instant::now();
        r.update(inputs(&mut inv, &[200, 201]), t0);
        assert!(matches!(r.stage, Stage::FetchCof { .. }));
        assert!(r.keeps_cof_link(ring), "the COF is not read yet");
        assert!(r.keeps_cof_link(unlinked), "the COF is not read yet: its links are not known");
        // a detach asked for (resident, script, RLVa) still removes its link
        r.on_detach_requested(&[taken_off]);
        assert!(!r.keeps_cof_link(taken_off));
        // the ring came off meanwhile; the COF still links it
        fetch_cof(&mut inv, &[200, 201, 202, 203]);
        r.update(inputs(&mut inv, &[200]), t0);
        assert!(matches!(r.stage, Stage::Settle));
        assert!(r.keeps_cof_link(ring));
        assert!(!r.keeps_cof_link(unlinked), "not in the outfit");
        r.commands.clear();
        r.update(inputs(&mut inv, &[200]), t0 + SETTLE);
        assert_eq!(
            requested(&r.commands),
            vec![201, 202],
            "the ring is worn again, not what was taken off"
        );
        r.update(inputs(&mut inv, &[200, 201, 202]), t0 + SETTLE + RETRY_AFTER);
        assert!(r.is_done());
        assert!(!r.keeps_cof_link(body), "after the login, a detach removes the link");
        assert!(!r.keeps_cof_link(ring));
    }

    /// Replace outfit with a cold inventory cache: the new COF links to
    /// objects the inventory has not fetched; they are looked up, then worn
    /// if the COF still links to them.
    #[test]
    fn objects_linked_before_their_items_are_known_are_worn_once_fetched() {
        let root = Uuid::from_u128(10);
        let cof = Uuid::from_u128(11);
        let mut inv = unfetched_cof();
        let mut r = OutfitRestore::default();
        let t0 = Instant::now();
        // the login restore is over (nothing to restore); a first login now
        // runs the initial outfit (#3411), so set the end state directly
        r.stage = Stage::Done;
        assert!(r.is_idle());
        inv.apply(vec![FolderContents {
            folder_id: cof,
            owner_id: Uuid::from_u128(1),
            version: 9,
            folders: Vec::new(),
            items: vec![item(100, cof, ASSET_LINK, INV_OBJECT, 300)],
        }]);
        r.wear_when_known(vec![Uuid::from_u128(300), Uuid::from_u128(301)], Uuid::from_u128(1), t0);
        assert!(!r.is_idle());
        match &r.commands[..] {
            [NetCommand::FetchItems { items, .. }] => assert_eq!(items.len(), 2),
            other => panic!("unexpected {other:?}"),
        }
        r.commands.clear();
        // 301 is no longer linked: dropped; 300 waits for its item
        r.update(inputs(&mut inv, &[]), t0);
        assert!(r.commands.is_empty());
        inv.items.insert(Uuid::from_u128(300), item(300, root, ASSET_OBJECT, INV_OBJECT, 0));
        r.update(inputs(&mut inv, &[]), t0 + Duration::from_secs(1));
        assert_eq!(requested(&r.commands), vec![300]);
        assert!(r.is_idle());
    }

    /// An inventory holding only the Current Outfit folder (11), at `version`.
    fn cof_inventory(version: i32) -> Inventory {
        let mut inv = Inventory::default();
        inv.folders.insert(
            Uuid::from_u128(11),
            super::super::inventory::Folder {
                info: InvFolder {
                    id: Uuid::from_u128(11),
                    parent: Uuid::nil(),
                    name: "COF".into(),
                    type_default: FT_CURRENT_OUTFIT,
                    version,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        inv
    }

    #[test]
    fn rebake_steps_the_cof_version_then_takes_the_servers() {
        let mut inv = cof_inventory(8);
        let mut r = OutfitRestore::default();
        r.rebake(&inv, true, 8);
        // a second rebake while the first is answered asks nothing more
        r.rebake(&inv, true, 8);
        assert!(matches!(&r.commands[..], [NetCommand::IncrementCofVersion]));
        r.commands.clear();
        r.on_cof_stepped(Some(9), &mut inv, 8);
        assert_eq!(inv.folders[&Uuid::from_u128(11)].info.version, 9);
        // the stepped version counts as received: the server bakes it
        // without a request (requestServerAppearanceUpdate returns early)
        assert!(r.commands.is_empty());
        r.rebake(&inv, true, 8);
        assert!(matches!(&r.commands[..], [NetCommand::IncrementCofVersion]));
    }

    #[test]
    fn rebake_without_the_capability_asks_for_the_current_version() {
        let inv = cof_inventory(8);
        let mut r = OutfitRestore::default();
        // the server already sent an appearance for it: nothing to ask
        r.rebake(&inv, false, 8);
        assert!(r.commands.is_empty());
        r.rebake(&inv, false, 7);
        assert!(matches!(&r.commands[..], [NetCommand::RequestServerAppearance { cof_version: 8 }]));
        // and a failed step still asks, with the folder's version
        let mut inv = cof_inventory(10);
        let mut r = OutfitRestore::default();
        r.rebake(&inv, true, 9);
        r.commands.clear();
        r.on_cof_stepped(None, &mut inv, 9);
        assert!(matches!(&r.commands[..], [NetCommand::RequestServerAppearance { cof_version: 10 }]));
    }

    fn folder_at(id: Uuid, parent: Uuid, name: &str, kind: i32, library: bool) -> super::super::inventory::Folder {
        super::super::inventory::Folder {
            info: InvFolder {
                id,
                parent,
                name: name.into(),
                type_default: kind,
                version: 1,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Fetched,
            library,
        }
    }

    /// Root, My Outfits, Clothing, an empty Current Outfit and a library
    /// root, all fetched. System folders are children of the root.
    fn initial_tree() -> Inventory {
        let root = Uuid::from_u128(1);
        let my = Uuid::from_u128(2);
        let cof = Uuid::from_u128(3);
        let clothing = Uuid::from_u128(4);
        let lib = Uuid::from_u128(5);
        let mut inv = Inventory {
            root,
            lib_root: lib,
            owner: Uuid::from_u128(9),
            ..Default::default()
        };
        inv.folders.insert(root, folder_at(root, Uuid::nil(), "Inventory", 8, false));
        inv.folders.insert(
            my,
            folder_at(my, root, "My Outfits", super::super::appearance::FT_MY_OUTFITS, false),
        );
        inv.folders
            .insert(cof, folder_at(cof, root, "Current Outfit", FT_CURRENT_OUTFIT, false));
        inv.folders.insert(clothing, folder_at(clothing, root, "Clothing", 5, false));
        inv.folders.insert(lib, folder_at(lib, Uuid::nil(), "Library", 8, true));
        inv.folders.get_mut(&root).unwrap().children = vec![my, clothing, cof];
        inv
    }

    fn put_clothing(inv: &mut Inventory, id: u128, parent: Uuid, name: &str) {
        let item_id = Uuid::from_u128(id);
        inv.items.insert(
            item_id,
            InvItem {
                id: item_id,
                parent,
                name: name.into(),
                asset_type: 5,
                inv_type: 18,
                flags: 4,
                base_mask: 0x7fffffff,
                owner_mask: 0x7fffffff,
                ..Default::default()
            },
        );
        inv.folders.get_mut(&parent).unwrap().items.push(item_id);
    }

    fn first<'a>(inv: &'a mut Inventory, folder: &'a str, gender: &'a str, second_life: bool) -> OutfitInputs<'a> {
        OutfitInputs {
            inventory: inv,
            agent: Uuid::from_u128(1),
            ready: true,
            first_login: true,
            second_life,
            initial_folder: folder,
            initial_gender: gender,
            worn: HashSet::new(),
            received_cof_version: 0,
        }
    }

    fn worn_targets(commands: &[NetCommand]) -> Vec<Uuid> {
        commands
            .iter()
            .find_map(|c| match c {
                NetCommand::UpdateOutfit { change, .. } => Some(change.links.iter().map(|l| l.target).collect()),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn created_kinds(commands: &[NetCommand]) -> Vec<u8> {
        let mut kinds: Vec<u8> = commands
            .iter()
            .filter_map(|c| match c {
                NetCommand::CreateInventoryWearable { kind, .. } => Some(*kind),
                _ => None,
            })
            .collect();
        kinds.sort();
        kinds
    }

    #[test]
    fn first_login_wears_the_named_outfit_in_my_outfits() {
        let mut inv = initial_tree();
        let my = Uuid::from_u128(2);
        let trash = Uuid::from_u128(6);
        let decoy = Uuid::from_u128(7);
        let starter = Uuid::from_u128(8);
        let shirt = Uuid::from_u128(100);
        inv.folders.insert(trash, folder_at(trash, my, "Trash", 14, false));
        inv.folders.insert(
            decoy,
            folder_at(decoy, trash, "starter", super::super::appearance::FT_OUTFIT, false),
        );
        inv.folders.insert(
            starter,
            folder_at(starter, my, "Starter", super::super::appearance::FT_OUTFIT, false),
        );
        inv.folders.get_mut(&my).unwrap().children = vec![trash, starter];
        inv.folders.get_mut(&trash).unwrap().children = vec![decoy];
        put_clothing(&mut inv, 101, decoy, "Decoy shirt");
        put_clothing(&mut inv, 100, starter, "Starter shirt");
        let mut r = OutfitRestore::default();
        let now = Instant::now();
        r.update(first(&mut inv, "starter", "female", true), now);
        let links = worn_targets(&r.commands);
        assert!(links.contains(&shirt), "the shirt in My Outfits is worn: {links:?}");
        assert!(!links.contains(&Uuid::from_u128(101)), "trash is not worn: {links:?}");
        assert!(r.commands.iter().any(|c| matches!(c, NetCommand::DummyWearablesUpdate)));
        // OpenSim does not send the dummy wearables update.
        let mut inv = initial_tree();
        let starter = Uuid::from_u128(8);
        inv.folders.insert(
            starter,
            folder_at(starter, my, "Starter", super::super::appearance::FT_OUTFIT, false),
        );
        inv.folders.get_mut(&my).unwrap().children = vec![starter];
        put_clothing(&mut inv, 100, starter, "Starter shirt");
        let mut r = OutfitRestore::default();
        r.update(first(&mut inv, "starter", "", false), now);
        assert!(worn_targets(&r.commands).contains(&shirt));
        assert!(!r.commands.iter().any(|c| matches!(c, NetCommand::DummyWearablesUpdate)));
    }

    #[test]
    fn first_login_copies_a_library_outfit_then_wears_it() {
        let mut inv = initial_tree();
        let lib = Uuid::from_u128(5);
        let clothing = Uuid::from_u128(4);
        let source = Uuid::from_u128(11);
        inv.folders
            .insert(source, folder_at(source, lib, "Starter", super::super::appearance::FT_OUTFIT, true));
        inv.folders.get_mut(&lib).unwrap().children = vec![source];
        put_clothing(&mut inv, 200, source, "Library shirt");
        let mut r = OutfitRestore::default();
        let now = Instant::now();
        r.update(first(&mut inv, "Starter", "female", true), now);
        assert!(
            r.commands.iter().any(|c| matches!(c, NetCommand::EditInventory { .. })),
            "a library folder is copied before it is worn"
        );
        assert!(r.commands.iter().any(|c| matches!(c, NetCommand::DummyWearablesUpdate)));
        assert!(worn_targets(&r.commands).is_empty(), "the library original is not worn");
        let copied = Uuid::from_u128(12);
        let shirt = Uuid::from_u128(201);
        inv.folders.insert(
            copied,
            folder_at(copied, clothing, "Starter", super::super::appearance::FT_OUTFIT, false),
        );
        inv.folders.get_mut(&clothing).unwrap().children.push(copied);
        put_clothing(&mut inv, 201, copied, "Copied shirt");
        r.commands.clear();
        r.update(first(&mut inv, "Starter", "female", true), now);
        let links = worn_targets(&r.commands);
        assert!(links.contains(&shirt), "the copied shirt is worn: {links:?}");
        assert!(!r.commands.iter().any(|c| matches!(c, NetCommand::DummyWearablesUpdate)));
    }

    #[test]
    fn first_login_without_a_folder_creates_the_standard_wearables() {
        let mut inv = initial_tree();
        let mut r = OutfitRestore::default();
        let now = Instant::now();
        r.update(first(&mut inv, "", "male", true), now);
        assert_eq!(created_kinds(&r.commands), vec![0, 1, 2, 3, 4, 5, 6, 7, 10, 11]);
        let shape = r.commands.iter().find_map(|c| match c {
            NetCommand::CreateInventoryWearable { kind: 0, data, .. } => Some(String::from_utf8_lossy(data).into_owned()),
            _ => None,
        });
        let shape = shape.expect("a shape wearable");
        assert!(shape.contains("\n80 1\n"), "a male shape sets param 80: {shape}");
        assert!(r.commands.iter().any(|c| matches!(c, NetCommand::DummyWearablesUpdate)));
        let mut inv = initial_tree();
        let mut r = OutfitRestore::default();
        r.update(first(&mut inv, "missing", "female", true), now);
        let shape = r
            .commands
            .iter()
            .find_map(|c| match c {
                NetCommand::CreateInventoryWearable { kind: 0, data, .. } => Some(String::from_utf8_lossy(data).into_owned()),
                _ => None,
            })
            .expect("a female shape");
        assert!(shape.contains("\n80 0\n"), "anything but male stays female: {shape}");
        assert!(!shape.contains("\n80 1\n"));
    }

    #[test]
    fn first_login_waits_to_read_my_outfits_before_creating_wearables() {
        let mut inv = initial_tree();
        let my = Uuid::from_u128(2);
        inv.folders.get_mut(&my).unwrap().state = FetchState::Unknown;
        inv.folders.get_mut(&my).unwrap().children.clear();
        let mut r = OutfitRestore::default();
        r.update(first(&mut inv, "Starter", "female", true), Instant::now());
        assert!(!r.is_done(), "My Outfits is still unread");
        assert!(
            created_kinds(&r.commands).is_empty(),
            "wearables wait until the name can be searched"
        );
        assert_eq!(inv.folders[&my].state, FetchState::Fetching);
    }
}
