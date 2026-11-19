//! AURORA_DEMO_PRIM_WATER (#2008): a pool raised 40 m above the region
//! water, its water a phantom prim, with the avatar put in it once the
//! swimming animations are ready. Modes:
//! - (empty) or `name`: the water prim is named "AuroraWater";
//! - `description`: named "Pool", "AuroraWater" in its description;
//! - `custom`: named "Garden pond", found through the added word "pond";
//! - `plane`: a 5 cm water plane over the pool's floor;
//! - `nomatch`: named "Liverpool sign" with the word "pool": not water;
//! - `off`: prim water left off, the region water only.

use super::*;

pub const FLOOR: u32 = 122_000;
pub const WATER: u32 = FLOOR + 1;
/// The pool's centre on the plaza, and the top of its floor.
pub const CENTER: Vec3 = Vec3::new(132.0, 118.0, 0.0);
pub const FLOOR_TOP: f32 = 60.0;
pub const DEPTH: f32 = 2.4;

/// `prim`'s id of a local id.
pub fn id(local_id: u32) -> Uuid {
    Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | u128::from(local_id))
}

pub fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_PRIM_WATER").ok()
}

/// The settings the scenario starts with.
pub fn settings(mode: &str) -> crate::movement::water_volume::Settings {
    let mut settings = crate::movement::water_volume::Settings {
        enabled: mode != "off",
        ..Default::default()
    };
    match mode {
        "custom" => settings.words = "AuroraWater, pond".into(),
        "nomatch" => settings.words = "pool".into(),
        _ => {}
    }
    settings
}

/// The water's top, where the avatar floats.
pub fn surface() -> f32 {
    FLOOR_TOP + DEPTH
}

pub fn events(mode: &str) -> Vec<NetEvent> {
    let box_shape = || {
        shape(
            aurora_prim::params::LL_PCODE_PATH_LINE,
            aurora_prim::params::LL_PCODE_PROFILE_SQUARE,
            100,
            0,
            0,
        )
    };
    let floor = prim(
        FLOOR,
        CENTER.with_z(FLOOR_TOP - 0.25),
        Quat::IDENTITY,
        Vec3::new(10.0, 10.0, 0.5),
        box_shape(),
        te([0.75, 0.78, 0.82, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        "",
    );
    let thickness = if mode == "plane" { 0.05 } else { DEPTH };
    let mut water = prim(
        WATER,
        CENTER.with_z(surface() - thickness * 0.5),
        Quat::IDENTITY,
        Vec3::new(8.0, 8.0, thickness),
        box_shape(),
        te([0.2, 0.5, 0.9, 0.45], 1, false, 0.0),
        ExtraParams::default(),
        "",
    );
    water.update_flags |= crate::build::edits::FLAGS_PHANTOM;
    vec![NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![floor, water],
    }]
}

/// The properties a hover would get for the scenario's prims.
pub fn props(object: Uuid) -> Option<aurora_net::build::ObjectProps> {
    let mode = mode()?;
    let (name, description) = if object == id(WATER) {
        match mode.as_str() {
            "description" => ("Pool", "AuroraWater, heated"),
            "custom" => ("Garden pond", "Fresh water"),
            "nomatch" => ("Liverpool sign", ""),
            _ => ("AuroraWater", ""),
        }
    } else if object == id(FLOOR) {
        ("Pool floor", "")
    } else {
        return None;
    };
    Some(aurora_net::build::ObjectProps {
        object_id: object,
        name: name.into(),
        description: description.into(),
        ..Default::default()
    })
}
