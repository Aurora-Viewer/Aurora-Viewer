//! AURORA_DEMO_PLANTS: Linden trees and grass (scene/plants.rs).
//!
//! - `1` / `near`: the 21 tree species in three rows 26, 42 and 58 m along
//!   the start view (species 0 to 6 in the nearest row, left to right), and
//!   the 6 grass species in a row 8 m ahead;
//! - `far`: the same rows 30 m further and 30 m to the right (clear of the
//!   plaza from "80,100"), 56 to 88 m away: within the 96 m draw distance;
//! - `dense`: about 600 trees of every species and 400 grass tufts over
//!   the land around and ahead of the start, for the cost of a forest.
//!
//! From the start position, or AURORA_DEMO_POS="x,y" (an open place:
//! "80,100"). The species textures are not fetched: stand-ins drawn here
//! under their ids (bark on the left half, leaves in the top right
//! quarter, as the species textures are laid out).

use super::{HANDLE, height, prim};
use aurora_net::NetEvent;
use aurora_net::objects::ObjectUpdate;
use aurora_prim::params::{LL_PCODE_LEGACY_GRASS, LL_PCODE_LEGACY_TREE};
use aurora_prim::{ExtraParams, TextureEntry, VolumeParams, grass, tree};
use glam::{Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

const FIRST_ID: u32 = 82_000;
/// Object scale of the demo trees: a mesh scale of 0.05 × |(6, 6, 6)| =
/// 0.52 (an oak about 4.6 m tall, the tallest pine 19 m).
const TREE_SCALE: f32 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Near,
    Far,
    Dense,
}

fn mode() -> Option<Mode> {
    match std::env::var("AURORA_DEMO_PLANTS").ok()?.trim() {
        "1" | "near" => Some(Mode::Near),
        "far" => Some(Mode::Far),
        "dense" => Some(Mode::Dense),
        _ => None,
    }
}

fn plant(id: u32, pcode: u8, species: u8, pos: Vec3, rot: Quat, scale: Vec3) -> ObjectUpdate {
    let mut o = prim(
        id,
        pos,
        rot,
        scale,
        VolumeParams::default(),
        Arc::new(TextureEntry::default()),
        ExtraParams::default(),
        "",
    );
    o.pcode = pcode;
    if pcode == LL_PCODE_LEGACY_TREE {
        o.tree_species = Some(species);
    } else {
        // grass: the species is the State byte
        o.state = species;
    }
    o
}

/// A tiny integer hash in [0, 1): the same scene in every run.
fn unit(k: u32, salt: u32) -> f32 {
    let mut h = k.wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h % 10_000) as f32 / 10_000.0
}

