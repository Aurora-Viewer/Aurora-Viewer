//! AURORA_DEMO_TEXREFRESH=1|auto: "Refresh textures" without a grid. A
//! panel stands in front of the avatar, wearing a texture whose file in the
//! disk cache is stale: a grey picture crossed in red, written as its
//! `.j2c` before the panel arrives, so that the streamer reads it from the
//! disk as it would a damaged cache entry. The local asset server
//! (demo_asset_server.rs) holds the right picture, Aurora's violet and teal
//! stripes. "Refresh textures" on the panel (right click), or `auto` at
//! frame 900, forgets the texture, removes its files and downloads it
//! again: the stripes replace the cross.

use super::{HANDLE, height, prim, shape, te};
use aurora_net::NetEvent;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec3};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// The panel's texture (apart from the demo's other fixed ids).
pub const TEXTURE: Uuid = Uuid::from_u128(0xde7e_f2e5_0000_4000_8000_0000_0000_0001);
const LOCAL_ID: u32 = 81_000;
/// Size of the two pictures.
const SIDE: u32 = 256;
/// The panel arrives once the loading screen is gone; `auto` refreshes it
/// at `AUTO_FRAME`.
const ARRIVE_FRAME: u64 = 300;
const AUTO_FRAME: u64 = 900;
/// Round trip of the local asset server.
const DELAY: Duration = Duration::from_millis(150);

pub struct TexRefreshDemo {
    auto: bool,
    armed: bool,
    sent: bool,
    refreshed: bool,
}

impl TexRefreshDemo {
    pub fn from_env() -> Option<Self> {
        let value = std::env::var("AURORA_DEMO_TEXREFRESH").ok()?;
        Some(TexRefreshDemo {
            auto: value.trim() == "auto",
            armed: false,
            sent: false,
            refreshed: false,
        })
    }

    /// Once a frame. Returns the object to refresh in `auto` mode, once.
    pub fn tick(&mut self, frame: u64, world: &mut crate::world::World, cache_dir: &Path) -> Option<Uuid> {
        if !self.armed {
            self.armed = true;
            self.arm(world, cache_dir);
        }
        if !self.sent && frame >= ARRIVE_FRAME {
            self.sent = true;
            world.apply(panel(world.agent.position));
            log::info!("demo texture refresh: the panel shows the stale picture from the disk cache");
        }
        if self.auto && self.sent && !self.refreshed && frame >= AUTO_FRAME {
            self.refreshed = true;
            log::info!("demo texture refresh: refreshing the panel");
            return Some(object_id());
        }
        None
    }

    /// The stale file in the disk cache, the right one on the local asset
    /// server, which becomes the region's `ViewerAsset`.
    fn arm(&self, world: &mut crate::world::World, cache_dir: &Path) {
        let (Some(stale), Some(right)) = (encode(&stale_picture()), encode(&right_picture())) else {
            log::warn!("demo texture refresh: the pictures could not be encoded");
            return;
        };
        let tex = cache_dir.join("tex");
        // (left by a previous run: the stale file must be the one read)
        for ext in ["part", "bc7"] {
            let _ = std::fs::remove_file(tex.join(format!("{TEXTURE}.{ext}")));
        }
        if let Err(e) = std::fs::create_dir_all(&tex).and_then(|()| std::fs::write(tex.join(format!("{TEXTURE}.j2c")), &stale)) {
            log::warn!("demo texture refresh: stale file not written ({e})");
        }
        let served: PathBuf = std::env::temp_dir().join("aurora-demo-texrefresh.j2c");
        if let Err(e) = std::fs::write(&served, &right) {
            log::warn!("demo texture refresh: served file not written ({e})");
            return;
        }
        match super::asset_server::start(Arc::new(vec![served]), DELAY, |id| (*id == TEXTURE).then_some(0)) {
            Ok(url) => {
                let mut caps = world.main().map(|r| (*r.caps).clone()).unwrap_or_default();
                caps.insert("ViewerAsset".to_owned(), url);
                world.apply(NetEvent::Capabilities {
                    handle: HANDLE,
                    caps: Arc::new(caps),
                });
            }
            Err(e) => log::warn!("demo texture refresh: no local asset server ({e})"),
        }
    }
}

/// The panel's object id (`prim` derives it from the local id).
pub fn object_id() -> Uuid {
    Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | LOCAL_ID as u128)
}

/// A thin square panel right of the avatar, at the height of its body (the
/// plaza stands above the terrain), facing the camera.
fn panel(avatar: Vec3) -> NetEvent {
    let (x, y) = (135.0, 124.0);
    let mut t = (*te([1.0; 4], 0, true, 0.0)).clone();
    for f in &mut t.faces {
        f.texture = TEXTURE;
    }
    let object = prim(
        LOCAL_ID,
        Vec3::new(x, y, avatar.z.max(height(x, y) + 0.6)),
        Quat::IDENTITY,
        Vec3::new(0.05, 1.2, 1.2),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        Arc::new(t),
        ExtraParams::default(),
        "",
    );
    NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![object],
    }
}

fn encode(rgba: &[u8]) -> Option<Vec<u8>> {
    aurora_assets::j2k::encode_texture_j2k(SIDE, SIDE, 4, rgba, false)
        .map_err(|e| log::warn!("demo texture refresh: {e}"))
        .ok()
}

/// What a damaged cache entry shows here: grey, crossed in red.
fn stale_picture() -> Vec<u8> {
    let mut px = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let cross = x.abs_diff(y) < 12 || x.abs_diff(SIDE - 1 - y) < 12;
            px.extend_from_slice(if cross { &[220, 40, 40, 255] } else { &[120, 120, 120, 255] });
        }
    }
    px
}

/// The texture as the asset server has it: violet and teal stripes.
fn right_picture() -> Vec<u8> {
    let mut px = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let stripe = ((x + y) / 32).is_multiple_of(2);
            px.extend_from_slice(if stripe { &[139, 92, 246, 255] } else { &[94, 234, 212, 255] });
        }
    }
    px
}
