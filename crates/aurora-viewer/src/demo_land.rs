//! About Land in the offline demo (`AURORA_DEMO_LAND=<tab>[:role][:classic]`):
//! the simulator's answers to the floater's requests, for the plaza
//! parcel (north-east quarter of the demo region). The role says who the
//! avatar is to the parcel: without one it belongs to a group the avatar
//! has no powers in (everything greyed); `owner` makes the avatar its owner,
//! `officer` gives the avatar the land powers of the owning group but those
//! of sale, deed and terraforming, `visitor` gives the parcel to another
//! resident and a group the avatar is not in. `classic` shows Firestorm's
//! nine tabs for the session, whatever Preferences › Aurora says.
//! `AURORA_DEMO_LAND_PRIMS=<used>[,<step>]` sets the objects on the owner's
//! land and makes them grow at each answer (the floater's refresh).
//! `AURORA_DEMO_BUY_LAND` puts the parcel up for sale, and a ParcelBuy
//! hands it to the avatar. `AURORA_DEMO_BUY_PASS` gives it to Loup, with or
//! without passes, or to the avatar.

use super::{DEMO_AGENT, DEMO_GROUP1, DEMO_LOUP, DEMO_NOVA, DEMO_TESS, TEX_GRADIENT};
use crate::world::land::powers;
use aurora_llsd::Llsd;
use aurora_net::land::{AL_ACCESS, AL_ALLOW_EXPERIENCE, AL_BAN, AccessEntry, FoundAvatar, LandCommand, LandEvent, ObjectOwner};
use aurora_net::{NetEvent, ParcelInfo, ParcelMedia, parcel_flags as pf};
use glam::Vec3;
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use uuid::Uuid;

const COVENANT: Uuid = Uuid::from_u128(0xC0E0_0000_0000_0000_0000_0000_0000_0001);
pub(super) const PARCEL_UUID: Uuid = Uuid::from_u128(0x1214_B5B2_7C42_D6C4_8741_B182_4390_8089);
const XP_AURORA: Uuid = Uuid::from_u128(0xE7E0_0000_0000_0000_0000_0000_0000_0001);
const XP_SITTER: Uuid = Uuid::from_u128(0xE7E0_0000_0000_0000_0000_0000_0000_0002);
const XP_INTERACT: Uuid = Uuid::from_u128(0xE7E0_0000_0000_0000_0000_0000_0000_0003);
const XP_BEACH: Uuid = Uuid::from_u128(0xE7E0_0000_0000_0000_0000_0000_0000_0004);
const XP_CLUB: Uuid = Uuid::from_u128(0xE7E0_0000_0000_0000_0000_0000_0000_0005);
const XP_GRID: Uuid = Uuid::from_u128(0xE7E0_0000_0000_0000_0000_0000_0000_0006);
/// Placeholder parcel music (also `AURORA_DEMO_MUSIC=1`).
pub const MUSIC_URL: &str = "http://flux.exemple.org:8000/";

/// The demo parcel as the "simulator" keeps it (updates are applied).
static PARCEL: Mutex<Option<ParcelInfo>> = Mutex::new(None);

/// The group of the visitor's parcel, which the avatar is not in.
const VISITOR_GROUP: Uuid = Uuid::from_u128(0x6E00_0000_0000_0000_0000_0000_0000_0009);

/// Who the avatar is to the demo parcel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A member of the owning group, without powers.
    Member,
    Owner,
    /// A member of the owning group with its land powers.
    Officer,
    /// Someone else's parcel, a group the avatar is not in.
    Visitor,
}

/// What AURORA_DEMO_LAND asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scenario {
    pub tab: String,
    pub role: Role,
    /// Firestorm's nine tabs for the session.
    pub classic: bool,
}

/// `<tab>[:role][:classic]`, the role and `classic` in any order.
fn parse(v: &str) -> Scenario {
    let mut parts = v.split(':');
    let tab = parts.next().unwrap_or_default().to_owned();
    let mut s = Scenario {
        tab,
        role: Role::Member,
        classic: false,
    };
    for part in parts {
        match part.trim() {
            "owner" => s.role = Role::Owner,
            "officer" => s.role = Role::Officer,
            "visitor" => s.role = Role::Visitor,
            "classic" => s.classic = true,
            _ => {}
        }
    }
    s
}

pub fn scenario() -> Option<Scenario> {
    match std::env::var("AURORA_DEMO_LAND") {
        Ok(v) => Some(parse(&v)),
        // AURORA_DEMO_ENV_EDITOR=custom: About Land's environment, as the
        // plaza's owner
        Err(_) => super::env::editor()
            .filter(|s| s.starts_with("custom"))
            .map(|_| parse("environment:owner")),
    }
}

