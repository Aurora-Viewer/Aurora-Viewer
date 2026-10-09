//! Places of the demo (`AURORA_DEMO_PLACE`, `AURORA_DEMO_PLACES`): the
//! RemoteParcelRequest, ParcelInfoRequest and RegionHandleRequest answers
//! for the regions of the demo map, the landmarks of the Landmarks and
//! Favorites folders with their assets, and a teleport history over several
//! days, so the « Lieux » window can be tried offline. Aurora Démo answers
//! with the parcel of "À propos du terrain"; Faille answers HTTP 404
//! (LLPanelPlaceInfo::setErrorStatus).

use super::{DEMO_LOUP, DEMO_SIMS, TEX_SNAPSHOT, u};
use crate::world::tphistory::HistoryItem;
use aurora_net::inventory::{FolderContents, InvFolder, InvItem};
use aurora_net::land::RemoteParcelError;
use aurora_net::{NetEvent, ParcelSummary, RegionHandle};
use glam::{DVec3, Vec3};
use uuid::Uuid;

/// Inventory folders of the demo skeleton (`demo_raw`).
pub const LANDMARKS_FOLDER: u128 = 12;
pub const FAVORITES_FOLDER: u128 = 20;
/// A sub-folder of the landmarks.
pub const SHOPS_FOLDER: u128 = 30;

/// One demo parcel per region (grid x, y in region units, as DEMO_SIMS).
struct DemoParcel {
    grid: (u32, u32),
    id: Uuid,
    name: &'static str,
    desc: &'static str,
    area: i32,
    dwell: f32,
    /// 0x1 mature, 0x2 adult, 0x4 group-owned (ParcelInfoReply Flags).
    flags: u8,
}

const PARCELS: &[DemoParcel] = &[
    DemoParcel {
        grid: (1001, 1000),
        id: Uuid::from_u128(0xDE40_91AC_0000_0000_0000_0000_0000_0002),
        name: "Lagune des aurores",
        desc: "Ponton, feu de camp et vue sur les aurores boréales.\nMusique douce le soir, venez comme vous êtes !",
        area: 4096,
        dwell: 312.6,
        flags: 0x1,
    },
    DemoParcel {
        grid: (1000, 1001),
        id: Uuid::from_u128(0xDE40_91AC_0000_0000_0000_0000_0000_0003),
        name: "Halle de Nordheim",
        desc: "Grande halle viking : banquets, combats et contes au coin du feu.",
        area: 8192,
        dwell: 1288.0,
        flags: 0x2 | 0x4,
    },
    DemoParcel {
        grid: (999, 1000),
        id: Uuid::from_u128(0xDE40_91AC_0000_0000_0000_0000_0000_0004),
        name: "",
        desc: "",
        area: 512,
        dwell: 0.0,
        flags: 0,
    },
];

/// Grid position of Faille, the region whose parcel requests fail.
const FAILING: (u32, u32) = (1001, 1001);

/// Region id of a demo region (RegionHandleRequest), by grid position.
fn region_id(grid: (u32, u32)) -> Uuid {
    Uuid::from_u128(0xDE40_5E61_0000_0000_0000_0000_0000_0000 | ((grid.0 as u128) << 16) | grid.1 as u128)
}

/// AURORA_DEMO_PLACE=1|lagune|nordheim|pinede|faille|inconnue: the region
/// name and position of the place link the demo opens (repere and
/// historique are handled by `profile_scenario`).
pub fn scenario() -> Option<(&'static str, Vec3)> {
    let v = std::env::var("AURORA_DEMO_PLACE").ok()?;
    Some(match v.to_lowercase().as_str() {
        "lagune" => ("Lagune Boréale", Vec3::new(60.0, 200.0, 22.0)),
        "nordheim" => ("Nordheim", Vec3::new(128.0, 64.0, 40.0)),
        "pinede" => ("Pinède", Vec3::new(200.0, 30.0, 24.0)),
        "faille" => ("Faille", Vec3::new(128.0, 128.0, 30.0)),
        "inconnue" => ("Atlantide", Vec3::new(128.0, 128.0, 20.0)),
        "repere" | "historique" => return None,
        // Loup Violet's home (the link of his profile)
        _ => ("Aurora Démo", Vec3::new(140.0, 120.0, 25.0)),
    })
}

