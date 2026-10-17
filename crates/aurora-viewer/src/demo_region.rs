//! Region / Estate in the offline demo
//! (`AURORA_DEMO_REGION=overview[:owner|:manager|:resident][:preview][:compact]`): the window
//! opens on the demo region, and the "simulator" answers its requests with
//! the mockup's figures. The role says who the avatar is to the estate:
//! its owner (the default), one of its managers (RegionHandshake
//! IsEstateManager), or a resident, who is not answered `getinfo` nor the
//! estate's lists, as the simulator does. `preview` looks at
//! the region as a resident ("View as"), `compact` opens the window 480 px
//! wide. On `manage`, `access`, `rules` or `terrain` scrolls to that
//! section, `pending` makes changes there that wait for "Apply", and
//! `ask-override`, `ask-linden`, `ask-bonus`, `ask-maturity`, `ask-heights`
//! or `ask-bake` shows that question. The simulator keeps what "Apply"
//! sends and answers with it, as RegionInfo and `estateupdateinfo` do. On
//! `people`, `select` ticks Orion in the allowed residents, `pick` opens the
//! resident picker on them, `ask-ban` asks to ban Orion and `ask-remove` to
//! remove him from the allowed residents; the simulator keeps the lists'
//! changes and answers `setaccess`, then the lists again.

use super::{DEMO_AGENT, DEMO_LOUP, DEMO_NOVA, DEMO_TESS};
use crate::ui::region::{Ask, ManageDemo, Page, PeopleAsk, PeopleDemo, PeopleList};
use aurora_llsd::Llsd;
use aurora_net::estate::{
    BannedAgent, EstateAccess, EstateCommand, EstateEvent, EstateInfo, RegionDetails, RegionExperiences, RestartSchedule, SimStats,
    estate_access as ea, estate_flags as ef, sim_stat,
};
use aurora_net::{NetEvent, region_flags as rf};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Who the avatar is to the demo estate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Owner,
    Manager,
    Resident,
}

/// What AURORA_DEMO_REGION asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    pub page: Page,
    pub role: Role,
    pub preview: bool,
    pub compact: bool,
    pub manage: ManageDemo,
    pub people: PeopleDemo,
}

fn parse(v: &str) -> Scenario {
    let mut s = Scenario {
        page: Page::Overview,
        role: Role::Owner,
        preview: false,
        compact: false,
        manage: ManageDemo::default(),
        people: PeopleDemo::default(),
    };
    for part in v.split(':').map(str::trim) {
        match part {
            "manage" => s.page = Page::Manage,
            "people" => s.page = Page::People,
            "manager" => s.role = Role::Manager,
            "resident" => s.role = Role::Resident,
            "owner" => s.role = Role::Owner,
            "preview" => s.preview = true,
            "compact" => s.compact = true,
            "access" => s.manage.section = Some(0),
            "rules" => s.manage.section = Some(1),
            "terrain" => s.manage.section = Some(2),
            "pending" => s.manage.pending = true,
            "ask-override" => s.manage.ask = Some(Ask::AccessOverride),
            "ask-linden" => s.manage.ask = Some(Ask::LindenEstate),
            "ask-bonus" => {
                s.manage.section = Some(1);
                s.manage.ask = Some(Ask::ObjectBonus { dont_ask: false });
            }
            "ask-maturity" => s.manage.ask = Some(Ask::MaturityChanged),
            "ask-heights" => {
                s.manage.section = Some(2);
                s.manage.ask = Some(Ask::TextureHeights);
            }
            "ask-bake" => {
                s.manage.section = Some(2);
                s.manage.ask = Some(Ask::Bake);
            }
            "select" => s.people.select = Some(ORION),
            "pick" => s.people.pick = true,
            "ask-ban" => {
                s.people.ask = Some(PeopleAsk::Scope {
                    list: PeopleList::Banned,
                    add: true,
                    ids: vec![ORION],
                    all: false,
                })
            }
            "ask-remove" => {
                s.people.select = Some(ORION);
                s.people.ask = Some(PeopleAsk::Scope {
                    list: PeopleList::Allowed,
                    add: false,
                    ids: vec![ORION],
                    all: false,
                });
            }
            _ => {}
        }
    }
    s
}

