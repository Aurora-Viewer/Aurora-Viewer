//! Region / Estate data, as LLFloaterRegionInfo reads it
//! (newview/llfloaterregioninfo.cpp, llregioninfomodel.cpp,
//! llestateinfomodel.cpp, llfloaterregionrestartschedule.cpp, originally
//! LGPL 2.1): the region's RegionInfo answer to RequestRegionInfo, the
//! estate's `estateupdateinfo` answer to the EstateOwnerMessage `getinfo`,
//! the access lists of the EstateAccess capability, the experiences of
//! RegionExperiences, the restart schedule of RegionSchedule and the
//! simulator's statistics (SimStats). The parsing is pure; the session
//! (`session/estate.rs`) sends and receives.

use crate::types::RegionHandle;
use aurora_llsd::Llsd;
use std::sync::Arc;
use uuid::Uuid;

/// `REGION_FLAGS_*` bits the estate uses (llregionflags.h), in the same
/// space as `types::region_flags`.
pub mod estate_flags {
    pub const SUN_FIXED: u64 = 1 << 4;
    pub const ALLOW_ACCESS_OVERRIDE: u64 = 1 << 5;
    pub const ALLOW_ENVIRONMENT_OVERRIDE: u64 = 1 << 9;
    pub const EXTERNALLY_VISIBLE: u64 = 1 << 15;
    pub const ALLOW_DIRECT_TELEPORT: u64 = 1 << 20;
    pub const DENY_ANONYMOUS: u64 = 1 << 23;
    pub const ALLOW_VOICE: u64 = 1 << 28;
    pub const DENY_AGEUNVERIFIED: u64 = 1 << 30;
    pub const DENY_BOTS: u64 = 1 << 31;
}

/// The last estate owned by Linden Lab (ESTATE_LAST_LINDEN): the mainland
/// and the Linden estates (isLindenEstate).
pub const ESTATE_LAST_LINDEN: u32 = 5;

/// `ESTATE_ACCESS_*` flags of `estateaccessdelta` (llregionflags.h).
pub mod estate_access {
    pub const APPLY_TO_ALL_ESTATES: u32 = 1 << 0;
    pub const APPLY_TO_MANAGED_ESTATES: u32 = 1 << 1;
    pub const ALLOWED_AGENT_ADD: u32 = 1 << 2;
    pub const ALLOWED_AGENT_REMOVE: u32 = 1 << 3;
    pub const ALLOWED_GROUP_ADD: u32 = 1 << 4;
    pub const ALLOWED_GROUP_REMOVE: u32 = 1 << 5;
    pub const BANNED_AGENT_ADD: u32 = 1 << 6;
    pub const BANNED_AGENT_REMOVE: u32 = 1 << 7;
    pub const MANAGER_ADD: u32 = 1 << 8;
    pub const MANAGER_REMOVE: u32 = 1 << 9;
    pub const NO_REPLY: u32 = 1 << 10;
}

/// `ESTATE_EXPERIENCE_*` flags of `estateexperiencedelta` (llregionflags.h),
/// with `estate_access`'s APPLY_TO_* and NO_REPLY.
pub mod estate_experience {
    pub const TRUSTED_ADD: u32 = 1 << 2;
    pub const TRUSTED_REMOVE: u32 = 1 << 3;
    pub const ALLOWED_ADD: u32 = 1 << 4;
    pub const ALLOWED_REMOVE: u32 = 1 << 5;
    pub const BLOCKED_ADD: u32 = 1 << 6;
    pub const BLOCKED_REMOVE: u32 = 1 << 7;
}

/// MAX_SPAWNPOINTS_PER_TELEHUB (llfloatertelehub.h).
pub const MAX_SPAWNPOINTS_PER_TELEHUB: usize = 16;

/// A request of the telehub floater (LLFloaterTelehub, through
/// LLSelectMgr::sendGodlikeRequest: EstateOwnerMessage `telehub` with a
/// nil invoice, the verb, then the selected root objects' local ids).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelehubVerb {
    /// "info ui": TelehubInfo comes back.
    Info,
    Connect,
    Delete,
    AddSpawn,
    /// "spawnpoint remove" with the spawn point's index, without the
    /// selection (onClickRemoveSpawnPoint).
    RemoveSpawn(u32),
}

impl TelehubVerb {
    /// The message's parameters.
    pub fn strings(self, objects: &[u32]) -> Vec<String> {
        let verb = match self {
            TelehubVerb::Info => "info ui",
            TelehubVerb::Connect => "connect",
            TelehubVerb::Delete => "delete",
            TelehubVerb::AddSpawn => "spawnpoint add",
            TelehubVerb::RemoveSpawn(i) => return vec!["spawnpoint remove".to_owned(), i.to_string()],
        };
        std::iter::once(verb.to_owned()).chain(objects.iter().map(u32::to_string)).collect()
    }
}

/// TelehubInfo: the region's telehub (a nil object without one) and its
/// spawn points, relative to it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TelehubInfo {
    pub object: Uuid,
    pub name: String,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub spawns: Vec<[f32; 3]>,
}

/// One terrain material's texture transform (LLGLTFMaterial::TextureTransform,
/// the same for its four texture infos: the PBR terrain shader takes one).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainTransform {
    pub scale: [f32; 2],
    /// Radians.
    pub rotation: f32,
    pub offset: [f32; 2],
}

impl Default for TerrainTransform {
    fn default() -> Self {
        TerrainTransform {
            scale: [1.0, 1.0],
            rotation: 0.0,
            offset: [0.0, 0.0],
        }
    }
}

/// GLTF_TEXTURE_INFO_COUNT.
const TEXTURE_INFOS: usize = 4;

/// The ModifyRegion POST of the terrain's material overrides
/// (LLPBRTerrainFeatures::queueModify, LLGLTFMaterial::getOverrideLLSD):
/// an empty map for a default transform, else `ti` with what differs.
pub fn terrain_overrides_body(transforms: &[TerrainTransform; 4]) -> Llsd {
    let overrides = transforms
        .iter()
        .map(|t| {
            let mut m = Llsd::new_map();
            if *t == TerrainTransform::default() {
                return m;
            }
            let pair = |v: [f32; 2]| Llsd::Array(v.iter().map(|x| Llsd::Real(f64::from(*x))).collect());
            let mut info = Llsd::new_map();
            if t.offset != [0.0, 0.0] {
                info.insert("o", pair(t.offset));
            }
            if t.scale != [1.0, 1.0] {
                info.insert("s", pair(t.scale));
            }
            if t.rotation != 0.0 {
                info.insert("r", Llsd::Real(f64::from(t.rotation)));
            }
            m.insert("ti", Llsd::Array(vec![info; TEXTURE_INFOS]));
            m
        })
        .collect();
    let mut body = Llsd::new_map();
    body.insert("overrides", Llsd::Array(overrides));
    body
}

