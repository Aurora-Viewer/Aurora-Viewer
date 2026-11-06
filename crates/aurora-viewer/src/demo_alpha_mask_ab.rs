//! Offline layered-alpha benchmark for issue #1696.
//! AURORA_DEMO_ALPHA_MASK_AB=planes|cross|spheres|none:<layers>:blend|mask[:touch].
use super::{HANDLE, height, prim, shape};
use aurora_net::NetEvent;
use aurora_prim::params::*;
use aurora_prim::{ExtraParams, TextureEntry};
use glam::{Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

const TEX_BLEND: Uuid = Uuid::from_u128(0xDE40_AB16_9600_0000_0000_0000_0000_0001);
const TEX_MASK: Uuid = Uuid::from_u128(0xDE40_AB16_9600_0000_0000_0000_0000_0002);
pub fn events() -> Vec<NetEvent> {
    let Ok(spec) = std::env::var("AURORA_DEMO_ALPHA_MASK_AB") else {
        return Vec::new();
    };
    let parts: Vec<_> = spec.split(':').collect();
    if parts.first() == Some(&"none") {
        log::info!("alpha mask AB: scene={spec}, existing demo geometry only");
        return Vec::new();
    }
    let layers = parts.get(1).and_then(|n| n.parse::<u32>().ok()).unwrap_or(64).clamp(1, 512);
    let texture = if parts.get(2) == Some(&"mask") { TEX_MASK } else { TEX_BLEND };
    let curved = parts.first() == Some(&"spheres");
    let crossing = parts.first() == Some(&"cross");
    let touch = parts.get(3) == Some(&"touch");
    let look = Vec3::new(0.8, 0.4, 0.0).normalize();
    let across = Vec3::new(-look.y, look.x, 0.0);
    let origin = Vec3::new(132.0, 126.0, 0.0);
    let mut objects = Vec::new();
    for k in 0..layers {
        let p = origin + look * (8.0 + k as f32 * 0.06) + across * ((k % 3) as f32 - 1.0) * 0.1;
        let mut te = TextureEntry::default();
        for f in &mut te.faces {
            f.texture = texture;
            f.color = match k % 3 {
                0 => [0.95, 0.45, 0.35, 1.0],
                1 => [0.35, 0.95, 0.45, 1.0],
                _ => [0.45, 0.35, 0.95, 1.0],
            };
            f.offset_s = k as f32 * 0.023;
        }
        let volume = if curved {
            shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, 0, 0)
        } else {
            shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0)
        };
        let mut o = prim(
            96_000 + k,
            Vec3::new(p.x, p.y, height(p.x, p.y) + 4.0),
            Quat::from_rotation_z(look.y.atan2(look.x) + if crossing { ((k % 3) as f32 - 1.0) * 0.45 } else { 0.0 }),
            Vec3::new(0.035, 18.0, 18.0),
            volume,
            Arc::new(te),
            ExtraParams::default(),
            "",
        );
        if touch {
            o.update_flags |= 1 << 7;
        }
        objects.push(o);
    }
    log::info!("alpha mask AB: scene={spec} layers={layers} touch={touch}");
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

pub fn texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    if *id != TEX_BLEND && *id != TEX_MASK {
        return None;
    }
    let mut rgba = Vec::with_capacity(64 * 64 * 4);
    for _y in 0..64 {
        for x in 0..64 {
            let alpha = if x % 16 < 4 { if *id == TEX_MASK { 255 } else { 160 } } else { 0 };
            rgba.extend_from_slice(&[150, 190, 230, alpha]);
        }
    }
    Some((rgba, 64, 64))
}
