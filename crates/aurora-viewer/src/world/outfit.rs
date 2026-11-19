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
//!
//! At the first login there is no outfit to restore: the account wears the
//! outfit chosen at registration (the login response's `initial-outfit`),
//! as LLStartUp::loadInitialOutfit (indra/newview/llstartup.cpp) does. The
//! folder of that name in My Outfits is worn, else the Library's is copied
//! then worn (findDescendentCategoryIDByName, wearInventoryCategory). The
//! restore only picks the folder (`take_initial_outfit`): the app wears it
//! like the Avatar Welcome Pack's `wear_folder` (`app/avatar_welcome.rs`).

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
/// Times the objects the region took off are asked for again.
const REWEAR_ROUNDS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingWear {
    OutfitChange,
    LoginRestore,
}

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

/// What the restore needs from the world each tick.
pub struct OutfitInputs<'a> {
    pub inventory: &'a mut Inventory,
    pub agent: Uuid,
    /// In a region with its capabilities.
    pub ready: bool,
    pub first_login: bool,
    /// The outfit folder chosen at registration (`initial-outfit`), on the
    /// first login only, when the response names a folder and a gender.
    pub initial_outfit: Option<String>,
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
    /// Still-linked originals whose metadata has not arrived. A wall-clock
    /// deadline is not proof of deletion; keep the intent without issuing
    /// more reads, until metadata arrives or an authoritative change cancels it.
    pending_wear: HashMap<Uuid, PendingWear>,
    bake_requested: Option<i32>,
    bake_retries: u32,
    /// Rebake: the COF version the server answered last, taken as received
    /// (syncCofVersionAndRefreshCoro slams mLastUpdateReceivedCOFVersion),
    /// and an IncrementCOFVersion request waiting for its answer.
    stepped_cof_version: i32,
    stepping: bool,
    /// The first login's outfit, until the app takes it.
    initial: Option<InitialOutfit>,
    pub commands: Vec<NetCommand>,
}

/// The outfit a new account wears at its first login.
#[derive(Debug, Clone, PartialEq)]
pub enum InitialOutfit {
    /// A folder of My Outfits, worn as it is.
    Mine(Uuid),
    /// A Library folder of that name, copied into Clothing then worn.
    Library(String),
}

/// LLStartUp::loadInitialOutfit's folder: `name` under My Outfits, else
/// under the Library. None when neither has it (the Legacy Viewer then
/// creates the standard wearables).
pub fn initial_outfit(inv: &Inventory, name: &str) -> Option<InitialOutfit> {
    use super::appearance::{FT_MY_OUTFITS, descendant_named, system_folder};
    if let Some(folder) = system_folder(inv, FT_MY_OUTFITS).and_then(|my| descendant_named(inv, my, name)) {
        return Some(InitialOutfit::Mine(folder));
    }
    descendant_named(inv, inv.lib_root, name).map(|_| InitialOutfit::Library(name.to_owned()))
}