pub fn scenario() -> Option<Scenario> {
    std::env::var("AURORA_DEMO_REGION").ok().map(|v| parse(&v))
}

/// The estate's owner: the avatar when it owns it, else Loup.
pub fn estate_owner() -> Uuid {
    match scenario() {
        Some(s) if s.role == Role::Owner => DEMO_AGENT,
        _ => DEMO_LOUP,
    }
}

/// RegionHandshake IsEstateManager of the demo region.
pub fn is_manager() -> bool {
    scenario().is_some_and(|s| s.role == Role::Manager)
}

/// The estate is answered to its owner and managers only.
fn manages() -> bool {
    scenario().is_some_and(|s| s.role != Role::Resident)
}

const ESTATE_ID: u32 = 104_123;
const XP_WELCOME: Uuid = Uuid::from_u128(0xE7E0_0000_0000_0000_0000_0000_0000_0001);
const XP_HUNT: Uuid = Uuid::from_u128(0xE7E0_0000_0000_0000_0000_0000_0000_0007);
const XP_TELEPORTERS: Uuid = Uuid::from_u128(0xE7E0_0000_0000_0000_0000_0000_0000_0008);
const ORION: Uuid = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0022);
const PIXEL: Uuid = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0021);
const BUILDERS: Uuid = Uuid::from_u128(0x6E00_0000_0000_0000_0000_0000_0000_0001);
/// Members of the demo's generated crowd (`demo_group.rs`): Lune Example
/// and Quinn Resident, so that every row has its own name.
const LUNE: Uuid = Uuid::from_u128(0x6E6E_0000_0000_0000_0000_0000_0000_0001);
const QUINN: Uuid = Uuid::from_u128(0x6E6E_0000_0000_0000_0000_0000_0000_0010);

/// The mockup's region: no terraforming, pushing limited, voice, resale
/// but no join / subdivide, parcels with their own environment.
pub fn region_details() -> RegionDetails {
    RegionDetails {
        name: aurora_i18n::tr!("common-aurora-demo").into(),
        estate_id: ESTATE_ID,
        parent_estate_id: 1,
        flags: u64::from(rf::BLOCK_TERRAFORM | rf::RESTRICT_PUSHOBJECT | rf::ALLOW_VOICE) | ef::ALLOW_ENVIRONMENT_OVERRIDE,
        sim_access: 13,
        max_agents: 40,
        hard_max_agents: 100,
        object_bonus: 1.0,
        billable_factor: 1.0,
        water_height: 20.0,
        terrain_raise_limit: 4.0,
        terrain_lower_limit: -4.0,
        price_per_meter: 1,
        use_estate_sun: true,
        sun_hour: 0.0,
        product_name: "Estate / Full Region".into(),
    }
}

/// The simulator's second: 12 avatars, 10 500 objects of 15 000, smooth.
fn stats() -> SimStats {
    SimStats {
        values: vec![
            (sim_stat::TIME_DILATION, 0.99),
            (sim_stat::FPS, 45.0),
            (sim_stat::PHYSFPS, 45.0),
            (sim_stat::FRAMEMS, 22.0),
            (sim_stat::SIMPHYSICSMS, 1.1),
            (sim_stat::SCRIPTMS, 4.2),
            (sim_stat::NUMTASKS, 10_500.0),
            (sim_stat::NUMTASKSACTIVE, 412.0),
            (sim_stat::NUMAGENTMAIN, 12.0),
            (sim_stat::NUMAGENTCHILD, 3.0),
            (sim_stat::NUMSCRIPTSACTIVE, 312.0),
            (sim_stat::SIMSPARETIME, 9.8),
        ],
        object_capacity: 15_000,
    }
}

