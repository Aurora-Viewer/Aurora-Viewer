//! `AURORA_DEMO_PARCEL_ENV`: an environment saved for a parcel of the four of
//! `demo_parcel_visibility.rs`, then walking off it and back.
use crate::world::World;
use uuid::Uuid;

pub fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_PARCEL_ENV").ok()
}

/// One id per demo parcel (RemoteParcelRequest).
pub fn parcel_uuid(local_id: i32) -> Uuid {
    Uuid::from_u128(0xA0_0000 + local_id as u128)
}

fn walk(world: &mut World, x: f32, y: f32) {
    world.agent.get_off(glam::Vec3::new(x, y, world.agent.position.z));
}

/// Sunrise on the first parcel, carried to the next, where another light is
/// saved; then back to the first (the sunrise) and to the second (the saved
/// light). `save` and `left` stop along the way, for captures; `update` then
/// changes that light, saves over it and walks off and back; `manual` does
/// nothing but follow the agent across the four parcels.
pub fn frame(world: &mut World, time_of_day: &mut u8, mode: &str, frame: u64) {
    let last = match mode {
        "manual" => 0,
        "save" => 420,
        "left" => 560,
        "update" => 1180,
        _ => 700,
    };
    match frame.min(last + 1) {
        240 => *time_of_day = 1,
        260 => walk(world, 124.0, 132.0),
        400 if mode == "personal" => {
            world.eep.capture_personal();
            world.eep.edit_personal(crate::demo::env::edit_lighting);
        }
        400 => *time_of_day = 4,
        420 => world.save_parcel_environment(),
        560 => walk(world, 132.0, 132.0),
        700 => walk(world, 124.0, 132.0),
        860 if mode == "update" => *time_of_day = 2,
        900 if mode == "update" => world.save_parcel_environment(),
        1040 if mode == "update" => walk(world, 132.0, 132.0),
        1180 if mode == "update" => walk(world, 124.0, 132.0),
        _ => {}
    }
    if matches!(frame, 380 | 540 | 680 | 840 | 880 | 1020 | 1160 | 1320) {
        log::info!(
            "parcel environment demo, frame {frame}: parcel {:?}, time of day {time_of_day}, {:?}, changed {}",
            world.parcel.as_ref().map(|p| p.local_id),
            world.eep.local_view(),
            world.parcel_environment_changed()
        );
    }
}
