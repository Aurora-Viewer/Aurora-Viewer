//! Region / Estate in the offline demo
//! (`AURORA_DEMO_REGION=overview[:owner|:manager|:resident][:preview][:compact]`): the window
//! opens on the demo region, and the "simulator" answers its requests with
//! the mockup's figures. The role says who the avatar is to the estate:
//! its owner (the default), one of its managers (RegionHandshake
//! IsEstateManager), or a resident, who is not answered `getinfo` nor the
//! estate's lists, as the simulator does. `preview` looks at
//! the region as a resident ("View as"), `compact` opens the window 480 px
//! wide.

use super::{DEMO_AGENT, DEMO_LOUP, DEMO_NOVA, DEMO_TESS};
use crate::ui::region::Page;
use aurora_llsd::Llsd;
use aurora_net::estate::{
    EstateAccess, EstateCommand, EstateEvent, EstateInfo, RegionDetails, RegionExperiences, RestartSchedule, SimStats, estate_flags as ef,
    sim_stat,
};
use aurora_net::{NetEvent, region_flags as rf};
use std::sync::Arc;
use uuid::Uuid;

/// Who the avatar is to the demo estate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Owner,
    Manager,
    Resident,
}

/// What AURORA_DEMO_REGION asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scenario {
    pub page: Page,
    pub role: Role,
    pub preview: bool,
    pub compact: bool,
}

fn parse(v: &str) -> Scenario {
    let mut s = Scenario {
        page: Page::Overview,
        role: Role::Owner,
        preview: false,
        compact: false,
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

/// The simulator's answers to the window.
pub fn reply(c: &EstateCommand) -> Vec<NetEvent> {
    let ev = NetEvent::Estate;
    match c {
        EstateCommand::Refresh { handle } => {
            let handle = *handle;
            let mut out = Vec::new();
            if manages() {
                out.push(ev(EstateEvent::EstateInfo {
                    handle,
                    info: Arc::new(EstateInfo {
                        name: aurora_i18n::tr!("common-aurora-demo").into(),
                        owner: estate_owner(),
                        id: ESTATE_ID,
                        flags: ef::EXTERNALLY_VISIBLE
                            | ef::ALLOW_DIRECT_TELEPORT
                            | ef::ALLOW_VOICE
                            | ef::ALLOW_ACCESS_OVERRIDE
                            | ef::ALLOW_ENVIRONMENT_OVERRIDE,
                        sun_hour: 0.0,
                    }),
                }));
            }
            out.push(ev(EstateEvent::Access {
                handle,
                access: manages().then(|| {
                    Arc::new(EstateAccess {
                        // in the demo, Tess is named Nova and Nova Tess: the mockup's order
                        managers: vec![DEMO_TESS, DEMO_NOVA],
                        allowed_agents: vec![ORION, DEMO_NOVA, Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0023)],
                        allowed_groups: vec![BUILDERS],
                        banned: vec![
                            aurora_net::estate::BannedAgent {
                                id: PIXEL,
                                last_login: Some("2026-10-13 18:02".into()),
                                banned_on: Some("2026-10-09 10:00".into()),
                                banned_by: Some(DEMO_TESS),
                            },
                            aurora_net::estate::BannedAgent {
                                id: Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0024),
                                last_login: Some("2026-07-02 21:40".into()),
                                banned_on: Some("2026-05-02 09:12".into()),
                                banned_by: Some(estate_owner()),
                            },
                        ],
                    })
                }),
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
        EstateCommand::RequestRegionInfo { handle } => vec![ev(EstateEvent::RegionInfo {
            handle: *handle,
            info: Arc::new(region_details()),
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
            }
        );
        assert_eq!(parse("overview:resident").role, Role::Resident);
        assert_eq!(parse("overview:manager").role, Role::Manager);
        let s = parse("overview:owner:preview:compact");
        assert!(s.preview && s.compact);
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
