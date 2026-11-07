//! AURORA_DEMO_LEGACY_BUMP=1: a grid of texture-entry bump codes 0–17,
//! followed by explicit legacy-normal and PBR-normal controls. Standard
//! preset panels use synthetic height images, not the grid's preset artwork.
//! The preset
//! numbering follows `LLDrawPoolBump::bindBumpMap` and `std_bump.ini`; the
//! generated standard-map normals use `LLBumpImageList::generateNormalMapFromAlpha`
//! and `normgenF.glsl` in the Legacy Viewer.

use super::{HANDLE, floor_at, prim, shape, te};
use aurora_assets::PbrMaterial;
use aurora_net::NetEvent;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

pub const DIFFUSE: Uuid = Uuid::from_u128(0xde19_b000_0000_4000_8000_0000_0000_0001);
pub const EXPLICIT_NORMAL: Uuid = Uuid::from_u128(0xde19_b000_0000_4000_8000_0000_0000_0002);
pub const LEGACY_MATERIAL: Uuid = Uuid::from_u128(0xde19_3a70_0000_4000_8000_0000_0000_0001);
pub const PBR_MATERIAL: Uuid = Uuid::from_u128(0xde19_3a70_0000_4000_8000_0000_0000_0002);
const FIRST_LOCAL_ID: u32 = 91_700;
const ARRIVE_FRAME: u64 = 180;
const PANEL: f32 = 1.55;

pub fn enabled() -> bool {
    static ENABLED: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        std::env::var("AURORA_DEMO").is_ok_and(|v| v.trim() == "1")
            && std::env::var("AURORA_DEMO_LEGACY_BUMP").is_ok_and(|v| v.trim() == "1")
    });
    *ENABLED
}

pub fn texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    if !enabled() {
        return None;
    }
    match *id {
        DIFFUSE => Some(diffuse()),
        EXPLICIT_NORMAL => Some(explicit_normal()),
        _ => (3..=17)
            .find(|&code| matches!(crate::scene::legacy_bump::bump_source(code), crate::scene::legacy_bump::BumpSource::Standard(source) if source == *id))
            .map(preset_height),
    }
}

/// Stand-in height images let the offline demo exercise the same standard
/// preset UUID path without contacting a simulator or asset service.
fn preset_height(code: u8) -> (Vec<u8>, u32, u32) {
    const N: usize = 128;
    let mut rgba = Vec::with_capacity(N * N * 4);
    for y in 0..N {
        for x in 0..N {
            let (u, v) = (x as f32 / N as f32, y as f32 / N as f32);
            let wave = match code {
                3 => (u * 22.0 + (v * 8.0).sin()).sin(),
                4 => (u * 15.0).sin() * (v * 6.0).sin(),
                5 | 9 | 14 => {
                    if (x / 24 + y / 24) % 2 == 0 {
                        0.75
                    } else {
                        -0.75
                    }
                }
                6 | 10 | 16 => (u * 18.0).sin() * (v * 18.0).sin(),
                7 | 15 => ((x * 17 + y * 31 + code as usize * 11) % 23) as f32 / 11.5 - 1.0,
                8 | 11 => (((x * 7) ^ (y * 19)) % 29) as f32 / 14.5 - 1.0,
                12 => ((u - 0.5).hypot(v - 0.5) * 50.0).sin(),
                13 => (v * 48.0).sin(),
                17 => (u * 22.0).sin() * (v * 4.0).sin(),
                _ => 0.0,
            };
            let height = (128.0 + wave * 90.0).round().clamp(0.0, 255.0) as u8;
            rgba.extend_from_slice(&[255, 255, 255, height]);
        }
    }
    (rgba, N as u32, N as u32)
}

fn diffuse() -> (Vec<u8>, u32, u32) {
    const N: usize = 128;
    let mut rgba = Vec::with_capacity(N * N * 4);
    for y in 0..N {
        for x in 0..N {
            let wave = ((x as f32 * 0.22).sin() * (y as f32 * 0.22).sin() * 32.0) as i16;
            let mortar = x % 32 < 2 || y % 32 < 2;
            let value = (145 + wave - if mortar { 65 } else { 0 }).clamp(0, 255) as u8;
            rgba.extend_from_slice(&[value, value.saturating_add(8), value.saturating_add(14), 255]);
        }
    }
    (rgba, N as u32, N as u32)
}

