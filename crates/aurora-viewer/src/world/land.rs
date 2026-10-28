//! The parcel shown in "About Land": LLViewerParcelMgr's selection
//! (selectParcelAt, the selected LLParcel with its access / ban /
//! experience lists, dwell), the estate covenant, the parcel environment,
//! and the rules deciding what the agent may change
//! (isParcelOwnedByAgent, isParcelModifiableByAgent, canAgentBuyParcel).
//!
//! Port of newview/llviewerparcelmgr.cpp, llfloaterland.cpp,
//! llpanelenvironment.cpp and llinventory/llparcel.cpp (originally LGPL 2.1).

use aurora_llsd::Llsd;
use aurora_net::land::{self, AccessEntry, FoundAvatar, LandCommand, LandEvent, ObjectOwner, ParcelUpdate};
use aurora_net::{GroupMembership, NetCommand, ParcelInfo, RegionHandle, parcel_flags as pf};
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Group powers (roles_constants.h GP_*).
pub mod powers {
    pub const NONE: u64 = 0;
    pub const LAND_DEED: u64 = 1 << 12;
    pub const LAND_RELEASE: u64 = 1 << 13;
    pub const LAND_SET_SALE_INFO: u64 = 1 << 14;
    pub const LAND_FIND_PLACES: u64 = 1 << 17;
    pub const LAND_CHANGE_IDENTITY: u64 = 1 << 18;
    pub const LAND_SET_LANDING_POINT: u64 = 1 << 19;
    pub const LAND_CHANGE_MEDIA: u64 = 1 << 20;
    pub const LAND_EDIT: u64 = 1 << 21;
    pub const LAND_OPTIONS: u64 = 1 << 22;
    pub const LAND_ALLOW_FLY: u64 = 1 << 24;
    pub const LAND_MANAGE_ALLOWED: u64 = 1 << 29;
    pub const LAND_MANAGE_BANNED: u64 = 1 << 30;
    /// Eject and freeze residents on the group's land.
    pub const LAND_ADMIN: u64 = 1 << 32;
    pub const LAND_RETURN_GROUP_SET: u64 = 1 << 33;
    pub const LAND_RETURN_NON_GROUP: u64 = 1 << 34;
    pub const LAND_ALLOW_ENVIRONMENT: u64 = 1 << 46;
    pub const LAND_RETURN_GROUP_OWNED: u64 = 1 << 48;
}

/// Parcels smaller than this have no environment of their own and are
/// not listed in search (MINIMUM_PARCEL_SIZE, MIN_PARCEL_AREA_FOR_SEARCH).
pub const MIN_PARCEL_AREA: i32 = 128;
/// The covenant is asked again after a minute (COVENANT_REFRESH_TIME_SEC).
const COVENANT_REFRESH: Duration = Duration::from_secs(60);
/// The selected parcel is asked again this long after an object appeared in
/// its region or left it, so that a burst of objects is one request.
const REFRESH_DEBOUNCE: Duration = Duration::from_secs(1);
/// Two refreshes are at least this far apart (a region arriving object by
/// object must not turn into a stream of requests).
const REFRESH_MIN_INTERVAL: Duration = Duration::from_secs(3);
/// With nothing seen changing, the parcel is still asked again this often:
/// its counts also move for reasons the viewer does not see (objects out of
/// range, the land impact of an object, an automatic return).
const REFRESH_IDLE: Duration = Duration::from_secs(15);
/// No refresh this long after a change sent from here: the simulator must
/// have it first (the capability's POST is not ordered with the UDP request),
/// or its answer would show the old values again, and the sound tab would
/// take a stream just set for a refused one.
const REFRESH_AFTER_UPDATE: Duration = Duration::from_secs(5);
/// A refresh without an answer is forgotten after this long.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(10);
/// PARCEL_GRID_STEP_METERS.
const GRID_STEP: f32 = 4.0;

/// Who the agent is, for the permission rules.
pub struct AgentRights<'a> {
    pub id: Uuid,
    pub groups: &'a [GroupMembership],
}

impl AgentRights<'_> {
    /// LLAgent::hasPowerInGroup (GP_NO_POWERS is never "had").
    pub fn has_power(&self, group: &Uuid, power: u64) -> bool {
        self.groups.iter().any(|g| g.id == *group && g.powers & power != 0)
    }

    pub fn in_group(&self, group: &Uuid) -> bool {
        self.groups.iter().any(|g| g.id == *group)
    }
}

/// LLViewerParcelMgr::isParcelOwnedByAgent (we are never godlike).
pub fn owned_by_agent(p: &ParcelInfo, a: &AgentRights, power: u64) -> bool {
    if p.owner_id == a.id {
        return true;
    }
    // only gods can assume ownership of public land
    if p.owner_id.is_nil() {
        return false;
    }
    a.has_power(&p.owner_id, power)
}

/// LLViewerParcelMgr::isParcelModifiableByAgent: our own land must be
/// leased (a purchase waiting for approval cannot be changed).
pub fn modifiable_by_agent(p: &ParcelInfo, a: &AgentRights, power: u64) -> bool {
    owned_by_agent(p, a, power) && !(p.owner_id == a.id && p.status != land::OS_LEASED)
}

/// LLViewerParcelMgr::canAgentBuyParcel; `can_access` tells if the
/// agent's maturity preference allows the region, `active_group` is the
/// group a group purchase would be for.
pub fn can_agent_buy(p: &ParcelInfo, a: &AgentRights, for_group: Option<Uuid>, can_access: bool) -> bool {
    if p.owner_id.is_nil() {
        return true;
    }
    if !can_access {
        return false;
    }
    let for_sale = p.flags & pf::FOR_SALE != 0 && (p.sale_price > 0 || !p.auth_buyer.is_nil());
    let empowered = for_group.is_none_or(|g| a.has_power(&g, powers::LAND_DEED));
    let owner = p.owner_id == for_group.unwrap_or(a.id);
    let authorized = p.auth_buyer.is_nil()
        || p.auth_buyer == a.id
        || (a.has_power(&p.auth_buyer, powers::LAND_DEED) && a.has_power(&p.auth_buyer, powers::LAND_SET_SALE_INFO));
    for_sale && !owner && authorized && empowered
}

/// LLViewerParcelMgr::selectParcelAt: a 4 m square around `pos` (region
/// coordinates), snapped to the parcel grid: (west, south, east, north).
pub fn parcel_rect_at(pos: Vec3, region_size: (f32, f32)) -> (f32, f32, f32, f32) {
    let snap = |v: f32| ((v / GRID_STEP) + 0.5).floor() * GRID_STEP;
    let west = snap(pos.x - GRID_STEP / 2.0).clamp(0.0, region_size.0);
    let south = snap(pos.y - GRID_STEP / 2.0).clamp(0.0, region_size.1);
    let east = snap(pos.x + GRID_STEP / 2.0).clamp(0.0, region_size.0);
    let north = snap(pos.y + GRID_STEP / 2.0).clamp(0.0, region_size.1);
    (west, south, east, north)
}

