//! Places of the demo (`AURORA_DEMO_PLACE`, `AURORA_DEMO_PLACES`): the
//! RemoteParcelRequest, ParcelInfoRequest and RegionHandleRequest answers
//! for the regions of the demo map, the landmarks of the Landmarks and
//! Favorites folders with their assets, and a teleport history over several
//! days, so the "Places" window can be tried offline. Aurora Demo answers
//! with the parcel of "About Land"; Flaw answers HTTP 404
//! (LLPanelPlaceInfo::setErrorStatus).

use super::{DEMO_LOUP, TEX_SNAPSHOT, demo_sims, u};
use crate::world::tphistory::{HistoryItem, TpEntry};
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
/// Two more of `AURORA_DEMO_MAP=landmarks`.
pub const CLUBS_FOLDER: u128 = 32;
pub const BEACHES_FOLDER: u128 = 33;
pub const WALKS_FOLDER: u128 = 34;
pub const MARKETS_FOLDER: u128 = 35;
/// Landmarks `AURORA_DEMO_MAP=landmarks` adds to the table's: 212 in all,
/// the count of a well-travelled inventory, for the world map's drawer and
/// pins.
const GENERATED: usize = 204;

/// AURORA_DEMO_MAP=…,landmarks.
pub fn many_landmarks() -> bool {
    std::env::var("AURORA_DEMO_MAP").is_ok_and(|v| v.split(',').any(|w| w.trim() == "landmarks"))
}

