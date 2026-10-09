//! Place details of the demo (`AURORA_DEMO_PLACE`): the RemoteParcelRequest
//! and ParcelInfoRequest answers for the regions of the demo map, so the
//! "Détails de l'emplacement" window of a place link can be tried offline.
//! Aurora Démo answers with the parcel of "À propos du terrain"; Faille
//! answers HTTP 404 (LLPanelPlaceInfo::setErrorStatus).

use super::{DEMO_LOUP, TEX_SNAPSHOT};
use aurora_net::land::RemoteParcelError;
use aurora_net::{NetEvent, ParcelSummary, RegionHandle};
use glam::{DVec3, Vec3};
use uuid::Uuid;

/// One demo parcel per region (grid x, y in region units, as DEMO_SIMS).
struct DemoParcel {
    grid: (u32, u32),
    id: Uuid,
    name: &'static str,
    desc: &'static str,
    area: i32,
    dwell: f32,
    /// 0x1 mature, 0x2 adult (ParcelInfoReply Flags).
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
        flags: 0x2,
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

/// AURORA_DEMO_PLACE=1|lagune|nordheim|pinede|faille|inconnue: the region
/// name and position of the place link the demo opens.
pub fn scenario() -> Option<(&'static str, Vec3)> {
    let v = std::env::var("AURORA_DEMO_PLACE").ok()?;
    Some(match v.to_lowercase().as_str() {
        "lagune" => ("Lagune Boréale", Vec3::new(60.0, 200.0, 22.0)),
        "nordheim" => ("Nordheim", Vec3::new(128.0, 64.0, 40.0)),
        "pinede" => ("Pinède", Vec3::new(200.0, 30.0, 24.0)),
        "faille" => ("Faille", Vec3::new(128.0, 128.0, 30.0)),
        "inconnue" => ("Atlantide", Vec3::new(128.0, 128.0, 20.0)),
        // Loup Violet's home (the link of his profile)
        _ => ("Aurora Démo", Vec3::new(140.0, 120.0, 25.0)),
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
        PARCELS
            .iter()
            .find(|p| p.grid == grid)
            .map(|p| p.id)
            .ok_or(RemoteParcelError::NoParcel)
    };
    vec![NetEvent::RemoteParcel { handle, position, result }]
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
        let name = super::DEMO_SIMS
            .iter()
            .find(|s| (s.0 as u32, s.1 as u32) == d.grid)
            .map_or("", |s| s.2);
        ParcelSummary {
            id,
            owner: DEMO_LOUP,
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