/// Apply an update to our copy of the parcel (the floater shows the new
/// values until the simulator answers, like LLParcel's setters).
pub fn apply_update(p: &mut ParcelInfo, u: &ParcelUpdate) {
    p.flags = u.flags;
    p.sale_price = u.sale_price;
    p.name = u.name.clone();
    p.desc = u.desc.clone();
    p.music_url = u.music_url.clone();
    p.media_url = u.media_url.clone();
    p.media.desc = u.media_desc.clone();
    p.media.mime = u.media_type.clone();
    p.media.width = u.media_width;
    p.media.height = u.media_height;
    p.media.auto_scale = u.media_auto_scale;
    p.media.looping = u.media_loop;
    p.media.media_id = u.media_id;
    p.media.obscure_moap = u.obscure_moap;
    p.group_id = u.group_id;
    p.pass_price = u.pass_price;
    p.pass_hours = u.pass_hours;
    p.category = u.category as i32;
    p.auth_buyer = u.auth_buyer;
    p.snapshot_id = u.snapshot_id;
    p.user_location = u.user_location;
    p.user_look_at = u.user_look_at;
    p.landing_type = u.landing_type as i32;
    p.see_avatars = u.see_avs;
    p.group_av_sounds = u.group_av_sounds;
    p.any_av_sounds = u.any_av_sounds;
}

/// LLParcel::addToAccessList / addToBanList: false when full, for the
/// owner, or when an existing entry expires later.
pub fn add_entry(list: &mut Vec<AccessEntry>, owner: &Uuid, id: Uuid, time: i32) -> bool {
    if list.len() >= land::PARCEL_MAX_ACCESS_LIST || id == *owner {
        return false;
    }
    if let Some(i) = list.iter().position(|e| e.id == id) {
        let e = list[i];
        if time == 0 || (e.time != 0 && e.time < time) {
            list.remove(i);
        } else {
            return false;
        }
    }
    list.push(AccessEntry { id, time, flags: 0 });
    true
}

/// Object owners list of the "Objects" tab.
#[derive(Debug, Clone, Default)]
pub enum OwnerList {
    /// Not asked yet (or the parcel changed).
    #[default]
    Idle,
    /// "Search..." until the first reply.
    Searching,
    Rows(Vec<ObjectOwner>),
}

/// What the "Environment" tab shows of the parcel environment
/// (LLEnvironment::EnvironmentInfo::extract).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EnvSummary {
    /// Seconds; -1 when the parcel has no day cycle.
    pub day_length: i64,
    pub day_offset: i64,
    /// Starts of sky tracks 2..4 (index 0 = ground).
    pub altitudes: [f32; 4],
    /// day_names per track (0 water, 1 ground, 2..4 skies).
    pub names: [String; 5],
    /// Name of the whole day cycle (day_names as a string, or the cycle's name).
    pub day_name: String,
    /// Tracks of the day cycle that have frames.
    pub tracks_set: [bool; 5],
    pub has_day: bool,
    /// The day cycle's settings map (EnvironmentInfo::mDayCycle), what
    /// "Custom" edits.
    pub day_cycle: Option<Arc<Llsd>>,
}

impl EnvSummary {
    pub fn from_llsd(env: &Llsd) -> EnvSummary {
        let has_day = env.has("day_cycle");
        let mut s = EnvSummary {
            day_cycle: has_day.then(|| Arc::new(env["day_cycle"].clone())),
            day_length: if has_day && env.has("day_length") {
                env["day_length"].as_i32() as i64
            } else {
                -1
            },
            day_offset: if has_day && env.has("day_offset") {
                env["day_offset"].as_i32() as i64
            } else {
                -1
            },
            has_day,
            ..Default::default()
        };
        let alts = &env["track_altitudes"];
        for i in 0..3 {
            s.altitudes[i + 1] = alts.at(i).as_f32();
        }
        let names = &env["day_names"];
        if names.is_array() {
            for (i, n) in s.names.iter_mut().enumerate() {
                *n = names.at(i).to_string_value();
            }
        } else if let Llsd::String(n) = names {
            s.day_name = n.clone();
        } else if has_day {
            s.day_name = env["day_cycle"]["name"].to_string_value();
        }
        let tracks = &env["day_cycle"]["tracks"];
        for (i, set) in s.tracks_set.iter_mut().enumerate() {
            *set = !tracks.at(i).as_array().is_empty();
        }
        s
    }

    /// LLPanelEnvironmentInfo::getNameForTrackIndex for a parcel.
    pub fn track_name(&self, index: i32) -> String {
        if !(0..5).contains(&index) {
            return aurora_i18n::tr!("status-empty").into();
        }
        let i = index as usize;
        let mut name = if self.day_name.is_empty() {
            let n = self.names[i].clone();
            if n.is_empty() && i <= 1 {
                aurora_i18n::tr!("world-land-region-environment").into()
            } else {
                n
            }
        } else if self.tracks_set[i] {
            self.day_name.clone()
        } else {
            String::new()
        };
        if name.is_empty() {
            name = self.track_name(index - 1);
            if !name.starts_with('(') {
                name = format!("({name})");
            }
        }
        name
    }

    /// Apparent time of day at Unix time `now` (udpateApparentTimeOfDay):
    /// (hours, minutes, percent), None when there is no usable cycle.
    pub fn apparent_time(&self, now: i64) -> Option<(i64, i64, i64)> {
        if self.day_length < 1 || self.day_offset < 1 {
            return None;
        }
        let perc = ((now + self.day_offset) % self.day_length) as f64 / self.day_length as f64;
        let second_of_day = (perc * 86400.0) as i64;
        Some((second_of_day / 3600, (second_of_day % 3600) / 60, (perc * 100.0) as i64))
    }
}

#[derive(Debug, Clone, Default)]
pub enum EnvState {
    #[default]
    None,
    Loading,
    Ready(EnvSummary),
    Failed(String),
}

/// The estate covenant of the selection region (EstateCovenantReply).
#[derive(Debug, Clone)]
pub struct Covenant {
    pub handle: RegionHandle,
    pub id: Uuid,
    pub timestamp: u32,
    pub estate_name: String,
    pub estate_owner: Uuid,
    /// None while the notecard is loading.
    pub text: Option<String>,
}

/// Objects held by the land of the parcel's owner in the region, against
/// what it may hold: the "[COUNT] out of [MAX] ([AVAILABLE] available)" of
/// LLPanelLandObjects::refresh (getSimWidePrimCount /
/// getSimWideMaxPrimCapacity). The parcels of one owner share their capacity,
/// so this, not the parcel's own count, tells what can still be rezzed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrimUsage {
    pub used: i32,
    pub max: i32,
}

impl PrimUsage {
    /// `region_max`: the region's object capacity (0 = unknown), which the
    /// owner's capacity never exceeds whatever the bonus.
    pub fn of(p: &ParcelInfo, region_max: u32) -> PrimUsage {
        let mut max = p.sim_max_prims.max(0);
        if region_max > 0 {
            max = max.min(i32::try_from(region_max).unwrap_or(i32::MAX));
        }
        PrimUsage {
            used: p.sim_total_prims.max(0),
            max,
        }
    }

    /// Objects that can still be rezzed; negative when some will be returned.
    pub fn available(&self) -> i32 {
        self.max - self.used
    }

