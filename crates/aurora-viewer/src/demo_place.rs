//! Places of the demo (`AURORA_DEMO_PLACE`, `AURORA_DEMO_PLACES`): the
//! RemoteParcelRequest, ParcelInfoRequest and RegionHandleRequest answers
//! for the regions of the demo map, the landmarks of the Landmarks and
//! Favorites folders with their assets, and a teleport history over several
//! days, so the « Lieux » window can be tried offline. Aurora Démo answers
//! with the parcel of "À propos du terrain"; Faille answers HTTP 404
//! (LLPanelPlaceInfo::setErrorStatus).

use super::{DEMO_LOUP, TEX_SNAPSHOT, demo_sims, u};
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

fn parcels() -> &'static Vec<DemoParcel> {
    static TABLE: aurora_i18n::Localized<Vec<DemoParcel>> = aurora_i18n::Localized::new();
    TABLE.get(|| {
        vec![
            DemoParcel {
                grid: (1001, 1000),
                id: Uuid::from_u128(0xDE40_91AC_0000_0000_0000_0000_0000_0002),
                name: aurora_i18n::tr!("demo-place-aurora-lagoon"),
                desc: aurora_i18n::tr!("demo-place-pier-campfire-and-a-view-of-the-northern-lights"),
                area: 4096,
                dwell: 312.6,
                flags: 0x1,
            },
            DemoParcel {
                grid: (1000, 1001),
                id: Uuid::from_u128(0xDE40_91AC_0000_0000_0000_0000_0000_0003),
                name: aurora_i18n::tr!("demo-place-nordheim-hall"),
                desc: aurora_i18n::tr!("demo-place-great-viking-hall-feasts-combat-and-fireside-tales"),
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
        ]
    })
}

/// The named demo parcels as a directory lists them: (parcel, name,
/// traffic, area), the parcel of "À propos du terrain" first.
pub fn directory() -> Vec<(Uuid, String, f32, i32)> {
    let l = super::land::parcel();
    std::iter::once((super::land::PARCEL_UUID, l.name, 145.0, l.area))
        .chain(
            parcels()
                .iter()
                .filter(|p| !p.name.is_empty())
                .map(|p| (p.id, p.name.to_owned(), p.dwell, p.area)),
        )
        .collect()
}

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
        "lagune" => (aurora_i18n::tr!("common-boreal-lagoon"), Vec3::new(60.0, 200.0, 22.0)),
        "nordheim" => ("Nordheim", Vec3::new(128.0, 64.0, 40.0)),
        "pinede" => (aurora_i18n::tr!("common-pine-grove"), Vec3::new(200.0, 30.0, 24.0)),
        "faille" => (aurora_i18n::tr!("common-flaw"), Vec3::new(128.0, 128.0, 30.0)),
        "inconnue" => (aurora_i18n::tr!("demo-place-atlantis"), Vec3::new(128.0, 128.0, 20.0)),
        "repere" | "historique" => return None,
        // Loup Violet's home (the link of his profile)
        _ => (aurora_i18n::tr!("common-aurora-demo"), Vec3::new(140.0, 120.0, 25.0)),
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

