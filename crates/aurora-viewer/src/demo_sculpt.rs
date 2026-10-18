//! AURORA_DEMO_SCULPT_ZERO: the same circular path/profile with no sculpt
//! block, a subtype-zero sculpt and a sphere sculpt, using a synthetic map.

use super::{HANDLE, floor_at, prim, shape, te};
use aurora_net::NetEvent;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec3};
use uuid::Uuid;

const MAP: Uuid = Uuid::from_u128(0xde7e_5c00_0000_4000_8000_0000_0000_0001);
const SIDE: u32 = 64;

pub fn events() -> Vec<NetEvent> {
    if std::env::var("AURORA_DEMO_SCULPT_ZERO").ok().as_deref() != Some("1") {
        return Vec::new();
    }
    let mut pixels = Vec::with_capacity((SIDE * SIDE * 3) as usize);
    for y in 0..SIDE {
        let theta = y as f32 / (SIDE - 1) as f32 * std::f32::consts::PI;
        for x in 0..SIDE {
            let phi = x as f32 / SIDE as f32 * std::f32::consts::TAU;
            let p = Vec3::new(theta.sin() * phi.cos(), theta.sin() * phi.sin(), theta.cos());
            let c = ((p * 0.5 + Vec3::splat(0.5)) * 255.0).round();
            pixels.extend_from_slice(&[c.x as u8, c.y as u8, c.z as u8]);
        }
    }
    let encoded = match aurora_assets::j2k::encode_texture_j2k(SIDE, SIDE, 3, &pixels, true) {
        Ok(encoded) => encoded,
        Err(e) => {
            log::warn!("demo sculpt: cannot encode the synthetic map ({e})");
            return Vec::new();
        }
    };
    // The demo has its own cache; exercise the ordinary asset/decode path.
    let cache = crate::settings::cache_dir().join("tex");
    if let Err(e) = std::fs::create_dir_all(&cache).and_then(|()| std::fs::write(cache.join(format!("{MAP}.j2c")), encoded)) {
        log::warn!("demo sculpt: cannot write the synthetic map ({e})");
        return Vec::new();
    }
    let objects = [
        (None, [0.55, 0.55, 0.55, 1.0]),
        (Some(LL_SCULPT_TYPE_NONE), [0.55, 0.3, 0.95, 1.0]),
        (Some(LL_SCULPT_TYPE_SPHERE), [0.1, 0.8, 0.7, 1.0]),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (kind, color))| {
        let mut volume = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE, 175, 0, 0);
        volume.sculpt = kind.map(|sculpt_type| SculptParams { texture: MAP, sculpt_type });
        prim(
            81_100 + i as u32,
            Vec3::new(134.0, 123.0 + i as f32 * 2.0, floor_at(134.0, 127.0) + 1.5),
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            Vec3::splat(1.5),
            volume,
            te(color, 0, false, 0.0),
            ExtraParams {
                sculpt: volume.sculpt,
                ..Default::default()
            },
            "",
        )
    })
    .collect();
    log::info!("demo sculpt: grey ordinary torus, violet subtype-zero map, teal sphere map");
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}