    /// Part of the capacity used, 0 to 1 (full when there is no capacity
    /// but objects, empty when there is neither).
    pub fn fraction(&self) -> f32 {
        if self.max > 0 {
            (self.used as f32 / self.max as f32).clamp(0.0, 1.0)
        } else if self.used > 0 {
            1.0
        } else {
            0.0
        }
    }
}

/// Room left on the owner's land, as the header's gauge colors it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimRoom {
    /// More than a fifth of the capacity is free.
    Plenty,
    /// A fifth of the capacity or less is left.
    Low,
    /// A tenth or less is left, or the capacity is exceeded.
    Critical,
}

impl PrimUsage {
    /// The thresholds are the human's: caution with 20 % left, danger with
    /// 10 % left.
    pub fn room(&self) -> PrimRoom {
        let (left, max) = (i64::from(self.available()), i64::from(self.max));
        if left * 10 <= max {
            PrimRoom::Critical
        } else if left * 5 <= max {
            PrimRoom::Low
        } else {
            PrimRoom::Plenty
        }
    }
}

/// The selected parcel and what was fetched about it.
#[derive(Debug, Clone)]
pub struct Selection {
    pub handle: RegionHandle,
    pub parcel: Option<Arc<ParcelInfo>>,
    /// PARCEL_RESULT_* of the last answer.
    pub result: i32,
    /// None = "Loading..." (DWELL_NAN).
    pub dwell: Option<f32>,
    pub access: Vec<AccessEntry>,
    pub bans: Vec<AccessEntry>,
    pub allowed_experiences: Vec<Uuid>,
    pub blocked_experiences: Vec<Uuid>,
    pub owners: OwnerList,
    /// Parcel UUID (RemoteParcelRequest): None asked, Some(None) failed.
    pub parcel_uuid: Option<Option<Uuid>>,
    uuid_local_id: i32,
    pub env: EnvState,
    env_key: Option<(i32, i32)>,
    /// Bumped on every answer, so the floater reloads its fields.
    pub revision: u64,
    /// Rectangle of the request (west, south, east, north), sent again by a
    /// refresh.
    rect: (f32, f32, f32, f32),
    /// When the parcel is asked again (`Land::tick`), and not before
    /// `refresh_hold` (a change sent from here).
    refresh_at: Instant,
    refresh_hold: Instant,
    /// The last refresh sent, and whether its answer is still awaited.
    refresh_sent: Option<Instant>,
    refresh_pending: bool,
    /// The parcel was changed from here while that refresh was on its way:
    /// its answer is older than what the floater shows.
    refresh_stale: bool,
}

impl Selection {
    fn new(handle: RegionHandle, rect: (f32, f32, f32, f32)) -> Selection {
        Selection {
            rect,
            refresh_at: Instant::now() + REFRESH_IDLE,
            refresh_hold: Instant::now(),
            refresh_sent: None,
            refresh_pending: false,
            refresh_stale: false,
            handle,
            parcel: None,
            result: land::PARCEL_RESULT_NO_DATA,
            dwell: None,
            access: Vec::new(),
            bans: Vec::new(),
            allowed_experiences: Vec::new(),
            blocked_experiences: Vec::new(),
            owners: OwnerList::Idle,
            parcel_uuid: None,
            uuid_local_id: 0,
            env: EnvState::None,
            env_key: None,
            revision: 0,
        }
    }

    pub fn local_id(&self) -> Option<i32> {
        self.parcel.as_ref().map(|p| p.local_id)
    }
}

/// Context of the agent the selection logic needs from the world.
pub struct SelectionContext {
    /// Region the agent is in.
    pub agent_region: Option<RegionHandle>,
    /// RegionID of the selection region (for RemoteParcelRequest).
    pub region_id: Uuid,
}

/// What "Script info" knows (LLFloaterScriptLimits / LLFloaterMyScripts):
/// None while asked, then the answer or why it failed.
#[derive(Debug, Clone, Default)]
pub struct ScriptInfo {
    /// Resource replies or a new request invalidate the UI's sorted rows.
    pub revision: u64,
    pub summary: Option<Result<land::ScriptSummary, String>>,
    pub details: Option<Result<Vec<land::ScriptParcel>, String>>,
    pub mine: Option<Result<(land::ScriptSummary, Vec<land::ScriptAttachment>), String>>,
}

impl ScriptInfo {
    pub fn clear_parcel(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.summary = None;
        self.details = None;
    }
}

#[derive(Default)]
pub struct Land {
    pub sel: Option<Selection>,
    pub covenant: Option<Covenant>,
    covenant_asked: Option<(RegionHandle, Instant)>,
    pub experience_names: HashMap<Uuid, String>,
    experiences_asked: HashSet<Uuid>,
    pub group_names: HashMap<Uuid, String>,
    groups_asked: HashSet<Uuid>,
    /// Avatar picker search: the query and its results (None while searching).
    pub avatar_search: Option<(String, Option<Vec<FoundAvatar>>)>,
    /// MIME type found for a media URL being set (url, type).
    pub media_type: Option<(String, Option<String>)>,
    /// Experience picker search: the query and its results (None while
    /// searching, Some(None) when it failed).
    pub experience_search: Option<(String, Option<Option<Vec<land::FoundExperience>>>)>,
    /// Objects the simulator asked to highlight (ForceObjectSelect after
    /// "Show"): LLSelectMgr's highlighted objects, by region and local ID.
    pub highlight: HashSet<(RegionHandle, u32)>,
    /// "Script info" of the selected parcel and of my attachments.
    pub scripts: ScriptInfo,
    /// "Buy Land" asked for the selection (LLViewerParcelMgr::startBuyLand),
    /// true for the active group; the app opens the window.
    pub buy_request: Option<bool>,
    out: Vec<NetCommand>,
}

impl Land {
    pub fn take_commands(&mut self) -> Vec<NetCommand> {
        std::mem::take(&mut self.out)
    }

    fn send(&mut self, c: LandCommand) {
        self.out.push(NetCommand::Land(c));
    }

    /// selectParcelAt: ask for the parcel under `pos` of region `handle`.
    pub fn select_at(&mut self, handle: RegionHandle, pos: Vec3, region_size: (f32, f32)) {
        let (west, south, east, north) = parcel_rect_at(pos, region_size);
        // selectLand refuses a selection of a meter or less
        if east - west <= 1.0 || north - south <= 1.0 {
            return;
        }
        self.sel = Some(Selection::new(handle, (west, south, east, north)));
        self.send(LandCommand::Select {
            handle,
            west,
            south,
            east,
            north,
            snap: true,
        });
    }

    /// LLViewerParcelMgr::startBuyLand: the Buy Land window on the
    /// selection, for the agent or for the active group.
    pub fn request_buy(&mut self, for_group: bool) {
        self.buy_request = Some(for_group);
    }

    /// EstateCovenantRequest for `handle`, sent at once (the Buy Land
    /// window asks each time its parcel changes, updateCovenantInfo).
    pub fn request_covenant(&mut self, handle: RegionHandle) {
        self.covenant_asked = Some((handle, Instant::now()));
        self.send(LandCommand::CovenantRequest { handle });
    }

    /// The same request for a caller that sends it in its own order (the
    /// Region / Estate window, between its panels' requests).
    pub fn covenant_request(&mut self, handle: RegionHandle) -> NetCommand {
        self.covenant_asked = Some((handle, Instant::now()));
        NetCommand::Land(LandCommand::CovenantRequest { handle })
    }