/// The n-th landmark `many_landmarks` adds: (folder, name, region grid,
/// position), spread over the regions of the demo map and four folders.
fn generated(n: usize) -> (u128, String, (u32, u32), [f32; 3]) {
    let (folder, name) = match n % 4 {
        0 => (CLUBS_FOLDER, aurora_i18n::tr!("demo-map-club", n = (n / 4 + 1) as i64)),
        1 => (BEACHES_FOLDER, aurora_i18n::tr!("demo-map-beach", n = (n / 4 + 1) as i64)),
        2 => (MARKETS_FOLDER, aurora_i18n::tr!("demo-map-shop", n = (n / 4 + 1) as i64)),
        _ => (WALKS_FOLDER, aurora_i18n::tr!("demo-map-spot", n = (n / 4 + 1) as i64)),
    };
    // scattered like real ones: a hash of n, not a pattern
    let mut h = (n as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    let sims = demo_sims();
    let s = &sims[(h >> 40) as usize % sims.len()];
    let pos = [((h >> 8) % 232 + 12) as f32, ((h >> 20) % 232 + 12) as f32, 25.0];
    (folder, name.to_string(), (s.0 as u32, s.1 as u32), pos)
}

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
/// traffic, area), the parcel of "About Land" first.
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

/// Grid position of Flaw, the region whose parcel requests fail.
const FAILING: (u32, u32) = (1001, 1001);

/// Region id of a demo region (RegionHandleRequest), by grid position.
fn region_id(grid: (u32, u32)) -> Uuid {
    Uuid::from_u128(0xDE40_5E61_0000_0000_0000_0000_0000_0000 | ((grid.0 as u128) << 16) | grid.1 as u128)
}

/// AURORA_DEMO_PLACE=1|lagoon|nordheim|grove|flaw|unknown: the region
/// name and position of the place link the demo opens (landmark and
/// history are handled by `profile_scenario`).
pub fn scenario() -> Option<(&'static str, Vec3)> {
    let v = std::env::var("AURORA_DEMO_PLACE").ok()?;
    Some(match v.to_lowercase().as_str() {
        "lagoon" | "lagune" => (aurora_i18n::tr!("common-boreal-lagoon"), Vec3::new(60.0, 200.0, 22.0)),
        "nordheim" => ("Nordheim", Vec3::new(128.0, 64.0, 40.0)),
        "grove" | "pinede" => (aurora_i18n::tr!("common-pine-grove"), Vec3::new(200.0, 30.0, 24.0)),
        "flaw" | "faille" => (aurora_i18n::tr!("common-flaw"), Vec3::new(128.0, 128.0, 30.0)),
        "unknown" | "inconnue" => (aurora_i18n::tr!("demo-place-atlantis"), Vec3::new(128.0, 128.0, 20.0)),
        "landmark" | "repere" | "history" | "historique" => return None,
        // Violet Wolf's home (the link of his profile)
        _ => (aurora_i18n::tr!("common-aurora-demo"), Vec3::new(140.0, 120.0, 25.0)),
    })
}

/// AURORA_DEMO_PLACE=repere|historique: a landmark's or a history entry's
/// profile.
pub fn profile_scenario() -> Option<crate::world::place_details::Source> {
    use crate::world::place_details::Source;
    match std::env::var("AURORA_DEMO_PLACE").ok()?.to_lowercase().as_str() {
        "landmark" | "repere" => {
            let (id, asset) = landmark_ids(1);
            Some(Source::Landmark { item: id, asset })
        }
        "history" | "historique" => history().into_iter().rev().nth(1).map(|h| Source::History {
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

/// AURORA_DEMO_TP_CONFIRM=link|landmark: the confirmation of a teleport asked
/// by a place link (the default) or by a landmark of "Places".
pub fn teleport_confirm_scenario() -> Option<TeleportConfirm> {
    let v = std::env::var("AURORA_DEMO_TP_CONFIRM").ok()?;
    Some(match v.to_lowercase().as_str() {
        "landmark" | "repere" => {
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
        // the parcel of "About Land", seen from afar
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

/// Where the bars' demo favourites start in `landmark_ids`.
const BAR_FAVORITES: usize = 2000;

/// The favourites of the bars mockup (`AURORA_DEMO_BARS=fav12|added`):
/// name, region grid, position, created at (newest first: the mockup's
/// order), and the three colours of the thumbnail's gradient. The last one
/// is the place we stand on, added from the star with `added`.
#[allow(clippy::type_complexity)]
fn bar_favorites() -> &'static Vec<(&'static str, (u32, u32), [f32; 3], i64, [[u8; 3]; 3])> {
    static TABLE: aurora_i18n::Localized<Vec<(&'static str, (u32, u32), [f32; 3], i64, [[u8; 3]; 3])>> = aurora_i18n::Localized::new();
    TABLE.get(|| {
        let g = (1000, 1000);
        let m = (1001, 1000);
        let pine = (999, 1000);
        let sand = (999, 999);
        let adult = (1000, 1001);
        vec![
            (
                aurora_i18n::tr!("demo-fav-fireflies-beach"),
                g,
                [60.0, 40.0, 22.0],
                1_760_000_000,
                [[0xff, 0xcf, 0x8a], [0xe0, 0x7a, 0x9a], [0x2b, 0x3a, 0x7a]],
            ),
            (
                aurora_i18n::tr!("demo-fav-nova-market"),
                m,
                [128.0, 90.0, 22.0],
                1_759_000_000,
                [[0xf9, 0xd3, 0x6b], [0xc2, 0x60, 0x3f], [0x3a, 0x23, 0x48]],
            ),
            (
                aurora_i18n::tr!("demo-fav-mist-garden"),
                pine,
                [80.0, 160.0, 24.0],
                1_758_000_000,
                [[0xb8, 0xf0, 0xd8], [0x4f, 0xa3, 0x9a], [0x1f, 0x3b, 0x4a]],
            ),
            (
                aurora_i18n::tr!("demo-fav-neon-club"),
                m,
                [200.0, 60.0, 22.0],
                1_757_000_000,
                [[0xff, 0x7a, 0xd9], [0x7a, 0x3c, 0xff], [0x14, 0x0b, 0x33]],
            ),
            (
                aurora_i18n::tr!("demo-fav-north-lighthouse"),
                pine,
                [30.0, 220.0, 24.0],
                1_756_000_000,
                [[0xd6, 0xe8, 0xff], [0x6b, 0x8f, 0xc9], [0x1a, 0x25, 0x47]],
            ),
            (
                aurora_i18n::tr!("demo-fav-alexandria-library"),
                g,
                [200.0, 200.0, 25.0],
                1_755_000_000,
                [[0xf2, 0xe2, 0xc0], [0xb0, 0x8a, 0x55], [0x3b, 0x2c, 0x1c]],
            ),
            (
                aurora_i18n::tr!("demo-fav-sandbox-workshop"),
                sand,
                [128.0, 128.0, 22.0],
                1_754_000_000,
                [[0xff, 0xe0, 0x8a], [0x9b, 0xc9, 0x6b], [0x2f, 0x4a, 0x2a]],
            ),
            (
                aurora_i18n::tr!("demo-fav-light-cafe"),
                g,
                [90.0, 120.0, 25.0],
                1_753_000_000,
                [[0xff, 0xd1, 0xa8], [0xb5, 0x73, 0x4e], [0x2d, 0x1d, 0x1a]],
            ),
            (
                aurora_i18n::tr!("demo-fav-cats-island"),
                sand,
                [40.0, 40.0, 22.0],
                1_752_000_000,
                [[0xa8, 0xe6, 0xff], [0x3f, 0xa0, 0xc9], [0x10, 0x3a, 0x52]],
            ),
            (
                aurora_i18n::tr!("demo-fav-prism-gallery"),
                m,
                [60.0, 180.0, 22.0],
                1_751_000_000,
                [[0xe4, 0xd4, 0xff], [0x8a, 0x6b, 0xd6], [0x24, 0x1a, 0x46]],
            ),
            (
                aurora_i18n::tr!("demo-fav-shadow-forest"),
                adult,
                [128.0, 128.0, 30.0],
                1_750_000_000,
                [[0x9f, 0xd6, 0x8a], [0x2f, 0x6b, 0x3f], [0x0e, 0x1f, 0x14]],
            ),
            (
                aurora_i18n::tr!("demo-fav-orbital-station"),
                m,
                [240.0, 240.0, 40.0],
                1_749_000_000,
                [[0xc9, 0xd6, 0xff], [0x4d, 0x5f, 0xb0], [0x09, 0x0c, 0x24]],
            ),
            (
                aurora_i18n::tr!("demo-bars-parcel"),
                g,
                [132.0, 126.0, 33.0],
                1_761_000_000,
                [[0xe3, 0xa5, 0x81], [0x4f, 0x55, 0xa3], [0x1b, 0x21, 0x47]],
            ),
        ]
    })
}

/// Whether the demo shows the k-th of the bars' favourites: the twelve, or
/// eleven and this place with `added` (the mockup's twelve).
fn bar_favorite_shown(k: usize) -> bool {
    match crate::ui::bars::demo::get().favorites {
        Some(crate::ui::bars::demo::Favorites::Twelve | crate::ui::bars::demo::Favorites::Snapshots) => k < 12,
        Some(crate::ui::bars::demo::Favorites::Added) => k < 11 || k == 12,
        _ => false,
    }
}

/// The favourite `added` marks as just added from the star.
pub fn added_favorite() -> Option<Uuid> {
    (crate::ui::bars::demo::get().favorites == Some(crate::ui::bars::demo::Favorites::Added)).then(|| landmark_ids(BAR_FAVORITES + 12).0)
}

/// The thumbnails of the bars' favourites: (texture id, gradient colours).
pub fn favorite_thumbnails() -> Vec<(Uuid, [[u8; 3]; 3])> {
    bar_favorites().iter().enumerate().map(|(n, f)| (thumbnail_id(n), f.4)).collect()
}

/// The item of the k-th of the bars' favourites.
pub fn bar_favorite_item(k: usize) -> Uuid {
    landmark_ids(BAR_FAVORITES + k).0
}

fn thumbnail_id(n: usize) -> Uuid {
    u(0x1C_0000 + n as u128)
}

/// Item and asset ids of the n-th demo landmark.
pub(crate) fn landmark_ids(n: usize) -> (Uuid, Uuid) {
    (u(0x1A_0000 + n as u128), u(0x1B_0000 + n as u128))
}

/// The landmark asset text (LLLandmark version 2) of a demo landmark.
pub fn landmark_asset(asset: Uuid) -> Option<String> {
    let table = landmarks()
        .iter()
        .enumerate()
        .find(|(n, _)| landmark_ids(*n).1 == asset)
        .map(|(_, l)| (l.2, l.3))
        .or_else(|| {
            bar_favorites()
                .iter()
                .enumerate()
                .find(|(n, _)| landmark_ids(BAR_FAVORITES + n).1 == asset)
                .map(|(_, f)| (f.1, f.2))
        });
    let (grid, pos) = match table {
        Some((grid, pos)) => (grid, pos),
        None => {
            let n = (0..GENERATED).find(|n| landmark_ids(landmarks().len() + n).1 == asset)?;
            let (_, _, grid, pos) = generated(n);
            (grid, pos)
        }
    };
    let (grid, pos) = (&grid, &pos);
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
    let n = [
        LANDMARKS_FOLDER,
        FAVORITES_FOLDER,
        SHOPS_FOLDER,
        CLUBS_FOLDER,
        BEACHES_FOLDER,
        WALKS_FOLDER,
        MARKETS_FOLDER,
    ]
    .into_iter()
    .find(|n| u(*n) == folder_id)?;
    let sub = |id: u128, name: &str| InvFolder {
        id: u(id),
        parent: folder_id,
        name: name.into(),
        type_default: -1,
        version: 2,
        ..Default::default()
    };
    let folders = if n == LANDMARKS_FOLDER {
        let mut v = vec![sub(SHOPS_FOLDER, aurora_i18n::tr!("common-shopping"))];
        if many_landmarks() {
            v.push(sub(CLUBS_FOLDER, aurora_i18n::tr!("demo-map-clubs")));
            v.push(sub(BEACHES_FOLDER, aurora_i18n::tr!("demo-map-beaches")));
            v.push(sub(WALKS_FOLDER, aurora_i18n::tr!("demo-map-walks")));
            v.push(sub(MARKETS_FOLDER, aurora_i18n::tr!("demo-map-markets")));
        }
        v
    } else {
        Vec::new()
    };
    let item = |i: usize, name: &str, created: i64| {
        let (id, asset) = landmark_ids(i);
        InvItem {
            id,
            parent: folder_id,
            name: name.into(),
            desc: String::new(),
            asset_type: 3,
            inv_type: 3,
            asset_id: asset,
            creator: DEMO_LOUP,
            created_at: created,
            owner,
            base_mask: 0x7fffffff,
            owner_mask: 0x7fffffff,
            ..Default::default()
        }
    };
    let extra: Vec<InvItem> = if many_landmarks() {
        (0..GENERATED)
            .map(|g| (g, generated(g)))
            .filter(|(_, l)| l.0 == n)
            .map(|(g, l)| item(landmarks().len() + g, &l.1, 1_700_000_000 + g as i64 * 86_400))
            .collect()
    } else {
        Vec::new()
    };
    let items = landmarks()
        .iter()
        .enumerate()
        .filter(|(_, l)| l.0 == n)
        .map(|(i, (_, name, _, _, created))| {
            let (id, own) = landmark_ids(i);
            // a favourite of a landmark is its copy: the same asset
            let asset = if n == FAVORITES_FOLDER {
                landmarks()
                    .iter()
                    .position(|l| l.0 != FAVORITES_FOLDER && l.1 == *name)
                    .map_or(own, |j| landmark_ids(j).1)
            } else {
                own
            };
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
        .chain(extra)
        .collect();
    // AURORA_DEMO_BARS's favourites take the place of the demo's own
    let bars = crate::ui::bars::demo::get().favorites;
    let snapshots = bars == Some(crate::ui::bars::demo::Favorites::Snapshots);
    let items = if n == FAVORITES_FOLDER && bars.is_some() {
        bar_favorites()
            .iter()
            .enumerate()
            .filter(|(k, _)| bar_favorite_shown(*k))
            .map(|(k, f)| InvItem {
                // favsnap: the parcels' snapshots instead
                thumbnail: if snapshots { Uuid::nil() } else { thumbnail_id(k) },
                ..item(BAR_FAVORITES + k, f.0, f.3)
            })
            .collect()
    } else {
        items
    };
    Some(FolderContents {
        folder_id,
        owner_id: owner,
        version: 2,
        folders,
        items,
    })
}

/// The places of the session before the demo's arrival, oldest first, the
/// last one twenty minutes ago (`AURORA_DEMO_BARS=tplist`).
pub fn session() -> Vec<TpEntry> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let entry = |grid: (u32, u32), region: &str, parcel: &str, position: Vec3, minutes_ago: i64| TpEntry {
        region: region.to_owned(),
        handle: aurora_net::origin_to_handle(grid.0 * 256, grid.1 * 256),
        position,
        parcel: parcel.to_owned(),
        at: now - minutes_ago * 60,
    };
    vec![
        entry(
            (999, 999),
            aurora_i18n::tr!("common-sandbox"),
            "",
            Vec3::new(128.0, 128.0, 25.0),
            95,
        ),
        entry(
            (1000, 1001),
            "Nordheim",
            aurora_i18n::tr!("demo-place-nordheim-hall"),
            Vec3::new(128.0, 64.0, 40.0),
            71,
        ),
        entry(
            (999, 1000),
            aurora_i18n::tr!("common-pine-grove"),
            "",
            Vec3::new(200.0, 30.0, 24.0),
            48,
        ),
        entry(
            (1001, 1000),
            aurora_i18n::tr!("common-boreal-lagoon"),
            aurora_i18n::tr!("demo-place-aurora-lagoon"),
            Vec3::new(60.0, 200.0, 22.0),
            20,
        ),
    ]
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
        // the same parcel as the lagoon's landmark, on a deck above it: a
        // place of its own in the history
        item(
            aurora_i18n::tr!("demo-place-aurora-lagoon-boreal-lagoon"),
            at((1001, 1000), 64.0, 196.0, 60.0),
            2.0 * DAY,
        ),
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