/// The region's day cycle: "Aube nordique", four hours long.
fn environment() -> Llsd {
    let mut env = Llsd::new_map();
    env.insert("day_length", 4 * 3600);
    env.insert("day_offset", -8 * 3600);
    let mut day = Llsd::new_map();
    day.insert("tracks", Llsd::Array(vec![Llsd::Array(vec![Llsd::new_map()]); 5]));
    env.insert("day_cycle", day);
    env.insert("day_names", Llsd::String(aurora_i18n::tr!("demo-region-day-cycle").into()));
    env
}

/// What "Apply" sent: the demo simulator keeps the region and the estate's
/// flags it was given.
static SENT: Mutex<Option<(RegionDetails, u64)>> = Mutex::new(None);

fn with_sent<R>(f: impl FnOnce(&mut (RegionDetails, u64)) -> R) -> R {
    let mut sent = SENT.lock().unwrap_or_else(|e| e.into_inner());
    let state = sent.get_or_insert_with(|| {
        (
            region_details(),
            ef::EXTERNALLY_VISIBLE
                | ef::ALLOW_DIRECT_TELEPORT
                | ef::ALLOW_VOICE
                | ef::ALLOW_ACCESS_OVERRIDE
                | ef::ALLOW_ENVIRONMENT_OVERRIDE,
        )
    });
    f(state)
}

/// The estate's lists, as the simulator keeps them: the mockup's, then
/// what `estateaccessdelta` changed.
static LISTS: Mutex<Option<EstateAccess>> = Mutex::new(None);

fn with_lists<R>(f: impl FnOnce(&mut EstateAccess) -> R) -> R {
    let mut lists = LISTS.lock().unwrap_or_else(|e| e.into_inner());
    let lists = lists.get_or_insert_with(|| EstateAccess {
        // in the demo, Tess is named Nova and Nova Tess: the mockup's order
        managers: vec![DEMO_TESS, DEMO_NOVA],
        allowed_agents: vec![ORION, DEMO_NOVA, LUNE],
        allowed_groups: vec![BUILDERS],
        banned: vec![
            BannedAgent {
                id: PIXEL,
                last_login: Some("2026-10-13 18:02".into()),
                banned_on: Some("2026-10-09 10:00".into()),
                banned_by: Some(DEMO_TESS),
            },
            BannedAgent {
                id: QUINN,
                last_login: Some("2026-07-02 21:40".into()),
                banned_on: Some("2026-05-02 09:12".into()),
                banned_by: Some(estate_owner()),
            },
        ],
    });
    f(lists)
}

/// One `estateaccessdelta` applied to the lists (the scope's flags change
/// nothing in a single estate). A ban is dated now, by the avatar.
fn apply_delta(lists: &mut EstateAccess, flags: u32, id: Uuid, now: &str) {
    let add = |list: &mut Vec<Uuid>| {
        if !list.contains(&id) {
            list.push(id);
        }
    };
    if flags & ea::MANAGER_ADD != 0 {
        add(&mut lists.managers);
    }
    if flags & ea::MANAGER_REMOVE != 0 {
        lists.managers.retain(|m| *m != id);
    }
    if flags & ea::ALLOWED_AGENT_ADD != 0 {
        add(&mut lists.allowed_agents);
    }
    if flags & ea::ALLOWED_AGENT_REMOVE != 0 {
        lists.allowed_agents.retain(|m| *m != id);
    }
    if flags & ea::ALLOWED_GROUP_ADD != 0 {
        add(&mut lists.allowed_groups);
    }
    if flags & ea::ALLOWED_GROUP_REMOVE != 0 {
        lists.allowed_groups.retain(|m| *m != id);
    }
    if flags & ea::BANNED_AGENT_ADD != 0 && !lists.banned.iter().any(|b| b.id == id) {
        lists.banned.push(BannedAgent {
            id,
            last_login: None,
            banned_on: Some(now.to_owned()),
            banned_by: Some(DEMO_AGENT),
        });
    }
    if flags & ea::BANNED_AGENT_REMOVE != 0 {
        lists.banned.retain(|b| b.id != id);
    }
}

