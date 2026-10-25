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
    let mut parcel = ParcelInfo {
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
    };
    if let Ok(mode) = std::env::var("AURORA_DEMO_PARCEL_VISIBILITY")
        && matches!(mode.as_str(), "same-group" | "group-owned-current")
        && north
    {
        let owned = east == (mode == "same-group");
        parcel.group_id = if owned { Uuid::nil() } else { crate::demo::DEMO_GROUP1 };
        parcel.is_group_owned = owned;
        if owned {
            parcel.owner_id = crate::demo::DEMO_GROUP1;
        }
    }
    Arc::new(parcel)
}

pub fn parcel_by_id(local_id: i32) -> Option<Arc<ParcelInfo>> {
    match local_id {
        1 => Some(parcel_at(130.0, 130.0)),
        2 => Some(parcel_at(120.0, 130.0)),
        3 => Some(parcel_at(130.0, 120.0)),
        4 => Some(parcel_at(120.0, 120.0)),
        _ => None,
    }
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
    let (x, y) = if mode == "crossings" { (124.0, 132.0) } else { (x, y) };
    let mut parcel = parcel_at(x, y);
    Arc::make_mut(&mut parcel).name = aurora_i18n::tr!("demo-parcel-visibility-current").into();
    world.parcel_name.clone_from(&parcel.name);
    world.parcel = Some(parcel);
    world.agent.get_off(glam::Vec3::new(x, y, world.agent.position.z));
    settings.parcel_visibility.mode = match mode {
        "off" => Mode::Off,
        "listed" | "crossings" => Mode::Listed,
        "except" | "except-listed" => Mode::ExceptListed,
        _ => Mode::Always,
    };
    settings.parcel_visibility.same_owner = mode == "same-owner";
    settings.parcel_visibility.same_group = matches!(mode, "same-group" | "group-owned-current");
    settings.parcel_visibility.always_visible.clear();
    if matches!(mode, "always-visible" | "crossings")
        && let Some(handle) = world.main_region
        && let Some(mut entry) = world.parcel_entry(handle, &parcel_at(130.0, 130.0))
    {
        entry.name = aurora_i18n::tr!("demo-parcel-visibility-pinned").into();
        settings.parcel_visibility.always_visible.push(entry);
    }
    settings.parcel_visibility.parcels.clear();
    if matches!(mode, "listed" | "except-listed" | "crossings")
        && let Some(entry) = world.current_parcel_entry()
    {
        settings.parcel_visibility.parcels.push(entry);
    }
    if mode == "lists"
        && let Some(handle) = world.main_region
    {
        for local_id in 1..=3 {
            if let Some(parcel) = parcel_by_id(local_id)
                && let Some(mut entry) = world.parcel_entry(handle, &parcel)
            {
                entry.name = format!("{} — {}", entry.name, aurora_i18n::tr!("demo-parcel-visibility-long"));
                settings.parcel_visibility.parcels.push(entry.clone());
                settings.parcel_visibility.always_visible.push(entry);
            }
        }
    }
}

/// Simulate arrival data lagging behind movement, without a grid connection.
pub fn update_agent_parcel(world: &mut World, mode: &str, frame: u64) {
    if mode == "crossings" && frame >= 700 {
        let phase = (frame - 700) % 120;
        let outside = ((frame - 700) / 120).is_multiple_of(2);
        if phase == 0 {
            world
                .agent
                .get_off(glam::Vec3::new(124.0, if outside { 124.0 } else { 132.0 }, world.agent.position.z));
        }
        if phase < 12 {
            return;
        }
    }
    let position = world.agent.position;
    let local_id = 1 + i32::from(position.x < 128.0) + 2 * i32::from(position.y < 128.0);
    if world.parcel.as_ref().is_none_or(|p| p.local_id != local_id) {
        let mut parcel = parcel_at(position.x, position.y);
        if mode == "crossings" && local_id == 2 {
            Arc::make_mut(&mut parcel).name = aurora_i18n::tr!("demo-parcel-visibility-current").into();
        }
        world.parcel_name.clone_from(&parcel.name);
        world.parcel = Some(parcel);
    }
}

pub fn check_crossings(world: &World, frame: u64) {
    if frame < 650 {
        return;
    }
    let Some((ox, oy)) = world.main_origin() else {
        return;
    };
    let visible = world
        .parcel_visibility
        .contains(glam::DVec3::new(f64::from(ox) + 140.0, f64::from(oy) + 140.0, 30.0));
    let active = world.parcel_visibility.active();
    let expected = if frame < 700 {
        true
    } else {
        let outside = ((frame - 700) / 120).is_multiple_of(2);
        if (frame - 700) % 120 < 12 { outside } else { !outside }
    };
    if !visible || active != expected {
        log::error!(
            "parcel visibility crossing changed before a complete decision: frame={frame} visible={visible} active={active} expected={expected}"
        );
    }
    if frame == 650 || frame >= 700 && matches!((frame - 700) % 120, 0 | 12) {
        log::info!(
            "parcel visibility crossing: frame={frame} visible={visible} active={active} version={}",
            world.parcel_visibility.version
        );
    }
}