/// queryRegionCoro: the four materials' overrides (the base color's
/// transform), None when the answer failed or misses some.
pub fn parse_terrain_overrides(v: &Llsd) -> Option<[TerrainTransform; 4]> {
    if !v["success"].as_bool() {
        return None;
    }
    let overrides = v["overrides"].as_array();
    if overrides.len() < 4 {
        return None;
    }
    Some(std::array::from_fn(|i| {
        let info = &overrides[i]["ti"].at(0);
        let pair = |key: &str, default: [f32; 2]| {
            let a = &info[key];
            if a.is_array() {
                [a.at(0).as_f32(), a.at(1).as_f32()]
            } else {
                default
            }
        };
        TerrainTransform {
            scale: pair("s", [1.0, 1.0]),
            rotation: if info.has("r") { info["r"].as_f32() } else { 0.0 },
            offset: pair("o", [0.0, 0.0]),
        }
    }))
}

/// A RAW terrain file sent or asked (LLPanelRegionTerrainInfo
/// onDownloadRawFilepickerCB / onUploadRawFilepickerCB): `terrain`
/// "download filename" / "upload filename" with the local path, then the
/// file goes by Xfer.
#[derive(Clone, PartialEq)]
pub enum RawTransfer {
    Download { path: String },
    Upload { path: String, data: Arc<Vec<u8>> },
}

impl std::fmt::Debug for RawTransfer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RawTransfer::Download { path } => write!(f, "Download({path})"),
            RawTransfer::Upload { path, data } => write!(f, "Upload({path}, {} bytes)", data.len()),
        }
    }
}

/// ESTATE_MAX_EXPERIENCE_IDS: each of the region's experience lists.
pub const ESTATE_MAX_EXPERIENCE_IDS: usize = 8;

/// The estate lists' limits on Second Life (ESTATE_MAX_MANAGERS,
/// ESTATE_MAX_ACCESS_IDS, ESTATE_MAX_GROUP_IDS, ESTATE_MAX_BANNED_IDS).
pub const ESTATE_MAX_MANAGERS: usize = 20;
pub const ESTATE_MAX_ACCESS_IDS: usize = 500;
pub const ESTATE_MAX_GROUP_IDS: usize = 63;
pub const ESTATE_MAX_BANNED_IDS: usize = 750;

/// The flags of each `estateaccessdelta` of one change
/// (LLPanelEstateAccess::accessCoreConfirm): `NO_REPLY` on all but the last,
/// so that the simulator answers once. An import sends each one alone
/// (importListCallback): `batch` false, no `NO_REPLY`.
pub fn access_delta_flags(flags: u32, count: usize, batch: bool) -> Vec<u32> {
    (0..count)
        .map(|i| {
            if batch && i + 1 != count {
                flags | estate_access::NO_REPLY
            } else {
                flags & !estate_access::NO_REPLY
            }
        })
        .collect()
}

/// `estateaccessdelta`'s strings (sendEstateAccessDelta): who asks, the
/// flags (`"%u"`), the resident or group changed.
pub fn access_delta_strings(agent: Uuid, flags: u32, id: Uuid) -> Vec<String> {
    vec![agent.hyphenated().to_string(), flags.to_string(), id.hyphenated().to_string()]
}

/// `SWD_*` flags of a return of a resident's objects (llregionflags.h):
/// SimWideDeletes' Flags, `estateobjectreturn`'s first string.
pub mod swd {
    pub const OTHERS_LAND_ONLY: u32 = 1 << 0;
    pub const ALWAYS_RETURN_OBJECTS: u32 = 1 << 1;
    pub const SCRIPTED_ONLY: u32 = 1 << 2;
}

/// LLPanelRegionDebugInfo::onClickReturn: always returned, then the
/// panel's two filters.
pub fn return_flags(scripted_only: bool, others_land_only: bool) -> u32 {
    let mut f = swd::ALWAYS_RETURN_OBJECTS;
    if scripted_only {
        f |= swd::SCRIPTED_ONLY;
    }
    if others_land_only {
        f |= swd::OTHERS_LAND_ONLY;
    }
    f
}

/// The restart delay's spinner (panel_region_debug.xml `restart_delay`):
/// 120 s by default, 15 s to an hour.
pub const RESTART_DELAY_DEFAULT: u32 = 120;
pub const RESTART_DELAY_MIN: u32 = 15;
pub const RESTART_DELAY_MAX: u32 = 3600;

/// `simulatormessage`'s strings (LLPanelRegionGeneralInfo::onMessageCommit):
/// grid x and y (unused, "-1"), the sender, their name, the text.
pub fn region_message_strings(agent: Uuid, name: &str, text: &str) -> Vec<String> {
    vec![
        "-1".to_owned(),
        "-1".to_owned(),
        agent.hyphenated().to_string(),
        name.to_owned(),
        text.to_owned(),
    ]
}

/// `instantmessage`'s strings (LLPanelEstateInfo::onMessageCommit): the
/// sender's name, the text.
pub fn estate_message_strings(name: &str, text: &str) -> Vec<String> {
    vec![name.to_owned(), text.to_owned()]
}

/// The RegionSchedule capability's body (LLFloaterRegionRestartSchedule::
/// onSaveButtonClicked): every day is a daily restart ("D"), fewer a weekly
/// one ("W") with the days' capital letters, Sunday first — none clears the
/// schedule; the time in seconds after midnight UTC.
pub fn restart_schedule_body(days: [bool; 7], seconds: u32) -> Llsd {
    let chosen: String = DAY_LETTERS
        .iter()
        .zip(days)
        .filter(|(_, on)| *on)
        .map(|(c, _)| c.to_ascii_uppercase())
        .collect();
    let restart = if chosen.len() < 7 {
        aurora_llsd::llsd_map! {
            "type" => "W",
            "days" => chosen.as_str(),
            "time" => seconds.min(86_399) as i32,
        }
    } else {
        aurora_llsd::llsd_map! {
            "type" => "D",
            "time" => seconds.min(86_399) as i32,
        }
    };
    aurora_llsd::llsd_map! { "restart" => restart }
}

/// The Debug panel's three switches (LLPanelRegionDebugInfo): the region's
/// `REGION_FLAGS_SKIP_*` bits as processRegionInfo reads them, and
/// `setregiondebug`'s "Y" / "N" strings as sendUpdate writes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RegionDebug {
    pub skip_scripts: bool,
    pub skip_collisions: bool,
    pub skip_physics: bool,
}

/// REGION_FLAGS_SKIP_COLLISIONS, _SKIP_SCRIPTS, _SKIP_PHYSICS (llregionflags.h).
pub mod skip_flags {
    pub const COLLISIONS: u64 = 1 << 12;
    pub const SCRIPTS: u64 = 1 << 13;
    pub const PHYSICS: u64 = 1 << 14;
}

impl RegionDebug {
    pub fn of(d: &RegionDetails) -> Self {
        RegionDebug {
            skip_scripts: d.flags & skip_flags::SCRIPTS != 0,
            skip_collisions: d.flags & skip_flags::COLLISIONS != 0,
            skip_physics: d.flags & skip_flags::PHYSICS != 0,
        }
    }

    pub fn strings(&self) -> Vec<String> {
        vec![yes_no(self.skip_scripts), yes_no(self.skip_collisions), yes_no(self.skip_physics)]
    }
}

/// The Top scripts / Top colliders report (LLFloaterTopObjects,
/// llfloatertopobjects.h): its two kinds and the filters of LandStatRequest.
pub mod land_stat {
    pub const TOP_SCRIPTS: u32 = 0;
    pub const TOP_COLLIDERS: u32 = 1;
    pub const FILTER_BY_PARCEL: u32 = 1 << 0;
    pub const FILTER_BY_OWNER: u32 = 1 << 1;
    pub const FILTER_BY_OBJECT: u32 = 1 << 2;
    pub const FILTER_BY_PARCEL_NAME: u32 = 1 << 3;
}