/// "YYYY-MM-DD HH:MM" now, as the capability dates a ban.
fn now_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (y, m, d, _, h, mi, _) = crate::ui::land::slt_parts(secs);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}")
}

fn estate_info() -> EstateInfo {
    EstateInfo {
        name: aurora_i18n::tr!("common-aurora-demo").into(),
        owner: estate_owner(),
        id: ESTATE_ID,
        flags: with_sent(|s| s.1),
        sun_hour: 0.0,
    }
}

/// The region with the General panel's rules applied, as the simulator
/// stores them.
fn with_rules(mut d: RegionDetails, r: &aurora_net::estate::RegionRules) -> RegionDetails {
    let mut set = |flag: u32, on: bool| {
        if on {
            d.flags |= u64::from(flag);
        } else {
            d.flags &= !u64::from(flag);
        }
    };
    set(rf::BLOCK_TERRAFORM, r.block_terraform);
    set(rf::BLOCK_FLY, r.block_fly);
    set(rf::BLOCK_FLYOVER, r.block_fly_over);
    set(rf::ALLOW_DAMAGE, r.allow_damage);
    set(rf::RESTRICT_PUSHOBJECT, r.restrict_pushobject);
    set(rf::BLOCK_LAND_RESELL, !r.allow_land_resell);
    set(rf::ALLOW_PARCEL_CHANGES, r.allow_parcel_changes);
    set(rf::BLOCK_PARCEL_SEARCH, r.block_parcel_search);
    d.max_agents = r.agent_limit.round() as u32;
    d.object_bonus = r.object_bonus;
    d.sim_access = r.sim_access;
    d
}