/// AURORA_DEMO_PLACE=repere|historique: a landmark's or a history entry's
/// profile.
pub fn profile_scenario() -> Option<crate::world::place_details::Source> {
    use crate::world::place_details::Source;
    match std::env::var("AURORA_DEMO_PLACE").ok()?.to_lowercase().as_str() {
        "repere" => {
            let (id, asset) = landmark_ids(1);
            Some(Source::Landmark { item: id, asset })
        }
        "historique" => history().into_iter().rev().nth(1).map(|h| Source::History {
            title: h.title,
            global: h.global,
        }),
        _ => None,
    }
}

/// RemoteParcelRequest: the parcel of the region holding the slot.
pub fn remote_parcel(handle: RegionHandle, position: Vec3) -> Vec<NetEvent> {
    let (x, y) = aurora_net::handle_to_origin(handle);
    let grid = (x / 256, y / 256);
    let (hx, hy) = aurora_net::handle_to_origin(super::HANDLE);
    let result = if grid == (hx / 256, hy / 256) {
        Ok(super::land::PARCEL_UUID)
    } else if grid == FAILING {
        Err(RemoteParcelError::Status(404))
    } else {
        PARCELS
            .iter()
            .find(|p| p.grid == grid)
            .map(|p| p.id)
            .ok_or(RemoteParcelError::NoParcel)
    };
    vec![NetEvent::RemoteParcel { handle, position, result }]
}

/// RegionHandleRequest for one of the demo regions.
pub fn region_handle(id: Uuid) -> Vec<NetEvent> {
    DEMO_SIMS
        .iter()
        .find(|s| region_id((s.0 as u32, s.1 as u32)) == id)
        .map(|s| NetEvent::RegionIdHandle {
            region_id: id,
            handle: aurora_net::origin_to_handle(s.0 as u32 * 256, s.1 as u32 * 256),
        })
        .into_iter()
        .collect()
}

/// ParcelInfoRequest for a parcel of `remote_parcel`, None for another id.
pub fn parcel_info(id: Uuid) -> Option<NetEvent> {
    let p = if id == super::land::PARCEL_UUID {
        // the parcel of "À propos du terrain", seen from afar
        let l = super::land::parcel();
        ParcelSummary {
            id,
            owner: l.owner_id,
            name: l.name,
            desc: l.desc,
            actual_area: l.area,
            billable_area: l.area,
            flags: 0,
            sim_name: "Aurora Démo".into(),
            global: DVec3::new(256_000.0 + 128.0, 256_000.0 + 128.0, 25.0),
            snapshot: TEX_SNAPSHOT,
            dwell: 145.0,
            sale_price: 0,
            auction_id: 0,
        }
    } else {
        let d = PARCELS.iter().find(|p| p.id == id)?;
        let name = DEMO_SIMS.iter().find(|s| (s.0 as u32, s.1 as u32) == d.grid).map_or("", |s| s.2);
        ParcelSummary {
            id,
            // a group-owned parcel names its group
            owner: if d.flags & 0x4 != 0 { super::DEMO_GROUP2 } else { DEMO_LOUP },
            name: d.name.into(),
            desc: d.desc.into(),
            actual_area: d.area,
            billable_area: d.area,
            flags: d.flags,
            sim_name: name.into(),
            global: DVec3::new(d.grid.0 as f64 * 256.0 + 128.0, d.grid.1 as f64 * 256.0 + 128.0, 30.0),
            snapshot: if d.name.is_empty() { Uuid::nil() } else { TEX_SNAPSHOT },
            dwell: d.dwell,
            sale_price: 0,
            auction_id: 0,
        }
    };
    Some(NetEvent::ParcelInfo(Box::new(p)))
}

