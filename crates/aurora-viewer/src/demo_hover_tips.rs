//! The world's hover tips in the offline demo
//! (`AURORA_DEMO_HOVER_TIPS=<bubble|card|readout|legacy>[:all][:noland]`):
//! the tips on in that form for the session, without touching the saved
//! settings, on land too unless `noland`, on every object with `all`. The
//! simulator answers the hovered parcel (two parcels: the plaza, the
//! 128 m square in the middle, and a garden for sale around it) and the properties
//! of the demo's objects. Rest the cursor somewhere with
//! `AURORA_DEMO_HOVER="x,y"`; `AURORA_DEMO_ACTIONS=1` brings the cubes
//! that can be bought, paid and touched.

use super::{DEMO_LOUP, DEMO_NOVA};
use crate::settings::{HoverTipForm, HoverTips};
use aurora_net::{ParcelInfo, parcel_flags as pf};
use glam::Vec3;
use std::sync::{Arc, OnceLock};
use uuid::Uuid;

/// The tips of the scenario, and whether they take the Legacy text.
pub fn scenario() -> Option<(HoverTips, bool)> {
    static ON: OnceLock<Option<(HoverTips, bool)>> = OnceLock::new();
    ON.get_or_init(|| {
        let v = std::env::var("AURORA_DEMO_HOVER_TIPS").ok()?;
        let mut parts = v.split(':').map(|s| s.trim().to_lowercase());
        let form = parts.next().unwrap_or_default();
        let rest: Vec<String> = parts.collect();
        let tips = HoverTips {
            enabled: true,
            form: match form.as_str() {
                "bubble" => HoverTipForm::Bubble,
                "readout" => HoverTipForm::Readout,
                _ => HoverTipForm::Card,
            },
            all_objects: rest.iter().any(|p| p == "all"),
            land: !rest.iter().any(|p| p == "noland"),
            ..HoverTips::default()
        };
        Some((tips, form == "legacy"))
    })
    .clone()
}

/// The garden around the plaza: the region but the plaza's square.
const GARDEN: i32 = 2;

/// The plaza's cells (x and y 64..192 m, the demo's agent parcel).
fn in_plaza(x: usize, y: usize) -> bool {
    (16..48).contains(&x) && (16..48).contains(&y)
}

/// ParcelProperties of the parcel holding the cell at (west, south): the
/// plaza (`plaza`, the agent's parcel of the demo) or the garden around
/// it, on sale by Loup, where flying is forbidden, only the group's
/// scripts run and damage is on.
pub fn parcel_at(west: f32, south: f32, plaza: Option<Arc<ParcelInfo>>) -> Option<Arc<ParcelInfo>> {
    if in_plaza((west / 4.0) as usize, (south / 4.0) as usize) {
        return plaza;
    }
    let mut bitmap = vec![0u8; 64 * 64 / 8];
    for y in 0..64usize {
        for x in 0..64usize {
            if !in_plaza(x, y) {
                let i = y * 64 + x;
                bitmap[i / 8] |= 1 << (i % 8);
            }
        }
    }
    Some(Arc::new(ParcelInfo {
        local_id: GARDEN,
        sequence_id: aurora_net::land::HOVERED_PARCEL_SEQ_ID,
        request_result: aurora_net::land::PARCEL_RESULT_SUCCESS,
        name: aurora_i18n::tr!("demo-hover-tips-garden").into(),
        desc: aurora_i18n::tr!("demo-hover-tips-garden-desc").into(),
        owner_id: DEMO_LOUP,
        area: 49152,
        aabb_min: Vec3::ZERO,
        aabb_max: Vec3::new(256.0, 256.0, 0.0),
        bitmap,
        flags: pf::CREATE_OBJECTS | pf::ALLOW_TERRAFORM | pf::ALLOW_DAMAGE | pf::ALLOW_GROUP_SCRIPTS | pf::FOR_SALE,
        sale_price: 12000,
        ..Default::default()
    }))
}

/// ObjectPropertiesFamily of a demo object the scenario hovers (the cubes
/// of AURORA_DEMO_ACTIONS answer for themselves).
pub fn object_props(object: Uuid) -> aurora_net::build::ObjectProps {
    use aurora_net::build::perm;
    aurora_net::build::ObjectProps {
        object_id: object,
        owner_id: DEMO_NOVA,
        owner_mask: perm::TRANSFER | perm::MODIFY,
        name: aurora_i18n::tr!("common-demo-object").into(),
        description: aurora_i18n::tr!("demo-hover-tips-object-desc").into(),
        ..Default::default()
    }
}