/// The Debug panel's "Top scripts" / "Top colliders" (onClickTopScripts,
/// onClickTopColliders): the estate message that asks the report, and its
/// one string ("6": the top 5 scripts; "1": one physics step).
pub fn top_request(report: u32) -> (&'static str, &'static str) {
    if report == land_stat::TOP_COLLIDERS {
        ("colliders", "1")
    } else {
        ("scripts", "6")
    }
}

/// One object of the report (handleReply), with DataExtended when sent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TopObject {
    pub local_id: u32,
    pub task_id: Uuid,
    pub position: [f32; 3],
    /// Milliseconds of script time, or the collision score.
    pub score: f32,
    pub name: String,
    /// The owner's user name (LLCacheName::buildUsername of OwnerName).
    pub owner: String,
    /// "unknown" without the extended data, as the Legacy Viewer shows it.
    pub parcel: String,
    /// Seconds since 1970 (0 without the extended data).
    pub time: u32,
    /// Script memory in bytes and public URLs (top scripts, extended).
    pub memory: f32,
    pub urls: i32,
    pub extended: bool,
}

/// A LandStatReply.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TopObjects {
    pub report: u32,
    pub flags: u32,
    pub total: u32,
    pub objects: Vec<TopObject>,
}

impl TopObjects {
    /// The summary's figures: script time (the scores' sum) and memory in
    /// KB (handleReply's mtotalScore and total_memory / 1024).
    pub fn totals(&self) -> (f32, u64) {
        let time = self.objects.iter().map(|o| o.score).sum();
        let memory = self.objects.iter().map(|o| o.memory as u64).sum::<u64>() / 1024;
        (time, memory)
    }
}

/// LLCacheName::buildUsername: "First Last" → "first.last", "First
/// Resident" → "first"; the server's trailing spaces trimmed first.
pub fn username_of(legacy: &str) -> String {
    let name = legacy.trim();
    let mut parts = name.split_whitespace();
    let first = parts.next().unwrap_or_default().to_lowercase();
    match parts.next() {
        Some(last) if !last.eq_ignore_ascii_case("resident") => format!("{first}.{}", last.to_lowercase()),
        _ => first,
    }
}

/// LandStatReply as the event queue carries it (its blocks as arrays of
/// maps; DataExtended beside ReportData, one per object).
pub fn parse_land_stat(b: &Llsd) -> TopObjects {
    let rd = &b["RequestData"][0];
    let rows = &b["ReportData"];
    let ext = &b["DataExtended"];
    let objects = (0..rows.len())
        .map(|i| {
            let r = rows.at(i);
            let x = ext.at(i);
            let extended = ext.len() > i;
            let parcel = x["ParcelName"].to_string_value();
            let memory = x["Size"].as_f64() as f32;
            TopObject {
                local_id: r["TaskLocalID"].as_u32(),
                task_id: r["TaskID"].as_uuid(),
                position: [
                    r["LocationX"].as_f64() as f32,
                    r["LocationY"].as_f64() as f32,
                    r["LocationZ"].as_f64() as f32,
                ],
                score: r["Score"].as_f64() as f32,
                name: r["TaskName"].to_string_value(),
                owner: username_of(&r["OwnerName"].to_string_value()),
                parcel: if extended && (!parcel.is_empty() || memory > 0.0) {
                    parcel
                } else {
                    "unknown".to_owned()
                },
                time: if extended { x["TimeStamp"].as_u32() } else { 0 },
                memory: if extended { memory } else { 0.0 },
                urls: if extended { x["PublicURLs"].as_i32() } else { 0 },
                extended,
            }
        })
        .collect();
    TopObjects {
        report: rd["ReportType"].as_u32(),
        flags: rd["RequestFlags"].as_u32(),
        total: rd["TotalObjectCount"].as_u32(),
        objects,
    }
}

/// ESimStatID (llviewerstats.h): the stats of SimStats the window reads.
pub mod sim_stat {
    pub const TIME_DILATION: u32 = 0;
    pub const FPS: u32 = 1;
    pub const PHYSFPS: u32 = 2;
    pub const FRAMEMS: u32 = 4;
    pub const SIMPHYSICSMS: u32 = 7;
    pub const SCRIPTMS: u32 = 10;
    pub const NUMTASKS: u32 = 11;
    pub const NUMTASKSACTIVE: u32 = 12;
    pub const NUMAGENTMAIN: u32 = 13;
    pub const NUMAGENTCHILD: u32 = 14;
    pub const NUMSCRIPTSACTIVE: u32 = 15;
    pub const SIMSPARETIME: u32 = 32;
}

/// The RegionInfo message (LLRegionInfoModel::update and
/// LLFloaterRegionInfo::processRegionInfo).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegionDetails {
    pub name: String,
    pub estate_id: u32,
    pub parent_estate_id: u32,
    /// RegionInfo3 RegionFlagsExtended when sent, else RegionFlags.
    pub flags: u64,
    pub sim_access: u8,
    pub max_agents: u32,
    /// RegionInfo2 HardMaxAgents (the agent limit's maximum; 100 without it).
    pub hard_max_agents: u32,
    pub object_bonus: f32,
    pub billable_factor: f32,
    pub water_height: f32,
    pub terrain_raise_limit: f32,
    pub terrain_lower_limit: f32,
    pub price_per_meter: i32,
    pub use_estate_sun: bool,
    pub sun_hour: f32,
    /// RegionInfo2 ProductName ("Estate / Full Region"), empty when the
    /// simulator sends no product.
    pub product_name: String,
}

/// LLRegionInfoModel's default hard limit of agents.
pub const DEFAULT_HARD_MAX_AGENTS: u32 = 100;

/// LLRegionInfoModel::update: the extended flags when RegionInfo3 is there,
/// the product only when the SKU or the name is sent.
pub fn region_details(m: &aurora_msg::msgs::RegionInfo) -> RegionDetails {
    let r = &m.region_info;
    let r2 = &m.region_info2;
    let product = if r2.product_sku.is_empty() && r2.product_name.is_empty() {
        String::new()
    } else {
        aurora_msg::field_str(&r2.product_name)
    };
    RegionDetails {
        name: aurora_msg::field_str(&r.sim_name),
        estate_id: r.estate_id,
        parent_estate_id: r.parent_estate_id,
        flags: m
            .region_info3
            .first()
            .map_or(u64::from(r.region_flags), |x| x.region_flags_extended),
        sim_access: r.sim_access,
        max_agents: u32::from(r.max_agents),
        hard_max_agents: if r2.hard_max_agents > 0 {
            r2.hard_max_agents
        } else {
            DEFAULT_HARD_MAX_AGENTS
        },
        object_bonus: r.object_bonus_factor,
        billable_factor: r.billable_factor,
        water_height: r.water_height,
        terrain_raise_limit: r.terrain_raise_limit,
        terrain_lower_limit: r.terrain_lower_limit,
        price_per_meter: r.price_per_meter,
        use_estate_sun: r.use_estate_sun,
        sun_hour: r.sun_hour,
        product_name: product,
    }
}