/// The demo landmarks: (folder, name, region grid, position, created at).
const LANDMARKS: &[(u128, &str, (u32, u32), [f32; 3], i64)] = &[
    (
        LANDMARKS_FOLDER,
        "Plage d'Aurora",
        (1000, 1000),
        [140.0, 120.0, 25.0],
        1_712_068_948,
    ),
    (
        LANDMARKS_FOLDER,
        "Lagune des aurores",
        (1001, 1000),
        [60.0, 200.0, 22.0],
        1_740_000_000,
    ),
    (
        LANDMARKS_FOLDER,
        "Halle de Nordheim",
        (1000, 1001),
        [128.0, 64.0, 40.0],
        1_759_900_000,
    ),
    (LANDMARKS_FOLDER, "Pinède", (999, 1000), [200.0, 30.0, 24.0], 1_700_000_000),
    (SHOPS_FOLDER, "Atelier du Loup", (1000, 1000), [60.0, 200.0, 22.0], 1_750_000_000),
    (
        SHOPS_FOLDER,
        "Marché de la Place",
        (1000, 1000),
        [128.0, 128.0, 25.0],
        1_745_000_000,
    ),
    (
        FAVORITES_FOLDER,
        "Place d'Aurora",
        (1000, 1000),
        [128.0, 128.0, 25.0],
        1_712_068_948,
    ),
    (
        FAVORITES_FOLDER,
        "Lagune des aurores",
        (1001, 1000),
        [60.0, 200.0, 22.0],
        1_740_000_000,
    ),
];

/// Item and asset ids of the n-th demo landmark.
fn landmark_ids(n: usize) -> (Uuid, Uuid) {
    (u(0x1A_0000 + n as u128), u(0x1B_0000 + n as u128))
}

/// The landmark asset text (LLLandmark version 2) of a demo landmark.
pub fn landmark_asset(asset: Uuid) -> Option<String> {
    let (_, _, grid, pos, _) = LANDMARKS.iter().enumerate().find(|(n, _)| landmark_ids(*n).1 == asset)?.1;
    Some(format!(
        "Landmark version 2\nregion_id {}\nlocal_pos {} {} {}\n",
        region_id(*grid),
        pos[0],
        pos[1],
        pos[2]
    ))
}

/// FetchInventoryDescendents2 answer for the landmark folders.
pub fn folder_contents(folder_id: Uuid, owner: Uuid) -> Option<FolderContents> {
    let n = [LANDMARKS_FOLDER, FAVORITES_FOLDER, SHOPS_FOLDER]
        .into_iter()
        .find(|n| u(*n) == folder_id)?;
    let folders = if n == LANDMARKS_FOLDER {
        vec![InvFolder {
            id: u(SHOPS_FOLDER),
            parent: folder_id,
            name: "Boutiques".into(),
            type_default: -1,
            version: 2,
            ..Default::default()
        }]
    } else {
        Vec::new()
    };
    let items = LANDMARKS
        .iter()
        .enumerate()
        .filter(|(_, l)| l.0 == n)
        .map(|(i, (_, name, _, _, created))| {
            let (id, asset) = landmark_ids(i);
            InvItem {
                id,
                parent: folder_id,
                name: (*name).into(),
                desc: if i == 0 {
                    "Le coucher de soleil depuis le ponton".into()
                } else {
                    String::new()
                },
                asset_type: 3,
                inv_type: 3,
                asset_id: asset,
                flags: 0,
                favorite: false,
                creator: DEMO_LOUP,
                created_at: *created,
                owner,
                group_mask: 0,
                everyone_mask: 0,
                next_owner_mask: 0,
            }
        })
        .collect();
    Some(FolderContents {
        folder_id,
        owner_id: owner,
        version: 2,
        folders,
        items,
    })
}

/// The demo's teleport history, oldest first, around now.
pub fn history() -> Vec<HistoryItem> {
    const DAY: f64 = 86_400.0;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    let at = |grid: (u32, u32), x: f64, y: f64, z: f64| DVec3::new(grid.0 as f64 * 256.0 + x, grid.1 as f64 * 256.0 + y, z);
    let item = |title: &str, global: DVec3, ago: f64| HistoryItem {
        title: title.into(),
        global,
        date: now - ago,
        slurl: String::new(),
    };
    vec![
        item("Ahern", DVec3::new(254_208.0 + 128.0, 256_256.0 + 128.0, 30.0), 240.0 * DAY),
        item("Sablière", at((999, 999), 128.0, 128.0, 25.0), 40.0 * DAY),
        item("Atelier du Loup, Aurora Démo", at((1000, 1000), 60.0, 200.0, 22.0), 12.0 * DAY),
        item("Pinède", at((999, 1000), 200.0, 30.0, 24.0), 3.2 * DAY),
        item("Halle de Nordheim, Nordheim", at((1000, 1001), 128.0, 64.0, 40.0), 1.1 * DAY),
        item(
            "Lagune des aurores, Lagune Boréale",
            at((1001, 1000), 60.0, 200.0, 22.0),
            3.0 * 3600.0,
        ),
    ]
}