/// A teleport confirmation the demo puts on screen.
pub enum TeleportConfirm {
    /// A place link (TeleportViaSLAPP): region name and position.
    Link(&'static str, Vec3),
    /// A landmark (TeleportFromLandmark): asset and name.
    Landmark(Uuid, &'static str),
}

/// AURORA_DEMO_TP_CONFIRM=lien|repere: the confirmation of a teleport asked
/// by a place link (the default) or by a landmark of « Lieux ».
pub fn teleport_confirm_scenario() -> Option<TeleportConfirm> {
    let v = std::env::var("AURORA_DEMO_TP_CONFIRM").ok()?;
    Some(match v.to_lowercase().as_str() {
        "repere" => {
            let (_, name, ..) = landmarks().get(1)?;
            TeleportConfirm::Landmark(landmark_ids(1).1, name)
        }
        _ => TeleportConfirm::Link(aurora_i18n::tr!("common-boreal-lagoon"), Vec3::new(60.0, 200.0, 22.0)),
    })
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
        parcels()
            .iter()
            .find(|p| p.grid == grid)
            .map(|p| p.id)
            .ok_or(RemoteParcelError::NoParcel)
    };
    vec![NetEvent::RemoteParcel { handle, position, result }]
}

/// RegionHandleRequest for one of the demo regions.
pub fn region_handle(id: Uuid) -> Vec<NetEvent> {
    demo_sims()
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
            sim_name: aurora_i18n::tr!("common-aurora-demo").into(),
            global: DVec3::new(256_000.0 + 128.0, 256_000.0 + 128.0, 25.0),
            snapshot: TEX_SNAPSHOT,
            dwell: 145.0,
            sale_price: 0,
            auction_id: 0,
        }
    } else {
        let d = parcels().iter().find(|p| p.id == id)?;
        let name = demo_sims().iter().find(|s| (s.0 as u32, s.1 as u32) == d.grid).map_or("", |s| s.2);
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
fn landmarks() -> &'static Vec<(u128, &'static str, (u32, u32), [f32; 3], i64)> {
    static TABLE: aurora_i18n::Localized<Vec<(u128, &'static str, (u32, u32), [f32; 3], i64)>> = aurora_i18n::Localized::new();
    TABLE.get(|| {
        vec![
            (
                LANDMARKS_FOLDER,
                aurora_i18n::tr!("demo-place-aurora-beach"),
                (1000, 1000),
                [140.0, 120.0, 25.0],
                1_712_068_948,
            ),
            (
                LANDMARKS_FOLDER,
                aurora_i18n::tr!("demo-place-aurora-lagoon"),
                (1001, 1000),
                [60.0, 200.0, 22.0],
                1_740_000_000,
            ),
            (
                LANDMARKS_FOLDER,
                aurora_i18n::tr!("demo-place-nordheim-hall"),
                (1000, 1001),
                [128.0, 64.0, 40.0],
                1_759_900_000,
            ),
            (
                LANDMARKS_FOLDER,
                aurora_i18n::tr!("common-pine-grove"),
                (999, 1000),
                [200.0, 30.0, 24.0],
                1_700_000_000,
            ),
            (
                SHOPS_FOLDER,
                aurora_i18n::tr!("demo-place-wolf-s-workshop"),
                (1000, 1000),
                [60.0, 200.0, 22.0],
                1_750_000_000,
            ),
            (
                SHOPS_FOLDER,
                aurora_i18n::tr!("common-plaza-market"),
                (1000, 1000),
                [128.0, 128.0, 25.0],
                1_745_000_000,
            ),
            (
                FAVORITES_FOLDER,
                aurora_i18n::tr!("common-aurora-plaza"),
                (1000, 1000),
                [128.0, 128.0, 25.0],
                1_712_068_948,
            ),
            (
                FAVORITES_FOLDER,
                aurora_i18n::tr!("demo-place-aurora-lagoon"),
                (1001, 1000),
                [60.0, 200.0, 22.0],
                1_740_000_000,
            ),
        ]
    })
}

/// Item and asset ids of the n-th demo landmark.
pub(crate) fn landmark_ids(n: usize) -> (Uuid, Uuid) {
    (u(0x1A_0000 + n as u128), u(0x1B_0000 + n as u128))
}

/// The landmark asset text (LLLandmark version 2) of a demo landmark.
pub fn landmark_asset(asset: Uuid) -> Option<String> {
    let (_, _, grid, pos, _) = landmarks().iter().enumerate().find(|(n, _)| landmark_ids(*n).1 == asset)?.1;
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
            name: aurora_i18n::tr!("common-shopping").into(),
            type_default: -1,
            version: 2,
            ..Default::default()
        }]
    } else {
        Vec::new()
    };
    let items = landmarks()
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
                    aurora_i18n::tr!("demo-place-sunset-from-the-pier").into()
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
                base_mask: 0x7fffffff,
                owner_mask: 0x7fffffff,
                ..Default::default()
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
        item(aurora_i18n::tr!("common-sandbox"), at((999, 999), 128.0, 128.0, 25.0), 40.0 * DAY),
        item(
            aurora_i18n::tr!("demo-place-wolf-s-workshop-aurora-demo"),
            at((1000, 1000), 60.0, 200.0, 22.0),
            12.0 * DAY,
        ),
        item(aurora_i18n::tr!("common-pine-grove"), at((999, 1000), 200.0, 30.0, 24.0), 3.2 * DAY),
        item(
            aurora_i18n::tr!("demo-place-nordheim-hall-nordheim"),
            at((1000, 1001), 128.0, 64.0, 40.0),
            1.1 * DAY,
        ),
        item(
            aurora_i18n::tr!("demo-place-aurora-lagoon-boreal-lagoon"),
            at((1001, 1000), 60.0, 200.0, 22.0),
            3.0 * 3600.0,
        ),
    ]
}