    /// The selection is a parcel of `handle`: objects coming and going there
    /// change what the floater shows.
    pub fn watches(&self, handle: RegionHandle) -> bool {
        self.sel.as_ref().is_some_and(|s| s.handle == handle && s.parcel.is_some())
    }

    /// An object appeared in the selection's region or left it: its prim
    /// counts are asked again shortly.
    pub fn objects_changed(&mut self, handle: RegionHandle, now: Instant) {
        if let Some(sel) = self.sel.as_mut().filter(|s| s.handle == handle && s.parcel.is_some()) {
            sel.refresh_at = sel.refresh_at.min(now + REFRESH_DEBOUNCE);
        }
    }

    /// Ask the selected parcel again when its refresh is due, so the floater
    /// follows the simulator while it stays open. Firestorm only asks on a
    /// new selection (its counts stay as they were when the floater opened);
    /// the request is the same ParcelPropertiesRequest. Once a frame.
    pub fn tick(&mut self, now: Instant) {
        let Some(sel) = self.sel.as_mut().filter(|s| s.parcel.is_some()) else {
            return;
        };
        let since_sent = sel.refresh_sent.map(|t| now.saturating_duration_since(t));
        if sel.refresh_pending && since_sent.is_some_and(|d| d < REFRESH_TIMEOUT) {
            return;
        }
        if now < sel.refresh_at.max(sel.refresh_hold) || since_sent.is_some_and(|d| d < REFRESH_MIN_INTERVAL) {
            return;
        }
        sel.refresh_sent = Some(now);
        sel.refresh_pending = true;
        sel.refresh_stale = false;
        sel.refresh_at = now + REFRESH_IDLE;
        let (handle, (west, south, east, north)) = (sel.handle, sel.rect);
        self.send(LandCommand::Select {
            handle,
            west,
            south,
            east,
            north,
            snap: true,
        });
    }

    /// The floater closed: the selection is no longer referenced (deselectUnused).
    pub fn deselect(&mut self) {
        self.sel = None;
    }

    /// Group name like LLCacheName::getGroupName: Some when known (the
    /// null group is "(none)"), else asked and None.
    pub fn group_name(&mut self, id: &Uuid, ours: &[GroupMembership]) -> Option<String> {
        if id.is_nil() {
            return Some(aurora_i18n::tr!("status-none").into());
        }
        if let Some(g) = ours.iter().find(|g| g.id == *id) {
            return Some(g.name.clone());
        }
        if let Some(n) = self.group_names.get(id) {
            return Some(n.clone());
        }
        if self.groups_asked.insert(*id) {
            self.send(LandCommand::GroupNames(vec![*id]));
        }
        None
    }

    /// Search residents for the avatar picker.
    /// send_parcel_select_objects: the shown objects are dropped, the
    /// simulator answers with the new ones (`owners` for RT_LIST).
    pub fn show_objects(&mut self, return_type: u32, owners: Vec<Uuid>) {
        self.highlight.clear();
        let Some((handle, local_id)) = self.sel.as_ref().and_then(|s| Some((s.handle, s.local_id()?))) else {
            return;
        };
        self.send(LandCommand::SelectObjects {
            handle,
            local_id,
            return_type,
            owners,
        });
    }

    /// StartRequestChain once the parcel UUID is known: LandResources.
    pub fn request_scripts(&mut self, parcel_id: Uuid) {
        self.scripts.clear_parcel();
        self.send(LandCommand::ScriptResources { parcel_id });
    }

    /// LLFloaterMyScripts::requestAttachmentDetails.
    pub fn request_attachment_scripts(&mut self) {
        self.scripts.revision = self.scripts.revision.wrapping_add(1);
        self.scripts.mine = None;
        self.send(LandCommand::AttachmentScripts);
    }

    /// LLPanelWearing::requestAttachmentDetails: the same answer, kept by
    /// the Scripts window if open, names our temporary attachments.
    pub fn request_attachment_names(&mut self) {
        self.send(LandCommand::AttachmentScripts);
    }

    /// returnObjectsFromParcel: the objects by their IDs, one message per
    /// parcel of the selection's region.
    pub fn return_tasks(&mut self, local_id: i32, tasks: Vec<Uuid>) {
        let Some(handle) = self.sel.as_ref().map(|s| s.handle) else {
            return;
        };
        if !tasks.is_empty() {
            self.send(LandCommand::ReturnTasks { handle, local_id, tasks });
        }
    }

    /// LLSelectMgr::unhighlightAll (the floater closed, objects returned).
    pub fn unhighlight(&mut self) {
        self.highlight.clear();
    }

    /// LLPanelExperiencePicker::find: the first page of FindExperienceByName.
    pub fn search_experiences(&mut self, query: &str) {
        let query = query.trim().to_owned();
        self.experience_search = Some((query.clone(), None));
        self.send(LandCommand::ExperienceSearch { query, page: 1 });
    }

    /// LLPanelLandExperiences::experienceAdded: the experience takes `kind`
    /// (EXPERIENCE_KEY_TYPE_*: it leaves the other list) and the list of that
    /// kind goes out, as sendParcelAccessListUpdate(access_type).
    pub fn add_experience(&mut self, kind: u32, id: Uuid) {
        let Some(sel) = self.sel.as_mut() else {
            return;
        };
        let (into, other) = if kind == land::AL_BLOCK_EXPERIENCE {
            (&mut sel.blocked_experiences, &mut sel.allowed_experiences)
        } else {
            (&mut sel.allowed_experiences, &mut sel.blocked_experiences)
        };
        other.retain(|x| *x != id);
        if into.contains(&id) || into.len() >= land::PARCEL_MAX_EXPERIENCE_LIST {
            return;
        }
        into.push(id);
        sel.revision += 1;
        self.send_lists(kind);
    }

    pub fn search_avatars(&mut self, query: &str) {
        let query = query.trim().to_owned();
        if query.is_empty() {
            return;
        }
        self.avatar_search = Some((query.clone(), None));
        self.send(LandCommand::AvatarSearch { query });
    }

    /// LLFloaterURLEntry::onBtnOK: find the type of a media URL (http(s)
    /// only; another scheme is its own type).
    pub fn find_media_type(&mut self, url: &str) {
        let url = url.trim().to_owned();
        let scheme = url.split_once("://").map_or("https", |(s, _)| s).to_lowercase();
        if !url.is_empty() && (scheme == "http" || scheme == "https") {
            self.media_type = Some((url.clone(), None));
            self.send(LandCommand::MediaType { url });
        } else {
            let mime = if url.is_empty() { "none/none".to_owned() } else { scheme };
            self.media_type = Some((url, Some(mime)));
        }
    }

    /// Experience name, asked once when unknown.
    pub fn experience_name(&mut self, id: &Uuid) -> Option<String> {
        if let Some(n) = self.experience_names.get(id) {
            return Some(n.clone());
        }
        if self.experiences_asked.insert(*id) {
            self.send(LandCommand::ExperienceInfo(vec![*id]));
        }
        None
    }