/// The estate of `estateupdateinfo` (LLEstateInfoModel::update: only the
/// first five strings are read).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EstateInfo {
    pub name: String,
    pub owner: Uuid,
    pub id: u32,
    pub flags: u64,
    /// Fixed sun hour (the string carries it times 1024).
    pub sun_hour: f32,
}

impl EstateInfo {
    pub fn has(&self, flag: u64) -> bool {
        self.flags & flag != 0
    }
}

/// `estateupdateinfo`'s strings: name, owner, id, flags, sun hour × 1024,
/// then fields the viewer does not read. None when they are too few.
pub fn parse_estate_update_info(strings: &[String]) -> Option<EstateInfo> {
    if strings.len() < 5 {
        return None;
    }
    let number = |s: &str| s.trim().parse::<u64>().ok();
    Some(EstateInfo {
        name: strings[0].clone(),
        owner: Uuid::parse_str(strings[1].trim()).unwrap_or_default(),
        id: number(&strings[2]).unwrap_or(0) as u32,
        flags: number(&strings[3]).unwrap_or(0),
        sun_hour: strings[4].trim().parse::<f64>().map_or(0.0, |v| (v / 1024.0) as f32),
    })
}

/// LLDispatcher::unpackMessage: a parameter loses its closing NUL.
pub fn parameter_string(bytes: &[u8]) -> String {
    let b = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    String::from_utf8_lossy(b).into_owned()
}

/// A banned resident of the EstateAccess capability.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BannedAgent {
    pub id: Uuid,
    /// "YYYY-MM-DD HH:MM" (cut to 16 characters), None for "n/a".
    pub last_login: Option<String>,
    pub banned_on: Option<String>,
    pub banned_by: Option<Uuid>,
}

/// The four estate lists (requestEstateGetAccessCoro).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EstateAccess {
    pub managers: Vec<Uuid>,
    pub allowed_agents: Vec<Uuid>,
    pub allowed_groups: Vec<Uuid>,
    pub banned: Vec<BannedAgent>,
}

/// The EstateAccess capability's answer: AllowedAgents / AllowedGroups of
/// `{id}`, BannedAgents of `{id, last_login_date, ban_date, banning_id}`,
/// Managers of `{agent_id}`.
pub fn parse_estate_access(v: &Llsd) -> EstateAccess {
    let ids = |key: &str, field: &str| -> Vec<Uuid> {
        let list = &v[key];
        (0..list.len())
            .map(|i| list.at(i)[field].as_uuid())
            .filter(|id| !id.is_nil())
            .collect()
    };
    // dates come as "YYYY-MM-DD HH:MM:SS"; "0000-…" is a ban without date
    let date = |s: String| (!s.is_empty() && !s.starts_with('0')).then(|| s.chars().take(16).collect());
    let banned = &v["BannedAgents"];
    EstateAccess {
        managers: ids("Managers", "agent_id"),
        allowed_agents: ids("AllowedAgents", "id"),
        allowed_groups: ids("AllowedGroups", "id"),
        banned: (0..banned.len())
            .map(|i| banned.at(i))
            .filter(|b| !b["id"].as_uuid().is_nil())
            .map(|b| BannedAgent {
                id: b["id"].as_uuid(),
                last_login: date(b["last_login_date"].to_string_value()),
                banned_on: date(b["ban_date"].to_string_value()),
                banned_by: Some(b["banning_id"].as_uuid()).filter(|id| !id.is_nil()),
            })
            .collect(),
    }
}

/// The region's experiences (RegionExperiences: blocked, trusted, allowed,
/// and the grid's default experience, always trusted).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegionExperiences {
    pub trusted: Vec<Uuid>,
    pub allowed: Vec<Uuid>,
    pub blocked: Vec<Uuid>,
    pub default: Option<Uuid>,
}

pub fn parse_region_experiences(v: &Llsd) -> RegionExperiences {
    let ids = |key: &str| -> Vec<Uuid> {
        let list = &v[key];
        (0..list.len()).map(|i| list.at(i).as_uuid()).filter(|id| !id.is_nil()).collect()
    };
    RegionExperiences {
        trusted: ids("trusted"),
        allowed: ids("allowed"),
        blocked: ids("blocked"),
        default: Some(v["default"].as_uuid()).filter(|id| !id.is_nil()),
    }
}

/// A region's restart schedule (RegionSchedule: `restart` = {type, days,
/// time}); days Sunday first, seconds after midnight UTC.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RestartSchedule {
    pub days: [bool; 7],
    pub seconds: u32,
}

/// The days' letters of the capability, Sunday first (CHECKBOX_PREFIXES).
const DAY_LETTERS: [char; 7] = ['s', 'm', 't', 'w', 'r', 'f', 'a'];

/// None: no schedule. A weekly one ("W") lists its days, any other type is
/// every day.
pub fn parse_restart_schedule(v: &Llsd) -> Option<RestartSchedule> {
    if !v.has("restart") {
        return None;
    }
    let r = &v["restart"];
    let weekly = r["type"].to_string_value() == "W";
    let letters = r["days"].to_string_value().to_lowercase();
    let mut days = [true; 7];
    if weekly {
        for (day, letter) in days.iter_mut().zip(DAY_LETTERS) {
            *day = letters.contains(letter);
        }
    }
    Some(RestartSchedule {
        days,
        seconds: r["time"].as_i32().clamp(0, 86_399) as u32,
    })
}

/// The simulator's statistics of one SimStats message.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimStats {
    /// (ESimStatID, value), as sent.
    pub values: Vec<(u32, f32)>,
    /// Region ObjectCapacity (LLViewerRegion::getMaxTasks).
    pub object_capacity: u32,
}

impl SimStats {
    pub fn get(&self, id: u32) -> Option<f32> {
        self.values
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, v)| *v)
            .filter(|v| v.is_finite())
    }
}

/// LLFloaterRegionInfo's estate messages write their numbers with C's
/// `"%f"`: six decimals, a dot.
fn c_float(v: f32) -> String {
    format!("{v:.6}")
}

fn yes_no(v: bool) -> String {
    if v { "Y" } else { "N" }.to_owned()
}

/// The region's rules of the General panel, as its controls hold them
/// (processRegionInfo) and as "Apply" sends them (sendUpdate).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionRules {
    pub block_terraform: bool,
    pub block_fly: bool,
    pub block_fly_over: bool,
    pub allow_damage: bool,
    pub restrict_pushobject: bool,
    pub allow_land_resell: bool,
    pub allow_parcel_changes: bool,
    pub block_parcel_search: bool,
    /// The agent limit spinner (1 to the region's hard limit).
    pub agent_limit: f32,
    /// The object bonus spinner (1 to 10).
    pub object_bonus: f32,
    /// SIM_ACCESS_PG (13), _MATURE (21) or _ADULT (42).
    pub sim_access: u8,
}

impl RegionRules {
    /// LLFloaterRegionInfo::processRegionInfo, the General panel's part.
    pub fn of(d: &RegionDetails) -> Self {
        use crate::types::region_flags as rf;
        let has = |f: u32| d.flags & u64::from(f) != 0;
        RegionRules {
            block_terraform: has(rf::BLOCK_TERRAFORM),
            block_fly: has(rf::BLOCK_FLY),
            block_fly_over: has(rf::BLOCK_FLYOVER),
            allow_damage: has(rf::ALLOW_DAMAGE),
            restrict_pushobject: has(rf::RESTRICT_PUSHOBJECT),
            allow_land_resell: !has(rf::BLOCK_LAND_RESELL),
            allow_parcel_changes: has(rf::ALLOW_PARCEL_CHANGES),
            block_parcel_search: has(rf::BLOCK_PARCEL_SEARCH),
            agent_limit: d.max_agents as f32,
            object_bonus: d.object_bonus,
            sim_access: d.sim_access,
        }
    }

