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
//! changes and answers `setaccess`, then the lists again. On
//! `manage:interventions`, `ask-message`, `ask-home`, `ask-kick`,
//! `ask-return`, `ask-restart` or `ask-schedule` opens that intervention's
//! dialog (on Orion for the kick and the return); the simulator answers a
//! restart with its notice (a cancel with "cancelled") and keeps the
//! schedule it is sent. `scripts` scrolls to Scripts and physics,
//! `top-scripts` or `top-colliders` opens that report: the simulator lists
//! the mockup's objects, filters them as LandStatRequest asks, forgets the
//! returned ones and keeps the debug switches. `environment`,
//! `experiences` or `covenant` scrolls to that section; `pick-env` opens the
//! settings picker, `ask-env-reset` or `ask-env-override` asks about the
//! default environment or the parcels' own; `pick-trusted` opens the
//! experience picker on the trusted list, `ask-xp` asks where to remove an
//! allowed experience; `pick-notecard` opens the notecard picker and
//! `ask-covenant` asks before clearing the covenant. The simulator keeps
//! the region's day, timing and sky altitudes, the experience lists and
//! the covenant it is sent, and two notecards wait in the inventory.
//! `classic` opens the classic window on the tab of the section asked
//! (People: Access).
//! `telehub` opens the region rules' Advanced fold (the demo region has a
//! telehub with two spawn points, and keeps what Connect, Disconnect, Add
//! and the x send), `raw` the terrain's (a download answers a flat RAW
//! file, an upload is taken), `pbr` turns the terrain to PBR materials with
//! that fold open (the simulator keeps the transforms it is sent).
//! `pick-material` opens the first terrain picker with the demo's tiled PBR
//! material, also with `classic`.

use super::{DEMO_AGENT, DEMO_LOUP, DEMO_NOVA, DEMO_TESS};
use crate::ui::region::{Ask, EstateWideDialog, Home, InterventionDialog, ManageDemo, Page, PeopleAsk, PeopleDemo, PeopleList, XpList};
use crate::world::inventory::Inventory;
use aurora_llsd::Llsd;
use aurora_net::estate::{
    BannedAgent, EstateAccess, EstateCommand, EstateEvent, EstateInfo, RawTransfer, RegionDetails, RegionExperiences, RestartSchedule,
    SimStats, TelehubInfo, TelehubVerb, TerrainTransform, TopObject, TopObjects, estate_access as ea, estate_flags as ef, land_stat,
    sim_stat, skip_flags,
};
use aurora_net::inventory::InvItem;
use aurora_net::land::{FoundExperience, LandCommand, LandEvent, REGION_PARCEL_ID, XP_PROPERTY_GRID};
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
    /// The intervention's dialog shown on Manage › Interventions.
    pub intervention: Option<InterventionDialog>,
    /// The Top window opened on that report.
    pub top: Option<u32>,
    /// The picker or question of Environment, Experiences or Covenant.
    pub estate_wide: Option<EstateWideDialog>,
    /// The classic window (the Legacy Viewer's tabs).
    pub classic: bool,
}

