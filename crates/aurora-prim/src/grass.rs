//! Linden grass (pcode `LL_PCODE_LEGACY_GRASS`): the species of
//! `app_settings/grass.xml` and the tufts of blades.
//!
//! Port of LLVOGrass::initClass and getGeometry (indra/newview/llvograss.cpp,
//! originally LGPL 2.1): 32 blades spread around the object's position with
//! a normal distribution scaled by its X and Y size, each a quad standing on
//! the ground (the base of each side at the terrain height under it), drawn
//! from both sides. The object's rotation and height are not used.

use glam::Vec3;
use uuid::Uuid;

use crate::noise::MsvcRand;
use crate::volume::{VolumeFace, VolumeMesh};

/// GRASS_MAX_BLADES.
pub const GRASS_BLADES: usize = 32;
/// Width of a blade at its base (GRASS_BLADE_BASE).
const BLADE_BASE: f32 = 0.25;
/// Height of a blade in metres before the species' size (GRASS_BLADE_HEIGHT).
const BLADE_HEIGHT: f32 = 0.5;
/// Spread of the blades over the object's size (GRASS_DISTRIBUTION_SD,
/// "empirically defined").
const DISTRIBUTION_SD: f32 = 0.15;

/// One grass species of grass.xml (LLVOGrass::GrassSpeciesData).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GrassSpecies {
    pub name: &'static str,
    pub texture: Uuid,
    pub blade_size_x: f32,
    pub blade_size_y: f32,
}

const fn grass(name: &'static str, texture: &str, blade_size_x: f32, blade_size_y: f32) -> GrassSpecies {
    GrassSpecies {
        name,
        texture: match Uuid::try_parse(texture) {
            Ok(u) => u,
            Err(_) => panic!("bad species texture id"),
        },
        blade_size_x,
        blade_size_y,
    }
}

/// grass.xml, by species id (the object's State byte).
pub const GRASS_SPECIES: [GrassSpecies; 6] = [
    grass("Grass 0", "6c4727b8-ac79-ba44-3b81-f9aa887b47eb", 1.35, 1.35),
    grass("Grass 1", "79504bf5-c3ec-0763-6563-d843de66d0a1", 1.0, 0.66),
    grass("Grass 2", "6c4727b8-ac79-ba44-3b81-f9aa887b47eb", 1.8, 1.8),
    grass("Grass 3", "99bd60a2-3250-efc9-2e39-2fbcadefbecc", 1.0, 1.0),
    grass("Grass 4", "7a2b3a4a-53c2-53ac-5716-aac7d743c020", 2.25, 2.25),
    grass("undergrowth_1", "8f458549-173b-23ff-d4ff-bfaa5ea2371b", 2.0, 2.0),
];

/// The species a grass object is drawn as: its State byte
/// (LLVOGrass::updateSpecies, getAttachmentState), else the first.
pub fn grass_species(state: u8) -> &'static GrassSpecies {
    GRASS_SPECIES.get(state as usize).unwrap_or(&GRASS_SPECIES[0])
}

/// Where a blade grows, shared by every grass object (initClass).
#[derive(Debug, Clone, Copy)]
struct Blade {
    x: f32,
    y: f32,
    rot_x: f32,
    rot_y: f32,
    dz_x: f32,
    dz_y: f32,
    /// How much of the blade size this one gets (also its wind share).
    w_mod: f32,
}

/// The blade table. The Legacy Viewer draws it from its session's random
/// generator, so its tufts differ from one session to the next; here it is
/// drawn once from a fixed seed, the same in every session.
fn blades() -> &'static [Blade; GRASS_BLADES] {
    static B: std::sync::OnceLock<[Blade; GRASS_BLADES]> = std::sync::OnceLock::new();
    B.get_or_init(|| {
        let mut r = MsvcRand::new(0x5F);
        // ll_frand: [0, 1)
        let mut frand = || r.next() as f32 / 32768.0;
        std::array::from_fn(|_| {
            // Box-Muller (kept off log(0))
            let u = (-2.0 * (1.0 - frand()).ln()).sqrt();
            let v = 2.0 * std::f32::consts::PI * frand();
            let x = u * v.sin() * DISTRIBUTION_SD;
            let y = u * v.cos() * DISTRIBUTION_SD;
            let rot = frand() * std::f32::consts::PI;
            Blade {
                x,
                y,
                rot_x: rot.sin(),
                rot_y: rot.cos(),
                dz_x: frand() * BLADE_BASE * 0.25,
                dz_y: frand() * BLADE_BASE * 0.25,
                w_mod: 0.5 + frand(),
            }
        })
    })
}

/// The two base points of each blade, relative to the object's position
/// (X, Y in metres, region axes): where [`generate_grass`] wants the ground
/// height, in this order.
pub fn grass_bases(s: &GrassSpecies, scale: Vec3) -> [[f32; 2]; GRASS_BLADES * 2] {
    let mut out = [[0.0; 2]; GRASS_BLADES * 2];
    for (i, b) in blades().iter().enumerate() {
        let x = b.x * scale.x;
        let y = b.y * scale.y;
        let xf = b.rot_x * BLADE_BASE * s.blade_size_x * b.w_mod;
        let yf = b.rot_y * BLADE_BASE * s.blade_size_x * b.w_mod;
        out[i * 2] = [x + xf, y + yf];
        // getGeometry offsets the second base by -xf on both axes (its
        // "y - xf"): kept, it shapes the blades the Legacy Viewer draws
        out[i * 2 + 1] = [x - xf, y - xf];
    }
    out
}