/// Who the avatar is to the plaza: the scenario's role, the owner in the
/// environment editors' parcel scenarios.
fn role() -> Role {
    match scenario() {
        Some(s) => s.role,
        None if super::env::parcel_scenario() => Role::Owner,
        None => Role::Member,
    }
}

/// The powers an officer has in the owning group: everything of the land
/// but sale, deed, release and terraforming, so that some settings stay
/// read only.
pub const OFFICER_POWERS: u64 = powers::LAND_FIND_PLACES
    | powers::LAND_CHANGE_IDENTITY
    | powers::LAND_SET_LANDING_POINT
    | powers::LAND_CHANGE_MEDIA
    | powers::LAND_OPTIONS
    | powers::LAND_MANAGE_ALLOWED
    | powers::LAND_MANAGE_BANNED
    | powers::LAND_RETURN_GROUP_SET
    | powers::LAND_RETURN_NON_GROUP
    | powers::LAND_ALLOW_ENVIRONMENT
    | powers::LAND_RETURN_GROUP_OWNED;

/// The powers the avatar has in `group` beyond the demo's own.
pub fn group_powers(group: Uuid) -> u64 {
    match scenario() {
        Some(s) if s.role == Role::Officer && group == DEMO_GROUP1 => OFFICER_POWERS,
        _ => 0,
    }
}

/// AURORA_DEMO_BUY_LAND=<mode>[:confirm]: the plaza parcel for sale by
/// a demo resident (rich, short, upgrade, other, error; see docs/TESTING.md).
pub fn buy_scenario() -> Option<String> {
    let v = std::env::var("AURORA_DEMO_BUY_LAND").ok()?.trim().to_lowercase();
    Some(v.split(':').next().unwrap_or_default().to_owned())
}

/// AURORA_DEMO_BUY_LAND=<mode>:confirm: the confirmation opens by itself.
pub fn buy_confirms() -> bool {
    std::env::var("AURORA_DEMO_BUY_LAND").is_ok_and(|v| v.trim().to_lowercase().ends_with(":confirm"))
}

/// Price of the plaza parcel in AURORA_DEMO_BUY_LAND.
const SALE_PRICE: i32 = 4500;

fn now() -> i32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i32)
        .unwrap_or(0)
}