    pub fn apply(&mut self, ev: LandEvent, ctx: &SelectionContext) {
        match ev {
            LandEvent::Selected { handle, info, result } => self.on_selected(handle, info, result, ctx),
            LandEvent::AccessList { local_id, flags, entries } => {
                let Some(sel) = self.sel.as_mut().filter(|s| s.local_id() == Some(local_id)) else {
                    log::warn!("access list for parcel {local_id}, which isn't the selected parcel");
                    return;
                };
                // replies only add: the lists were cleared when asked
                let merge = |list: &mut Vec<AccessEntry>| {
                    for e in &entries {
                        list.retain(|x| x.id != e.id);
                        list.push(*e);
                    }
                };
                let merge_ids = |list: &mut Vec<Uuid>| {
                    for e in &entries {
                        if !list.contains(&e.id) {
                            list.push(e.id);
                        }
                    }
                };
                if flags & land::AL_ACCESS != 0 {
                    merge(&mut sel.access);
                } else if flags & land::AL_BAN != 0 {
                    merge(&mut sel.bans);
                } else if flags & land::AL_ALLOW_EXPERIENCE != 0 {
                    merge_ids(&mut sel.allowed_experiences);
                } else if flags & land::AL_BLOCK_EXPERIENCE != 0 {
                    merge_ids(&mut sel.blocked_experiences);
                }
                sel.revision += 1;
            }
            LandEvent::Dwell { local_id, dwell } => {
                if let Some(sel) = self.sel.as_mut().filter(|s| s.local_id() == Some(local_id)) {
                    sel.dwell = Some(dwell);
                    sel.revision += 1;
                }
            }
            LandEvent::ObjectOwners(rows) => {
                if let Some(sel) = self.sel.as_mut() {
                    // several replies may come: the first one replaces "Searching"
                    match &mut sel.owners {
                        OwnerList::Rows(list) => list.extend(rows),
                        o => *o = OwnerList::Rows(rows),
                    }
                    sel.revision += 1;
                }
            }
            LandEvent::Covenant {
                handle,
                covenant_id,
                timestamp,
                estate_name,
                estate_owner,
            } => {
                // a covenant whose text we already have keeps it
                let text = match &self.covenant {
                    Some(c) if c.id == covenant_id && !covenant_id.is_nil() => c.text.clone(),
                    _ if covenant_id.is_nil() => Some(
                        if estate_owner.is_nil() {
                            aurora_i18n::tr!("world-land-this-estate-has-no-covenant")
                        } else {
                            aurora_i18n::tr!("world-land-this-estate-has-no-covenant-land-here-is-sold")
                        }
                        .into(),
                    ),
                    _ => None,
                };
                self.covenant = Some(Covenant {
                    handle,
                    id: covenant_id,
                    timestamp,
                    estate_name,
                    estate_owner,
                    text,
                });
            }
            LandEvent::CovenantText { covenant_id, text } => {
                if let Some(c) = self.covenant.as_mut().filter(|c| c.id == covenant_id) {
                    c.text = Some(text.unwrap_or_else(|| aurora_i18n::tr!("world-land-could-not-load-this-estate-s-covenant").into()));
                }
            }
            LandEvent::ParcelId { local_id, id } => {
                if let Some(sel) = self.sel.as_mut().filter(|s| s.local_id() == Some(local_id)) {
                    if id.is_none() {
                        // setErrorStatus: asked again on the next refresh
                        sel.uuid_local_id = 0;
                    }
                    sel.parcel_uuid = Some(id);
                }
            }
            LandEvent::Environment { local_id, result } => {
                if let Some(sel) = self.sel.as_mut().filter(|s| s.local_id() == Some(local_id)) {
                    sel.env = match result {
                        Ok(env) => EnvState::Ready(EnvSummary::from_llsd(&env)),
                        Err(e) => EnvState::Failed(e),
                    };
                    sel.revision += 1;
                }
            }
            LandEvent::ExperienceInfo(list) => self.experience_names.extend(list),
            LandEvent::GroupNames(list) => self.group_names.extend(list),
            LandEvent::ScriptSummary(r) => {
                self.scripts.revision = self.scripts.revision.wrapping_add(1);
                self.scripts.summary = Some(r);
            }
            LandEvent::ScriptDetails(r) => {
                self.scripts.revision = self.scripts.revision.wrapping_add(1);
                self.scripts.details = Some(r);
            }
            LandEvent::AttachmentScripts(r) => {
                self.scripts.revision = self.scripts.revision.wrapping_add(1);
                self.scripts.mine = Some(r);
            }
            LandEvent::Highlight { handle, reset, local_ids } => {
                if reset {
                    self.highlight.clear();
                }
                self.highlight.extend(local_ids.into_iter().map(|id| (handle, id)));
            }
            LandEvent::ExperienceSearch { query, results } => {
                if let Some(list) = results.as_ref() {
                    self.experience_names.extend(list.iter().map(|e| (e.id, e.name.clone())));
                }
                if let Some((q, r)) = self.experience_search.as_mut()
                    && *q == query
                {
                    *r = Some(results);
                }
            }
            LandEvent::MediaType { url, mime } => {
                if let Some((u, m)) = self.media_type.as_mut()
                    && *u == url
                {
                    *m = Some(mime);
                }
            }
            LandEvent::AvatarSearch { query, results } => {
                if let Some((q, r)) = self.avatar_search.as_mut()
                    && *q == query
                {
                    *r = Some(results.unwrap_or_default());
                }
            }
        }
    }

    /// processParcelProperties for SELECTED_PARCEL_SEQ_ID, then what
    /// LLFloaterLand's refresh asks for.
    fn on_selected(&mut self, handle: RegionHandle, info: Arc<ParcelInfo>, result: i32, ctx: &SelectionContext) {
        let Some(sel) = self.sel.as_mut() else {
            return;
        };
        sel.handle = handle;
        let local_id = info.local_id;
        let changed = sel.local_id() != Some(local_id);
        let env_version = info.env_version;
        // the answer of a refresh, for the parcel already shown (cut or
        // joined meanwhile, it is a new selection)
        let refresh = std::mem::take(&mut sel.refresh_pending) && !changed;
        let now = Instant::now();
        if refresh && std::mem::take(&mut sel.refresh_stale) {
            // asked before a change made here: what the floater shows is
            // newer, the parcel is asked again once that change is through
            sel.refresh_at = now;
            return;
        }
        sel.refresh_at = now + REFRESH_IDLE;
        sel.parcel = Some(info);
        sel.result = result;
        sel.revision += 1;
        if refresh {
            // the parcel itself only: its lists, its traffic and its key are
            // kept (asking for the lists empties them, and the rows selected
            // in the floater with them); the environment follows its version
            let ask_env = ctx.agent_region == Some(handle) && sel.env_key != Some((local_id, env_version));
            if ask_env {
                sel.env_key = Some((local_id, env_version));
                sel.env = EnvState::Loading;
                self.send(LandCommand::EnvironmentRequest { local_id });
            }
            return;
        }
        if changed {
            sel.owners = OwnerList::Idle;
            sel.env = EnvState::None;
            sel.env_key = None;
        }
        // sendParcelAccessListRequest clears the lists it asks for
        sel.access.clear();
        sel.bans.clear();
        sel.allowed_experiences.clear();
        sel.blocked_experiences.clear();
        sel.dwell = None;
        let same_region = ctx.agent_region == Some(handle);
        let ask_uuid = same_region && (sel.uuid_local_id == 0 || sel.uuid_local_id != local_id);
        if ask_uuid {
            sel.uuid_local_id = local_id;
            sel.parcel_uuid = None;
        }
        // LLPanelLandEnvironment::refresh: only in the agent's region, again
        // when the parcel or its environment version changed
        let ask_env = same_region && sel.env_key != Some((local_id, env_version));
        if ask_env {
            sel.env_key = Some((local_id, env_version));
            sel.env = EnvState::Loading;
        }
        self.send(LandCommand::AccessListRequest {
            handle,
            local_id,
            flags: land::AL_ACCESS | land::AL_BAN | land::AL_ALLOW_EXPERIENCE | land::AL_BLOCK_EXPERIENCE,
        });
        // no dwell for public land
        if local_id != 0 {
            self.send(LandCommand::DwellRequest { handle, local_id });
        }
        if ask_uuid {
            let pos = self.sel.as_ref().and_then(|s| s.parcel.as_ref()).map_or(Vec3::ZERO, |p| {
                // LLParcel::getCenterpoint
                Vec3::new((p.aabb_min.x + p.aabb_max.x) * 0.5, (p.aabb_min.y + p.aabb_max.y) * 0.5, 0.0)
            });
            self.send(LandCommand::ParcelIdRequest {
                handle,
                local_id,
                region_id: ctx.region_id,
                position: pos,
            });
        }
        if ask_env {
            self.send(LandCommand::EnvironmentRequest { local_id });
        }
        let stale = match self.covenant_asked {
            Some((h, at)) => h != handle || at.elapsed() >= COVENANT_REFRESH,
            None => true,
        };
        if stale {
            self.covenant_asked = Some((handle, Instant::now()));
            self.send(LandCommand::CovenantRequest { handle });
        }
    }