pub fn events() -> Vec<NetEvent> {
    let Some(mode) = mode() else {
        return Vec::new();
    };
    let look = Vec3::new(0.8, 0.4, 0.0).normalize();
    let across = Vec3::new(-look.y, look.x, 0.0);
    let eye = std::env::var("AURORA_DEMO_POS")
        .ok()
        .map(|v| v.split(',').filter_map(|s| s.trim().parse::<f32>().ok()).collect::<Vec<_>>())
        .filter(|p| p.len() >= 2)
        .map_or(Vec3::new(132.0, 126.0, 0.0), |p| Vec3::new(p[0], p[1], 0.0));
    let ground = |p: Vec3| Vec3::new(p.x, p.y, height(p.x, p.y));
    let mut objects = Vec::new();
    let mut id = FIRST_ID;
    match mode {
        Mode::Near | Mode::Far => {
            let (far, right) = if mode == Mode::Far { (30.0, -30.0) } else { (0.0, 0.0) };
            for (i, _) in tree::TREE_SPECIES.iter().enumerate() {
                let (row, col) = (i / 7, i % 7);
                let p = eye + look * (26.0 + 16.0 * row as f32 + far) + across * ((col as f32 - 3.0) * 8.0 + right);
                objects.push(plant(
                    id,
                    LL_PCODE_LEGACY_TREE,
                    i as u8,
                    ground(p),
                    Quat::IDENTITY,
                    Vec3::splat(TREE_SCALE),
                ));
                id += 1;
            }
            for (i, _) in grass::GRASS_SPECIES.iter().enumerate() {
                let p = eye + look * (8.0 + far) + across * ((i as f32 - 2.5) * 4.5 + right);
                objects.push(plant(
                    id,
                    LL_PCODE_LEGACY_GRASS,
                    i as u8,
                    ground(p) + Vec3::Z * 0.5,
                    Quat::IDENTITY,
                    Vec3::new(4.0, 4.0, 1.0),
                ));
                id += 1;
            }
        }
        Mode::Dense => {
            // a 7 m grid jittered, from 8 m around the start to 170 m ahead,
            // on land only (the lagoon is under 20.5 m)
            let mut trees = 0;
            for gy in -12..12 {
                for gx in 0..25 {
                    let k = (gy + 12) as u32 * 25 + gx as u32;
                    let p = eye + look * (gx as f32 * 7.0 + unit(k, 1) * 5.0 - 2.0) + across * (gy as f32 * 7.0 + unit(k, 2) * 5.0);
                    if p.truncate().distance(eye.truncate()) < 8.0
                        || !(1.0..255.0).contains(&p.x)
                        || !(1.0..255.0).contains(&p.y)
                        || height(p.x, p.y) < 20.5
                    {
                        continue;
                    }
                    let species = (k * 7 + gx as u32) as usize % tree::TREE_SPECIES.len();
                    let s = TREE_SCALE * (0.7 + 0.6 * unit(k, 3));
                    objects.push(plant(
                        id,
                        LL_PCODE_LEGACY_TREE,
                        species as u8,
                        ground(p),
                        Quat::from_rotation_z(unit(k, 4) * std::f32::consts::TAU),
                        Vec3::splat(s),
                    ));
                    id += 1;
                    trees += 1;
                    // a grass tuft by two trees out of three
                    if !k.is_multiple_of(3) {
                        let q = p + across * 3.0 + look * 2.0;
                        objects.push(plant(
                            id,
                            LL_PCODE_LEGACY_GRASS,
                            (k % 6) as u8,
                            ground(q) + Vec3::Z * 0.5,
                            Quat::IDENTITY,
                            Vec3::new(5.0, 5.0, 1.0),
                        ));
                        id += 1;
                    }
                }
            }
            log::info!("demo plants: dense, {trees} trees and {} grass tufts", objects.len() - trees);
        }
    }
    if mode != Mode::Dense {
        log::info!("demo plants: {mode:?}, {} trees and grass", objects.len());
    }
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

/// How a species' stand-in texture is drawn.
#[derive(Clone, Copy)]
enum Look {
    /// Round crowns of small leaves.
    Broadleaf,
    /// Layers of needles narrowing to the top.
    Conifer,
    /// Long fronds from a centre.
    Frond,
    /// Grass blades rising from the bottom edge.
    Blades,
}

/// The stand-in drawn for a species texture id: its look, leaf and bark
/// colours.
fn stand_in(id: &Uuid) -> Option<(Look, [u8; 3], [u8; 3])> {
    let t = tree::TREE_SPECIES.iter().position(|s| s.texture == *id);
    if let Some(i) = t {
        let bark = [96, 72, 52];
        return Some(match i {
            // pines, cypresses
            0 | 7 | 8 | 9 => (Look::Conifer, [46, 92, 52], bark),
            // winter pines and aspen: snowy
            11 | 13 => (Look::Conifer, [196, 214, 220], [90, 80, 74]),
            12 => (Look::Broadleaf, [214, 222, 226], [170, 164, 150]),
            // palms, fern, sea plants, beach grass
            3 | 6 => (Look::Frond, [74, 132, 60], [140, 112, 80]),
            15 => (Look::Frond, [60, 140, 64], bark),
            16 | 17 | 18 | 20 => (Look::Frond, [88, 120, 52], [70, 90, 46]),
            19 => (Look::Frond, [168, 170, 102], bark),
            // plumeria, eucalyptus grey-green
            10 => (Look::Broadleaf, [84, 128, 60], bark),
            14 => (Look::Broadleaf, [106, 140, 112], [170, 150, 128]),
            _ => (Look::Broadleaf, [64, 118, 46], bark),
        });
    }
    let g = grass::GRASS_SPECIES.iter().position(|s| s.texture == *id)?;
    let leaf = [
        [92, 140, 56],
        [120, 150, 66],
        [92, 140, 56],
        [80, 120, 60],
        [140, 150, 80],
        [70, 110, 50],
    ][g];
    Some((Look::Blades, leaf, leaf))
}

/// The stand-ins replace the species textures only in the offline demo
/// with this scenario: they are real asset ids on a grid.
fn active() -> bool {
    static ACTIVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ACTIVE.get_or_init(|| std::env::var_os("AURORA_DEMO").is_some() && mode().is_some())
}

/// Whether `id` is a stand-in species texture (they get mips).
pub fn is_texture(id: &Uuid) -> bool {
    active() && stand_in(id).is_some()
}

/// Pixels of a species texture's stand-in (RGBA8, 128 × 128, top row
/// first: image top is texture v = 1), in the demo with this scenario.
pub fn texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    if !active() {
        return None;
    }
    draw(id)
}

