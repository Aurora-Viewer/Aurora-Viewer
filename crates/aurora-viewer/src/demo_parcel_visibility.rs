//! Four parcels around the demo plaza, with a same-owner northern neighbour.
use crate::world::{World, parcel_visibility::Mode};
use aurora_net::ParcelInfo;
use std::sync::Arc;
use uuid::Uuid;

pub fn parcel_at(x: f32, y: f32) -> Arc<ParcelInfo> {
    let east = x >= 128.0;
    let north = y >= 128.0;
    let mut bitmap = vec![0u8; 512];
    for cy in 0..64 {
        for cx in 0..64 {
            if (cx >= 32) == east && (cy >= 32) == north {
                let i = cy * 64 + cx;
                bitmap[i / 8] |= 1 << (i % 8);
            }
        }
    }
    Arc::new(ParcelInfo {
        local_id: 1 + i32::from(!east) + 2 * i32::from(!north),
        name: if east && north {
            aurora_i18n::tr!("demo-parcel-visibility-current")
        } else if north {
            aurora_i18n::tr!("demo-parcel-visibility-neighbour")
        } else {
            aurora_i18n::tr!("demo-parcel-visibility-other")
        }
        .into(),
        owner_id: if north { crate::demo::DEMO_LOUP } else { crate::demo::DEMO_NOVA },
        bitmap,
        ..Default::default()
    })
}

pub fn install(world: &mut World, settings: &mut crate::settings::Settings, mode: &str) {
    if let Some(region) = world.main_region.and_then(|h| world.regions.get_mut(&h))
        && let Some(info) = region.info.as_mut()
    {
        Arc::make_mut(info).region_id = Uuid::from_u128(0x946);
    }
    let (x, y) = std::env::var("AURORA_DEMO_POS")
        .ok()
        .and_then(|value| {
            let (x, y) = value.split_once(',')?;
            let (x, y) = (x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?);
            (x.is_finite() && y.is_finite()).then_some((x, y))
        })
        .unwrap_or((132.0, 132.0));
    let mut parcel = parcel_at(x, y);
    Arc::make_mut(&mut parcel).name = aurora_i18n::tr!("demo-parcel-visibility-current").into();
    world.parcel_name.clone_from(&parcel.name);
    world.parcel = Some(parcel);
    world.agent.get_off(glam::Vec3::new(x, y, world.agent.position.z));
    settings.parcel_visibility.mode = match mode {
        "off" => Mode::Off,
        "listed" => Mode::Listed,
        "except" | "except-listed" => Mode::ExceptListed,
        _ => Mode::Always,
    };
    settings.parcel_visibility.same_owner = mode == "same-owner";
    settings.parcel_visibility.parcels.clear();
    if matches!(mode, "listed" | "except-listed")
        && let Some(entry) = world.current_parcel_entry()
    {
        settings.parcel_visibility.parcels.push(entry);
    }
}