    /// sendParcelPropertiesUpdate after changing the selected parcel with `f`.
    pub fn update(&mut self, f: impl FnOnce(&mut ParcelUpdate)) {
        let Some(sel) = self.sel.as_mut() else {
            return;
        };
        let Some(p) = sel.parcel.as_mut() else {
            return;
        };
        let mut u = ParcelUpdate::from_parcel(p);
        f(&mut u);
        apply_update(Arc::make_mut(p), &u);
        sel.revision += 1;
        // a refresh on its way was asked before this change, and the next
        // one waits for the simulator to have it
        sel.refresh_stale = sel.refresh_pending;
        sel.refresh_hold = Instant::now() + REFRESH_AFTER_UPDATE;
        let handle = sel.handle;
        self.send(LandCommand::Update {
            handle,
            update: Box::new(u),
        });
    }

    /// sendParcelAccessListUpdate of the lists in `which` (AL_* bits).
    pub fn send_lists(&mut self, which: u32) {
        let Some(sel) = self.sel.as_ref() else {
            return;
        };
        let Some(local_id) = sel.local_id() else {
            return;
        };
        let handle = sel.handle;
        let ids = |v: &[Uuid]| -> Vec<AccessEntry> { v.iter().map(|&id| AccessEntry { id, time: 0, flags: 0 }).collect() };
        let lists = [
            (land::AL_ACCESS, sel.access.clone()),
            (land::AL_BAN, sel.bans.clone()),
            (land::AL_ALLOW_EXPERIENCE, ids(&sel.allowed_experiences)),
            (land::AL_BLOCK_EXPERIENCE, ids(&sel.blocked_experiences)),
        ];
        for (flag, entries) in lists {
            if which & flag != 0 {
                self.send(LandCommand::AccessListUpdate {
                    handle,
                    local_id,
                    flags: flag,
                    entries,
                });
            }
        }
    }

    /// Send a parcel request needing the selection's region and local id.
    pub fn parcel_command(&mut self, f: impl FnOnce(RegionHandle, i32) -> LandCommand) {
        if let Some(sel) = self.sel.as_ref()
            && let Some(local_id) = sel.local_id()
        {
            let c = f(sel.handle, local_id);
            self.send(c);
        }
    }

    /// onClickRefresh: ParcelObjectOwnersRequest, "Search..." meanwhile.
    pub fn refresh_owners(&mut self) {
        if let Some(sel) = self.sel.as_mut() {
            sel.owners = OwnerList::Searching;
        }
        self.parcel_command(|handle, local_id| LandCommand::ObjectOwnersRequest { handle, local_id });
    }

    /// Environment day length / offset (seconds) or reset, then the answer
    /// replaces what the tab shows.
    pub fn environment(&mut self, c: impl FnOnce(i32) -> LandCommand) {
        if let Some(sel) = self.sel.as_mut()
            && let Some(local_id) = sel.local_id()
        {
            sel.env = EnvState::Loading;
            let cmd = c(local_id);
            self.send(cmd);
        }
    }
}

#[cfg(test)]
mod tests {
    /// "Show": the old highlight goes at once, the simulator's answer
    /// (ForceObjectSelect) brings the new one; closing drops it.
    #[test]
    fn shown_objects_come_from_the_simulator() {
        let mut land = Land::default();
        let mut sel = Selection::new(7, (0.0, 0.0, 4.0, 4.0));
        sel.parcel = Some(Arc::new(ParcelInfo {
            local_id: 3,
            ..Default::default()
        }));
        land.sel = Some(sel);
        land.highlight.insert((7, 1));
        land.show_objects(land::RT_OWNER, Vec::new());
        assert!(land.highlight.is_empty());
        assert!(land.take_commands().iter().any(|c| matches!(
            c,
            NetCommand::Land(LandCommand::SelectObjects { handle: 7, local_id: 3, return_type, .. }) if *return_type == land::RT_OWNER
        )));
        land.apply(
            LandEvent::Highlight {
                handle: 7,
                reset: true,
                local_ids: vec![10, 11],
            },
            &SelectionContext {
                agent_region: Some(7),
                region_id: Uuid::nil(),
            },
        );
        assert_eq!(land.highlight.len(), 2);
        land.unhighlight();
        assert!(land.highlight.is_empty());
    }

    /// experienceAdded: the experience leaves the other list, and only
    /// the list it joined goes out.
    #[test]
    fn an_added_experience_changes_lists() {
        let mut land = Land::default();
        let mut sel = Selection::new(1, (0.0, 0.0, 4.0, 4.0));
        sel.parcel = Some(Arc::new(ParcelInfo {
            local_id: 3,
            ..Default::default()
        }));
        let xp = Uuid::from_u128(9);
        sel.blocked_experiences.push(xp);
        land.sel = Some(sel);
        land.add_experience(land::AL_ALLOW_EXPERIENCE, xp);
        let sel = land.sel.as_ref().expect("selection");
        assert_eq!(sel.allowed_experiences, vec![xp]);
        assert!(sel.blocked_experiences.is_empty());
        let sent: Vec<u32> = land
            .take_commands()
            .into_iter()
            .filter_map(|c| match c {
                NetCommand::Land(LandCommand::AccessListUpdate { flags, .. }) => Some(flags),
                _ => None,
            })
            .collect();
        assert_eq!(sent, vec![land::AL_ALLOW_EXPERIENCE]);
    }

    use super::*;

    fn me() -> Uuid {
        Uuid::from_u128(1)
    }