    /// `setregioninfo`'s strings (LLPanelRegionGeneralInfo::sendUpdate
    /// without the capability): terraform, fly, damage, resell, agent limit,
    /// object bonus, access, push, subdivide. The search block, documented
    /// as a tenth string, is not sent.
    pub fn strings(&self) -> Vec<String> {
        vec![
            yes_no(self.block_terraform),
            yes_no(self.block_fly),
            yes_no(self.allow_damage),
            yes_no(self.allow_land_resell),
            c_float(self.agent_limit),
            c_float(self.object_bonus),
            self.sim_access.to_string(),
            yes_no(self.restrict_pushobject),
            yes_no(self.allow_parcel_changes),
        ]
    }

    /// The DispatchRegionInfo capability's body (sendUpdate's first way).
    pub fn body(&self) -> Llsd {
        aurora_llsd::llsd_map! {
            "block_terraform" => self.block_terraform,
            "block_fly" => self.block_fly,
            "block_fly_over" => self.block_fly_over,
            "allow_damage" => self.allow_damage,
            "allow_land_resell" => self.allow_land_resell,
            "agent_limit" => f64::from(self.agent_limit),
            "prim_bonus" => f64::from(self.object_bonus),
            "sim_access" => i32::from(self.sim_access),
            "restrict_pushobject" => self.restrict_pushobject,
            "allow_parcel_changes" => self.allow_parcel_changes,
            "block_parcel_search" => self.block_parcel_search,
        }
    }
}

/// `estatechangeinfo`'s strings (LLEstateInfoModel::commitEstateInfoDataserver):
/// name, flags (`"%u"`), sun hour × 1024 (`"%d"`).
pub fn estate_change_strings(e: &EstateInfo) -> Vec<String> {
    vec![
        e.name.clone(),
        (e.flags as u32).to_string(),
        ((e.sun_hour * 1024.0) as i32).to_string(),
    ]
}

/// The EstateChangeInfo capability's body (commitEstateInfoCapsCoro), with
/// the invoice of the last `getinfo`.
pub fn estate_change_body(e: &EstateInfo, invoice: Uuid) -> Llsd {
    aurora_llsd::llsd_map! {
        "estate_name" => e.name.as_str(),
        "sun_hour" => f64::from(e.sun_hour),
        "is_sun_fixed" => e.has(estate_flags::SUN_FIXED),
        "is_externally_visible" => e.has(estate_flags::EXTERNALLY_VISIBLE),
        "allow_direct_teleport" => e.has(estate_flags::ALLOW_DIRECT_TELEPORT),
        "deny_anonymous" => e.has(estate_flags::DENY_ANONYMOUS),
        "deny_age_unverified" => e.has(estate_flags::DENY_AGEUNVERIFIED),
        "block_bots" => e.has(estate_flags::DENY_BOTS),
        "allow_voice_chat" => e.has(estate_flags::ALLOW_VOICE),
        "override_public_access" => e.has(estate_flags::ALLOW_ACCESS_OVERRIDE),
        "override_environment" => e.has(estate_flags::ALLOW_ENVIRONMENT_OVERRIDE),
        "invoice" => invoice,
    }
}

/// The Terrain panel's settings: water, raise and lower limits (of
/// RegionInfo), the four detail textures and the elevation ranges of the
/// four corners (of RegionHandshake: 0 south-west, 1 north-west, 2
/// south-east, 3 north-east).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainSettings {
    pub water_height: f32,
    pub raise_limit: f32,
    pub lower_limit: f32,
    pub textures: [Uuid; 4],
    pub start_height: [f32; 4],
    pub height_range: [f32; 4],
}

/// What `setregionterrain` repeats of the region besides the terrain
/// (LLRegionInfoModel: the sun, sent back as it was read).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RegionSun {
    pub use_estate_sun: bool,
    pub fixed_sun: bool,
    pub sun_hour: f32,
}

impl RegionSun {
    pub fn of(d: &RegionDetails) -> Self {
        RegionSun {
            use_estate_sun: d.use_estate_sun,
            fixed_sun: d.flags & estate_flags::SUN_FIXED != 0,
            sun_hour: d.sun_hour,
        }
    }
}

/// The messages of the Terrain panel's "Apply" (LLPanelRegionTerrainInfo::
/// sendUpdate), in their order: `setregionterrain` (LLRegionInfoModel::
/// sendRegionTerrain: water, raise, lower, estate sun, fixed sun, sun hour,
/// then the estate's global time, fixed sun and hour it resets),
/// `texturedetail` ("%d %s"), `textureheights` ("%d %f %f") and
/// `texturecommit`.
pub fn terrain_messages(t: &TerrainSettings, sun: RegionSun) -> Vec<(&'static str, Vec<String>)> {
    let region_terrain = vec![
        c_float(t.water_height),
        c_float(t.raise_limit),
        c_float(t.lower_limit),
        yes_no(sun.use_estate_sun),
        yes_no(sun.fixed_sun),
        c_float(sun.sun_hour),
        yes_no(true),
        yes_no(false),
        c_float(0.0),
    ];
    let detail = t
        .textures
        .iter()
        .enumerate()
        .map(|(i, id)| format!("{i} {}", id.hyphenated()))
        .collect();
    let heights = (0..4)
        .map(|i| format!("{i} {} {}", c_float(t.start_height[i]), c_float(t.height_range[i])))
        .collect();
    vec![
        ("setregionterrain", region_terrain),
        ("texturedetail", detail),
        ("textureheights", heights),
        ("texturecommit", Vec::new()),
    ]
}

