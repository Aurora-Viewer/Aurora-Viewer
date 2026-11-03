//! AURORA_DEMO_TEXTURE_RES=1: Preferences › Graphics › Maximum texture
//! resolution without a grid. The demo's own textures are drawn on the
//! spot, never streamed, so nothing in it ever went past 512 pixels; here
//! three boards east of the start position (8, 4 and 2 m wide) each wear a
//! 2048 × 2048 texture that goes through the streamer like a downloaded
//! one: the scenario draws the level the streamer asks for (`DemoSource`),
//! and the maximum resolution lowers it on the GPU as it would a grid
//! texture. Each level of the picture is tinted by its resolution, so what
//! the GPU shows reads at a glance: 2048 green, 1024 amber, 512 red,
//! smaller blue-grey, with 16 × 16 grid lines that get coarser as it drops.
//! The large board shows a close-up of its texture (a quarter of it across):
//! green under 2048, amber under 1024, red under 512. The two others show
//! it whole, smaller on screen: the GPU draws a coarser level there
//! whatever the setting, as the Legacy Viewer would.
//! Not with AURORA_DEMO_STREAM, which has its own demo source.

use super::{HANDLE, floor_at, prim, shape, te};
use crate::scene::jobs::{AlphaKind, Jobs};
use crate::scene::textures::{DemoSource, TexSource, TextureStreamer, texture_result};
use aurora_net::NetEvent;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use aurora_render::Renderer;
use glam::{Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

/// The boards' textures, one each (apart from the demo's other fixed ids).
const TEXTURES: u128 = 0xde7e_7e50_0000_4000_8000_0000_0000_0000;
const FIRST_LOCAL_ID: u32 = 81_300;
/// Side of the textures at full resolution.
const SIDE: u32 = 2048;
/// Width of the boards, in metres, where they stand (in a row facing the
/// start position, in the direction the camera first looks) and their
/// texture's repeats. The large one shows a quarter of its texture across,
/// as a tiled floor would: up close the GPU then draws its finest level
/// even in a small window (the sizing counts the repeats, as the Legacy
/// Viewer's).
const BOARDS: [(f32, f32, f32, f32); 3] = [(8.0, 150.0, 138.0, 0.25), (4.0, 150.0, 131.0, 1.0), (2.0, 150.0, 127.5, 1.0)];
/// The boards arrive once the loading screen is gone.
const ARRIVE_FRAME: u64 = 300;
/// The level the scenario shows first, as a texture's first bytes (64 px).
const FIRST_LEVEL: u8 = 5;

fn texture(i: u32) -> Uuid {
    Uuid::from_u128(TEXTURES | i as u128)
}

fn texture_index(id: &Uuid) -> Option<u32> {
    let n = id.as_u128();
    (n & !0xFF == TEXTURES && ((n & 0xFF) as usize) < BOARDS.len()).then_some(n as u32 & 0xFF)
}

/// The tint of a level of `side` pixels.
fn tint(side: u32) -> [u8; 3] {
    match side {
        2048.. => [52, 168, 83],
        1024.. => [232, 168, 36],
        512.. => [214, 64, 56],
        _ => [104, 118, 146],
    }
}

/// Mip chain of a board's texture from a discard level down to 1 × 1: each
/// level filled with the tint of its own resolution, crossed by 16 dark
/// lines a texel wide each way (the same grid at every level, finer the
/// larger the level).
pub fn texture_mips(discard: u32) -> Vec<(u32, u32, Vec<u8>)> {
    let mut out = Vec::new();
    let mut side = (SIDE >> discard.min(11)).max(1);
    loop {
        let c = tint(side);
        let dark = c.map(|v| v / 3);
        let cell = (side / 16).max(1);
        let mut px = Vec::with_capacity((side * side * 4) as usize);
        for y in 0..side {
            for x in 0..side {
                let line = side >= 16 && (x % cell == 0 || y % cell == 0);
                let p = if line { dark } else { c };
                px.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        out.push((side, side, px));
        if side == 1 {
            break;
        }
        side /= 2;
    }
    out
}

fn source() -> DemoSource {
    DemoSource {
        has: |id| texture_index(id).is_some(),
        texels: |_, discard| texture_mips(discard as u32),
    }
}

pub struct TextureResDemo {
    sent: bool,
}

impl TextureResDemo {
    pub fn from_env() -> Option<Self> {
        (std::env::var("AURORA_DEMO_TEXTURE_RES").ok()?.trim() == "1").then_some(TextureResDemo { sent: false })
    }

    pub fn tick(
        &mut self,
        frame: u64,
        world: &mut crate::world::World,
        textures: &mut TextureStreamer,
        renderer: &mut Renderer,
        jobs: &Jobs,
    ) {
        if self.sent || frame < ARRIVE_FRAME {
            return;
        }
        self.sent = true;
        textures.demo_source = Some(source());
        // the textures are held by the scenario (never evicted), and show
        // their first pixels as a download would; the streamer then asks
        // for the level their size on screen and the maximum resolution
        // allow
        for i in 0..BOARDS.len() as u32 {
            let _ = textures.acquire(renderer, texture(i), TexSource::Asset);
        }
        let pool = textures.staging.clone();
        jobs.spawn_many((0..BOARDS.len() as u32).collect(), move |i| {
            texture_result(
                texture(i),
                FIRST_LEVEL,
                texture_mips(FIRST_LEVEL as u32),
                (AlphaKind::Opaque, false),
                None,
                pool.as_deref(),
            )
        });
        let objects = BOARDS
            .iter()
            .enumerate()
            .map(|(i, &(width, x, y, repeats))| {
                let mut t = (*te([1.0; 4], 0, true, 0.0)).clone();
                for f in &mut t.faces {
                    f.texture = texture(i as u32);
                    f.scale_s = repeats;
                    f.scale_t = repeats;
                }
                prim(
                    FIRST_LOCAL_ID + i as u32,
                    Vec3::new(x, y, floor_at(x, y) + width * 0.5 + 0.2),
                    Quat::IDENTITY,
                    Vec3::new(0.05, width, width),
                    shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
                    Arc::new(t),
                    ExtraParams::default(),
                    "",
                )
            })
            .collect();
        world.apply(NetEvent::ObjectUpdates { handle: HANDLE, objects });
        log::info!(
            "demo texture resolution: three boards with 2048 px textures, maximum resolution {} px",
            textures.max_resolution()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_level_is_tinted_by_its_own_resolution() {
        let mips = texture_mips(1);
        assert_eq!(mips.len(), 11);
        assert_eq!((mips[0].0, mips[0].1), (1024, 1024));
        assert_eq!(mips.last().map(|m| (m.0, m.1)), Some((1, 1)));
        // a texel inside a cell (not on a grid line) of each level
        let at = |level: usize| {
            let (side, _, px) = &mips[level];
            let i = ((side / 32 + 1) * side + side / 32 + 1) as usize * 4;
            [px[i], px[i + 1], px[i + 2]]
        };
        assert_eq!(at(0), tint(1024));
        assert_eq!(at(1), tint(512));
        assert_eq!(at(2), tint(256));
        assert_ne!(tint(2048), tint(1024));
        assert_eq!(texture_mips(0)[0].0, 2048);
    }

    #[test]
    fn only_the_boards_textures_are_the_scenarios() {
        for i in 0..BOARDS.len() as u32 {
            assert_eq!(texture_index(&texture(i)), Some(i));
        }
        assert_eq!(texture_index(&texture(BOARDS.len() as u32)), None);
        assert_eq!(texture_index(&Uuid::from_u128(7)), None);
    }
}
