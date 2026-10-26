//! AURORA_DEMO_TEXTURE_MISSING: boxes whose texture the asset server does
//! not have.

use super::{HANDLE, floor_at, prim, shape};
use aurora_net::NetEvent;
use aurora_prim::ExtraParams;
use aurora_prim::TextureEntry;
use aurora_prim::params::*;
use glam::{Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

const MISSING_TEXTURE: Uuid = Uuid::from_u128(0xde7e_7e00_0000_4000_8000_0000_0000_0001);
const FIRST_LOCAL_ID: u32 = 81_120;
/// The boxes arrive once the loading screen is gone.
const ARRIVE_FRAME: u64 = 300;

/// AURORA_DEMO_TEXTURE_MISSING=1: two boxes east of the start position
/// whose texture the asset server does not have (a 403 from the asset CDN):
/// a white one and a dark, mostly transparent one, as the layers of a pool
/// edge that showed it. With the rez effect on, they are the grid while the
/// texture is asked for, then the Legacy Viewer's grey, never the grid for
/// ever (`TextureStreamer::show_unavailable`).
pub struct MissingTextureDemo {
    sent: bool,
}

impl MissingTextureDemo {
    pub fn from_env() -> Option<Self> {
        (std::env::var("AURORA_DEMO_TEXTURE_MISSING").ok()?.trim() == "1").then_some(MissingTextureDemo { sent: false })
    }

    pub fn tick(&mut self, frame: u64, world: &mut crate::world::World, textures: &mut crate::scene::textures::TextureStreamer) {
        if !self.sent && frame >= ARRIVE_FRAME {
            self.sent = true;
            let tex = textures.cache_dir().join("tex");
            for ext in ["j2c", "part", "bc7"] {
                let _ = std::fs::remove_file(tex.join(format!("{MISSING_TEXTURE}.{ext}")));
            }
            let objects = [[1.0, 1.0, 1.0, 1.0], [0.24, 0.24, 0.24, 0.3]]
                .into_iter()
                .enumerate()
                .map(|(i, color)| {
                    let (x, y) = (134.0, 133.0 + i as f32 * 2.5);
                    let volume = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
                    prim(
                        FIRST_LOCAL_ID + i as u32,
                        Vec3::new(x, y, floor_at(x, y) + 1.0),
                        Quat::IDENTITY,
                        Vec3::splat(1.8),
                        volume,
                        faces(color),
                        ExtraParams::default(),
                        "",
                    )
                })
                .collect();
            world.apply(NetEvent::ObjectUpdates { handle: HANDLE, objects });
            log::info!("demo texture missing: two boxes whose texture the asset server does not have");
        }
        // nothing on the disk: the asset CDN would answer 403
        if textures.demo_needs_data(&MISSING_TEXTURE) && textures.demo_missing(&MISSING_TEXTURE) {
            log::info!("demo texture missing: their texture is missing (403)");
        }
    }
}

fn faces(color: [f32; 4]) -> Arc<TextureEntry> {
    let mut t = TextureEntry::default();
    for f in t.faces.iter_mut() {
        f.texture = MISSING_TEXTURE;
        f.color = color;
    }
    Arc::new(t)
}