fn parse(v: &str) -> Scenario {
    let mut s = Scenario {
        page: Page::Overview,
        role: Role::Owner,
        preview: false,
        compact: false,
        manage: ManageDemo::default(),
        people: PeopleDemo::default(),
        intervention: None,
        top: None,
        estate_wide: None,
        classic: false,
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
            "classic" => s.classic = true,
            "access" => s.manage.section = Some(0),
            "rules" => s.manage.section = Some(1),
            "terrain" => s.manage.section = Some(2),
            "interventions" => s.manage.section = Some(3),
            "scripts" => s.manage.section = Some(4),
            "telehub" => {
                s.manage.section = Some(1);
                s.manage.advanced = true;
            }
            "raw" => {
                s.manage.section = Some(2);
                s.manage.advanced = true;
            }
            "pbr" => {
                s.manage.section = Some(2);
                s.manage.advanced = true;
                s.manage.pbr = true;
            }
            "pick-material" => {
                s.manage.section = Some(2);
                s.manage.pbr = true;
                s.manage.pick_material = true;
            }
            "pbr-ground" | "pbr-preview" | "pbr-cancel" | "pbr-apply" | "pbr-paint" | "pbr-paint-reset" | "pbr-mask" | "pbr-slopes" => {
                s.manage.section = Some(2);
                s.manage.pbr_ground = true;
                s.manage.surface_demo = match part {
                    "pbr-preview" => 1,
                    "pbr-cancel" => 2,
                    "pbr-apply" => 3,
                    "pbr-paint" => 4,
                    "pbr-paint-reset" => 5,
                    "pbr-mask" => 6,
                    "pbr-slopes" => 8,
                    _ => 0,
                };
                s.manage.advanced = part.starts_with("pbr-paint");
            }
            "environment" => s.manage.section = Some(5),
            "experiences" => s.manage.section = Some(6),
            "covenant" => s.manage.section = Some(7),
            "pick-env" => {
                s.manage.section = Some(5);
                s.estate_wide = Some(EstateWideDialog::environment());
            }
            "ask-env-reset" => {
                s.manage.section = Some(5);
                s.manage.ask = Some(Ask::ResetEnvironment);
            }
            "ask-env-override" => {
                s.manage.section = Some(5);
                s.manage.ask = Some(Ask::EnvironmentOverride { allow: false });
            }
            "pick-trusted" => {
                s.manage.section = Some(6);
                s.estate_wide = Some(EstateWideDialog::experience(XpList::Trusted));
            }
            "ask-xp" => {
                s.manage.section = Some(6);
                s.estate_wide = Some(EstateWideDialog::Scope {
                    list: XpList::Allowed,
                    add: false,
                    id: XP_HUNT,
                    all: false,
                });
            }
            "pick-notecard" => {
                s.manage.section = Some(7);
                s.estate_wide = Some(EstateWideDialog::notecard());
            }
            "ask-covenant" => {
                s.manage.section = Some(7);
                s.manage.ask = Some(Ask::Covenant { asset: Uuid::nil() });
            }
            "top-scripts" => {
                s.manage.section = Some(4);
                s.top = Some(land_stat::TOP_SCRIPTS);
            }
            "top-colliders" => {
                s.manage.section = Some(4);
                s.top = Some(land_stat::TOP_COLLIDERS);
            }
            "ask-message" => s.intervention = Some(InterventionDialog::message()),
            "ask-home" => {
                s.intervention = Some(InterventionDialog::Home {
                    query: String::new(),
                    target: Some(Home::One(PIXEL)),
                })
            }
            "ask-kick" => s.intervention = Some(InterventionDialog::Kick { target: ORION, ban: false }),
            "ask-return" => {
                s.intervention = Some(InterventionDialog::Return {
                    target: ORION,
                    scripted: true,
                    others_land: true,
                    estate_wide: false,
                })
            }
            "ask-restart" => s.intervention = Some(InterventionDialog::restart()),
            "ask-schedule" => {
                s.intervention = Some(InterventionDialog::Schedule {
                    days: [true, false, false, false, false, false, false],
                    hour: 6,
                    minute: 0,
                })
            }
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

pub fn terrain_ids() -> Option<[Uuid; 4]> {
    scenario().filter(|s| s.manage.pbr_ground).map(|s| match s.manage.surface_demo {
        4 | 5 => [
            super::MAT_PBR_TILES,
            terrain_replacement(),
            crate::scene::meshes::BLANK_MATERIAL,
            super::MAT_PBR_TILES,
        ],
        6 => [terrain_mask(); 4],
        _ => [super::MAT_PBR_TILES; 4],
    })
}

pub fn terrain_mask() -> Uuid {
    Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0016)
}

pub fn terrain_start_height() -> [f32; 4] {
    match scenario().filter(|s| s.manage.pbr_ground) {
        Some(s) if !matches!(s.manage.surface_demo, 4 | 5) => [80.0; 4],
        _ => [20.0; 4],
    }
}

pub fn terrain_replacement() -> Uuid {
    Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0015)
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
/// A grid-wide experience, the only kind a region can block.
const XP_GRID_GAME: Uuid = Uuid::from_u128(0xE7E0_0000_0000_0000_0000_0000_0000_0009);
/// The notecards seeded in the inventory, and their assets.
const NOTECARDS: [(u128, u128); 2] = [
    (0x0C0E_0000_0000_0000_0000_0000_0000_0001, 0x0C0E_A000_0000_0000_0000_0000_0000_0001),
    (0x0C0E_0000_0000_0000_0000_0000_0000_0002, 0x0C0E_A000_0000_0000_0000_0000_0000_0002),
];
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

/// The region's environment as the simulator keeps it: the names of its
/// tracks (0 water, 1 ground, 2–4 skies), the day's length and offset
/// (seconds) and the skies' altitudes.
#[derive(Clone)]
struct RegionDay {
    names: [String; 5],
    length: i32,
    offset: i32,
    altitudes: [f32; 3],
}

static REGION_DAY: Mutex<Option<RegionDay>> = Mutex::new(None);

/// "Aube nordique", four hours long, eight hours early; once reset, the
/// grid's default day without a name.
fn region_day(reset: bool) -> RegionDay {
    let name = if reset {
        String::new()
    } else {
        aurora_i18n::tr!("demo-region-day-cycle").to_owned()
    };
    RegionDay {
        names: std::array::from_fn(|_| name.clone()),
        length: 4 * 3600,
        offset: 16 * 3600,
        altitudes: aurora_net::land::DEFAULT_SKY_ALTITUDES,
    }
}

fn with_day<R>(f: impl FnOnce(&mut RegionDay) -> R) -> R {
    let mut day = REGION_DAY.lock().unwrap_or_else(|e| e.into_inner());
    f(day.get_or_insert_with(|| region_day(false)))
}

/// The region's ExtEnvironment answer.
fn environment() -> Llsd {
    let d = with_day(|d| d.clone());
    let mut env = Llsd::new_map();
    env.insert("day_length", d.length);
    env.insert("day_offset", d.offset);
    let alts = d.altitudes.iter().map(|a| Llsd::Real(f64::from(*a))).collect();
    env.insert("track_altitudes", Llsd::Array(alts));
    let mut day = Llsd::new_map();
    day.insert("tracks", Llsd::Array(vec![Llsd::Array(vec![Llsd::new_map()]); 5]));
    env.insert("day_cycle", day);
    if d.names.iter().all(|n| *n == d.names[0]) {
        env.insert("day_names", Llsd::String(d.names[0].clone()));
    } else {
        env.insert("day_names", Llsd::Array(d.names.iter().map(|n| Llsd::String(n.clone())).collect()));
    }
    env
}

/// The demo simulator's answer to a change of the region environment
/// (ExtEnvironment without a parcel); None for a parcel's. An item or a day
/// sent inline still changes the demo's sky (`demo_land.rs`).
pub fn environment_reply(c: &LandCommand) -> Option<Vec<NetEvent>> {
    let region = |id: &i32| *id == REGION_PARCEL_ID;
    let mut out = Vec::new();
    match c {
        LandCommand::EnvironmentRequest { local_id } if region(local_id) => {}
        LandCommand::EnvironmentReset { local_id } if region(local_id) => {
            *REGION_DAY.lock().unwrap_or_else(|e| e.into_inner()) = Some(region_day(true));
        }
        LandCommand::EnvironmentUpdate {
            local_id,
            day_length,
            day_offset,
        } if region(local_id) => with_day(|d| {
            if *day_length > 0 {
                d.length = *day_length;
            }
            if *day_offset > 0 {
                d.offset = *day_offset;
            }
        }),
        LandCommand::EnvironmentAltitudes { local_id, altitudes, .. } if region(local_id) => with_day(|d| {
            let mut a = *altitudes;
            a.sort_by(f32::total_cmp);
            d.altitudes = a;
        }),
        LandCommand::EnvironmentApply { local_id, track, name, .. } if region(local_id) => {
            out = super::land::reply(c);
            with_day(|d| match track {
                Some(t) if (0..5).contains(t) => d.names[*t as usize] = name.clone(),
                _ => d.names = std::array::from_fn(|_| name.clone()),
            });
        }
        LandCommand::EnvironmentApplyDay { local_id, day, .. } if region(local_id) => {
            out = super::land::reply(c);
            // a refused day keeps the region's
            if out
                .iter()
                .any(|e| matches!(e, NetEvent::Land(LandEvent::Environment { result: Err(_), .. })))
            {
                return Some(out);
            }
            let name = day["name"].to_string_value();
            with_day(|d| d.names = std::array::from_fn(|_| name.clone()));
        }
        _ => return None,
    }
    out.retain(|e| !matches!(e, NetEvent::Land(LandEvent::Environment { .. })));
    out.push(NetEvent::Land(LandEvent::Environment {
        local_id: REGION_PARCEL_ID,
        result: Ok(environment()),
    }));
    Some(out)
}

/// The region's experience lists, then what `estateexperiencedelta` changed.
static EXPERIENCES: Mutex<Option<RegionExperiences>> = Mutex::new(None);

fn with_experiences<R>(f: impl FnOnce(&mut RegionExperiences) -> R) -> R {
    let mut x = EXPERIENCES.lock().unwrap_or_else(|e| e.into_inner());
    f(x.get_or_insert_with(|| RegionExperiences {
        trusted: vec![XP_WELCOME],
        allowed: vec![XP_HUNT, XP_TELEPORTERS],
        blocked: Vec::new(),
        default: None,
    }))
}

/// One `estateexperiencedelta` applied to the lists.
fn apply_experience(x: &mut RegionExperiences, flags: u32, id: Uuid) {
    use aurora_net::estate::estate_experience as xe;
    for (add, remove, list) in [
        (xe::TRUSTED_ADD, xe::TRUSTED_REMOVE, &mut x.trusted),
        (xe::ALLOWED_ADD, xe::ALLOWED_REMOVE, &mut x.allowed),
        (xe::BLOCKED_ADD, xe::BLOCKED_REMOVE, &mut x.blocked),
    ] {
        if flags & add != 0 && !list.contains(&id) {
            list.push(id);
        }
        if flags & remove != 0 {
            list.retain(|e| *e != id);
        }
    }
}

/// The names of the region's experiences (GetExperienceInfo).
pub fn experience_name(id: Uuid) -> Option<&'static str> {
    Some(match id {
        XP_HUNT => aurora_i18n::tr!("demo-region-xp-hunt"),
        XP_TELEPORTERS => aurora_i18n::tr!("demo-region-xp-teleporters"),
        XP_GRID_GAME => aurora_i18n::tr!("demo-region-xp-grid-game"),
        _ => return None,
    })
}

