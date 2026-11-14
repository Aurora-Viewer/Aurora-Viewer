//! AURORA_DEMO_GROUP_REGION: the Group titles window and the title worn by
//! region (`world/region_titles.rs`). The account has three rules: the demo
//! region wears "Aurora Builders" › Builder (already worn at the start:
//! nothing is sent), "Boreal Lagoon" wears "Northern Wolves" › Officer, and
//! a third region a group left since ("Boreal Photographers", listed as
//! ignored). The demo server answers the titles of every group, keeps the
//! title chosen in each (GroupTitleUpdate) and names the active group with
//! it (ActivateGroup → AgentDataUpdate), as the grid does. "Aurora
//! Builders" also offers a role whose title is blank: the Group titles
//! window lists it as "<Blank title>" and it is not worn at the start.
//!
//! `1`: the window opens at frame 600; at frame 900 the avatar arrives in
//! "Boreal Lagoon" (a teleport: the Officer title and the group are sent),
//! at frame 1500 back in the demo region (Builder again). `still`: the
//! window and the rules, no teleport.

use aurora_net::groups::GroupTitle;
use aurora_net::{NetCommand, NetEvent, RegionInfo};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use super::{DEMO_GROUP1, DEMO_GROUP2, DEMO_GROUP3, HANDLE};
use crate::world::region_titles::{Saved, Target};

/// The roles of the two other groups (their "everyone" role is nil).
const WOLF_OFFICERS: Uuid = Uuid::from_u128(0x6E41_0000_0000_0000_0000_0000_0000_0002);
const WOLF_OWNERS: Uuid = Uuid::from_u128(0x6E41_0000_0000_0000_0000_0000_0000_0001);
const MARKET_OFFICERS: Uuid = Uuid::from_u128(0x6E42_0000_0000_0000_0000_0000_0000_0002);
/// The role of "Aurora Builders" the demo wears (demo_group.rs).
const BUILDERS: Uuid = Uuid::from_u128(0x6E40_0000_0000_0000_0000_0000_0000_0003);
/// A role of "Aurora Builders" whose title is blank. Listed, not worn.
const BUILDERS_BLANK: Uuid = Uuid::from_u128(0x6E40_0000_0000_0000_0000_0000_0000_0004);

/// The region the scripted teleport reaches, out of reach of the demo
/// region like AURORA_DEMO_TP_AWAY's.
const LAGOON: u64 = HANDLE + ((100 * 256u64) << 32);

/// The title chosen in each group (GroupTitleUpdate), the demo's own.
static SELECTED: Mutex<Option<HashMap<Uuid, Uuid>>> = Mutex::new(None);

fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_GROUP_REGION").ok()
}

pub fn on() -> bool {
    mode().is_some()
}

fn target(group: Uuid, role: Uuid, title: &str, group_name: &str) -> Target {
    Target {
        group,
        role,
        title: title.into(),
        group_name: group_name.into(),
    }
}

/// The account's rules in this scenario; none without it.
pub fn rules() -> Saved {
    let mut saved = Saved::default();
    if !on() {
        return saved;
    }
    saved.assign(
        aurora_i18n::tr!("common-aurora-demo"),
        target(DEMO_GROUP1, BUILDERS, aurora_i18n::tr!("demo-builder"), "Aurora Builders"),
    );
    saved.assign(
        aurora_i18n::tr!("common-boreal-lagoon"),
        target(
            DEMO_GROUP2,
            WOLF_OFFICERS,
            aurora_i18n::tr!("demo-group-officer"),
            aurora_i18n::tr!("demo-northern-wolves"),
        ),
    );
    saved.assign(
        aurora_i18n::tr!("common-pine-grove"),
        target(
            super::search::boreal(),
            Uuid::nil(),
            aurora_i18n::tr!("demo-group-founder"),
            "Boreal Photographers",
        ),
    );
    saved
}