/// The registration's outfit in the login response (`initial-outfit`):
/// its folder name, when both it and the gender are given
/// (LLStartUp's sInitialOutfit and sInitialOutfitGender).
pub fn registration_outfit(folder_name: &str, gender: &str) -> Option<String> {
    (!folder_name.is_empty() && !gender.is_empty()).then(|| folder_name.to_owned())
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
            initial: None,
            commands: Vec::new(),
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
        if link.asset_type != ASSET_LINK || link.asset_id.is_nil() || inv.broken(&link.asset_id) {
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

    /// The first login's outfit, once: the app wears it.
    pub fn take_initial_outfit(&mut self) -> Option<InitialOutfit> {
        self.initial.take()
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
        for item in items {
            self.pending_wear.remove(item);
        }
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
            Stage::Done => self.pending_wear.get(&item) == Some(&PendingWear::LoginRestore),
        }
    }

    /// A COF change (Replace outfit, Add…) linked objects whose items the
    /// inventory does not know yet: they are looked up, then worn if they
    /// are still in the COF (callAfterCOFFetch fetches the links' items
    /// before updateAppearanceFromCOF wears them).
    pub fn wear_when_known(&mut self, items: Vec<Uuid>, agent: Uuid, _now: Instant) {
        let new: Vec<Uuid> = items
            .into_iter()
            .filter(|id| {
                self.user_removed.remove(id);
                self.pending_wear.insert(*id, PendingWear::OutfitChange).is_none()
            })
            .collect();
        if new.is_empty() {
            return;
        }
        log::info!("outfit: {} worn objects to look up before wearing them", new.len());
        self.commands.push(NetCommand::FetchItems { items: new, owner: agent });
    }

    pub fn update(&mut self, mut w: OutfitInputs, now: Instant) {
        self.wear_known(&mut w, now);
        let restore_deadline = match self.stage {
            Stage::Settle => Some(self.ready_at.unwrap_or(now) + MAX_SETTLE),
            Stage::Attaching { since, .. } => Some(since + RETRY_AFTER),
            _ => None,
        };
        if let Some(deadline) = restore_deadline {
            if !w.ready || !w.inventory.folders.get(&self.cof).is_some_and(|f| f.state == FetchState::Fetched) {
                if now >= deadline + FETCH_TIMEOUT {
                    log::info!("outfit: login restore timed out waiting for Current Outfit and region readiness");
                    self.stage = Stage::Done;
                }
                return;
            }
            // A login retry can outlive the outfit snapshot that started it.
            let linked = cof_objects(w.inventory, self.cof);
            self.objects.retain(|id| linked.contains(id) && !self.user_removed.contains(id));
            self.worn_objects.retain(|id| self.objects.contains(id));
            self.knocked_off.retain(|id| self.objects.contains(id));
        }
        match self.stage {
            Stage::Done => {}
            Stage::Waiting => {
                if !w.ready || w.inventory.folders.is_empty() {
                    return;
                }
                if w.first_login {
                    self.start_initial(&w);
                    self.stage = Stage::Done;
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
                    // LLAppearanceMgr::updateAppearanceFromCOF observes links
                    // whose originals are still loading. Let known objects
                    // restore now, retaining late originals without more GETs.
                    let before = self.objects.len();
                    self.objects.retain(|id| !w.inventory.broken(id));
                    if self.objects.len() < before {
                        log::warn!(
                            "outfit: {} confirmed missing originals in the Current Outfit folder",
                            before - self.objects.len()
                        );
                    }
                    let mut deferred = 0;
                    for id in &self.objects {
                        if !w.inventory.items.contains_key(id) && !self.user_removed.contains(id) {
                            self.pending_wear.entry(*id).or_insert(PendingWear::LoginRestore);
                            deferred += 1;
                        }
                    }
                    if deferred > 0 {
                        log::info!("outfit: {deferred} originals still loading; keeping their restore intent");
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

    /// LLStartUp::loadInitialOutfit: nothing to restore, the outfit chosen
    /// at registration is worn instead (by the app, `take_initial_outfit`).
    fn start_initial(&mut self, w: &OutfitInputs) {
        let Some(name) = w.initial_outfit.as_deref() else {
            log::info!("outfit: first login, registration chose no outfit");
            return;
        };
        self.initial = initial_outfit(w.inventory, name);
        match &self.initial {
            Some(outfit) => {
                log::info!("outfit: first login, wearing the initial outfit {name:?} ({outfit:?})");
                if w.second_life {
                    self.commands.push(NetCommand::DummyWearablesUpdate);
                }
            }
            // the Legacy Viewer creates the standard wearables
            // (createStandardWearables); not done yet, see TASKS.md
            None => log::warn!("outfit: first login, the initial outfit {name:?} is in neither My Outfits nor the Library"),
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
        if self.pending_wear.is_empty() || !w.ready {
            return;
        }
        let cof = if self.cof.is_nil() { find_cof(w.inventory) } else { Some(self.cof) };
        let loaded = cof.is_some_and(|id| w.inventory.folders.get(&id).is_some_and(|f| f.state == FetchState::Fetched));
        let linked: HashSet<Uuid> = cof
            .map(|cof| cof_objects(w.inventory, cof))
            .unwrap_or_default()
            .into_iter()
            .collect();
        let mut list = Vec::new();
        let mut obsolete = Vec::new();
        let mut resumed_login = false;
        self.pending_wear.retain(|id, source| {
            // taken out of the outfit, or worn meanwhile
            if w.worn.contains(id) {
                return false;
            }
            if self.user_removed.contains(id) || w.inventory.broken(id) || loaded && !linked.contains(id) {
                if *source == PendingWear::LoginRestore {
                    obsolete.push(*id);
                }
                return false;
            }
            // A refreshing COF is not an authoritative unlink or permission
            // to wear a stale target. Wait for its accepted snapshot.
            if !loaded {
                return true;
            }
            if let Some(item) = w.inventory.items.get(id) {
                if *source == PendingWear::LoginRestore {
                    if self.requested.contains_key(id) {
                        return false;
                    }
                    // The region's own restore must settle before a late
                    // original is requested, just like the known originals.
                    if !matches!(self.stage, Stage::Attaching { .. } | Stage::Done) {
                        return true;
                    }
                    resumed_login = true;
                }
                list.push(attach_request(item, w.agent));
                return false;
            }
            true
        });
        self.objects.retain(|id| !obsolete.contains(id));
        if resumed_login {
            // Give newly available originals the usual bounded attach/retry
            // window, even if the known part of the login restore has ended.
            self.stage = Stage::Attaching {
                since: now,
                retried: false,
            };
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
            initial_outfit: None,
            second_life: true,
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
    fn recovery_login_restores_late_originals_without_holding_known_attachments() {
        let mut inv = unfetched_cof();
        let mut restore = OutfitRestore::default();
        let now = Instant::now();
        restore.update(inputs(&mut inv, &[200]), now);
        fetch_cof(&mut inv, &[200, 201, 202]);
        inv.items.remove(&Uuid::from_u128(201));
        restore.update(inputs(&mut inv, &[200]), now);
        assert!(matches!(restore.commands.last(), Some(NetCommand::FetchItems { items, .. }) if items == &[Uuid::from_u128(201)]));
        restore.commands.clear();
        restore.update(inputs(&mut inv, &[200]), now + Duration::from_secs(16));
        assert_eq!(restore.objects.len(), 3, "unreadable is not deleted");
        restore.update(inputs(&mut inv, &[200]), now + Duration::from_secs(20));
        assert_eq!(
            requested(&restore.commands),
            vec![202],
            "healthy attachments do not wait for the cold inventory"
        );
        restore.commands.clear();
        restore.update(inputs(&mut inv, &[200, 202]), now + Duration::from_secs(50));
        restore.update(inputs(&mut inv, &[200, 202]), now + Duration::from_secs(80));
        assert!(restore.is_done());
        assert!(!restore.is_idle(), "late metadata still has a live restore intent");
        assert!(restore.keeps_cof_link(Uuid::from_u128(201)));
        assert!(restore.commands.is_empty(), "waiting does not generate more reads or attaches");
        inv.add_items(vec![item(201, Uuid::from_u128(10), ASSET_OBJECT, INV_OBJECT, 0)]);
        restore.update(inputs(&mut inv, &[200, 202]), now + Duration::from_secs(150));
        assert_eq!(requested(&restore.commands), vec![201]);
        restore.commands.clear();
        restore.update(inputs(&mut inv, &[200, 201, 202]), now + Duration::from_secs(180));
        assert!(restore.is_idle());
        assert!(restore.commands.is_empty(), "an arriving attachment is not worn twice");
    }

    fn deferred_login_original() -> (Inventory, OutfitRestore) {
        let mut inv = unfetched_cof();
        fetch_cof(&mut inv, &[201]);
        inv.items.remove(&Uuid::from_u128(201));
        let restore = OutfitRestore {
            stage: Stage::Done,
            cof: Uuid::from_u128(11),
            objects: vec![Uuid::from_u128(201)],
            pending_wear: HashMap::from([(Uuid::from_u128(201), PendingWear::LoginRestore)]),
            ..Default::default()
        };
        (inv, restore)
    }

    #[test]
    fn recovery_login_does_not_wear_a_late_original_removed_from_the_cof() {
        let (mut inv, mut restore) = deferred_login_original();
        fetch_cof(&mut inv, &[]);
        inv.add_items(vec![item(201, Uuid::from_u128(10), ASSET_OBJECT, INV_OBJECT, 0)]);
        restore.update(inputs(&mut inv, &[]), Instant::now());
        assert!(restore.is_idle());
        assert!(restore.objects.is_empty());
        assert!(restore.commands.is_empty());
    }

    #[test]
    fn recovery_late_attachment_retry_waits_for_the_cof_and_respects_its_unlink() {
        let (mut inv, mut restore) = deferred_login_original();
        let now = Instant::now();
        let cof = Uuid::from_u128(11);
        inv.add_items(vec![item(201, Uuid::from_u128(10), ASSET_OBJECT, INV_OBJECT, 0)]);
        restore.update(inputs(&mut inv, &[]), now);
        assert_eq!(requested(&restore.commands), vec![201]);
        restore.commands.clear();

        inv.folders.get_mut(&cof).unwrap().state = FetchState::Fetching;
        restore.update(inputs(&mut inv, &[]), now + RETRY_AFTER);
        assert!(
            restore.commands.is_empty(),
            "the old outfit cannot authorize a retry during refresh"
        );

        fetch_cof(&mut inv, &[]);
        restore.update(inputs(&mut inv, &[]), now + RETRY_AFTER + Duration::from_secs(1));
        assert!(restore.commands.is_empty(), "the new outfit removed this attachment");
        assert!(restore.is_idle());
    }

    #[test]
    fn recovery_unavailable_cof_ends_active_restore_without_dropping_pending_originals() {
        let (mut inv, mut restore) = deferred_login_original();
        let now = Instant::now();
        let cof = Uuid::from_u128(11);
        restore.stage = Stage::Attaching {
            since: now,
            retried: false,
        };
        inv.folders.get_mut(&cof).unwrap().state = FetchState::Failed;
        restore.update(inputs(&mut inv, &[]), now + RETRY_AFTER + FETCH_TIMEOUT);
        assert!(restore.is_done());
        assert!(!restore.is_idle());
        assert!(restore.commands.is_empty());

        fetch_cof(&mut inv, &[201]);
        restore.update(inputs(&mut inv, &[]), now + RETRY_AFTER + FETCH_TIMEOUT + Duration::from_secs(1));
        assert_eq!(requested(&restore.commands), vec![201]);
    }

    #[test]
    fn recovery_explicit_detach_cancels_an_attachment_queued_for_rewear() {
        let (mut inv, mut restore) = deferred_login_original();
        let now = Instant::now();
        let id = Uuid::from_u128(201);
        inv.add_items(vec![item(201, Uuid::from_u128(10), ASSET_OBJECT, INV_OBJECT, 0)]);
        restore.update(inputs(&mut inv, &[]), now);
        restore.commands.clear();
        restore.update(inputs(&mut inv, &[201]), now + Duration::from_secs(1));
        let loss = now + Duration::from_secs(2);
        restore.update(inputs(&mut inv, &[]), loss);
        assert_eq!(restore.knocked_off, vec![id]);

        restore.on_detach_requested(&[id]);
        restore.update(inputs(&mut inv, &[]), loss + QUIET);
        assert!(restore.commands.is_empty(), "an explicit detach cancels the queued rewear");
    }

    #[test]
    fn recovery_login_honors_an_explicit_detach_while_metadata_is_loading() {
        let (mut inv, mut restore) = deferred_login_original();
        restore.on_detach_requested(&[Uuid::from_u128(201)]);
        inv.add_items(vec![item(201, Uuid::from_u128(10), ASSET_OBJECT, INV_OBJECT, 0)]);
        restore.update(inputs(&mut inv, &[]), Instant::now());
        assert!(restore.is_idle());
        assert!(!restore.keeps_cof_link(Uuid::from_u128(201)));
        assert!(restore.commands.is_empty());
    }

    #[test]
    fn recovery_login_waits_for_a_refreshing_cof_and_a_ready_region() {
        let (mut inv, mut restore) = deferred_login_original();
        let now = Instant::now();
        inv.folders.get_mut(&Uuid::from_u128(11)).unwrap().state = FetchState::Fetching;
        inv.folders.get_mut(&Uuid::from_u128(11)).unwrap().items.clear();
        inv.add_items(vec![item(201, Uuid::from_u128(10), ASSET_OBJECT, INV_OBJECT, 0)]);
        restore.update(inputs(&mut inv, &[]), now);
        assert!(!restore.is_idle(), "an incomplete COF is not an unlink");
        assert!(restore.commands.is_empty());
        fetch_cof(&mut inv, &[201]);
        let mut crossing = inputs(&mut inv, &[]);
        crossing.ready = false;
        restore.update(crossing, now + Duration::from_secs(1));
        assert!(restore.commands.is_empty());
        restore.update(inputs(&mut inv, &[]), now + Duration::from_secs(2));
        assert_eq!(requested(&restore.commands), vec![201]);
    }

    #[test]
    fn recovery_login_cancels_confirmed_missing_and_non_attachable_originals() {
        for flags in [None, Some(crate::ui::inventory::OBJECT_HAS_MULTIPLE_ITEMS)] {
            let (mut inv, mut restore) = deferred_login_original();
            if let Some(flags) = flags {
                let mut object = item(201, Uuid::from_u128(10), ASSET_OBJECT, INV_OBJECT, 0);
                object.flags = flags;
                inv.add_items(vec![object]);
            } else {
                inv.items_missing(vec![Uuid::from_u128(201)]);
            }
            restore.update(inputs(&mut inv, &[]), Instant::now());
            assert!(restore.is_idle());
            assert!(restore.objects.is_empty());
            assert!(restore.commands.is_empty());
        }
    }

    #[test]
    fn recovery_outfit_change_keeps_late_metadata_and_a_new_wear_overrides_an_old_detach() {
        let (mut inv, _) = deferred_login_original();
        let mut restore = OutfitRestore::finished();
        let now = Instant::now();
        let id = Uuid::from_u128(201);
        restore.wear_when_known(vec![id], Uuid::from_u128(1), now);
        restore.commands.clear();
        restore.update(inputs(&mut inv, &[]), now + Duration::from_secs(150));
        assert!(!restore.is_idle());
        assert!(restore.commands.is_empty(), "no new GETs while metadata is pending");
        restore.on_detach_requested(&[id]);
        restore.wear_when_known(vec![id], Uuid::from_u128(1), now + Duration::from_secs(151));
        restore.commands.clear();
        inv.add_items(vec![item(201, Uuid::from_u128(10), ASSET_OBJECT, INV_OBJECT, 0)]);
        restore.update(inputs(&mut inv, &[]), now + Duration::from_secs(152));
        assert_eq!(requested(&restore.commands), vec![201]);
        assert!(restore.is_idle());
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
        // the login restore is over (nothing to restore)
        let mut first = inputs(&mut inv, &[]);
        first.first_login = true;
        r.update(first, t0);
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

    fn add_folder(inv: &mut Inventory, n: u128, parent: u128, name: &str, kind: i32, library: bool) {
        let (id, parent) = (Uuid::from_u128(n), Uuid::from_u128(parent));
        inv.folders.insert(
            id,
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
                state: FetchState::Unknown,
                library,
            },
        );
        if let Some(p) = inv.folders.get_mut(&parent) {
            p.children.push(id);
        }
    }

    /// A new account: root (1) with its COF (11) and My Outfits (12)
    /// holding "Female Avatar" (13), Library (20) › Avatars (21) ›
    /// "Male Avatar" (22).
    fn new_account() -> Inventory {
        let mut inv = Inventory {
            root: Uuid::from_u128(1),
            lib_root: Uuid::from_u128(20),
            ..Default::default()
        };
        add_folder(&mut inv, 1, 0, "My Inventory", 8, false);
        add_folder(&mut inv, 11, 1, "Current Outfit", FT_CURRENT_OUTFIT, false);
        add_folder(&mut inv, 12, 1, "My Outfits", super::super::appearance::FT_MY_OUTFITS, false);
        add_folder(&mut inv, 13, 12, "Female Avatar", super::super::appearance::FT_OUTFIT, false);
        add_folder(&mut inv, 20, 0, "Library", 8, true);
        add_folder(&mut inv, 21, 20, "Avatars", -1, true);
        add_folder(&mut inv, 22, 21, "Male Avatar", -1, true);
        inv
    }

    fn first_login(r: &mut OutfitRestore, inv: &mut Inventory, initial: Option<&str>, second_life: bool) {
        let mut w = inputs(inv, &[]);
        w.first_login = true;
        w.initial_outfit = initial.map(str::to_owned);
        w.second_life = second_life;
        r.update(w, Instant::now());
    }

    #[test]
    fn a_new_account_wears_its_registration_outfit_from_my_outfits_first() {
        let mut inv = new_account();
        // the same name in the Library too: My Outfits wins
        add_folder(&mut inv, 23, 21, "Female Avatar", -1, true);
        let mut r = OutfitRestore::default();
        first_login(&mut r, &mut inv, Some("female avatar"), true);
        assert!(r.is_done(), "nothing to restore");
        assert!(matches!(r.commands.as_slice(), [NetCommand::DummyWearablesUpdate]));
        assert_eq!(r.take_initial_outfit(), Some(InitialOutfit::Mine(Uuid::from_u128(13))));
        assert_eq!(r.take_initial_outfit(), None, "taken once");
    }

    #[test]
    fn a_registration_outfit_only_in_the_library_is_copied() {
        let mut inv = new_account();
        let mut r = OutfitRestore::default();
        // OpenSim: no DummyWearablesUpdate (Legacy Bake)
        first_login(&mut r, &mut inv, Some("Male Avatar"), false);
        assert!(r.commands.is_empty());
        assert_eq!(r.take_initial_outfit(), Some(InitialOutfit::Library("Male Avatar".into())));
    }

    #[test]
    fn a_first_login_without_a_known_outfit_wears_nothing() {
        for initial in [None, Some("Nowhere")] {
            let mut inv = new_account();
            let mut r = OutfitRestore::default();
            first_login(&mut r, &mut inv, initial, true);
            assert!(r.is_done());
            assert!(r.commands.is_empty());
            assert_eq!(r.take_initial_outfit(), None);
        }
    }

    #[test]
    fn a_returning_account_restores_its_outfit_and_ignores_the_registration_one() {
        let mut inv = new_account();
        let mut r = OutfitRestore::default();
        let mut w = inputs(&mut inv, &[]);
        w.initial_outfit = Some("Female Avatar".into());
        r.update(w, Instant::now());
        assert!(matches!(r.stage, Stage::FetchCof { .. }));
        assert_eq!(r.take_initial_outfit(), None);
    }

    #[test]
    fn the_registration_outfit_needs_a_folder_name_and_a_gender() {
        assert_eq!(registration_outfit("Female Avatar", "female").as_deref(), Some("Female Avatar"));
        assert_eq!(registration_outfit("", "female"), None);
        assert_eq!(registration_outfit("Female Avatar", ""), None);
    }
}