/// What the experience picker also finds: the region's allowed ones (the
/// trusted one is About Land's "Aurora experience"), and a grid-wide one
/// to block.
pub fn found_experiences() -> Vec<FoundExperience> {
    [(XP_HUNT, 0), (XP_TELEPORTERS, 0), (XP_GRID_GAME, XP_PROPERTY_GRID)]
        .into_iter()
        .map(|(id, properties)| FoundExperience {
            id,
            name: experience_name(id).unwrap_or_default().into(),
            owner: DEMO_LOUP,
            maturity: 13,
            properties,
        })
        .collect()
}

/// The demo region's telehub: "Arrival point" with two spawn points, then
/// what the window changed.
static TELEHUB: Mutex<Option<TelehubInfo>> = Mutex::new(None);

fn with_telehub<R>(f: impl FnOnce(&mut TelehubInfo) -> R) -> R {
    let mut t = TELEHUB.lock().unwrap_or_else(|e| e.into_inner());
    f(t.get_or_insert_with(|| TelehubInfo {
        object: Uuid::from_u128(0x7E1E_0000_0000_0000_0000_0000_0000_0001),
        name: aurora_i18n::tr!("demo-region-telehub").into(),
        position: [128.0, 128.0, 22.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        spawns: vec![[4.0, 0.0, 0.0], [-4.0, 2.5, 0.0]],
    }))
}

/// A telehub request applied as the simulator does: Connect makes the
/// first selected object the telehub, Disconnect drops it and its spawn
/// points, Add puts a spawn point near each selected object.
fn apply_telehub(t: &mut TelehubInfo, verb: TelehubVerb, objects: &[u32]) {
    match verb {
        TelehubVerb::Info => {}
        TelehubVerb::Connect => {
            if let Some(id) = objects.first() {
                t.object = Uuid::from_u128(0x7E1E_0000_0000_0000_0000_0000_0000_0000 + u128::from(*id));
                t.name = aurora_i18n::tr!("demo-region-telehub-object").into();
            }
        }
        TelehubVerb::Delete => {
            t.object = Uuid::nil();
            t.name.clear();
            t.spawns.clear();
        }
        TelehubVerb::AddSpawn => {
            for id in objects {
                if t.spawns.len() < aurora_net::estate::MAX_SPAWNPOINTS_PER_TELEHUB {
                    t.spawns.push([(id % 7) as f32, 3.0, 0.0]);
                }
            }
        }
        TelehubVerb::RemoveSpawn(i) => {
            if (i as usize) < t.spawns.len() {
                t.spawns.remove(i as usize);
            }
        }
    }
}

/// The materials' transforms ModifyRegion keeps.
static TRANSFORMS: Mutex<Option<[TerrainTransform; 4]>> = Mutex::new(None);

fn transforms() -> [TerrainTransform; 4] {
    TRANSFORMS.lock().unwrap_or_else(|e| e.into_inner()).unwrap_or_default()
}

/// A flat RAW terrain file of 256 × 256 points (13 channels, the height in
/// the first, scaled by the second / 128): 20 m everywhere.
fn raw_file() -> Vec<u8> {
    let mut data = vec![0u8; 256 * 256 * 13];
    for px in data.as_chunks_mut::<13>().0 {
        px[0] = 20;
        px[1] = 128;
    }
    data
}

/// The covenant the simulator was last sent (its asset, when), if any.
static COVENANT: Mutex<Option<(Uuid, u32)>> = Mutex::new(None);

pub fn covenant() -> Option<(Uuid, u32)> {
    *COVENANT.lock().unwrap_or_else(|e| e.into_inner())
}

/// The text of a covenant replaced by one of the seeded notecards.
pub fn covenant_text(asset: Uuid) -> Option<&'static str> {
    NOTECARDS.iter().position(|(_, a)| Uuid::from_u128(*a) == asset).map(|n| {
        if n == 0 {
            aurora_i18n::tr!("demo-region-notecard-rules-text")
        } else {
            aurora_i18n::tr!("demo-region-notecard-welcome-text")
        }
    })
}