fn titles(group: &Uuid) -> Option<Vec<(String, Uuid)>> {
    let member = aurora_i18n::tr!("demo-group-member").to_owned();
    Some(match *group {
        DEMO_GROUP1 => vec![
            (member, Uuid::nil()),
            (aurora_i18n::tr!("demo-builder").to_owned(), BUILDERS),
            (String::new(), BUILDERS_BLANK),
        ],
        DEMO_GROUP2 => vec![
            (member, Uuid::nil()),
            (aurora_i18n::tr!("demo-group-officer").to_owned(), WOLF_OFFICERS),
            (aurora_i18n::tr!("demo-group-founder").to_owned(), WOLF_OWNERS),
        ],
        DEMO_GROUP3 => vec![
            (member, Uuid::nil()),
            (aurora_i18n::tr!("demo-group-officer").to_owned(), MARKET_OFFICERS),
        ],
        _ => return None,
    })
}

fn group_name(group: &Uuid) -> String {
    match *group {
        DEMO_GROUP1 => "Aurora Builders".into(),
        DEMO_GROUP2 => aurora_i18n::tr!("demo-northern-wolves").into(),
        DEMO_GROUP3 => aurora_i18n::tr!("common-plaza-market").into(),
        _ => String::new(),
    }
}

/// The title worn in `group`: the one chosen, else Builder in "Aurora
/// Builders" and the members' title elsewhere.
fn selected(group: &Uuid) -> Uuid {
    let chosen = SELECTED.lock().ok().and_then(|s| s.as_ref().and_then(|m| m.get(group).copied()));
    chosen.unwrap_or(if *group == DEMO_GROUP1 { BUILDERS } else { Uuid::nil() })
}

/// The demo server's answers in this scenario.
pub fn reply(cmd: &NetCommand) -> Option<Vec<NetEvent>> {
    if !on() {
        return None;
    }
    match cmd {
        NetCommand::GroupTitlesRequest { group, request } => {
            let chosen = selected(group);
            let list = titles(group)?
                .into_iter()
                .map(|(title, role)| GroupTitle {
                    title,
                    role,
                    selected: role == chosen,
                })
                .collect();
            Some(vec![NetEvent::GroupTitles {
                group: *group,
                request: *request,
                titles: list,
            }])
        }
        NetCommand::GroupTitleUpdate { group, role } => {
            log::info!("demo group region: GroupTitleUpdate {group} {role}");
            if let Ok(mut s) = SELECTED.lock() {
                s.get_or_insert_with(HashMap::new).insert(*group, *role);
            }
            Some(Vec::new())
        }
        NetCommand::ActivateGroup(group) => {
            log::info!("demo group region: ActivateGroup {group}");
            let role = selected(group);
            let title = titles(group)
                .and_then(|l| l.into_iter().find(|(_, r)| *r == role).map(|(t, _)| t))
                .unwrap_or_default();
            Some(vec![NetEvent::ActiveGroup {
                id: *group,
                name: group_name(group),
                title,
            }])
        }
        _ => None,
    }
}

/// `1`: the teleport to "Boreal Lagoon" at frame 900 and back at 1500.
pub fn trip(frame: u64, world: &crate::world::World) -> Option<Vec<NetEvent>> {
    let handle = match frame {
        900 => LAGOON,
        1500 => HANDLE,
        _ => return None,
    };
    if mode().as_deref() != Some("1") {
        return None;
    }
    log::info!("demo group region: teleport at frame {frame}");
    let (position, look_at) = (world.agent.position, world.agent.forward());
    let mut events = vec![
        NetEvent::TeleportFinished { handle, flying: false },
        NetEvent::MainRegionChanged { handle },
    ];
    if handle == LAGOON {
        events.push(NetEvent::RegionHandshake(Arc::new(RegionInfo {
            handle,
            name: aurora_i18n::tr!("common-boreal-lagoon").into(),
            region_id: Uuid::nil(),
            water_height: 20.0,
            sim_access: 21,
            region_flags: 0,
            terrain_base: [Uuid::nil(); 4],
            terrain_detail: [Uuid::nil(); 4],
            terrain_start_height: [0.0; 4],
            terrain_height_range: [30.0; 4],
            size_x: 256,
            size_y: 256,
            is_main: true,
            owner: Uuid::nil(),
            is_estate_manager: false,
            product_name: "Estate / Full Region".into(),
            billable_factor: 1.0,
        })));
    }
    events.push(NetEvent::AgentMovementComplete { handle, position, look_at });
    Some(events)
}