fn initial(role: Role) -> ParcelInfo {
    let (owner_id, group_id) = match role {
        Role::Owner => (DEMO_AGENT, DEMO_GROUP1),
        Role::Member | Role::Officer => (DEMO_GROUP1, DEMO_GROUP1),
        Role::Visitor => (DEMO_NOVA, VISITOR_GROUP),
    };
    ParcelInfo {
        local_id: 1,
        sequence_id: aurora_net::land::SELECTED_PARCEL_SEQ_ID,
        name: aurora_i18n::tr!("demo-land-aurora-plaza-offline-demo").into(),
        desc: aurora_i18n::tr!("demo-land-aurora-viewer-s-offline-demo-parcel-visit-the-fountain").into(),
        owner_id,
        group_id,
        is_group_owned: owner_id == group_id,
        status: aurora_net::land::OS_LEASED,
        area: 16384,
        claim_date: 1_655_576_350,
        aabb_min: Vec3::new(128.0, 128.0, 0.0),
        aabb_max: Vec3::new(256.0, 256.0, 0.0),
        max_prims: 3750,
        total_prims: 61,
        prim_bonus: 1.0,
        owner_prims: 47,
        group_prims: 12,
        other_prims: 2,
        selected_prims: 0,
        sim_max_prims: 15000,
        sim_total_prims: 5387,
        other_clean_time: 0,
        flags: pf::ALLOW_FLY
            | pf::CREATE_GROUP_OBJECTS
            | pf::ALLOW_OTHER_SCRIPTS
            | pf::ALLOW_GROUP_SCRIPTS
            | pf::ALLOW_GROUP_OBJECT_ENTRY
            | pf::ALLOW_LANDMARK
            | pf::ALLOW_VOICE_CHAT
            | pf::SOUND_LOCAL
            | pf::RESTRICT_PUSHOBJECT
            | pf::USE_BAN_LIST
            | pf::MATURE_PUBLISH,
        // residential, with a fixed landing point (the overview's mockup)
        category: 10,
        pass_price: 88,
        pass_hours: 1.0,
        music_url: MUSIC_URL.into(),
        snapshot_id: TEX_GRADIENT,
        user_location: Vec3::new(140.0, 130.0, 25.0),
        user_look_at: Vec3::new(1.0, 0.0, 0.0),
        landing_type: 1,
        see_avatars: true,
        any_av_sounds: true,
        group_av_sounds: true,
        have_new_parcel_limit_data: true,
        region_allow_access_override: true,
        region_allow_env_override: true,
        env_version: 1,
        media: ParcelMedia {
            mime: "text/html".into(),
            width: 512,
            height: 512,
            auto_scale: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// The plaza parcel on sale by a resident, with its objects; "other": set
/// to sell to Nova only.
fn for_sale(mut p: ParcelInfo, mode: &str) -> ParcelInfo {
    p.owner_id = DEMO_LOUP;
    p.is_group_owned = false;
    p.flags |= pf::FOR_SALE | pf::SELL_PARCEL_OBJECTS;
    p.sale_price = SALE_PRICE;
    if mode == "other" {
        p.auth_buyer = DEMO_NOVA;
    }
    p
}

/// AURORA_DEMO_BUY_PASS=<mode>: the plaza parcel as the ground menu's
/// "Buy access" finds it (sell, none, own; see docs/TESTING.md).
pub fn pass_scenario() -> Option<String> {
    std::env::var("AURORA_DEMO_BUY_PASS").ok().map(|v| v.trim().to_lowercase())
}

/// Loup's parcel selling passes of an hour for L$ 88 to whoever is not on
/// its access list (`sell`), Loup's without passes (`none`), or the
/// avatar's own (`own`).
fn pass_parcel(mut p: ParcelInfo, mode: &str) -> ParcelInfo {
    p.owner_id = if mode == "own" { DEMO_AGENT } else { DEMO_LOUP };
    p.group_id = VISITOR_GROUP;
    p.is_group_owned = false;
    if mode == "sell" {
        p.flags |= pf::USE_ACCESS_LIST | pf::USE_PASS_LIST;
    }
    p
}

pub(super) fn parcel() -> ParcelInfo {
    let mut g = PARCEL.lock();
    g.get_or_insert_with(|| {
        let p = initial(role());
        let p = match pass_scenario() {
            Some(mode) => pass_parcel(p, &mode),
            None => p,
        };
        match buy_scenario() {
            Some(mode) => for_sale(p, &mode),
            None => p,
        }
    })
    .clone()
}

/// AURORA_DEMO_LAND_PRIMS=<used>[,<step>]: objects on the owner's land in
/// the region, and how many more each answer finds.
fn prims_scenario() -> Option<(i32, i32)> {
    let v = std::env::var("AURORA_DEMO_LAND_PRIMS").ok()?;
    let (used, step) = v.split_once(',').unwrap_or((v.as_str(), "0"));
    Some((used.trim().parse().ok()?, step.trim().parse().unwrap_or(0)))
}

/// Answers given since the start, for the objects that "arrive" meanwhile.
static ANSWERS: AtomicI32 = AtomicI32::new(0);

fn selected() -> NetEvent {
    let mut info = parcel();
    if let Some((used, step)) = prims_scenario() {
        let more = step.saturating_mul(ANSWERS.fetch_add(1, Ordering::Relaxed));
        info.sim_total_prims = used.saturating_add(more);
        info.total_prims = info.total_prims.saturating_add(more);
        info.owner_prims = info.owner_prims.saturating_add(more);
    }
    NetEvent::Land(LandEvent::Selected {
        handle: super::HANDLE,
        info: Arc::new(info),
        result: aurora_net::land::PARCEL_RESULT_SUCCESS,
    })
}

/// The parcel's own day cycle once one is applied: the names of its
/// tracks (0 water, 1 ground, 2–4 skies), its length and offset.
#[derive(Clone)]
struct DemoDay {
    names: [String; 5],
    length: i32,
    offset: i32,
    /// The day's settings map once a day cycle is applied (an item or
    /// edited inline): what About Land's "Custom" edits.
    cycle: Option<Llsd>,
}

static DAY: Mutex<Option<DemoDay>> = Mutex::new(None);

/// `AURORA_DEMO_ENV_EDITOR=custom`: the parcel starts with the user's day
/// cycle applied, for "Custom" to edit (once: a reset then leaves it).
fn custom_day_once() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    if !super::env::editor().is_some_and(|s| s.starts_with("custom")) {
        return;
    }
    ONCE.call_once(|| {
        let asset = super::env::day_asset();
        let name = aurora_i18n::tr!("demo-env-my-day-cycle").to_owned();
        *DAY.lock() = Some(DemoDay {
            names: std::array::from_fn(|t| if t <= 1 { name.clone() } else { String::new() }),
            length: 4 * 3600,
            offset: 16 * 3600,
            cycle: super::env::asset_bytes(asset).and_then(|b| crate::world::eep::Settings::decode_asset(&b)),
        });
    });
}

/// Scripted objects returned from "Script info".
static RETURNED: Mutex<Vec<Uuid>> = Mutex::new(Vec::new());

/// LLSD environment of the parcel: the region's (no day cycle) until a
/// settings item is applied, then a day with the names of its tracks;
/// skies at 1000 / 2000 / 3000 m.
fn environment() -> Llsd {
    custom_day_once();
    let mut env = Llsd::new_map();
    env.insert("parcel_id", 1);
    env.insert(
        "track_altitudes",
        Llsd::Array(vec![Llsd::Real(1000.0), Llsd::Real(2000.0), Llsd::Real(3000.0)]),
    );
    if let Some(d) = DAY.lock().clone() {
        env.insert("day_length", d.length);
        env.insert("day_offset", d.offset);
        let day = d.cycle.clone().unwrap_or_else(|| {
            let mut day = Llsd::new_map();
            let track = |name: &str| {
                if name.is_empty() {
                    Llsd::new_array()
                } else {
                    Llsd::Array(vec![Llsd::new_map()])
                }
            };
            day.insert("tracks", Llsd::Array(d.names.iter().map(|n| track(n)).collect()));
            day
        });
        env.insert("day_cycle", day);
        env.insert("day_names", Llsd::Array(d.names.iter().map(|n| Llsd::String(n.clone())).collect()));
    }
    env
}

fn demo_day() -> DemoDay {
    custom_day_once();
    DAY.lock().clone().unwrap_or(DemoDay {
        names: Default::default(),
        length: 4 * 3600,
        offset: 16 * 3600,
        cycle: None,
    })
}

/// ExtEnvironment PUT of a settings asset: the whole day or one track. A
/// whole day cycle item becomes the parcel's day.
fn apply_environment(track: Option<i32>, asset: Uuid, name: &str) {
    let mut day = demo_day();
    match (track, super::env::kind_of(asset)) {
        (Some(t), _) if (0..5).contains(&t) => day.names[t as usize] = name.to_owned(),
        // a day takes every track; a sky the ground's, a water the water's
        (_, Some(crate::world::env_select::SettingsKind::Water)) => day.names[0] = name.to_owned(),
        (_, Some(crate::world::env_select::SettingsKind::Sky)) => day.names[1] = name.to_owned(),
        _ => {
            day.names = std::array::from_fn(|_| name.to_owned());
            day.cycle = super::env::asset_bytes(asset).and_then(|b| crate::world::eep::Settings::decode_asset(&b));
        }
    }
    *DAY.lock() = Some(day);
}

/// ExtEnvironment PUT of a day sent inline (the editors' "Apply To Parcel /
/// Region", "Custom"): the parcel's day becomes it, named by its tracks; the
/// sky of the demo follows (the simulator would push the new environment).
fn apply_day_inline(local_id: i32, cycle: &Llsd, day_length: i32, day_offset: i32) -> Vec<NetEvent> {
    let mut day = demo_day();
    if day_length > 0 {
        day.length = day_length;
    }
    if day_offset > 0 {
        day.offset = day_offset;
    }
    let name = cycle["name"].to_string_value();
    let tracks = &cycle["tracks"];
    day.names = std::array::from_fn(|t| {
        if tracks.at(t).as_array().is_empty() {
            String::new()
        } else {
            name.clone()
        }
    });
    day.cycle = Some(cycle.clone());
    let (length, offset) = (day.length, day.offset);
    if local_id == aurora_net::land::REGION_PARCEL_ID {
        log::info!("demo environment: the region takes the day \"{name}\"");
    } else {
        log::info!("demo environment: parcel {local_id} takes the day \"{name}\"");
        *DAY.lock() = Some(day);
    }
    let mut env = Llsd::new_map();
    env.insert("parcel_id", local_id);
    env.insert("day_length", length);
    env.insert("day_offset", offset);
    env.insert(
        "track_altitudes",
        Llsd::Array(vec![Llsd::Real(1000.0), Llsd::Real(2000.0), Llsd::Real(3000.0)]),
    );
    env.insert("day_cycle", cycle.clone());
    vec![NetEvent::Environment {
        handle: super::HANDLE,
        parcel_id: local_id,
        environment: env,
    }]
}

/// Experiences the picker finds: (id, name, rating, properties).
fn found_experiences() -> Vec<aurora_net::land::FoundExperience> {
    let e = |id: Uuid, name: &str, maturity: u8, properties: i32| aurora_net::land::FoundExperience {
        id,
        name: name.into(),
        owner: DEMO_LOUP,
        maturity,
        properties,
    };
    vec![
        e(XP_AURORA, aurora_i18n::tr!("demo-land-aurora-experience"), 13, 0),
        e(XP_SITTER, "AVSitter", 13, 0),
        e(XP_INTERACT, aurora_i18n::tr!("demo-land-interactions-general"), 21, 0),
        e(XP_BEACH, aurora_i18n::tr!("demo-land-beach-games"), 13, 0),
        e(XP_CLUB, aurora_i18n::tr!("demo-land-night-club"), 42, 0),
        // grid-wide and privileged: in no parcel list
        e(
            XP_GRID,
            "Linden Realms",
            13,
            aurora_net::land::XP_PROPERTY_GRID | aurora_net::land::XP_PROPERTY_PRIVILEGED,
        ),
    ]
}

/// A new owner and group for the parcel (deed, release, reclaim), off sale.
fn change_owner(owner: Uuid, group: Uuid, group_owned: bool) -> Vec<NetEvent> {
    let mut g = PARCEL.lock();
    if let Some(p) = g.as_mut() {
        p.owner_id = owner;
        p.group_id = group;
        p.is_group_owned = group_owned;
        p.flags &= !(pf::FOR_SALE | pf::ALLOW_DEED_TO_GROUP | pf::CONTRIBUTE_WITH_DEED);
        p.sale_price = 0;
        p.auth_buyer = Uuid::nil();
    }
    drop(g);
    vec![selected()]
}

/// The demo simulator's answer to an About Land request.
pub fn reply(c: &LandCommand) -> Vec<NetEvent> {
    let ev = |e: LandEvent| NetEvent::Land(e);
    match c {
        LandCommand::Select { .. } => vec![selected()],
        LandCommand::Update { update, .. } => {
            let mut g = PARCEL.lock();
            let voice = g.as_mut().map(|p| {
                crate::world::land::apply_update(p, update);
                (p.local_id, p.flags)
            });
            drop(g);
            let mut out = vec![selected()];
            // the simulator tells the agents on the parcel of its voice
            // settings (ParcelVoiceInfo), as the grid does
            if let Some((local_id, flags)) = voice {
                out.push(NetEvent::ParcelVoiceInfo {
                    handle: super::HANDLE,
                    local_id,
                    allowed: flags & pf::ALLOW_VOICE_CHAT != 0,
                    use_estate: Some(flags & pf::USE_ESTATE_VOICE_CHAN != 0),
                });
            }
            out
        }
        LandCommand::AccessListRequest { flags, .. } => {
            let e = |id: Uuid, time: i32| AccessEntry { id, time, flags: 0 };
            let mut out = Vec::new();
            if flags & AL_ACCESS != 0 {
                out.push(ev(LandEvent::AccessList {
                    local_id: 1,
                    flags: AL_ACCESS,
                    entries: vec![e(DEMO_NOVA, 0), e(DEMO_TESS, 0), e(DEMO_LOUP, now() + 25 * 60)],
                }));
            }
            if flags & AL_BAN != 0 {
                out.push(ev(LandEvent::AccessList {
                    local_id: 1,
                    flags: AL_BAN,
                    entries: (0..4u128)
                        .map(|i| e(Uuid::from_u128(0xBA00 + i), if i == 1 { now() + 5 * 3600 } else { 0 }))
                        .collect(),
                }));
            }
            if flags & AL_ALLOW_EXPERIENCE != 0 {
                out.push(ev(LandEvent::AccessList {
                    local_id: 1,
                    flags: AL_ALLOW_EXPERIENCE,
                    entries: [XP_AURORA, XP_SITTER, XP_INTERACT].into_iter().map(|id| e(id, 0)).collect(),
                }));
            }
            out
        }
        LandCommand::Buy { for_group, group, .. } => {
            // the sale goes through: the parcel changes hands
            let mut p = parcel();
            p.owner_id = if *for_group { *group } else { DEMO_AGENT };
            p.is_group_owned = *for_group;
            p.flags &= !(pf::FOR_SALE | pf::SELL_PARCEL_OBJECTS);
            p.sale_price = 0;
            p.auth_buyer = Uuid::nil();
            p.claim_date = now();
            *PARCEL.lock() = Some(p);
            vec![selected()]
        }
        LandCommand::BuyPass { local_id, .. } => {
            log::info!("demo action: ParcelBuyPass on parcel {local_id} (simulé)");
            Vec::new()
        }
        LandCommand::DwellRequest { .. } => vec![ev(LandEvent::Dwell { local_id: 1, dwell: 145.0 })],
        LandCommand::ObjectOwnersRequest { .. } => {
            let o = |id: Uuid, is_group: bool, count: i32, ago: i32| ObjectOwner {
                id,
                is_group,
                count,
                online: false,
                most_recent: (now() - ago) as u32,
            };
            vec![ev(LandEvent::ObjectOwners(vec![
                o(DEMO_AGENT, false, 47, 3600),
                o(DEMO_GROUP1, true, 12, 86400 * 3),
                o(DEMO_LOUP, false, 2, 86400 * 40),
            ]))]
        }
        LandCommand::CovenantRequest { .. } => {
            // the covenant Region / Estate replaced, else the estate's
            let (covenant_id, timestamp) = super::region::covenant().unwrap_or((COVENANT, 1_717_647_528));
            let mut out = vec![ev(LandEvent::Covenant {
                handle: super::HANDLE,
                covenant_id,
                timestamp,
                estate_name: aurora_i18n::tr!("common-aurora-demo").into(),
                estate_owner: super::region::estate_owner(),
            })];
            if !covenant_id.is_nil() {
                let text = super::region::covenant_text(covenant_id)
                    .unwrap_or(aurora_i18n::tr!("demo-land-welcome-to-the-aurora-demo-estate-by-buying-land"));
                out.push(ev(LandEvent::CovenantText {
                    covenant_id,
                    text: Some(text.into()),
                }));
            }
            out
        }
        LandCommand::ParcelIdRequest { local_id, .. } => vec![ev(LandEvent::ParcelId {
            local_id: *local_id,
            id: Some(PARCEL_UUID),
        })],
        LandCommand::EnvironmentRequest { local_id } => vec![ev(LandEvent::Environment {
            local_id: *local_id,
            result: Ok(environment()),
        })],
        LandCommand::EnvironmentReset { local_id } => {
            *DAY.lock() = None;
            vec![ev(LandEvent::Environment {
                local_id: *local_id,
                result: Ok(environment()),
            })]
        }
        LandCommand::EnvironmentUpdate {
            local_id,
            day_length,
            day_offset,
        } => {
            if let Some(d) = DAY.lock().as_mut() {
                if *day_length > 0 {
                    d.length = *day_length;
                }
                if *day_offset > 0 {
                    d.offset = *day_offset;
                }
            }
            vec![ev(LandEvent::Environment {
                local_id: *local_id,
                result: Ok(environment()),
            })]
        }
        LandCommand::EnvironmentApply {
            local_id,
            track,
            asset,
            name,
            ..
        } => {
            let mut out = Vec::new();
            if *local_id == aurora_net::land::REGION_PARCEL_ID {
                // the region's day: the demo's sky follows a day cycle item
                let cycle = super::env::asset_bytes(*asset).and_then(|b| crate::world::eep::Settings::decode_asset(&b));
                if let Some(cycle) = cycle.filter(|c| c["type"].as_str() == "daycycle") {
                    out = apply_day_inline(*local_id, &cycle, -1, -1);
                }
            } else {
                apply_environment(*track, *asset, name);
            }
            out.push(ev(LandEvent::Environment {
                local_id: *local_id,
                result: Ok(environment()),
            }));
            out
        }
        LandCommand::EnvironmentApplyDay {
            local_id,
            day,
            day_length,
            day_offset,
            ..
        } => {
            // AURORA_DEMO_ENV_EDITOR=…-refused: the simulator refuses it
            if super::env::editor().is_some_and(|s| s.ends_with("-refused")) {
                return vec![ev(LandEvent::Environment {
                    local_id: *local_id,
                    result: Err(aurora_i18n::tr!("demo-env-apply-refused").into()),
                })];
            }
            let mut out = apply_day_inline(*local_id, day, *day_length, *day_offset);
            out.push(ev(LandEvent::Environment {
                local_id: *local_id,
                result: Ok(environment()),
            }));
            out
        }
        LandCommand::ExperienceSearch { query, .. } => {
            let q = query.to_lowercase();
            let results = found_experiences()
                .into_iter()
                .chain(super::region::found_experiences())
                .filter(|e| q.is_empty() || e.name.to_lowercase().contains(&q))
                .collect();
            vec![ev(LandEvent::ExperienceSearch {
                query: query.clone(),
                results: Some(results),
            })]
        }
        // Script info: the plaza's scripted objects, then my attachments
        LandCommand::ScriptResources { .. } => {
            let o = |n: u128, name: &str, owner: Uuid, group: bool, kb: i64, urls: i64, at: Vec3| aurora_net::land::ScriptObject {
                id: Uuid::from_u128(0x5C00 + n),
                name: name.into(),
                owner,
                group_owned: group,
                memory: kb * 1024,
                urls,
                location: Some(at),
            };
            let objects: Vec<_> = vec![
                o(
                    1,
                    aurora_i18n::tr!("demo-fountain"),
                    DEMO_AGENT,
                    false,
                    192,
                    1,
                    Vec3::new(135.0, 125.0, 23.6),
                ),
                o(
                    2,
                    aurora_i18n::tr!("demo-land-script-door"),
                    DEMO_GROUP1,
                    true,
                    64,
                    0,
                    Vec3::new(150.0, 130.0, 24.0),
                ),
                o(
                    3,
                    aurora_i18n::tr!("demo-land-script-radio"),
                    DEMO_AGENT,
                    false,
                    128,
                    2,
                    Vec3::new(140.0, 140.0, 24.0),
                ),
                o(
                    4,
                    aurora_i18n::tr!("demo-land-script-sign"),
                    DEMO_LOUP,
                    false,
                    16,
                    0,
                    Vec3::new(130.0, 120.0, 24.0),
                ),
            ]
            .into_iter()
            .filter(|o| !RETURNED.lock().contains(&o.id))
            .collect();
            let used: i64 = objects.iter().map(|o| o.memory).sum();
            let urls: i64 = objects.iter().map(|o| o.urls).sum();
            vec![
                ev(LandEvent::ScriptSummary(Ok(aurora_net::land::ScriptSummary {
                    memory: Some((used, 8 * 1024 * 1024)),
                    urls: Some((urls, 750)),
                }))),
                ev(LandEvent::ScriptDetails(Ok(vec![aurora_net::land::ScriptParcel {
                    name: aurora_i18n::tr!("demo-land-aurora-plaza-offline-demo").into(),
                    local_id: Some(1),
                    objects,
                }]))),
            ]
        }
        LandCommand::AttachmentScripts => {
            let o = |n: u128, name: &str, kb: i64, urls: i64| aurora_net::land::ScriptObject {
                id: Uuid::from_u128(0x5D00 + n),
                name: name.into(),
                owner: DEMO_AGENT,
                group_owned: false,
                memory: kb * 1024,
                urls,
                location: None,
            };
            vec![ev(LandEvent::AttachmentScripts(Ok((
                aurora_net::land::ScriptSummary {
                    memory: Some((320 * 1024, 0)),
                    urls: Some((1, 38)),
                },
                vec![
                    aurora_net::land::ScriptAttachment {
                        location: "Skull".into(),
                        objects: vec![o(1, aurora_i18n::tr!("demo-land-script-hud"), 256, 1)],
                    },
                    aurora_net::land::ScriptAttachment {
                        location: "Right Hand".into(),
                        objects: vec![o(2, aurora_i18n::tr!("demo-land-script-ao"), 64, 0)],
                    },
                ]
                .into_iter()
                .chain(crate::demo::worn::temporary_resources())
                .collect(),
            ))))]
        }
        LandCommand::ReturnTasks { tasks, .. } => {
            RETURNED.lock().extend(tasks.iter().copied());
            Vec::new()
        }
        // ForceObjectSelect: the plaza's disc is the group's, its pillars and
        // orbs the owner's (the demo scene's local IDs 100 to 116)
        LandCommand::SelectObjects { return_type, owners, .. } => {
            use aurora_net::land::{RT_GROUP, RT_LIST, RT_OWNER};
            let listed = |id: &Uuid| *return_type & RT_LIST != 0 && owners.contains(id);
            let mut ids = Vec::new();
            if *return_type & RT_OWNER != 0 || listed(&DEMO_AGENT) {
                ids.extend(101..=116);
            }
            if *return_type & RT_GROUP != 0 || listed(&DEMO_GROUP1) {
                ids.push(100);
            }
            vec![ev(LandEvent::Highlight {
                handle: super::HANDLE,
                reset: true,
                local_ids: ids,
            })]
        }
        // the ownership changes of the General tab, as the simulator makes them
        LandCommand::Release { .. } => change_owner(Uuid::nil(), Uuid::nil(), false),
        LandCommand::Reclaim { .. } => change_owner(DEMO_LOUP, Uuid::nil(), false),
        LandCommand::DeedToGroup { group, .. } => change_owner(*group, *group, true),
        LandCommand::ReturnObjects { return_type, owners, .. } => {
            let mut g = PARCEL.lock();
            if let Some(p) = g.as_mut() {
                use aurora_net::land::{RT_GROUP, RT_LIST, RT_OTHER, RT_OWNER};
                let listed = |id: &Uuid| *return_type & RT_LIST != 0 && owners.contains(id);
                if *return_type & RT_OWNER != 0 || listed(&p.owner_id) || listed(&DEMO_AGENT) && p.owner_id == DEMO_AGENT {
                    p.owner_prims = 0;
                }
                if *return_type & RT_GROUP != 0 || listed(&p.group_id) {
                    p.group_prims = 0;
                }
                if *return_type & RT_OTHER != 0 || listed(&DEMO_LOUP) {
                    p.other_prims = 0;
                }
                p.total_prims = p.owner_prims + p.group_prims + p.other_prims;
            }
            drop(g);
            vec![selected()]
        }
        LandCommand::ExperienceInfo(ids) => vec![ev(LandEvent::ExperienceInfo(
            ids.iter()
                .map(|id| {
                    let name = match *id {
                        XP_AURORA => aurora_i18n::tr!("demo-land-aurora-experience"),
                        XP_SITTER => "AVSitter",
                        XP_INTERACT => aurora_i18n::tr!("demo-land-interactions-general"),
                        XP_BEACH => aurora_i18n::tr!("demo-land-beach-games"),
                        XP_CLUB => aurora_i18n::tr!("demo-land-night-club"),
                        XP_GRID => "Linden Realms",
                        id => super::region::experience_name(id).unwrap_or(aurora_i18n::tr!("demo-land-unknown-experience")),
                    };
                    (*id, name.to_owned())
                })
                .collect(),
        ))],
        LandCommand::GroupNames(ids) => vec![ev(LandEvent::GroupNames(
            ids.iter()
                .map(|id| (*id, aurora_i18n::tr!("demo-land-demo-group").to_owned()))
                .collect(),
        ))],
        LandCommand::AvatarSearch { query } => vec![ev(LandEvent::AvatarSearch {
            query: query.clone(),
            results: Some(vec![
                FoundAvatar {
                    id: DEMO_NOVA,
                    display_name: "Nova ♪".into(),
                    username: "nova.exemple".into(),
                },
                FoundAvatar {
                    id: DEMO_TESS,
                    display_name: "Tess ✨".into(),
                    username: "tesstouch".into(),
                },
            ]),
        })],
        LandCommand::MediaType { url } => vec![ev(LandEvent::MediaType {
            url: url.clone(),
            mime: "text/html".into(),
        })],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::land::{AgentRights, modifiable_by_agent};
    use aurora_net::GroupMembership;

    #[test]
    fn roles_and_the_classic_window() {
        assert_eq!(
            parse("options"),
            Scenario {
                tab: "options".into(),
                role: Role::Member,
                classic: false
            }
        );
        assert_eq!(parse("general:owner").role, Role::Owner);
        let s = parse("overview:classic:visitor");
        assert_eq!((s.tab.as_str(), s.role, s.classic), ("overview", Role::Visitor, true));
        assert_eq!(parse("access:officer").role, Role::Officer);
    }

    /// Each role has the rights it is named after.
    #[test]
    fn each_role_gets_its_rights() {
        let member = |powers: u64| GroupMembership {
            id: DEMO_GROUP1,
            name: "Aurora Builders".into(),
            insignia: Uuid::nil(),
            powers: (1 << 16) | powers,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        };
        let can = |role: Role, groups: &[GroupMembership], power: u64| {
            let rights = AgentRights { id: DEMO_AGENT, groups };
            modifiable_by_agent(&initial(role), &rights, power)
        };
        let officer = [member(OFFICER_POWERS)];
        let plain = [member(0)];
        assert!(can(Role::Owner, &plain, powers::LAND_OPTIONS));
        assert!(can(Role::Officer, &officer, powers::LAND_OPTIONS));
        assert!(!can(Role::Officer, &officer, powers::LAND_EDIT), "terraforming stays read only");
        assert!(!can(Role::Officer, &officer, powers::LAND_SET_SALE_INFO));
        assert!(!can(Role::Member, &plain, powers::LAND_OPTIONS));
        assert!(!can(Role::Visitor, &plain, powers::LAND_OPTIONS));
        let visitor = initial(Role::Visitor);
        assert!(!visitor.is_group_owned && visitor.group_id != DEMO_GROUP1);
    }
}