/// AURORA_DEMO_REGION: two notecards in the inventory, for the covenant.
pub fn seed_notecards(inv: &mut Inventory, agent: Uuid) {
    if inv.items.contains_key(&Uuid::from_u128(NOTECARDS[0].0)) {
        return;
    }
    let names = [
        aurora_i18n::tr!("demo-region-notecard-rules"),
        aurora_i18n::tr!("demo-region-notecard-welcome"),
    ];
    inv.add_items(
        NOTECARDS
            .iter()
            .zip(names)
            .map(|((id, asset), name)| InvItem {
                id: Uuid::from_u128(*id),
                parent: inv.root,
                asset_id: Uuid::from_u128(*asset),
                asset_type: 7,
                inv_type: 7,
                name: name.into(),
                owner: agent,
                creator: agent,
                base_mask: 0x7fffffff,
                owner_mask: 0x7fffffff,
                next_owner_mask: 0x7fffffff,
                ..Default::default()
            })
            .collect(),
    );
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

/// The objects the demo simulator reports, minus the returned ones:
/// scripts (time in ms, memory, URLs) and colliders.
static RETURNED: Mutex<Vec<Uuid>> = Mutex::new(Vec::new());

fn top_objects(report: u32, flags: u32, filter: &str) -> TopObjects {
    let returned = RETURNED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let row = |n: u128, score: f32, name: &str, owner: &str, pos: [f32; 3], parcel: &str, memory: f32, urls: i32| TopObject {
        local_id: n as u32,
        task_id: Uuid::from_u128(0x70B0_0000_0000_0000_0000_0000_0000_0000 + n),
        position: pos,
        score,
        name: name.into(),
        owner: owner.into(),
        parcel: parcel.into(),
        time: 1_781_000_000 + n as u32 * 86_400,
        memory,
        urls,
        extended: true,
    };
    let all = if report == land_stat::TOP_COLLIDERS {
        vec![
            row(11, 412.0, "Bouncy castle", "orion.exemple", [96.0, 140.0, 22.0], "Plaza", 0.0, 0),
            row(12, 88.5, "Pool balls", "pixelboutique", [151.0, 92.0, 21.5], "North shore", 0.0, 0),
            row(13, 12.25, "Swing", "lune.example", [130.0, 118.0, 24.0], "Plaza", 0.0, 0),
        ]
    } else {
        vec![
            row(
                1,
                0.842,
                "Fountain particles",
                "orion.exemple",
                [128.0, 130.0, 22.0],
                "Plaza",
                262_144.0,
                1,
            ),
            row(
                2,
                0.415,
                "Radio stream",
                "pixelboutique",
                [151.0, 92.0, 23.0],
                "North shore",
                131_072.0,
                2,
            ),
            row(3, 0.231, "Greeter", "aurora.demo", [132.0, 126.0, 33.0], "Plaza", 65_536.0, 0),
            row(4, 0.104, "Door", "lune.example", [110.0, 140.0, 22.5], "Market", 16_384.0, 0),
            row(
                5,
                0.021,
                "Tip jar",
                "pixelboutique",
                [149.0, 95.0, 22.0],
                "North shore",
                16_384.0,
                1,
            ),
        ]
    };
    let f = filter.to_lowercase();
    let objects: Vec<TopObject> = all
        .into_iter()
        .filter(|o| !returned.contains(&o.task_id))
        .filter(|o| match flags {
            0 => true,
            x if x == land_stat::FILTER_BY_OWNER => o.owner.contains(&f),
            x if x == land_stat::FILTER_BY_PARCEL_NAME => o.parcel.to_lowercase().contains(&f),
            _ => o.name.to_lowercase().contains(&f),
        })
        .collect();
    TopObjects {
        report,
        flags,
        total: if report == land_stat::TOP_COLLIDERS { 3 } else { 312 },
        objects,
    }
}

/// The restart schedule the simulator keeps: Sunday at 06:00 UTC, then
/// what was saved.
static SCHEDULE: Mutex<Option<Option<RestartSchedule>>> = Mutex::new(None);

fn schedule() -> Option<RestartSchedule> {
    let mut s = SCHEDULE.lock().unwrap_or_else(|e| e.into_inner());
    *s.get_or_insert(Some(RestartSchedule {
        days: [true, false, false, false, false, false, false],
        seconds: 6 * 3600,
    }))
}

fn save_schedule(days: [bool; 7], seconds: u32) {
    // a weekly schedule without days clears it
    let kept = days.iter().any(|d| *d).then_some(RestartSchedule { days, seconds });
    *SCHEDULE.lock().unwrap_or_else(|e| e.into_inner()) = Some(kept);
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
                experiences: Some(with_experiences(|x| x.clone())),
            }));
            out.push(ev(EstateEvent::Environment {
                handle,
                environment: Some(environment()),
            }));
            out.push(ev(EstateEvent::Schedule {
                handle,
                schedule: Ok(schedule()),
            }));
            out.push(ev(EstateEvent::TerrainTransforms {
                handle,
                transforms: Ok(transforms()),
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
        // the region's notice, as the simulator's AlertMessage tells it
        EstateCommand::Restart { seconds, .. } if *seconds < 0 => vec![NetEvent::RegionRestartCancelled],
        EstateCommand::Restart { seconds, .. } => vec![NetEvent::RegionRestart(aurora_net::region_restart::Notice {
            seconds: *seconds,
            region: Some(aurora_i18n::tr!("common-aurora-demo").into()),
            opensim: false,
        })],
        EstateCommand::SetRestartSchedule { handle, days, seconds } => {
            save_schedule(*days, *seconds);
            vec![ev(EstateEvent::Schedule {
                handle: *handle,
                schedule: Ok(schedule()),
            })]
        }
        EstateCommand::SetRegionDebug { handle, debug } => {
            with_sent(|s| {
                let mut f = s.0.flags & !(skip_flags::SCRIPTS | skip_flags::COLLISIONS | skip_flags::PHYSICS);
                for (on, bit) in [
                    (debug.skip_scripts, skip_flags::SCRIPTS),
                    (debug.skip_collisions, skip_flags::COLLISIONS),
                    (debug.skip_physics, skip_flags::PHYSICS),
                ] {
                    if on {
                        f |= bit;
                    }
                }
                s.0.flags = f;
            });
            region_info(*handle)
        }
        EstateCommand::TopObjects { handle, report } => vec![ev(EstateEvent::TopObjects {
            handle: *handle,
            report: Arc::new(top_objects(*report, 0, "")),
        })],
        EstateCommand::LandStat {
            handle,
            report,
            flags,
            filter,
        } => vec![ev(EstateEvent::TopObjects {
            handle: *handle,
            report: Arc::new(top_objects(*report, *flags, filter)),
        })],
        EstateCommand::ReturnTopObjects { tasks, .. } => {
            RETURNED.lock().unwrap_or_else(|e| e.into_inner()).extend(tasks.iter().copied());
            Vec::new()
        }
        EstateCommand::Telehub { handle, verb, objects } => {
            let info = with_telehub(|t| {
                apply_telehub(t, *verb, objects);
                t.clone()
            });
            vec![ev(EstateEvent::Telehub {
                handle: *handle,
                info: Arc::new(info),
            })]
        }
        EstateCommand::TerrainRaw { handle, transfer } => vec![match transfer {
            RawTransfer::Download { path } => ev(EstateEvent::RawDownloaded {
                handle: *handle,
                path: path.clone(),
                result: Ok(Arc::new(raw_file())),
            }),
            RawTransfer::Upload { .. } => ev(EstateEvent::RawUploaded {
                handle: *handle,
                result: Ok(()),
            }),
        }],
        EstateCommand::TerrainTransforms { handle, transforms: t } => {
            *TRANSFORMS.lock().unwrap_or_else(|e| e.into_inner()) = Some(*t);
            vec![ev(EstateEvent::TerrainTransforms {
                handle: *handle,
                transforms: Ok(*t),
            })]
        }
        EstateCommand::ExperienceDelta { handle, flags, ids } => {
            with_experiences(|x| {
                for id in ids {
                    apply_experience(x, *flags, *id);
                }
            });
            vec![ev(EstateEvent::ExperiencesChanged { handle: *handle })]
        }
        EstateCommand::RefreshExperiences { handle } => vec![ev(EstateEvent::Experiences {
            handle: *handle,
            experiences: Some(with_experiences(|x| x.clone())),
        })],
        EstateCommand::ChangeCovenant { asset, .. } => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as u32);
            *COVENANT.lock().unwrap_or_else(|e| e.into_inner()) = Some((*asset, if asset.is_nil() { 0 } else { now }));
            Vec::new()
        }
        // nobody to send home, kick or return in the demo
        EstateCommand::Message { .. }
        | EstateCommand::TeleportHome { .. }
        | EstateCommand::KickFromEstate { .. }
        | EstateCommand::ReturnObjects { .. } => Vec::new(),
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
                intervention: None,
                top: None,
                estate_wide: None,
                classic: false,
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
        let i = parse("manage:interventions:ask-kick");
        assert_eq!(i.manage.section, Some(3));
        assert_eq!(i.intervention, Some(InterventionDialog::Kick { target: ORION, ban: false }));
        assert_eq!(parse("manage:ask-restart").intervention, Some(InterventionDialog::restart()));
        let t = parse("manage:top-colliders");
        assert_eq!((t.manage.section, t.top), (Some(4), Some(land_stat::TOP_COLLIDERS)));
        let x = parse("manage:ask-xp");
        assert_eq!(x.manage.section, Some(6));
        assert!(matches!(x.estate_wide, Some(EstateWideDialog::Scope { add: false, .. })));
        assert_eq!(parse("manage:ask-covenant").manage.ask, Some(Ask::Covenant { asset: Uuid::nil() }));
    }

    /// The telehub follows Connect, Add, the x and Disconnect.
    #[test]
    fn the_demo_telehub() {
        let mut t = TelehubInfo::default();
        apply_telehub(&mut t, TelehubVerb::Connect, &[42]);
        assert!(!t.object.is_nil());
        apply_telehub(&mut t, TelehubVerb::AddSpawn, &[1, 2]);
        apply_telehub(&mut t, TelehubVerb::RemoveSpawn(0), &[]);
        assert_eq!(t.spawns.len(), 1);
        apply_telehub(&mut t, TelehubVerb::Delete, &[]);
        assert!(t.object.is_nil() && t.spawns.is_empty());
        assert_eq!(raw_file().len(), 851_968);
        assert!(parse("manage:pbr").manage.pbr);
        let picker = parse("manage:pick-material:classic");
        assert!(picker.manage.pbr && picker.manage.pick_material && picker.classic);
        assert_eq!(picker.manage.section, Some(2));
    }

    /// The lists change as `estateexperiencedelta` says.
    #[test]
    fn the_demo_simulator_keeps_the_experiences() {
        use aurora_net::estate::estate_experience as xe;
        let mut x = RegionExperiences::default();
        apply_experience(&mut x, xe::BLOCKED_ADD | ea::APPLY_TO_ALL_ESTATES, XP_GRID_GAME);
        apply_experience(&mut x, xe::BLOCKED_ADD, XP_GRID_GAME);
        assert_eq!(x.blocked, [XP_GRID_GAME]);
        apply_experience(&mut x, xe::BLOCKED_REMOVE | ea::NO_REPLY, XP_GRID_GAME);
        assert!(x.blocked.is_empty());
        assert!(found_experiences().iter().any(|e| e.properties & XP_PROPERTY_GRID != 0));
    }

    /// A parcel's environment is not the region's.
    #[test]
    fn only_the_region_environment_is_answered_here() {
        assert!(environment_reply(&LandCommand::EnvironmentReset { local_id: 3 }).is_none());
        let answer = environment_reply(&LandCommand::EnvironmentRequest {
            local_id: REGION_PARCEL_ID,
        });
        assert!(matches!(
            answer.as_deref(),
            Some([NetEvent::Land(LandEvent::Environment {
                local_id: REGION_PARCEL_ID,
                result: Ok(_)
            })])
        ));
    }

    /// A restart is answered with the region's notice, a cancel with
    /// "cancelled"; a saved schedule is the one answered.
    #[test]
    fn the_demo_simulator_answers_the_interventions() {
        let restart = reply(&EstateCommand::Restart { handle: 1, seconds: 120 });
        assert!(matches!(&restart[..], [NetEvent::RegionRestart(n)] if n.seconds == 120));
        let cancel = reply(&EstateCommand::Restart { handle: 1, seconds: -1 });
        assert!(matches!(cancel[..], [NetEvent::RegionRestartCancelled]));
        let days = [false, true, false, false, false, false, false];
        let saved = reply(&EstateCommand::SetRestartSchedule {
            handle: 1,
            days,
            seconds: 3600,
        });
        assert!(matches!(
            &saved[..],
            [NetEvent::Estate(EstateEvent::Schedule { schedule: Ok(Some(s)), .. })] if s.days == days && s.seconds == 3600
        ));
        assert!(reply(&EstateCommand::KickFromEstate { handle: 1, target: ORION }).is_empty());
        // the report, filtered by owner as LandStatRequest asks
        let filtered = top_objects(land_stat::TOP_SCRIPTS, land_stat::FILTER_BY_OWNER, "PIXEL");
        assert_eq!(filtered.objects.len(), 2);
        assert!(filtered.objects.iter().all(|o| o.owner == "pixelboutique"));
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
