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

/// The region's experiences (RegionExperiences: blocked, trusted, allowed).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegionExperiences {
    pub trusted: Vec<Uuid>,
    pub allowed: Vec<Uuid>,
    pub blocked: Vec<Uuid>,
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
}