fn explicit_normal() -> (Vec<u8>, u32, u32) {
    const N: usize = 128;
    let heights: Vec<u8> = (0..N * N)
        .map(|i| {
            let (x, y) = ((i % N) as f32, (i / N) as f32);
            (128.0 + 70.0 * (x * 0.2).sin() * (y * 0.2).sin()).round() as u8
        })
        .collect();
    let pixels = crate::scene::legacy_bump::normal_from_height(N, N, &heights, 64.0)
        .expect("fixed demo dimensions")
        .into_iter()
        .flatten()
        .collect();
    (pixels, N as u32, N as u32)
}

pub fn legacy_material() -> crate::scene::legacy_mat::LegacyMaterial {
    crate::scene::legacy_mat::LegacyMaterial {
        normal_map: EXPLICIT_NORMAL,
        normal_st: [1.0, 1.0, 0.0, 0.0],
        normal_rot: 0.0,
        diffuse_alpha_mode: crate::scene::legacy_mat::alpha_mode::NONE,
        ..Default::default()
    }
}

pub fn pbr_material() -> PbrMaterial {
    PbrMaterial {
        base_color_texture: Some(DIFFUSE),
        normal_texture: Some(EXPLICIT_NORMAL),
        metallic_factor: 0.0,
        roughness_factor: 0.45,
        ..Default::default()
    }
}

fn panel(code: u32, x: f32, y: f32, material: Option<Uuid>) -> aurora_net::objects::ObjectUpdate {
    let mut entry = (*te([1.0; 4], 0, false, 0.0)).clone();
    for face in &mut entry.faces {
        face.texture = DIFFUSE;
        face.scale_s = 2.0;
        face.scale_t = 2.0;
        face.bump_shiny_fullbright = (code as u8 & 0x1f) | (3 << 6);
        if let Some(material) = material {
            face.material_id = material;
        }
    }
    prim(
        FIRST_LOCAL_ID + code,
        Vec3::new(x, y, floor_at(x, y) + PANEL * 0.5 + 0.2),
        Quat::IDENTITY,
        Vec3::new(0.06, PANEL, PANEL),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        Arc::new(entry),
        ExtraParams::default(),
        "",
    )
}

pub struct LegacyBumpDemo {
    sent: bool,
}

impl LegacyBumpDemo {
    pub fn from_env() -> Option<Self> {
        enabled().then_some(Self { sent: false })
    }

    /// Inject once after the demo's initial objects. `app.rs` owns this
    /// value and calls `tick` on the main thread with the world frame.
    pub fn tick(&mut self, frame: u64, world: &mut crate::world::World) {
        if self.sent || frame < ARRIVE_FRAME {
            return;
        }
        self.sent = true;
        let mut objects = Vec::with_capacity(20);
        for code in 0..18 {
            let col = code % 6;
            let row = code / 6;
            let x = 146.0 + row as f32 * 2.4;
            let y = 121.0 + col as f32 * 2.1;
            objects.push(panel(code, x, y, None));
        }
        objects.push(panel(18, 153.2, 123.1, Some(LEGACY_MATERIAL)));
        objects.push(panel(19, 153.2, 125.2, Some(PBR_MATERIAL)));
        world.apply(NetEvent::ObjectUpdates { handle: HANDLE, objects });
        log::info!("demo legacy bump: codes 0–17, explicit legacy normal, and PBR normal controls");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_preset_fixture_does_not_replace_a_grid_asset() {
        if !enabled() {
            let crate::scene::legacy_bump::BumpSource::Standard(id) = crate::scene::legacy_bump::bump_source(9) else {
                panic!("Cut Stone preset missing");
            };
            assert!(crate::demo::local_texture(&id).is_none());
        }
    }
}
