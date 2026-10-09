//! About Land in the offline demo (`AURORA_DEMO_LAND=<onglet>[:owner]`):
//! the simulator's answers to the floater's requests, for the plaza
//! parcel (north-east quarter of the demo region). Without `:owner` the
//! parcel belongs to a group the avatar has no powers in (everything
//! greyed, like a visitor); with it, the avatar owns it and can change it.

use super::{DEMO_AGENT, DEMO_GROUP1, DEMO_LOUP, DEMO_NOVA, DEMO_TESS, TEX_GRADIENT};
use aurora_llsd::Llsd;
use aurora_net::land::{AL_ACCESS, AL_ALLOW_EXPERIENCE, AL_BAN, AccessEntry, FoundAvatar, LandCommand, LandEvent, ObjectOwner};
use aurora_net::{NetEvent, ParcelInfo, ParcelMedia, parcel_flags as pf};
use glam::Vec3;
use parking_lot::Mutex;
use std::sync::Arc;
use uuid::Uuid;

const COVENANT: Uuid = Uuid::from_u128(0xC0E0_0000_0000_0000_0000_0000_0000_0001);
const PARCEL_UUID: Uuid = Uuid::from_u128(0x1214_B5B2_7C42_D6C4_8741_B182_4390_8089);
const XP_AURORA: Uuid = Uuid::from_u128(0xE7E0_0000_0000_0000_0000_0000_0000_0001);
const XP_SITTER: Uuid = Uuid::from_u128(0xE7E0_0000_0000_0000_0000_0000_0000_0002);
const XP_INTERACT: Uuid = Uuid::from_u128(0xE7E0_0000_0000_0000_0000_0000_0000_0003);

/// The demo parcel as the "simulator" keeps it (updates are applied).
static PARCEL: Mutex<Option<ParcelInfo>> = Mutex::new(None);

/// AURORA_DEMO_LAND: (tab name, the avatar owns the parcel).
pub fn scenario() -> Option<(String, bool)> {
    let v = std::env::var("AURORA_DEMO_LAND").ok()?;
    let (tab, owner) = v.split_once(':').map_or((v.as_str(), false), |(t, o)| (t, o == "owner"));
    Some((tab.to_owned(), owner))
}

fn now() -> i32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i32)
        .unwrap_or(0)
}

fn initial(owner: bool) -> ParcelInfo {
    ParcelInfo {
        local_id: 1,
        sequence_id: aurora_net::land::SELECTED_PARCEL_SEQ_ID,
        name: "Place d'Aurora (démo hors ligne)".into(),
        desc: "Parcelle de démonstration hors ligne d'Aurora Viewer.\nVisitez la fontaine, la plage et la colline.".into(),
        owner_id: if owner { DEMO_AGENT } else { DEMO_GROUP1 },
        group_id: DEMO_GROUP1,
        is_group_owned: !owner,
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
            | pf::CREATE_OBJECTS
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
        category: 9,
        pass_price: 88,
        pass_hours: 1.0,
        music_url: "http://flux.exemple.org:8000/".into(),
        snapshot_id: TEX_GRADIENT,
        user_location: Vec3::new(140.0, 130.0, 25.0),
        user_look_at: Vec3::new(1.0, 0.0, 0.0),
        landing_type: 2,
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

fn parcel() -> ParcelInfo {
    let mut g = PARCEL.lock();
    g.get_or_insert_with(|| initial(scenario().is_some_and(|s| s.1))).clone()
}

fn selected() -> NetEvent {
    NetEvent::Land(LandEvent::Selected {
        handle: super::HANDLE,
        info: Arc::new(parcel()),
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
                estate_name: "Aurora Démo".into(),
                estate_owner: DEMO_LOUP,
            }),
            ev(LandEvent::CovenantText {
                covenant_id: COVENANT,
                text: Some(
                    "Bienvenue sur le domaine Aurora Démo.\n\nEn achetant un terrain sur ce domaine, vous acceptez les règles suivantes :\n\n1. Les constructions restent dans le thème nordique du domaine.\n2. Pas de panneaux publicitaires visibles depuis les parcelles voisines.\n3. Les bruits de fond (radios, scripts sonores) sont limités à votre parcelle.\n4. Les parcelles inoccupées pendant 90 jours peuvent être reprises.\n\nMerci de garder la place d'Aurora accueillante pour tout le monde."
                        .into(),
                ),
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
                        XP_AURORA => "Expérience Aurora",
                        XP_SITTER => "AVSitter",
                        XP_INTERACT => "Interactions (General)",
                        _ => "Expérience inconnue",
                    };
                    (*id, name.to_owned())
                })
                .collect(),
        ))],
        LandCommand::GroupNames(ids) => vec![ev(LandEvent::GroupNames(
            ids.iter().map(|id| (*id, "Groupe de démonstration".to_owned())).collect(),
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
