//! AURORA_DEMO_SCULPT_ZERO: the same circular path/profile with no sculpt
//! block, a subtype-zero sculpt and a sphere sculpt, using a synthetic map.
//! AURORA_DEMO_SCULPT_MISSING: a sculpt whose map is missing.

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
    let pixels = map_pixels(|v, u| {
        let (theta, phi) = (v * std::f32::consts::PI, u * std::f32::consts::TAU);
        Vec3::new(theta.sin() * phi.cos(), theta.sin() * phi.sin(), theta.cos())
    });
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

/// The map of AURORA_DEMO_SCULPT_MISSING, a torus.
const MISSING_MAP: Uuid = Uuid::from_u128(0xde7e_5c00_0000_4000_8000_0000_0000_0002);
const MISSING_LOCAL_ID: u32 = 81_110;
/// The sculpt arrives once the loading screen is gone; with `late` its map
/// turns up at `LATE_FRAME`.
const ARRIVE_FRAME: u64 = 300;
const LATE_FRAME: u64 = 900;

/// AURORA_DEMO_SCULPT_MISSING=1|late: a gold torus sculpt east of the start
/// position whose map the asset server does not have (a 404): LL's sphere
/// placeholder stands in for it. With `late`, the map turns up at frame
/// `LATE_FRAME` (in the disk cache, read again as a retry would find it on
/// the server) and the torus replaces the sphere.
pub struct MissingMapDemo {
    late: bool,
    sent: bool,
    found: bool,
}

impl MissingMapDemo {
    pub fn from_env() -> Option<Self> {
        let value = std::env::var("AURORA_DEMO_SCULPT_MISSING").ok()?;
        Some(MissingMapDemo {
            late: value.trim() == "late",
            sent: false,
            found: false,
        })
    }

    pub fn tick(&mut self, frame: u64, world: &mut crate::world::World, textures: &mut crate::scene::textures::TextureStreamer) {
        if !self.sent && frame >= ARRIVE_FRAME {
            self.sent = true;
            let tex = textures.cache_dir().join("tex");
            // (left by a previous `late` run)
            for ext in ["j2c", "part", "bc7"] {
                let _ = std::fs::remove_file(tex.join(format!("{MISSING_MAP}.{ext}")));
            }
            let mut volume = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE, 175, 0, 0);
            volume.sculpt = Some(SculptParams {
                texture: MISSING_MAP,
                sculpt_type: LL_SCULPT_TYPE_TORUS,
            });
            let object = prim(
                MISSING_LOCAL_ID,
                Vec3::new(134.0, 130.0, floor_at(134.0, 130.0) + 1.5),
                Quat::IDENTITY,
                Vec3::splat(2.0),
                volume,
                te([0.95, 0.75, 0.25, 1.0], 0, false, 0.0),
                ExtraParams {
                    sculpt: volume.sculpt,
                    ..Default::default()
                },
                "",
            );
            world.apply(NetEvent::ObjectUpdates {
                handle: HANDLE,
                objects: vec![object],
            });
            log::info!("demo sculpt missing: a sculpt whose map the asset server does not have");
        }
        if self.found {
            return;
        }
        // nothing on the disk: the grid would answer 404
        if textures.demo_needs_data(&MISSING_MAP) && textures.demo_missing(&MISSING_MAP) {
            log::info!("demo sculpt missing: its map is missing (404)");
        }
        if self.late && frame >= LATE_FRAME {
            self.found = true;
            let pixels = map_pixels(|v, u| {
                // (big radius 0.3, tube 0.15, in the -0.5..0.5 box)
                let (theta, phi) = (v * std::f32::consts::TAU, u * std::f32::consts::TAU);
                let ring = 0.3 + 0.15 * theta.cos();
                Vec3::new(ring * phi.cos(), ring * phi.sin(), 0.15 * theta.sin()) * 2.0
            });
            match aurora_assets::j2k::encode_texture_j2k(SIDE, SIDE, 3, &pixels, true) {
                Ok(encoded) => match std::fs::write(textures.cache_dir().join("tex").join(format!("{MISSING_MAP}.j2c")), encoded) {
                    Ok(()) => {
                        textures.demo_reread(&MISSING_MAP);
                        log::info!("demo sculpt missing: its map turned up");
                    }
                    Err(e) => log::warn!("demo sculpt missing: cannot write the map ({e})"),
                },
                Err(e) => log::warn!("demo sculpt missing: cannot encode the map ({e})"),
            }
        }
    }
}

/// A SIDE×SIDE sculpt map: `point(v, u)` in -1..1 for each pixel, v 0..1
/// down the rows (last row included), u 0..1 along them (last column
/// excluded, the seam).
fn map_pixels(point: impl Fn(f32, f32) -> Vec3) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((SIDE * SIDE * 3) as usize);
    for y in 0..SIDE {
        let v = y as f32 / (SIDE - 1) as f32;
        for x in 0..SIDE {
            let u = x as f32 / SIDE as f32;
            let c = ((point(v, u) * 0.5 + Vec3::splat(0.5)) * 255.0).round();
            pixels.extend_from_slice(&[c.x as u8, c.y as u8, c.z as u8]);
        }
    }
    pixels
}
