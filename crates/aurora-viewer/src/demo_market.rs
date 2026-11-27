//! A market for Area Search and Inspect (AURORA_DEMO_AREA_SEARCH=market, or
//! AURORA_DEMO_INSPECT=market): furniture of several prims around the plaza,
//! of several owners, creators, land impacts and kinds.

use super::{prim, shape, te};
use aurora_net::objects::ObjectUpdate;
use aurora_prim::ExtraParams;
use aurora_prim::params::{LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, LL_PCODE_PROFILE_SQUARE};
use glam::{Quat, Vec3};
use uuid::Uuid;

const FIRST: u32 = 60_000;
const COUNT: u32 = 360;
/// Ids a piece of furniture may take: its root and its linked prims.
const STRIDE: u32 = 12;

const STYLES: [&str; 8] = ["Sunset", "Cozy", "Linen", "Neon", "Paper", "Driftwood", "Harbour", "Velvet"];
const KINDS: [&str; 4] = ["Bench", "Lamp", "Shelf", "Table"];

/// The first pieces' roots, for Inspect on objects of several owners.
pub fn some() -> Vec<u32> {
    (0..8).map(|i| FIRST + i * STRIDE).collect()
}

pub fn enabled() -> bool {
    ["AURORA_DEMO_AREA_SEARCH", "AURORA_DEMO_INSPECT"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .any(|v| v.split([',', ':']).any(|o| o == "market" || o == "piece"))
}

/// A number that looks random, the same at each launch.
fn mix(n: u32, salt: u32) -> u32 {
    let mut h = n.wrapping_mul(0x9E37_79B1) ^ salt.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^ (h >> 12)
}

fn unit(n: u32, salt: u32) -> f32 {
    (mix(n, salt) % 10_000) as f32 / 10_000.0
}

/// The boxes of a piece: position and size, the root first.
fn parts(kind: u32) -> Vec<(Vec3, Vec3, bool)> {
    let v = Vec3::new;
    let legs = |x: f32, y: f32, h: f32| {
        [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)].map(|(a, b)| (v(a * x, b * y, -h / 2.0), v(0.08, 0.08, h), false))
    };
    match kind {
        0 => {
            let mut out = vec![(Vec3::ZERO, v(0.5, 1.6, 0.1), false)];
            out.extend(legs(0.2, 0.7, 0.45));
            out.extend((0..4).map(|i| (v(-0.22, 0.0, 0.16 + 0.14 * i as f32), v(0.05, 1.6, 0.09), false)));
            out
        }
        1 => vec![
            (Vec3::ZERO, v(0.35, 0.35, 0.08), true),
            (v(0.0, 0.0, 0.8), v(0.06, 0.06, 1.6), true),
            (v(0.0, 0.0, 1.7), v(0.4, 0.4, 0.35), true),
        ],
        2 => {
            let mut out = vec![(Vec3::ZERO, v(0.4, 1.0, 0.05), false)];
            out.extend((1..4).map(|i| (v(0.0, 0.0, 0.45 * i as f32), v(0.4, 1.0, 0.05), false)));
            out.extend([
                (v(0.0, 0.5, 0.68), v(0.4, 0.05, 1.4), false),
                (v(0.0, -0.5, 0.68), v(0.4, 0.05, 1.4), false),
            ]);
            out
        }
        _ => {
            let mut out = vec![(Vec3::ZERO, v(1.0, 1.0, 0.07), true)];
            out.extend(legs(0.3, 0.3, 0.7));
            out
        }
    }
}

/// The market's prims around `center`, on the ground `height` gives.
pub fn objects(center: Vec3, height: &dyn Fn(f32, f32) -> f32) -> Vec<ObjectUpdate> {
    let mut out = Vec::new();
    for i in 0..COUNT {
        let (angle, far) = (unit(i, 1) * std::f32::consts::TAU, unit(i, 2));
        let distance = 4.0 + far * far * 120.0;
        let (x, y) = (center.x + angle.cos() * distance, center.y + angle.sin() * distance);
        let (x, y) = (x.clamp(2.0, 254.0), y.clamp(2.0, 254.0));
        let ground = Vec3::new(x, y, height(x, y).max(20.0) + 0.5);
        let shade = |salt| 0.25 + 0.6 * unit(i, salt);
        let color = [shade(3), shade(4), shade(5), 1.0];
        let root = FIRST + i * STRIDE;
        for (n, (at, size, round)) in parts(mix(i, 6) % 4).into_iter().enumerate() {
            let volume = if round {
                shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, 0, 0)
            } else {
                shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0)
            };
            let turn = if n == 0 {
                Quat::from_rotation_z(unit(i, 7) * std::f32::consts::TAU)
            } else {
                Quat::IDENTITY
            };
            let look = te(color, 0, false, 0.0);
            let id = root + n as u32;
            let mut o = prim(
                id,
                if n == 0 { ground } else { at },
                turn,
                size,
                volume,
                look,
                ExtraParams::default(),
                "",
            );
            o.parent_id = if n == 0 { 0 } else { root };
            match mix(i, 8) % 12 {
                0 => o.update_flags |= 1,
                1 => o.update_flags |= 1 << 10,
                2 => o.update_flags |= 1 << 29,
                _ => {}
            }
            out.push(o);
        }
    }
    out
}

/// Name, owner, creator and the group that owns, by local id.
pub fn props(local_id: u32) -> Option<(String, Uuid, Uuid, Option<Uuid>)> {
    if !enabled() || !(FIRST..FIRST + COUNT * STRIDE).contains(&local_id) {
        return None;
    }
    let (i, part) = ((local_id - FIRST) / STRIDE, (local_id - FIRST) % STRIDE);
    let name = if part == 0 {
        let kind = KINDS[(mix(i, 6) % 4) as usize];
        format!("{} {kind}", STYLES[(mix(i, 9) % 8) as usize])
    } else {
        format!("Part {part}")
    };
    let who = |n: u32| match n {
        0 => super::DEMO_NOVA,
        1 => super::DEMO_TESS,
        2 => super::DEMO_FRIEND,
        3 => super::DEMO_LOUP,
        _ => super::DEMO_AGENT,
    };
    let owner = mix(i, 10) % 9;
    let group = (owner >= 7).then_some(super::DEMO_GROUP1);
    Some((name, who(owner.min(4)), who(mix(i, 11) % 4), group))
}

/// The land impact of a piece, by the local id of its root: a few heavy ones.
pub fn land_impact(local_id: u32) -> Option<f64> {
    if !enabled() || !(FIRST..FIRST + COUNT * STRIDE).contains(&local_id) {
        return None;
    }
    let i = (local_id - FIRST) / STRIDE;
    let n = 1 + mix(i, 12) % 9;
    Some(f64::from(match mix(i, 13) % 12 {
        0 => n * 12 + 20,
        1..=3 => n * 3 + 6,
        _ => n,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_market_s_ids_do_not_meet() {
        let all = objects(Vec3::new(128.0, 128.0, 22.0), &|_, _| 21.0);
        let mut ids: Vec<u32> = all.iter().map(|o| o.local_id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count);
        assert!(all.iter().all(|o| o.parent_id == 0 || ids.binary_search(&o.parent_id).is_ok()));
        assert!((0..4).all(|k| parts(k).len() <= STRIDE as usize));
    }
}
