//! The parcel shown in "À propos du terrain": LLViewerParcelMgr's selection
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
    pub const LAND_MANAGE_ALLOWED: u64 = 1 << 29;
    pub const LAND_MANAGE_BANNED: u64 = 1 << 30;
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

/// Object owners list of the "Objets" tab.
#[derive(Debug, Clone, Default)]
pub enum OwnerList {
    /// Not asked yet (or the parcel changed).
    #[default]
    Idle,
    /// "Recherche..." until the first reply.
    Searching,
    Rows(Vec<ObjectOwner>),
}

/// What the "Environnement" tab shows of the parcel environment
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
}

impl EnvSummary {
    pub fn from_llsd(env: &Llsd) -> EnvSummary {
        let has_day = env.has("day_cycle");
        let mut s = EnvSummary {
            day_length: if has_day && env.has("day_length") { env["day_length"].as_i32() as i64 } else { -1 },
            day_offset: if has_day && env.has("day_offset") { env["day_offset"].as_i32() as i64 } else { -1 },
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
            return "(vide)".into();
        }
        let i = index as usize;
        let mut name = if self.day_name.is_empty() {
            let n = self.names[i].clone();
            if n.is_empty() && i <= 1 {
                "(envt de la région)".into()
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

/// The selected parcel and what was fetched about it.
#[derive(Debug, Clone)]
pub struct Selection {
    pub handle: RegionHandle,
    pub parcel: Option<Arc<ParcelInfo>>,
    /// PARCEL_RESULT_* of the last answer.
    pub result: i32,
    /// None = "Chargement..." (DWELL_NAN).
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
}

impl Selection {
    fn new(handle: RegionHandle) -> Selection {
        Selection {
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
        self.sel = Some(Selection::new(handle));
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
    /// null group is "(aucun)"), else asked and None.
    pub fn group_name(&mut self, id: &Uuid, ours: &[GroupMembership]) -> Option<String> {
        if id.is_nil() {
            return Some("(aucun)".into());
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
                            "Il n'y a aucun règlement pour ce domaine."
                        } else {
                            "Il n'y a aucun règlement pour ce domaine. Le terrain sur ce domaine est vendu par le propriétaire.  Pour en savoir plus, veuillez contacter le propriétaire."
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
                    c.text = Some(text.unwrap_or_else(|| "Impossible de charger le règlement de ce domaine.".into()));
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
        sel.parcel = Some(info);
        sel.result = result;
        sel.revision += 1;
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
        let ids = |v: &[Uuid]| -> Vec<AccessEntry> {
            v.iter()
                .map(|&id| AccessEntry {
                    id,
                    time: 0,
                    flags: 0,
                })
                .collect()
        };
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

    /// onClickRefresh: ParcelObjectOwnersRequest, "Recherche..." meanwhile.
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
    use super::*;

    fn me() -> Uuid {
        Uuid::from_u128(1)
    }

    fn group(id: u128, powers: u64) -> GroupMembership {
        GroupMembership {
            id: Uuid::from_u128(id),
            name: "Groupe".into(),
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
        assert_eq!(parcel_rect_at(Vec3::new(130.3, 61.0, 22.0), (256.0, 256.0)), (128.0, 60.0, 132.0, 64.0));
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
    fn track_names_fall_back_like_the_panel() {
        let mut e = EnvSummary::default();
        // no names at all: ground and water say "region environment",
        // skies take the track below in parentheses
        assert_eq!(e.track_name(1), "(envt de la région)");
        assert_eq!(e.track_name(2), "(envt de la région)");
        e.names[2] = "Ciel bleu".into();
        assert_eq!(e.track_name(3), "(Ciel bleu)");
        e.day_name = "Ma journée".into();
        e.tracks_set = [true, true, false, false, false];
        assert_eq!(e.track_name(1), "Ma journée");
        assert_eq!(e.track_name(3), "(Ma journée)");
        assert_eq!(e.track_name(-1), "(vide)");
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
        assert!(matches!(land.take_commands()[..], [NetCommand::Land(LandCommand::Select { snap: true, .. })]));
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
