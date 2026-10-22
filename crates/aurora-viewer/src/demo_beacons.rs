//! Offline fixtures for #682, shared by the world filters and the viewer.

use super::*;
use aurora_net::objects::{AttachedSound, ParticleUpdate};

pub fn objects() -> Vec<ObjectUpdate> {
    let labels = [
        aurora_i18n::tr!("beacons-scripted"),
        aurora_i18n::tr!("beacons-touch"),
        aurora_i18n::tr!("beacons-physical"),
        aurora_i18n::tr!("beacons-sound"),
        aurora_i18n::tr!("beacons-particles"),
        aurora_i18n::tr!("beacons-media"),
        aurora_i18n::tr!("beacons-scripted"),
    ];
    let mut objects: Vec<_> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let y = 118.0 + i as f32 * 3.0;
            let pos = Vec3::new(140.0, y, floor_at(140.0, y) + 1.0);
            prim(
                68200 + i as u32,
                pos,
                Quat::IDENTITY,
                Vec3::splat(1.2),
                Default::default(),
                te([0.7, 0.7, 0.7, if i == 6 { 0.0 } else { 1.0 }], 0, false, 0.0),
                Default::default(),
                label,
            )
        })
        .collect();
    objects[0].update_flags |= 1 << 6;
    objects[1].update_flags |= (1 << 6) | (1 << 7);
    objects[2].update_flags |= 1;
    objects[2].scale = Vec3::splat(0.2);
    objects[3].sound = AttachedSound {
        id: chime().0,
        gain: 0.1,
        flags: 1,
        ..Default::default()
    };
    objects[4].particles = ParticleUpdate::Set(fountain_particles());
    let mut media = (*objects[5].texture_entry.clone().expect("demo TE")).clone();
    media.faces[0].media_flags = 1;
    objects[5].texture_entry = Some(Arc::new(media));
    objects[6].update_flags |= 1 << 6;
    objects
}

pub fn events() -> NetEvent {
    NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: objects(),
    }
}

pub fn view(mode: &str) -> (Vec3, Vec3) {
    let ground = floor_at(140.0, 127.0);
    if mode == "parcels" {
        (Vec3::new(160.0, 100.0, ground + 45.0), Vec3::new(128.0, 128.0, ground))
    } else {
        (Vec3::new(122.0, 119.0, ground + 10.0), Vec3::new(140.0, 127.0, ground + 1.0))
    }
}

pub fn overlay_events() -> Vec<NetEvent> {
    let mut cells = vec![0u8; 64 * 64];
    for y in 0..64 {
        for x in 0..64 {
            cells[y * 64 + x] = match (x < 32, y < 32) {
                (true, true) => 3,
                (false, true) => 2,
                (true, false) => 1,
                (false, false) => 4,
            } | if x == 0 || x == 32 { 64 } else { 0 }
                | if y == 0 || y == 32 { 128 } else { 0 };
        }
    }
    (0..4)
        .map(|sequence| NetEvent::ParcelOverlay {
            handle: HANDLE,
            sequence: sequence as i32,
            data: cells[sequence * 1024..(sequence + 1) * 1024].to_vec(),
        })
        .collect()
}