/// What the Region / Estate window asks the simulator.
#[derive(Debug, Clone, PartialEq)]
pub enum EstateCommand {
    /// LLFloaterRegionInfo::refreshFromRegion, as each panel asks: the
    /// EstateOwnerMessage `getinfo` (a new invoice), the EstateAccess lists,
    /// the region's experiences, its environment and its restart schedule.
    Refresh { handle: RegionHandle },
    /// RequestRegionInfo (requestRegionInfo), after the panels.
    RequestRegionInfo { handle: RegionHandle },
    /// The General panel's "Apply": DispatchRegionInfo, else `setregioninfo`.
    SetRegionInfo { handle: RegionHandle, rules: RegionRules },
    /// The Estate panel's "Apply" (LLEstateInfoModel::sendEstateInfo):
    /// EstateChangeInfo, else `estatechangeinfo` with a new invoice.
    SetEstateInfo { handle: RegionHandle, info: EstateInfo },
    /// The Terrain panel's "Apply".
    SetTerrain {
        handle: RegionHandle,
        terrain: TerrainSettings,
        sun: RegionSun,
    },
    /// "Bake Terrain" (callbackBakeTerrain): `terrain` with "bake".
    BakeTerrain { handle: RegionHandle },
    /// A change of the Access panel's lists (accessCoreConfirm,
    /// importListCallback): one `estateaccessdelta` per resident or group,
    /// with the flags `access_delta_flags` gives.
    AccessDelta {
        handle: RegionHandle,
        flags: u32,
        ids: Vec<Uuid>,
        batch: bool,
    },
    /// The lists again (LLPanelEstateAccess::updateLists), once the
    /// simulator answered a change with `setaccess`.
    RefreshAccess { handle: RegionHandle },
    /// "Message to everyone": the region's (`simulatormessage`) or the
    /// whole estate's (`instantmessage`), signed with `name`.
    Message {
        handle: RegionHandle,
        estate: bool,
        name: String,
        text: String,
    },
    /// "Teleport home" one resident (`teleporthomeuser`) or everyone in
    /// the region (`teleporthomeallusers`, `target` None).
    TeleportHome { handle: RegionHandle, target: Option<Uuid> },
    /// "Kick from estate" (`kickestate`).
    KickFromEstate { handle: RegionHandle, target: Uuid },
    /// A resident's objects returned (LLPanelRegionDebugInfo::callbackReturn):
    /// through the estate (`estateobjectreturn`) or this region only
    /// (SimWideDeletes), with `swd` flags.
    ReturnObjects {
        handle: RegionHandle,
        target: Uuid,
        flags: u32,
        estate_wide: bool,
    },
    /// `restart` in `seconds`, or -1 to cancel a restart.
    Restart { handle: RegionHandle, seconds: i32 },
    /// The restart schedule saved (RegionSchedule POST), then asked again.
    SetRestartSchedule {
        handle: RegionHandle,
        days: [bool; 7],
        seconds: u32,
    },
    /// The Debug panel's "Apply": `setregiondebug`.
    SetRegionDebug { handle: RegionHandle, debug: RegionDebug },
    /// The Debug panel's "Top scripts" / "Top colliders": the estate message
    /// that has the simulator send its report.
    TopObjects { handle: RegionHandle, report: u32 },
    /// The Top objects window's refresh and filters (onRefresh):
    /// LandStatRequest for the whole region.
    LandStat {
        handle: RegionHandle,
        report: u32,
        flags: u32,
        filter: String,
    },
    /// A change of the region's experience lists (experienceCoreConfirm):
    /// one `estateexperiencedelta` per experience, NO_REPLY on all but the last.
    ExperienceDelta { handle: RegionHandle, flags: u32, ids: Vec<Uuid> },
    /// The experiences again (refreshFromRegion), once the simulator
    /// answered a change with `setexperience`.
    RefreshExperiences { handle: RegionHandle },
    /// The covenant replaced by a notecard's asset, or cleared with nil
    /// (LLPanelEstateCovenant::sendChangeCovenantID).
    ChangeCovenant { handle: RegionHandle, asset: Uuid },
    /// The telehub floater's requests (`telehub`).
    Telehub {
        handle: RegionHandle,
        verb: TelehubVerb,
        objects: Vec<u32>,
    },
    /// "Download RAW…" / "Upload RAW…".
    TerrainRaw { handle: RegionHandle, transfer: RawTransfer },
    /// The PBR materials' transforms (ModifyRegion POST, then GET).
    TerrainTransforms {
        handle: RegionHandle,
        transforms: [TerrainTransform; 4],
    },
    /// "Return selected" / "Return all" (returnObjects): ParcelReturnObjects
    /// for the whole region (local id -1, RT_NONE) with the tasks.
    ReturnTopObjects { handle: RegionHandle, tasks: Vec<Uuid> },
}