/// A grass tuft relative to the object's position: `ground` holds the
/// terrain height under each of [`grass_bases`] minus the object's height.
/// Each blade is written twice, front and back with their own normals.
pub fn generate_grass(s: &GrassSpecies, scale: Vec3, ground: &[f32; GRASS_BLADES * 2]) -> VolumeMesh {
    let bases = grass_bases(s, scale);
    let mut f = VolumeFace::default();
    for (i, b) in blades().iter().enumerate() {
        let height = BLADE_HEIGHT * s.blade_size_y * b.w_mod;
        let lift = Vec3::new(b.dz_x, b.dz_y, height);
        let v1 = Vec3::new(bases[i * 2][0], bases[i * 2][1], ground[i * 2]);
        let v2 = v1 + lift;
        let v3 = Vec3::new(bases[i * 2 + 1][0], bases[i * 2 + 1][1], ground[i * 2 + 1]);
        let v4 = v3 + lift;
        let mut n1 = (v1 - v2).cross(v2 - v3);
        n1.z = 0.75;
        let n1 = n1.normalize_or(Vec3::Z);
        let n2 = Vec3::new(-n1.x, -n1.y, n1.z);
        let base = f.positions.len() as u16;
        for (p, u, v) in [(v1, 0.0, 0.0), (v2, 0.0, 0.98), (v3, 1.0, 0.0), (v4, 1.0, 0.98)] {
            for n in [n1, n2] {
                f.positions.push(p.to_array());
                f.normals.push(n.to_array());
                f.uvs.push([u, v]);
            }
        }
        f.indices.extend([0, 2, 4, 2, 6, 4, 1, 5, 3, 3, 5, 7].into_iter().map(|k| base + k));
    }
    VolumeMesh {
        faces: vec![f],
        surface_area: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(h: f32) -> [f32; GRASS_BLADES * 2] {
        [h; GRASS_BLADES * 2]
    }

    #[test]
    fn a_tuft_has_32_two_sided_blades() {
        let mesh = generate_grass(&GRASS_SPECIES[0], Vec3::new(4.0, 4.0, 1.0), &flat(0.0));
        let f = &mesh.faces[0];
        assert_eq!(f.positions.len(), GRASS_BLADES * 8);
        assert_eq!(f.indices.len(), GRASS_BLADES * 12);
        assert!(f.indices.iter().all(|&i| (i as usize) < f.positions.len()));
        // the back of each blade: same corners, mirrored horizontal normal
        for b in 0..GRASS_BLADES {
            let (n1, n2) = (f.normals[b * 8], f.normals[b * 8 + 1]);
            assert_eq!(f.positions[b * 8], f.positions[b * 8 + 1]);
            assert_eq!([n1[0], n1[1], n1[2]], [-n2[0], -n2[1], n2[2]]);
            assert!(n1[2] > 0.0);
        }
    }

    #[test]
    fn blades_stand_on_the_ground_and_grow_with_the_species() {
        let ground: [f32; GRASS_BLADES * 2] = std::array::from_fn(|i| i as f32 * 0.01 - 0.3);
        let s = &GRASS_SPECIES[4];
        let mesh = generate_grass(s, Vec3::new(2.0, 3.0, 1.0), &ground);
        let f = &mesh.faces[0];
        for b in 0..GRASS_BLADES {
            assert_eq!(f.positions[b * 8][2], ground[b * 2]);
            assert_eq!(f.positions[b * 8 + 4][2], ground[b * 2 + 1]);
            let tall = f.positions[b * 8 + 2][2] - ground[b * 2];
            // 0.5 m × size × (0.5 to 1.5)
            assert!(tall > 0.5 * 2.25 * 0.5 - 1e-4 && tall < 0.5 * 2.25 * 1.5 + 1e-4, "{tall}");
        }
    }

    #[test]
    fn the_tuft_spreads_with_the_object_size() {
        let s = &GRASS_SPECIES[0];
        let spread = |scale: Vec3| {
            grass_bases(s, scale)
                .iter()
                .map(|p| (p[0] * p[0] + p[1] * p[1]).sqrt())
                .fold(0.0f32, f32::max)
        };
        let small = spread(Vec3::new(1.0, 1.0, 1.0));
        let big = spread(Vec3::new(10.0, 10.0, 1.0));
        assert!(big > 3.0 * small, "{small} {big}");
        // one standard deviation is 0.15 of the size: most blades within
        // half of it
        let inside = grass_bases(s, Vec3::new(10.0, 10.0, 1.0))
            .iter()
            .filter(|p| p[0].abs() < 5.0 && p[1].abs() < 5.0)
            .count();
        assert!(inside > GRASS_BLADES * 2 * 3 / 4);
    }

    #[test]
    fn the_blades_are_the_same_every_time() {
        let a = generate_grass(&GRASS_SPECIES[1], Vec3::splat(3.0), &flat(0.5));
        let b = generate_grass(&GRASS_SPECIES[1], Vec3::splat(3.0), &flat(0.5));
        assert_eq!(a, b);
    }

    #[test]
    fn unknown_species_are_the_first() {
        assert_eq!(grass_species(9).name, "Grass 0");
        assert_eq!(grass_species(5).name, "undergrowth_1");
    }
}
