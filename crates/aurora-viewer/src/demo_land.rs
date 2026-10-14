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
    std::env::var("AURORA_DEMO_LAND").ok().map(|v| parse(&v))
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

pub(super) fn parcel() -> ParcelInfo {
    let mut g = PARCEL.lock();
    g.get_or_insert_with(|| initial(scenario().map_or(Role::Member, |s| s.role)))
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

/// LLSD environment of the parcel: a 4 h day 16 h ahead (shown as -8 h),
/// skies at 1000 / 2000 / 3000 m, no names (the region's environment).
fn environment(day_length: i32, day_offset: i32) -> Llsd {
    let mut env = Llsd::new_map();
    env.insert("parcel_id", 1);
    env.insert("day_length", day_length);
    env.insert("day_offset", day_offset);
    env.insert(
        "track_altitudes",
        Llsd::Array(vec![Llsd::Real(1000.0), Llsd::Real(2000.0), Llsd::Real(3000.0)]),
    );
    let mut day = Llsd::new_map();
    let frame = || Llsd::Array(vec![Llsd::new_map()]);
    day.insert(
        "tracks",
        Llsd::Array(vec![frame(), frame(), Llsd::new_array(), Llsd::new_array(), Llsd::new_array()]),
    );
    env.insert("day_cycle", day);
    env
}

/// The demo simulator's answer to an About Land request.
pub fn reply(c: &LandCommand) -> Vec<NetEvent> {
    let ev = |e: LandEvent| NetEvent::Land(e);
    match c {
        LandCommand::Select { .. } => vec![selected()],
        LandCommand::Update { update, .. } => {
            let mut g = PARCEL.lock();
            if let Some(p) = g.as_mut() {
                crate::world::land::apply_update(p, update);
            }
            drop(g);
            vec![selected()]
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
        LandCommand::CovenantRequest { .. } => vec![
            ev(LandEvent::Covenant {
                handle: super::HANDLE,
                covenant_id: COVENANT,
                timestamp: 1_717_647_528,
                estate_name: aurora_i18n::tr!("common-aurora-demo").into(),
                estate_owner: DEMO_LOUP,
            }),
            ev(LandEvent::CovenantText {
                covenant_id: COVENANT,
                text: Some(aurora_i18n::tr!("demo-land-welcome-to-the-aurora-demo-estate-by-buying-land").into()),
            }),
        ],
        LandCommand::ParcelIdRequest { local_id, .. } => vec![ev(LandEvent::ParcelId {
            local_id: *local_id,
            id: Some(PARCEL_UUID),
        })],
        LandCommand::EnvironmentRequest { local_id } | LandCommand::EnvironmentReset { local_id } => vec![ev(LandEvent::Environment {
            local_id: *local_id,
            result: Ok(environment(4 * 3600, 16 * 3600)),
        })],
        LandCommand::EnvironmentUpdate {
            local_id,
            day_length,
            day_offset,
        } => vec![ev(LandEvent::Environment {
            local_id: *local_id,
            result: Ok(environment(*day_length, *day_offset)),
        })],
        LandCommand::ExperienceInfo(ids) => vec![ev(LandEvent::ExperienceInfo(
            ids.iter()
                .map(|id| {
                    let name = match *id {
                        XP_AURORA => aurora_i18n::tr!("demo-land-aurora-experience"),
                        XP_SITTER => "AVSitter",
                        XP_INTERACT => aurora_i18n::tr!("demo-land-interactions-general"),
                        _ => aurora_i18n::tr!("demo-land-unknown-experience"),
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