    fn group(id: u128, powers: u64) -> GroupMembership {
        GroupMembership {
            id: Uuid::from_u128(id),
            name: aurora_i18n::tr!("common-group").into(),
            insignia: Uuid::nil(),
            powers,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        }
    }

    #[test]
    fn modify_rules_follow_ownership_and_powers() {
        let groups = [group(50, powers::LAND_OPTIONS)];
        let a = AgentRights { id: me(), groups: &groups };
        let mut p = ParcelInfo {
            owner_id: me(),
            ..Default::default()
        };
        assert!(modifiable_by_agent(&p, &a, powers::LAND_OPTIONS));
        // a purchase not approved yet: ours, but not modifiable
        p.status = land::OS_LEASE_PENDING;
        assert!(owned_by_agent(&p, &a, powers::NONE));
        assert!(!modifiable_by_agent(&p, &a, powers::NONE));
        // group land: the matching power only (GP_NO_POWERS is never had)
        p.owner_id = Uuid::from_u128(50);
        p.status = land::OS_LEASED;
        assert!(modifiable_by_agent(&p, &a, powers::LAND_OPTIONS));
        assert!(!modifiable_by_agent(&p, &a, powers::LAND_CHANGE_MEDIA));
        assert!(!modifiable_by_agent(&p, &a, powers::NONE));
        // public land: never
        p.owner_id = Uuid::nil();
        assert!(!owned_by_agent(&p, &a, powers::LAND_OPTIONS));
    }

    #[test]
    fn buying_needs_a_sale_for_us() {
        let a = AgentRights { id: me(), groups: &[] };
        let mut p = ParcelInfo {
            owner_id: Uuid::from_u128(9),
            ..Default::default()
        };
        assert!(!can_agent_buy(&p, &a, None, true));
        p.flags = pf::FOR_SALE;
        p.sale_price = 500;
        assert!(can_agent_buy(&p, &a, None, true));
        assert!(!can_agent_buy(&p, &a, None, false));
        p.auth_buyer = Uuid::from_u128(7);
        assert!(!can_agent_buy(&p, &a, None, true));
        // for a group we have no deed power in
        p.auth_buyer = Uuid::nil();
        assert!(!can_agent_buy(&p, &a, Some(Uuid::from_u128(50)), true));
    }

    #[test]
    fn select_snaps_to_the_parcel_grid() {
        assert_eq!(
            parcel_rect_at(Vec3::new(130.3, 61.0, 22.0), (256.0, 256.0)),
            (128.0, 60.0, 132.0, 64.0)
        );
        assert_eq!(parcel_rect_at(Vec3::new(255.5, 0.5, 0.0), (256.0, 256.0)), (252.0, 0.0, 256.0, 4.0));
    }