fn draw(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    const N: u32 = 128;
    let (look, leaf, bark) = stand_in(id)?;
    let salt = id.as_u128() as u32;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let shade = |k: u32| 0.75 + 0.5 * unit(x / 3 + (y / 3) * 64 + k, salt);
            let tint = |c: [u8; 3], s: f32| [c[0] as f32 * s, c[1] as f32 * s, c[2] as f32 * s].map(|v| v.min(255.0) as u8);
            let mut texel = [0u8; 4];
            let (u, v) = ((x as f32 + 0.5) / N as f32, 1.0 - (y as f32 + 0.5) / N as f32);
            let on = match look {
                Look::Blades => {
                    // blades 3 texels wide every 8, leaning, of varied height
                    let lean = x as f32 + (1.0 - v) * 6.0 * (unit(x / 8, 9) - 0.5);
                    (lean % 8.0) < 3.0 && v < 0.35 + 0.6 * unit(x / 8, salt)
                }
                _ if u < 0.5 => {
                    // bark: vertical grain
                    let grain = 0.8 + 0.4 * unit(x / 2, salt ^ 7) * shade(3);
                    let c = tint(bark, grain);
                    texel = [c[0], c[1], c[2], 255];
                    false
                }
                _ if v < 0.5 => false,
                // the leaves' quarter, in its own coordinates (0..1)
                look => {
                    let (lu, lv) = ((u - 0.5) * 2.0, (v - 0.5) * 2.0);
                    let (dx, dy) = (lu - 0.5, lv - 0.45);
                    let cell = (x / 3) + (y / 3) * 64;
                    let leafy = unit(cell, salt) < 0.7;
                    match look {
                        Look::Broadleaf => dx * dx + dy * dy < 0.2 && leafy,
                        Look::Conifer => dx.abs() < 0.45 * (1.0 - lv) && leafy && (lv * 9.0).fract() < 0.8,
                        _ => {
                            // fronds: a few thin rays from the top centre
                            let a = dy.atan2(dx);
                            let r = (dx * dx + dy * dy).sqrt();
                            let ray = ((a * 7.0 / std::f32::consts::PI + 0.5).fract() - 0.5).abs();
                            r < 0.5 && ray < 0.12 + 0.1 * r
                        }
                    }
                }
            };
            if on {
                // a few pale flowers on the plumeria
                let flower = matches!(look, Look::Broadleaf) && unit(x / 4 + (y / 4) * 32, salt ^ 5) < 0.04;
                let c = if flower && leaf == [84, 128, 60] {
                    [236, 230, 196]
                } else {
                    tint(leaf, shade(1))
                };
                texel = [c[0], c[1], c[2], 255];
            }
            px.extend_from_slice(&texel);
        }
    }
    Some((px, N, N))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::jobs::{AlphaKind, classify_alpha};

    #[test]
    fn every_species_has_a_masked_stand_in() {
        for s in &tree::TREE_SPECIES {
            let (px, w, h) = draw(&s.texture).expect("tree stand-in");
            assert_eq!((w, h, px.len()), (128, 128, 128 * 128 * 4));
            assert_eq!(classify_alpha(&px), AlphaKind::Mask, "{}", s.name);
            // the bark half is opaque, the lower right quarter empty
            let alpha = |x: u32, y: u32| px[((y * w + x) * 4 + 3) as usize];
            assert_eq!(alpha(10, 100), 255);
            assert_eq!(alpha(100, 120), 0);
        }
        for s in &grass::GRASS_SPECIES {
            let (px, _, _) = draw(&s.texture).expect("grass stand-in");
            assert_eq!(classify_alpha(&px), AlphaKind::Mask, "{}", s.name);
        }
        assert!(draw(&Uuid::nil()).is_none());
        // never outside the demo: they are real asset ids
        assert!(texture(&tree::TREE_SPECIES[0].texture).is_none());
    }
}