/// The simulator's answers to the window.
pub fn reply(c: &EstateCommand) -> Vec<NetEvent> {
    let ev = NetEvent::Estate;
    let region_info = |handle| {
        vec![ev(EstateEvent::RegionInfo {
            handle,
            info: Arc::new(with_sent(|s| s.0.clone())),
        })]
    };
    match c {
        EstateCommand::SetRegionInfo { handle, rules } => {
            with_sent(|s| s.0 = with_rules(s.0.clone(), rules));
            region_info(*handle)
        }
        EstateCommand::SetEstateInfo { handle, info } => {
            with_sent(|s| s.1 = info.flags);
            vec![ev(EstateEvent::EstateInfo {
                handle: *handle,
                info: Arc::new(estate_info()),
            })]
        }
        EstateCommand::SetTerrain { handle, terrain, .. } => {
            with_sent(|s| {
                s.0.water_height = terrain.water_height;
                s.0.terrain_raise_limit = terrain.raise_limit;
                s.0.terrain_lower_limit = terrain.lower_limit;
            });
            region_info(*handle)
        }
        EstateCommand::BakeTerrain { .. } => Vec::new(),
        EstateCommand::Refresh { handle } => {
            let handle = *handle;
            let mut out = Vec::new();
            if manages() {
                out.push(ev(EstateEvent::EstateInfo {
                    handle,
                    info: Arc::new(estate_info()),
                }));
            }
            out.push(ev(EstateEvent::Access {
                handle,
                access: manages().then(|| Arc::new(with_lists(|l| l.clone()))),
            }));
            out.push(ev(EstateEvent::Experiences {
                handle,
                experiences: Some(RegionExperiences {
                    trusted: vec![XP_WELCOME],
                    allowed: vec![XP_HUNT, XP_TELEPORTERS],
                    blocked: Vec::new(),
                }),
            }));
            out.push(ev(EstateEvent::Environment {
                handle,
                environment: Some(environment()),
            }));
            out.push(ev(EstateEvent::Schedule {
                handle,
                schedule: Ok(Some(RestartSchedule {
                    days: [true, false, false, false, false, false, false],
                    seconds: 6 * 3600,
                })),
            }));
            out.push(ev(EstateEvent::Stats {
                handle,
                stats: Arc::new(stats()),
            }));
            out.push(ev(EstateEvent::SimVersion {
                handle,
                version: "Second Life Server 2026-10-02.4893".into(),
            }));
            out
        }
        EstateCommand::RequestRegionInfo { handle } => region_info(*handle),
        // the last of a batch is answered (NO_REPLY on the others)
        EstateCommand::AccessDelta { handle, flags, ids, batch } => {
            let now = now_stamp();
            with_lists(|l| {
                for (id, f) in ids.iter().zip(aurora_net::estate::access_delta_flags(*flags, ids.len(), *batch)) {
                    apply_delta(l, f, *id, &now);
                }
            });
            vec![ev(EstateEvent::AccessChanged { handle: *handle })]
        }
        EstateCommand::RefreshAccess { handle } => vec![ev(EstateEvent::Access {
            handle: *handle,
            access: manages().then(|| Arc::new(with_lists(|l| l.clone()))),
        })],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scenario_reads_its_page_and_role() {
        assert_eq!(
            parse("overview"),
            Scenario {
                page: Page::Overview,
                role: Role::Owner,
                preview: false,
                compact: false,
                manage: ManageDemo::default(),
                people: PeopleDemo::default(),
            }
        );
        assert_eq!(parse("overview:resident").role, Role::Resident);
        assert_eq!(parse("overview:manager").role, Role::Manager);
        let s = parse("overview:owner:preview:compact");
        assert!(s.preview && s.compact);
        let m = parse("manage:manager:rules:pending");
        assert_eq!((m.page, m.manage.section, m.manage.pending), (Page::Manage, Some(1), true));
        assert_eq!(parse("manage:ask-heights").manage.ask, Some(Ask::TextureHeights));
        let pp = parse("people:manager:select");
        assert_eq!((pp.page, pp.role, pp.people.select), (Page::People, Role::Manager, Some(ORION)));
        assert!(matches!(
            parse("people:ask-ban").people.ask,
            Some(PeopleAsk::Scope {
                list: PeopleList::Banned,
                add: true,
                ..
            })
        ));
    }

    /// What the window sends changes the lists the simulator answers.
    #[test]
    fn the_demo_simulator_keeps_the_lists() {
        let mut l = EstateAccess::default();
        apply_delta(&mut l, ea::BANNED_AGENT_ADD | ea::APPLY_TO_ALL_ESTATES, ORION, "2026-10-17 01:00");
        apply_delta(&mut l, ea::BANNED_AGENT_ADD, ORION, "2026-10-17 01:00");
        assert_eq!(l.banned.len(), 1);
        assert_eq!(l.banned[0].banned_on.as_deref(), Some("2026-10-17 01:00"));
        apply_delta(&mut l, ea::ALLOWED_GROUP_ADD, BUILDERS, "");
        apply_delta(&mut l, ea::BANNED_AGENT_REMOVE | ea::NO_REPLY, ORION, "");
        assert!(l.banned.is_empty());
        assert_eq!(l.allowed_groups, [BUILDERS]);
    }

    /// What "Apply" sends comes back in the next RegionInfo.
    #[test]
    fn the_demo_simulator_keeps_the_rules() {
        let mut r = aurora_net::estate::RegionRules::of(&region_details());
        r.block_fly = true;
        r.agent_limit = 50.0;
        let d = with_rules(region_details(), &r);
        assert_eq!(aurora_net::estate::RegionRules::of(&d), r);
    }

    #[test]
    fn the_mockup_s_region() {
        let d = region_details();
        assert_eq!((d.max_agents, d.hard_max_agents, d.estate_id), (40, 100, 104_123));
        let s = stats();
        assert_eq!(s.get(sim_stat::NUMAGENTMAIN), Some(12.0));
        assert_eq!(s.get(sim_stat::NUMTASKS), Some(10_500.0));
    }
}