    #[test]
    fn access_entries_like_llparcel() {
        let owner = Uuid::from_u128(9);
        let mut list = Vec::new();
        assert!(add_entry(&mut list, &owner, Uuid::from_u128(2), 100));
        // the owner cannot be listed
        assert!(!add_entry(&mut list, &owner, owner, 0));
        // an entry that expires sooner is not kept over a later one
        assert!(!add_entry(&mut list, &owner, Uuid::from_u128(2), 50));
        assert!(add_entry(&mut list, &owner, Uuid::from_u128(2), 0));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].time, 0);
    }

    #[test]
    fn prim_usage_is_the_owner_s_in_the_region() {
        let mut p = ParcelInfo {
            total_prims: 61,
            max_prims: 3750,
            sim_total_prims: 5387,
            sim_max_prims: 15000,
            ..Default::default()
        };
        let u = PrimUsage::of(&p, 0);
        assert_eq!((u.used, u.max, u.available()), (5387, 15000, 9613));
        assert!((u.fraction() - 5387.0 / 15000.0).abs() < 1e-6);
        // never more than the region holds, whatever the bonus
        assert_eq!(PrimUsage::of(&p, 5000).max, 5000);
        // over the capacity: a full bar, the excess is to be returned
        p.sim_total_prims = 16000;
        let over = PrimUsage::of(&p, 0);
        assert_eq!((over.available(), over.fraction()), (-1000, 1.0));
        // public land has no capacity
        let none = PrimUsage::of(&ParcelInfo::default(), 15000);
        assert_eq!((none.max, none.fraction()), (0, 0.0));
    }

    #[test]
    fn the_gauge_warns_with_a_fifth_left_and_alarms_with_a_tenth() {
        let room = |used: i32| PrimUsage { used, max: 1000 }.room();
        assert_eq!(room(0), PrimRoom::Plenty);
        assert_eq!(room(799), PrimRoom::Plenty);
        // 20 % left
        assert_eq!(room(800), PrimRoom::Low);
        assert_eq!(room(899), PrimRoom::Low);
        // 10 % left, nothing left, more than the land may hold
        assert_eq!(room(900), PrimRoom::Critical);
        assert_eq!(room(1000), PrimRoom::Critical);
        assert_eq!(room(1200), PrimRoom::Critical);
    }

    const REGION: RegionHandle = 7;

    fn parcel(local_id: i32, used: i32, name: &str) -> Arc<ParcelInfo> {
        Arc::new(ParcelInfo {
            local_id,
            name: name.into(),
            sim_total_prims: used,
            sim_max_prims: 100,
            ..Default::default()
        })
    }

    fn answer(land: &mut Land, info: Arc<ParcelInfo>) {
        let ctx = SelectionContext {
            agent_region: Some(REGION),
            region_id: Uuid::nil(),
        };
        let ev = LandEvent::Selected {
            handle: REGION,
            info,
            result: land::PARCEL_RESULT_SUCCESS,
        };
        land.apply(ev, &ctx);
    }

    /// (parcel requests, list requests) queued since the last call.
    fn asked(land: &mut Land) -> (usize, usize) {
        let cmds = land.take_commands();
        let count = |f: fn(&LandCommand) -> bool| cmds.iter().filter(|c| matches!(c, NetCommand::Land(l) if f(l))).count();
        (
            count(|l| matches!(l, LandCommand::Select { .. })),
            count(|l| matches!(l, LandCommand::AccessListRequest { .. })),
        )
    }

    fn selected(land: &mut Land) -> Instant {
        land.select_at(REGION, Vec3::new(10.0, 10.0, 0.0), (256.0, 256.0));
        answer(land, parcel(1, 10, "A"));
        assert_eq!(asked(land), (1, 1));
        Instant::now()
    }

    #[test]
    fn the_selected_parcel_follows_the_objects_of_its_region() {
        let mut land = Land::default();
        land.select_at(REGION, Vec3::new(10.0, 10.0, 0.0), (256.0, 256.0));
        assert_eq!(asked(&mut land), (1, 0));
        // nothing to refresh before the parcel is known
        land.objects_changed(REGION, Instant::now());
        land.tick(Instant::now() + REFRESH_IDLE * 2);
        assert_eq!(asked(&mut land), (0, 0));
        answer(&mut land, parcel(1, 10, "A"));
        assert_eq!(asked(&mut land), (0, 1));
        let t = Instant::now();
        land.tick(t + REFRESH_DEBOUNCE);
        assert_eq!(asked(&mut land), (0, 0), "nothing changed yet");
        // an object in another region is not ours to count
        land.objects_changed(REGION + 1, t);
        land.tick(t + REFRESH_DEBOUNCE);
        assert_eq!(asked(&mut land), (0, 0));
        // a burst of objects in ours is one request, a moment later
        land.objects_changed(REGION, t);
        land.objects_changed(REGION, t + REFRESH_DEBOUNCE / 4);
        land.tick(t + REFRESH_DEBOUNCE / 2);
        assert_eq!(asked(&mut land), (0, 0));
        let sent = t + REFRESH_DEBOUNCE;
        land.tick(sent);
        assert_eq!(asked(&mut land), (1, 0));
        // none while its answer is awaited
        land.objects_changed(REGION, sent);
        land.tick(sent + REFRESH_MIN_INTERVAL);
        assert_eq!(asked(&mut land), (0, 0));
        // the answer brings the counts and keeps the lists (and their rows)
        let entry = AccessEntry {
            id: Uuid::from_u128(2),
            time: 0,
            flags: 0,
        };
        land.sel.as_mut().expect("selection").access.push(entry);
        answer(&mut land, parcel(1, 12, "A"));
        let sel = land.sel.as_ref().expect("selection");
        assert_eq!(sel.parcel.as_ref().map(|p| p.sim_total_prims), Some(12));
        assert_eq!(sel.access.len(), 1);
        assert_eq!(asked(&mut land), (0, 0));
        // two refreshes keep their distance
        land.objects_changed(REGION, sent + REFRESH_DEBOUNCE);
        land.tick(sent + REFRESH_DEBOUNCE * 2);
        assert_eq!(asked(&mut land), (0, 0));
        land.tick(sent + REFRESH_MIN_INTERVAL);
        assert_eq!(asked(&mut land), (1, 0));
    }

    #[test]
    fn the_selected_parcel_is_asked_again_when_idle() {
        let mut land = Land::default();
        let t = selected(&mut land);
        land.tick(t + REFRESH_IDLE / 2);
        assert_eq!(asked(&mut land), (0, 0));
        land.tick(t + REFRESH_IDLE + REFRESH_DEBOUNCE);
        assert_eq!(asked(&mut land), (1, 0));
        // a refresh never answered is sent again later
        land.tick(t + REFRESH_IDLE + REFRESH_TIMEOUT);
        assert_eq!(asked(&mut land), (0, 0));
        land.tick(t + REFRESH_IDLE * 2 + REFRESH_TIMEOUT);
        assert_eq!(asked(&mut land), (1, 0));
    }

    #[test]
    fn a_refresh_does_not_undo_a_change_made_meanwhile() {
        let mut land = Land::default();
        let t = selected(&mut land);
        land.objects_changed(REGION, t);
        land.tick(t + REFRESH_DEBOUNCE);
        assert_eq!(asked(&mut land), (1, 0));
        // renamed here while the refresh is on its way: its answer is older
        land.update(|u| u.name = "B".into());
        answer(&mut land, parcel(1, 10, "A"));
        let name = |land: &Land| land.sel.as_ref().and_then(|s| s.parcel.as_ref()).map(|p| p.name.clone());
        assert_eq!(name(&land).as_deref(), Some("B"));
        // and the parcel is asked again, once the simulator has the change
        land.take_commands();
        land.objects_changed(REGION, Instant::now());
        land.tick(Instant::now() + REFRESH_MIN_INTERVAL + REFRESH_DEBOUNCE);
        assert_eq!(asked(&mut land), (0, 0));
        land.tick(Instant::now() + REFRESH_AFTER_UPDATE);
        assert_eq!(asked(&mut land), (1, 0));
        answer(&mut land, parcel(1, 10, "B"));
        assert_eq!(name(&land).as_deref(), Some("B"));
    }

    #[test]
    fn a_refresh_finding_another_parcel_is_a_new_selection() {
        let mut land = Land::default();
        let t = selected(&mut land);
        land.objects_changed(REGION, t);
        land.tick(t + REFRESH_DEBOUNCE);
        assert_eq!(asked(&mut land), (1, 0));
        // the parcel was cut: the piece under the selection has its own lists
        answer(&mut land, parcel(2, 3, "C"));
        assert_eq!(asked(&mut land), (0, 1));
    }

    #[test]
    fn track_names_fall_back_like_the_panel() {
        let mut e = EnvSummary::default();
        // no names at all: ground and water say "region environment",
        // skies take the track below in parentheses
        assert_eq!(e.track_name(1), aurora_i18n::tr!("world-land-region-environment"));
        assert_eq!(e.track_name(2), aurora_i18n::tr!("world-land-region-environment"));
        e.names[2] = "Ciel bleu".into();
        assert_eq!(e.track_name(3), "(Ciel bleu)");
        e.day_name = "Ma journée".into();
        e.tracks_set = [true, true, false, false, false];
        assert_eq!(e.track_name(1), "Ma journée");
        assert_eq!(e.track_name(3), "(Ma journée)");
        assert_eq!(e.track_name(-1), aurora_i18n::tr!("status-empty"));
    }

    #[test]
    fn apparent_time_of_the_cycle() {
        let e = EnvSummary {
            day_length: 4 * 3600,
            day_offset: 3600,
            ..Default::default()
        };
        // a quarter into the 4 h cycle: 06:00, 25 %
        assert_eq!(e.apparent_time(0), Some((6, 0, 25)));
        assert_eq!(EnvSummary::default().apparent_time(0), None);
    }

    #[test]
    fn selection_asks_for_what_the_floater_shows() {
        let mut land = Land::default();
        land.select_at(5, Vec3::new(100.0, 100.0, 20.0), (256.0, 256.0));
        assert!(matches!(
            land.take_commands()[..],
            [NetCommand::Land(LandCommand::Select { snap: true, .. })]
        ));
        let ctx = SelectionContext {
            agent_region: Some(5),
            region_id: Uuid::from_u128(3),
        };
        let info = Arc::new(ParcelInfo {
            local_id: 4,
            ..Default::default()
        });
        land.apply(
            LandEvent::Selected {
                handle: 5,
                info,
                result: 0,
            },
            &ctx,
        );
        let cmds = land.take_commands();
        let has = |f: &dyn Fn(&LandCommand) -> bool| cmds.iter().any(|c| matches!(c, NetCommand::Land(l) if f(l)));
        assert!(has(&|l| matches!(l, LandCommand::AccessListRequest { local_id: 4, .. })));
        assert!(has(&|l| matches!(l, LandCommand::DwellRequest { .. })));
        assert!(has(&|l| matches!(l, LandCommand::ParcelIdRequest { .. })));
        assert!(has(&|l| matches!(l, LandCommand::EnvironmentRequest { local_id: 4 })));
        assert!(has(&|l| matches!(l, LandCommand::CovenantRequest { handle: 5 })));
        // the ban list arrives in two messages
        for n in [10, 11] {
            land.apply(
                LandEvent::AccessList {
                    local_id: 4,
                    flags: land::AL_BAN,
                    entries: vec![AccessEntry {
                        id: Uuid::from_u128(n),
                        time: 0,
                        flags: 0,
                    }],
                },
                &ctx,
            );
        }
        assert_eq!(land.sel.as_ref().map(|s| s.bans.len()), Some(2));
    }
}