/// What the simulator answered.
#[derive(Debug, Clone)]
pub enum EstateEvent {
    /// RegionInfo (sent on request and when the estate changes it).
    RegionInfo { handle: RegionHandle, info: Arc<RegionDetails> },
    /// `estateupdateinfo` of the last `getinfo`.
    EstateInfo { handle: RegionHandle, info: Arc<EstateInfo> },
    /// The EstateAccess capability (None: absent or refused).
    Access {
        handle: RegionHandle,
        access: Option<Arc<EstateAccess>>,
    },
    /// `setaccess` of the last invoice: the lists changed
    /// (LLDispatchSetEstateAccess).
    AccessChanged { handle: RegionHandle },
    /// RegionExperiences (None: absent or refused).
    Experiences {
        handle: RegionHandle,
        experiences: Option<RegionExperiences>,
    },
    /// RegionSchedule: Ok(None) when there is no schedule, Err when the
    /// capability is absent or refused.
    Schedule {
        handle: RegionHandle,
        schedule: Result<Option<RestartSchedule>, ()>,
    },
    /// The region environment of ExtEnvironment (its `environment` map).
    Environment { handle: RegionHandle, environment: Option<Llsd> },
    /// SimStats of the agent's region, about once a second.
    Stats { handle: RegionHandle, stats: Arc<SimStats> },
    /// AgentMovementComplete SimData ChannelVersion (gLastVersionChannel).
    SimVersion { handle: RegionHandle, version: String },
    /// `setexperience` of the last invoice: the experience lists changed
    /// (LLDispatchSetEstateExperience).
    ExperiencesChanged { handle: RegionHandle },
    /// TelehubInfo.
    Telehub { handle: RegionHandle, info: Arc<TelehubInfo> },
    /// The RAW file downloaded (callback_download_complete), or why not.
    RawDownloaded {
        handle: RegionHandle,
        path: String,
        result: Result<Arc<Vec<u8>>, String>,
    },
    /// The RAW file sent to the simulator, or why not.
    RawUploaded { handle: RegionHandle, result: Result<(), String> },
    /// ModifyRegion's material overrides, or why they are not there (no
    /// capability, a failed query or change).
    TerrainTransforms {
        handle: RegionHandle,
        transforms: Result<[TerrainTransform; 4], String>,
    },
    /// LandStatReply: the Top scripts / Top colliders report.
    TopObjects { handle: RegionHandle, report: Arc<TopObjects> },
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    #[test]
    fn estate_update_info_reads_its_first_five_strings() {
        let s: Vec<String> = [
            "Aurora Demo",
            "a0e0a6e1-0000-0000-0000-000000000001",
            "104123",
            "2415951904",
            "6144",
            "1",
            "c0e00000-0000-0000-0000-000000000001",
            "1717647528",
            "0",
            "",
        ]
        .map(String::from)
        .to_vec();
        let e = parse_estate_update_info(&s).expect("ten strings");
        assert_eq!(e.name, "Aurora Demo");
        assert_eq!(e.owner, Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0001));
        assert_eq!(e.id, 104_123);
        assert_eq!(e.flags, 2_415_951_904);
        assert!(e.has(estate_flags::DENY_BOTS) && e.has(estate_flags::EXTERNALLY_VISIBLE));
        assert!((e.sun_hour - 6.0).abs() < 1e-6);
        assert!(parse_estate_update_info(&s[..4]).is_none());
        assert_eq!(parameter_string(b"104123\0"), "104123");
        assert_eq!(parameter_string(b""), "");
    }

    /// accessCoreConfirm answers once for a change of several residents;
    /// an import asks an answer of each.
    #[test]
    fn access_delta_flags_reply_once() {
        use estate_access::*;
        let f = BANNED_AGENT_ADD | APPLY_TO_ALL_ESTATES;
        assert_eq!(access_delta_flags(f, 3, true), [f | NO_REPLY, f | NO_REPLY, f]);
        assert_eq!(access_delta_flags(f, 1, true), [f]);
        assert_eq!(access_delta_flags(f, 2, false), [f, f]);
        assert!(access_delta_flags(f, 0, true).is_empty());
        let s = access_delta_strings(Uuid::from_u128(1), ALLOWED_GROUP_ADD, Uuid::from_u128(2));
        assert_eq!(
            s,
            ["00000000-0000-0000-0000-000000000001", "16", "00000000-0000-0000-0000-000000000002"]
        );
    }

    #[test]
    fn the_access_lists_of_the_capability() {
        let id = |n: u128| Llsd::Uuid(Uuid::from_u128(n));
        let v = llsd_map! {
            "Managers" => Llsd::Array(vec![llsd_map! { "agent_id" => id(3) }, llsd_map! { "agent_id" => id(4) }]),
            "AllowedAgents" => Llsd::Array(vec![llsd_map! { "id" => id(5) }]),
            "AllowedGroups" => Llsd::Array(vec![llsd_map! { "id" => id(9) }]),
            "BannedAgents" => Llsd::Array(vec![
                llsd_map! {
                    "id" => id(7),
                    "last_login_date" => Llsd::String("2026-10-13 18:02:11".into()),
                    "ban_date" => Llsd::String("2026-10-09 10:00:00".into()),
                    "banning_id" => id(4),
                },
                llsd_map! {
                    "id" => id(8),
                    "last_login_date" => Llsd::String(String::new()),
                    "ban_date" => Llsd::String("0000-00-00 00:00:00".into()),
                    "banning_id" => id(0),
                },
            ]),
        };
        let a = parse_estate_access(&v);
        assert_eq!(a.managers, vec![Uuid::from_u128(3), Uuid::from_u128(4)]);
        assert_eq!(a.allowed_agents, vec![Uuid::from_u128(5)]);
        assert_eq!(a.allowed_groups, vec![Uuid::from_u128(9)]);
        assert_eq!(a.banned[0].banned_on.as_deref(), Some("2026-10-09 10:00"));
        assert_eq!(a.banned[0].banned_by, Some(Uuid::from_u128(4)));
        assert_eq!(a.banned[1].banned_on, None, "a ban without a date is n/a");
        assert_eq!(a.banned[1].banned_by, None);
        assert_eq!(parse_estate_access(&Llsd::Undef), EstateAccess::default());
    }

    #[test]
    fn the_restart_schedule() {
        let weekly = llsd_map! { "restart" => llsd_map! {
            "type" => Llsd::String("W".into()),
            "days" => Llsd::String("TR".into()),
            "time" => Llsd::Integer(7200),
        }};
        let s = parse_restart_schedule(&weekly).expect("a schedule");
        assert_eq!(s.days, [false, false, true, false, true, false, false]);
        assert_eq!(s.seconds, 7200);
        let daily = llsd_map! { "restart" => llsd_map! { "type" => Llsd::String("D".into()), "time" => Llsd::Integer(21600) }};
        assert_eq!(parse_restart_schedule(&daily).map(|s| s.days), Some([true; 7]));
        assert_eq!(parse_restart_schedule(&llsd_map! {}), None);
    }

    /// setregioninfo in sendUpdate's order, with C's "%f"; the search
    /// block only goes through the capability.
    #[test]
    fn the_region_rules_as_firestorm_sends_them() {
        use crate::types::region_flags as rf;
        let d = RegionDetails {
            flags: u64::from(rf::BLOCK_TERRAFORM | rf::RESTRICT_PUSHOBJECT | rf::BLOCK_LAND_RESELL | rf::BLOCK_PARCEL_SEARCH),
            max_agents: 40,
            object_bonus: 1.5,
            sim_access: 21,
            ..Default::default()
        };
        let r = RegionRules::of(&d);
        assert!(r.block_terraform && r.restrict_pushobject && !r.allow_land_resell && r.block_parcel_search);
        assert!(!r.block_fly && !r.allow_damage && !r.allow_parcel_changes);
        assert_eq!(
            r.strings(),
            ["Y", "N", "N", "N", "40.000000", "1.500000", "21", "Y", "N"]
                .map(String::from)
                .to_vec()
        );
        let body = r.body();
        assert_eq!(body["agent_limit"].as_f64(), 40.0);
        assert_eq!(body["prim_bonus"].as_f64(), 1.5);
        assert_eq!(body["sim_access"].as_i32(), 21);
        assert!(body["block_parcel_search"].as_bool() && !body["allow_land_resell"].as_bool());
        assert_eq!(body.as_map().map(|m| m.len()), Some(11));
    }

    #[test]
    fn the_estate_change_as_firestorm_sends_it() {
        let e = EstateInfo {
            name: "Aurora Demo".into(),
            owner: Uuid::from_u128(1),
            id: 104_123,
            flags: estate_flags::EXTERNALLY_VISIBLE | estate_flags::DENY_BOTS | estate_flags::ALLOW_VOICE,
            sun_hour: 6.0,
        };
        assert_eq!(
            estate_change_strings(&e),
            ["Aurora Demo", "2415951872", "6144"].map(String::from).to_vec()
        );
        let invoice = Uuid::from_u128(9);
        let body = estate_change_body(&e, invoice);
        assert!(body["is_externally_visible"].as_bool() && body["block_bots"].as_bool() && body["allow_voice_chat"].as_bool());
        assert!(!body["deny_anonymous"].as_bool() && !body["is_sun_fixed"].as_bool());
        assert_eq!(body["invoice"].as_uuid(), invoice);
        assert_eq!(body["estate_name"].as_str(), "Aurora Demo");
    }

    #[test]
    fn the_terrain_messages_in_their_order() {
        let t = TerrainSettings {
            water_height: 20.0,
            raise_limit: 4.0,
            lower_limit: -4.0,
            textures: [Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4)],
            start_height: [10.0, 10.0, 12.5, 10.0],
            height_range: [60.0; 4],
        };
        let sun = RegionSun {
            use_estate_sun: true,
            fixed_sun: false,
            sun_hour: 0.0,
        };
        let m = terrain_messages(&t, sun);
        let names: Vec<&str> = m.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, ["setregionterrain", "texturedetail", "textureheights", "texturecommit"]);
        assert_eq!(
            m[0].1,
            ["20.000000", "4.000000", "-4.000000", "Y", "N", "0.000000", "Y", "N", "0.000000"]
                .map(String::from)
                .to_vec()
        );
        assert_eq!(m[1].1[1], "1 00000000-0000-0000-0000-000000000002");
        assert_eq!(m[2].1[2], "2 12.500000 60.000000");
        assert!(m[3].1.is_empty());
    }

    /// The interventions' strings in the panels' order.
    #[test]
    fn the_interventions_as_the_panels_send_them() {
        let me = Uuid::from_u128(1);
        assert_eq!(
            region_message_strings(me, "Aurora Demo", "Back in 5"),
            ["-1", "-1", "00000000-0000-0000-0000-000000000001", "Aurora Demo", "Back in 5"]
                .map(String::from)
                .to_vec()
        );
        assert_eq!(
            estate_message_strings("Aurora Demo", "Hi"),
            ["Aurora Demo", "Hi"].map(String::from).to_vec()
        );
        assert_eq!(return_flags(false, false), swd::ALWAYS_RETURN_OBJECTS);
        assert_eq!(return_flags(true, true), 7);
        assert_eq!(return_flags(true, false), swd::ALWAYS_RETURN_OBJECTS | swd::SCRIPTED_ONLY);
    }

    /// Saved as LLFloaterRegionRestartSchedule does, and read back.
    #[test]
    fn the_restart_schedule_body() {
        let tue_thu = [false, false, true, false, true, false, false];
        let b = restart_schedule_body(tue_thu, 7200);
        assert_eq!(b["restart"]["type"].to_string_value(), "W");
        assert_eq!(b["restart"]["days"].to_string_value(), "TR");
        assert_eq!(b["restart"]["time"].as_i32(), 7200);
        assert_eq!(parse_restart_schedule(&b).map(|s| (s.days, s.seconds)), Some((tue_thu, 7200)));
        let daily = restart_schedule_body([true; 7], 21_600);
        assert_eq!(daily["restart"]["type"].to_string_value(), "D");
        assert!(!daily["restart"].has("days"));
        // no day: a weekly schedule without days clears it
        let none = restart_schedule_body([false; 7], 0);
        assert_eq!(none["restart"]["days"].to_string_value(), "");
    }

    #[test]
    fn the_debug_switches_follow_the_region_flags() {
        let d = RegionDetails {
            flags: skip_flags::SCRIPTS | skip_flags::PHYSICS,
            ..Default::default()
        };
        let r = RegionDebug::of(&d);
        assert!(r.skip_scripts && !r.skip_collisions && r.skip_physics);
        assert_eq!(r.strings(), ["Y", "N", "Y"].map(String::from).to_vec());
        assert_eq!(top_request(land_stat::TOP_SCRIPTS), ("scripts", "6"));
        assert_eq!(top_request(land_stat::TOP_COLLIDERS), ("colliders", "1"));
    }

    #[test]
    fn owner_names_become_user_names() {
        assert_eq!(username_of("Orion Exemple  "), "orion.exemple");
        assert_eq!(username_of("Pixelboutique Resident"), "pixelboutique");
        assert_eq!(username_of("lumen"), "lumen");
        assert_eq!(username_of(""), "");
    }

    /// handleReply: the extended data when sent, "unknown" parcel without.
    #[test]
    fn the_land_stat_reply_of_the_event_queue() {
        let row = |n: u128, score: f64| {
            llsd_map! {
                "TaskLocalID" => Llsd::Integer(n as i32),
                "TaskID" => Llsd::Uuid(Uuid::from_u128(n)),
                "LocationX" => Llsd::Real(128.0),
                "LocationY" => Llsd::Real(64.5),
                "LocationZ" => Llsd::Real(22.0),
                "Score" => Llsd::Real(score),
                "TaskName" => Llsd::String("Fountain".into()),
                "OwnerName" => Llsd::String("Orion Exemple ".into()),
            }
        };
        let b = llsd_map! {
            "RequestData" => Llsd::Array(vec![llsd_map! {
                "ReportType" => Llsd::Integer(0),
                "RequestFlags" => Llsd::Integer(0),
                "TotalObjectCount" => Llsd::Integer(412),
            }]),
            "ReportData" => Llsd::Array(vec![row(1, 0.25), row(2, 0.125)]),
            "DataExtended" => Llsd::Array(vec![
                llsd_map! {
                    "TimeStamp" => Llsd::Integer(1_760_000_000),
                    "MonoScore" => Llsd::Real(0.2),
                    "PublicURLs" => Llsd::Integer(2),
                    "ParcelName" => Llsd::String("Plaza".into()),
                    "Size" => Llsd::Real(65_536.0),
                },
                llsd_map! {
                    "TimeStamp" => Llsd::Integer(0),
                    "MonoScore" => Llsd::Real(0.0),
                    "PublicURLs" => Llsd::Integer(0),
                    "ParcelName" => Llsd::String(String::new()),
                    "Size" => Llsd::Real(0.0),
                },
            ]),
        };
        let t = parse_land_stat(&b);
        assert_eq!((t.report, t.total, t.objects.len()), (land_stat::TOP_SCRIPTS, 412, 2));
        let o = &t.objects[0];
        assert_eq!(
            (o.local_id, o.owner.as_str(), o.parcel.as_str(), o.urls),
            (1, "orion.exemple", "Plaza", 2)
        );
        assert_eq!(o.position, [128.0, 64.5, 22.0]);
        assert_eq!(t.objects[1].parcel, "unknown", "no parcel nor memory sent");
        let (time, memory) = t.totals();
        assert!((time - 0.375).abs() < 1e-6);
        assert_eq!(memory, 64);
        // without DataExtended
        let mut short = b.clone();
        short.insert("DataExtended", Llsd::Array(Vec::new()));
        let s = parse_land_stat(&short);
        assert!(!s.objects[0].extended && s.objects[0].parcel == "unknown" && s.objects[0].time == 0);
    }

    #[test]
    fn experiences_and_stats() {
        let v = llsd_map! {
            "trusted" => Llsd::Array(vec![Llsd::Uuid(Uuid::from_u128(1))]),
            "allowed" => Llsd::Array(vec![Llsd::Uuid(Uuid::from_u128(2)), Llsd::Uuid(Uuid::from_u128(3))]),
        };
        let x = parse_region_experiences(&v);
        assert_eq!((x.trusted.len(), x.allowed.len(), x.blocked.len()), (1, 2, 0));
        let stats = SimStats {
            values: vec![(sim_stat::TIME_DILATION, 0.99), (sim_stat::FPS, f32::NAN)],
            object_capacity: 15_000,
        };
        assert_eq!(stats.get(sim_stat::TIME_DILATION), Some(0.99));
        assert_eq!(stats.get(sim_stat::FPS), None, "not a number");
        assert_eq!(stats.get(sim_stat::SCRIPTMS), None);
    }

    /// sendGodlikeRequest: the verb, then the selected objects; a spawn
    /// point's removal names its index alone.
    #[test]
    fn the_telehub_requests() {
        assert_eq!(TelehubVerb::Connect.strings(&[12, 40]), ["connect", "12", "40"]);
        assert_eq!(TelehubVerb::Info.strings(&[]), ["info ui"]);
        assert_eq!(TelehubVerb::RemoveSpawn(2).strings(&[9]), ["spawnpoint remove", "2"]);
    }

    /// getOverrideLLSD writes only what differs; a default transform is an
    /// empty map; queryRegionCoro reads them back.
    #[test]
    fn the_terrain_overrides() {
        let mut t = [TerrainTransform::default(); 4];
        t[1].scale = [2.0, 2.0];
        t[1].rotation = 0.5;
        let body = terrain_overrides_body(&t);
        let overrides = body["overrides"].as_array();
        assert_eq!(overrides.len(), 4);
        assert!(!overrides[0].has("ti"));
        assert!(overrides[1]["ti"].at(0).has("s") && !overrides[1]["ti"].at(0).has("o"));
        let mut answer = body.clone();
        answer.insert("success", Llsd::Boolean(true));
        assert_eq!(parse_terrain_overrides(&answer), Some(t));
        assert_eq!(parse_terrain_overrides(&body), None, "no success");
    }
}
