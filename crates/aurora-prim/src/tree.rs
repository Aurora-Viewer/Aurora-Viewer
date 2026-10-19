//! Linden trees (pcode `LL_PCODE_LEGACY_TREE`): the species of
//! `app_settings/trees.xml` and their procedural mesh.
//!
//! Port of LLVOTree::initClass, updateGeometry, updateMesh and
//! genBranchPipeline (indra/newview/llvotree.cpp, originally LGPL 2.1): a
//! trunk of noisy tapered cylinders that forks recursively into branches,
//! each tip ending in two crossed, two-sided leaf quads cut out of the right
//! half of the species texture (the bark is its left half). The four detail
//! levels only change the slices of the cylinders (10, 5, 4, 3); the leaves
//! are the same at every level.
//!
//! The mesh is built in the tree's own space; the object places it with
//! [`tree_model_scale`] and the rotation `rot * Rz(90°)`, 0.1 m under its
//! position (LLVOTree::updateMesh). The wind bend of the Legacy Viewer
//! (mTrunkBend) is never fed any more, so its trees stand still: the droop
//! is the species' plus 25°.

use glam::{Mat3, Mat4, Quat, Vec3};
use uuid::Uuid;

use crate::noise::turbulence3;
use crate::volume::{VolumeFace, VolumeMesh};

/// One tree species of trees.xml (LLVOTree::TreeSpeciesData).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeSpecies {
    pub name: &'static str,
    /// Bark on the left half, leaves in the top right quarter.
    pub texture: Uuid,
    /// Degrees between a branch and its parent (plus 25 without wind).
    pub droop: f32,
    /// Degrees added to and taken from the even and odd branches' turn.
    pub twist: f32,
    /// Branches at each fork (the turn between them uses the fraction).
    pub branches: f32,
    /// Forks between the trunk and the leaves.
    pub depth: u16,
    pub scale_step: f32,
    /// Trunk segments above the first, each turned by 70.5°.
    pub trunk_depth: u16,
    pub branch_length: f32,
    pub trunk_length: f32,
    pub leaf_scale: f32,
    pub billboard_scale: f32,
    pub billboard_ratio: f32,
    pub trunk_aspect: f32,
    pub branch_aspect: f32,
    /// Bark noise amplitude.
    pub noise_mag: f32,
    /// Bark noise frequency.
    pub noise_scale: f32,
    /// Top radius of a segment over its base radius.
    pub taper: f32,
    /// Bark texture repeats along a segment.
    pub repeat_z: f32,
}

const fn uuid(s: &str) -> Uuid {
    match Uuid::try_parse(s) {
        Ok(u) => u,
        Err(_) => panic!("bad species texture id"),
    }
}

#[allow(clippy::too_many_arguments)]
const fn species(
    name: &'static str,
    texture: &str,
    droop: f32,
    twist: f32,
    branches: f32,
    depth: u16,
    scale_step: f32,
    trunk_depth: u16,
    branch_length: f32,
    trunk_length: f32,
    leaf_scale: f32,
    billboard: (f32, f32),
    aspect: (f32, f32),
    noise: (f32, f32),
    taper: f32,
    repeat_z: f32,
) -> TreeSpecies {
    TreeSpecies {
        name,
        texture: uuid(texture),
        droop,
        twist,
        branches,
        depth,
        scale_step,
        trunk_depth,
        branch_length,
        trunk_length,
        leaf_scale,
        billboard_scale: billboard.0,
        billboard_ratio: billboard.1,
        trunk_aspect: aspect.0,
        branch_aspect: aspect.1,
        noise_mag: noise.0,
        noise_scale: noise.1,
        taper,
        repeat_z,
    }
}

/// trees.xml, by species id. `trunk_depth` is read as an integer there
/// ("3.0" is 3, the fern's "0.1" is 0); `leaf_rotate` is never used.
#[rustfmt::skip]
pub const TREE_SPECIES: [TreeSpecies; 21] = [
    species("Pine 1", "0187babf-6c0d-5891-ebed-4ecab1426683", 60.0, 5.0, 5.0, 1, 0.7, 6, 8.0, 11.5, 22.0, (39.5, 1.1), (0.1, 0.05), (0.5, 2.5), 0.8, 3.0),
    species("Oak", "8a515889-eac9-fb55-8eba-d2dc09eb32c8", 35.0, 3.0, 4.0, 3, 0.7, 0, 3.0, 3.8, 7.0, (10.25, 1.0), (0.15, 0.07), (1.2, 4.0), 0.3, 4.0),
    species("Tropical Bush 1", "5bc11cd6-2f40-071e-a8da-0903394204f9", 10.0, 0.0, 6.0, 1, 0.5, 1, 0.5, 0.15, 7.5, (5.0, 1.25), (1.0, 0.08), (1.0, 1.0), 0.2, 1.0),
    species("Palm 1", "ca4e8c27-473c-eb1c-2f5d-50ee3f07d85c", 0.0, 0.0, 3.0, 1, 0.5, 0, 0.7, 9.0, 10.0, (13.25, 1.0), (0.035, 0.03), (0.2, 6.0), 0.7, 10.0),
    species("Dogwood", "64367bd1-697e-b3e6-0b65-3f862a577366", 30.0, 0.0, 3.0, 2, 0.7, 1, 2.75, 4.0, 5.5, (10.0, 1.0), (0.06, 0.05), (1.5, 2.0), 0.8, 3.0),
    species("Tropical Bush 2", "cdd9a9fc-6d0b-f90d-8416-c72b6019bca8", 10.0, 0.0, 3.0, 1, 0.5, 1, 0.5, 0.15, 6.0, (4.5, 0.9), (1.0, 0.08), (1.0, 1.0), 0.2, 1.0),
    species("Palm 2", "2d784476-d0db-9979-0cff-9408745a7cf3", 0.0, 0.0, 3.0, 1, 0.5, 0, 0.7, 10.0, 7.5, (13.5, 1.0), (0.035, 0.03), (0.2, 6.0), 0.6, 12.0),
    species("Cypress 1", "fb2ae204-3fd1-df33-594f-c9f882830e66", 30.0, 0.0, 3.0, 4, 0.5, 0, 10.0, 10.0, 70.0, (22.5, 1.0), (0.05, 0.03), (1.2, 1.0), 0.5, 6.0),
    species("Cypress 2", "30047cec-269d-408e-0c30-b2603b887268", 30.0, 0.0, 3.0, 4, 0.6, 3, 7.5, 10.0, 35.0, (25.0, 0.8), (0.05, 0.04), (1.2, 1.0), 0.5, 5.0),
    species("Pine 2", "d691a01c-13b7-578d-57c0-5caef0b4e7e1", 50.0, 7.5, 4.0, 2, 0.7, 6, 6.0, 10.0, 15.5, (33.0, 1.35), (0.1, 0.08), (0.5, 2.5), 0.7, 3.0),
    species("Plumeria", "6de37e4e-7029-61f5-54b8-f5e63f983f58", 8.0, 7.0, 3.0, 2, 0.6, 0, 3.0, 0.1, 20.0, (10.0, 1.35), (0.1, 0.075), (0.0, 0.0), 0.85, 2.0),
    species("Winter Pine 1", "10d2a01a-0818-84b9-4b96-c2eb63256519", 90.0, 2.5, 6.0, 1, 0.66, 8, 0.0, 4.0, 6.75, (12.5, 0.6), (0.1, 0.05), (0.0, 2.5), 0.85, 2.0),
    species("Winter Aspen", "7c0cf89b-44b1-1ce2-dd74-07102a98ac2a", 85.0, 3.0, 5.0, 1, 0.6, 8, 3.0, 4.5, 8.0, (12.0, 0.675), (0.06, 0.05), (0.75, 2.5), 0.8, 2.0),
    species("Winter Pine 2", "67931331-0c02-4876-1255-28770896c6a2", 140.0, 5.0, 6.0, 1, 0.6, 7, 0.0, 3.0, 5.0, (7.5, 0.5), (0.1, 0.05), (0.75, 2.5), 0.5, 2.0),
    species("Eucalyptus", "a6162133-724b-54df-a12f-51cd070ad6f3", 20.0, 5.0, 3.6, 4, 0.6, 0, 12.0, 8.0, 33.0, (24.0, 1.3), (0.15, 0.08), (0.0, 0.0), 0.675, 3.0),
    species("Fern", "8872f2b8-31db-42d8-580a-b3e4a91262de", 12.0, 0.0, 7.0, 1, 0.5, 0, 0.01, 0.0, 4.0, (3.5, 0.85), (1.0, 0.08), (1.0, 1.0), 0.2, 1.0),
    species("Eelgrass", "96b4de31-f4fa-337d-ec78-451e3609769e", 0.0, 0.0, 5.0, 1, 0.5, 1, 0.5, 0.15, 5.0, (3.0, 1.0), (1.0, 0.08), (1.0, 1.0), 0.2, 1.0),
    species("Sea Sword", "5894e2e7-ab8d-edfa-e61c-18cf16854ba3", 0.0, 0.0, 6.0, 1, 0.7, 1, 0.0, 0.0, 2.0, (2.0, 1.0), (1.0, 1.0), (0.5, 0.0), 0.0, 1.0),
    species("Kelp 1", "2caf1179-7861-6ff3-4b7d-46e17780bdfa", -15.0, 0.0, 1.0, 1, 1.0, 3, 2.5, 0.75, 1.85, (4.9, 1.0), (0.04, 0.05), (1.0, 2.0), 0.8, 2.0),
    species("Beach Grass 1", "18fb888b-e8f1-dce7-7da7-321d651ea6b0", 0.0, 0.0, 4.0, 1, 0.7, 1, 0.0, 0.0, 4.0, (2.5, 1.2), (1.0, 1.0), (0.5, 0.0), 0.0, 1.0),
    species("Kelp 2", "2a4880b6-b7a3-690a-2049-bfbe38eafb9f", -15.0, 0.0, 1.0, 1, 1.0, 3, 2.5, 1.35, 2.0, (4.9, 1.0), (0.025, 0.05), (1.0, 2.0), 0.8, 2.0),
];

/// The species a tree is drawn as: the one its data names, else the first
/// of the table (LLVOTree::processUpdateMessage, `sSpeciesTable.begin()`).
pub fn tree_species(id: Option<u8>) -> &'static TreeSpecies {
    TREE_SPECIES.get(id.unwrap_or(0) as usize).unwrap_or(&TREE_SPECIES[0])
}

/// Detail levels of the trunk (LLVOTree::sMAX_NUM_TREE_LOD_LEVELS), 0 the
/// finest.
pub const TREE_LODS: usize = 4;
/// Slices of a trunk cylinder at each detail level (sLODSlices).
const LOD_SLICES: [u32; TREE_LODS] = [10, 5, 4, 3];

/// Uniform scale of a tree's mesh: LLVOTree::updateMesh scales it by a
/// twentieth of the length of the object's scale ("trees have very broken
/// scale"), whatever its proportions.
pub fn tree_model_scale(object_scale: Vec3) -> f32 {
    object_scale.length() * 0.05
}

/// Half the side of the box the Legacy Viewer culls a tree with, in the
/// tree's own space (LLVOTree::updateSpatialExtents): what its detail
/// level is judged on.
pub fn tree_extent(species: &TreeSpecies) -> f32 {
    species.billboard_scale * species.billboard_ratio * 0.5
}

/// Reference geometry, appended once per segment or tip.
#[derive(Default)]
struct Part {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u16>,
}

/// The leaf quads (LEAF_VERTICES, LEAF_INDICES of updateGeometry): one
/// quad in the XZ plane and one in the YZ plane, each written twice with
/// opposite windings and normals, textured with the right half of the
/// species texture.
fn leaf() -> Part {
    const SRR3: f32 = 0.577_350_26; // sqrt(1/3)
    const SRR2: f32 = 0.707_106_77; // sqrt(1/2)
    const LEFT: f32 = 0.52;
    const RIGHT: f32 = 0.98;
    const TOP: f32 = 1.0;
    const BOTTOM: f32 = 0.52;
    const W: f32 = 0.5; // LEAF_WIDTH / 2
    let v = |n: [f32; 3], uv: [f32; 2], p: [f32; 3]| (Vec3::from_array(n), uv, Vec3::from_array(p));
    let verts = [
        // first leaf
        v([-SRR2, -SRR2, 0.0], [LEFT, BOTTOM], [-W, 0.0, 0.0]),
        v([SRR3, -SRR3, SRR3], [RIGHT, TOP], [W, 0.0, 1.0]),
        v([-SRR3, -SRR3, SRR3], [LEFT, TOP], [-W, 0.0, 1.0]),
        v([SRR2, -SRR2, 0.0], [RIGHT, BOTTOM], [W, 0.0, 0.0]),
        // same leaf, inverse winding / normals
        v([-SRR2, SRR2, 0.0], [LEFT, BOTTOM], [-W, 0.0, 0.0]),
        v([SRR3, SRR3, SRR3], [RIGHT, TOP], [W, 0.0, 1.0]),
        v([-SRR3, SRR3, SRR3], [LEFT, TOP], [-W, 0.0, 1.0]),
        v([SRR2, SRR2, 0.0], [RIGHT, BOTTOM], [W, 0.0, 0.0]),
        // next leaf
        v([SRR2, -SRR2, 0.0], [LEFT, BOTTOM], [0.0, -W, 0.0]),
        v([SRR3, SRR3, SRR3], [RIGHT, TOP], [0.0, W, 1.0]),
        v([SRR3, -SRR3, SRR3], [LEFT, TOP], [0.0, -W, 1.0]),
        v([SRR2, SRR2, 0.0], [RIGHT, BOTTOM], [0.0, W, 0.0]),
        // other side of the same leaf
        v([-SRR2, -SRR2, 0.0], [LEFT, BOTTOM], [0.0, -W, 0.0]),
        v([-SRR3, SRR3, SRR3], [RIGHT, TOP], [0.0, W, 1.0]),
        v([-SRR3, -SRR3, SRR3], [LEFT, TOP], [0.0, -W, 1.0]),
        v([-SRR2, SRR2, 0.0], [RIGHT, BOTTOM], [0.0, W, 0.0]),
    ];
    Part {
        normals: verts.iter().map(|v| v.0).collect(),
        uvs: verts.iter().map(|v| v.1).collect(),
        positions: verts.iter().map(|v| v.2).collect(),
        indices: vec![0, 1, 2, 0, 3, 1, 4, 6, 5, 4, 5, 7, 8, 9, 10, 8, 11, 9, 12, 14, 13, 12, 13, 15],
    }
}

/// A unit trunk segment (updateGeometry): `slices` rings of `slices`
/// vertices from z = -0.1 to 1.1, the first and last rings closing its
/// ends, the radius tapering from 0.65 and roughened by the bark noise. The
/// seam column repeats the first one with u = 0.49 (left half of the
/// texture, kept off its middle so the leaves do not bleed in).
fn cylinder(s: &TreeSpecies, slices: u32) -> Part {
    const BASE_RADIUS: f32 = 0.65;
    const CAP_NUDGE: f32 = 0.1;
    const FRACTAL_DEPTH: f32 = 5.0;
    const HEIGHT: f32 = 1.0;
    let top_radius = BASE_RADIUS * s.taper;
    let angle_inc = 360.0 / (slices - 1) as f32;
    let z_inc = if slices > 3 { 1.0 / (slices - 3) as f32 } else { 1.0 };
    let nvec_scale = s.noise_scale;
    let nvec_scalez = 4.0 * s.noise_scale;
    let mut p = Part::default();
    for i in 0..slices {
        let (z, r0) = if i == 0 {
            (-CAP_NUDGE, 0.0)
        } else if i == slices - 1 {
            (1.0 + CAP_NUDGE, 0.0)
        } else {
            let z = (i - 1) as f32 * z_inc;
            (z, BASE_RADIUS + (top_radius - BASE_RADIUS) * z)
        };
        for j in 0..slices {
            let angle = if j == slices - 1 { 0.0 } else { j as f32 * angle_inc };
            let (y1, x1) = angle.to_radians().sin_cos();
            // "This isn't totally accurate. Should compute based on slope as well."
            let start_radius = r0 * (1.0 + 1.2 * (z - 0.66 * HEIGHT).abs() / HEIGHT);
            let nvec = [x1 * start_radius * nvec_scale, y1 * start_radius * nvec_scale, z * nvec_scalez];
            let radius = start_radius + turbulence3(nvec, FRACTAL_DEPTH) * s.noise_mag;
            let u = if j == slices - 1 { 0.490 } else { (angle / 360.0) * 0.5 };
            p.positions.push(Vec3::new(x1 * radius, y1 * radius, z));
            p.normals.push(Vec3::new(x1, y1, 0.0));
            p.uvs.push([u, (1.0 - z / 2.0) * s.repeat_z]);
        }
    }
    for i in 0..slices - 1 {
        for j in 0..slices - 1 {
            let at = |col: u32, row: u32| (col + row * slices) as u16;
            p.indices
                .extend_from_slice(&[at(j, i), at(j + 1, i + 1), at(j, i + 1), at(j, i), at(j + 1, i), at(j + 1, i + 1)]);
        }
    }
    p
}

/// genBranchPipeline's recursion, appending to one face.
struct Grower<'a> {
    s: &'a TreeSpecies,
    cylinder: Part,
    leaf: Part,
    droop: f32,
    out: VolumeFace,
}

impl Grower<'_> {
    fn append(&mut self, leaf: bool, m: Mat4) {
        let part = if leaf { &self.leaf } else { &self.cylinder };
        let base = self.out.positions.len();
        // the indices stay 16-bit: the largest species is about 5,300
        // vertices at the finest level
        if base + part.positions.len() > u16::MAX as usize {
            return;
        }
        let normal = Mat3::from_mat4(m).inverse().transpose();
        for ((p, n), uv) in part.positions.iter().zip(&part.normals).zip(&part.uvs) {
            self.out.positions.push(m.transform_point3(*p).to_array());
            // (a zero-length segment collapses to a point: any normal)
            self.out.normals.push((normal * *n).normalize_or(Vec3::Z).to_array());
            self.out.uvs.push(*uv);
        }
        self.out.indices.extend(part.indices.iter().map(|i| i + base as u16));
    }

    fn grow(&mut self, m: Mat4, depth: u16, trunk_depth: u16, scale: f32) {
        let s = self.s;
        let trunk = trunk_depth != 0 || scale == 1.0;
        let length = if trunk { s.trunk_length } else { s.branch_length };
        let aspect = if trunk { s.trunk_aspect } else { s.branch_aspect };
        let constant_twist = 360.0 / s.branches;
        if depth == 0 {
            // a tip: two crossed leaves
            self.append(true, m * Mat4::from_scale(Vec3::splat(scale * s.leaf_scale)));
            return;
        }
        let width = scale * length * aspect;
        self.append(false, m * Mat4::from_scale(Vec3::new(width, width, scale * length)));
        let up = m * Mat4::from_translation(Vec3::new(0.0, 0.0, scale * length));
        for i in 0..s.branches as i32 {
            // LL's product applies the 20° turn first, then the droop, then
            // the branch's own turn around the parent
            let turn = (constant_twist + if i % 2 == 0 { s.twist } else { -s.twist }) * i as f32;
            let rot = Quat::from_rotation_z(turn.to_radians())
                * Quat::from_rotation_y(self.droop.to_radians())
                * Quat::from_rotation_z(20f32.to_radians());
            self.grow(up * Mat4::from_quat(rot), depth - 1, 0, scale * s.scale_step);
        }
        if trunk_depth != 0 {
            // the trunk goes on, turned a bit around its axis
            self.grow(
                up * Mat4::from_rotation_z(70.5f32.to_radians()),
                depth,
                trunk_depth - 1,
                scale * s.scale_step,
            );
        }
    }
}

/// The mesh of a tree of species `s` at detail level `lod` (0 the finest,
/// clamped to [`TREE_LODS`]), in the tree's own space: one face, bark and
/// leaves, about 4 to 15 m tall before [`tree_model_scale`].
pub fn generate_tree(s: &TreeSpecies, lod: usize) -> VolumeMesh {
    let mut g = Grower {
        s,
        cylinder: cylinder(s, LOD_SLICES[lod.min(TREE_LODS - 1)]),
        leaf: leaf(),
        // mDroop + 25 * (1 - |mTrunkBend|), the bend being zero
        droop: s.droop + 25.0,
        out: VolumeFace::default(),
    };
    g.grow(Mat4::IDENTITY, s.depth, s.trunk_depth, 1.0);
    VolumeMesh {
        faces: vec![g.out],
        surface_area: 1.0,
    }
}

/// The vertex and index counts LLVOTree::calcNumVerts gives a species at a
/// detail level (what [`generate_tree`] must produce).
pub fn tree_counts(s: &TreeSpecies, lod: usize) -> (usize, usize) {
    let slices = LOD_SLICES[lod.min(TREE_LODS - 1)] as usize;
    fn walk(s: &TreeSpecies, depth: u16, trunk_depth: u16, seg: &mut usize, leaves: &mut usize) {
        if depth == 0 {
            *leaves += 1;
            return;
        }
        *seg += 1;
        for _ in 0..s.branches as i32 {
            walk(s, depth - 1, 0, seg, leaves);
        }
        if trunk_depth != 0 {
            walk(s, depth, trunk_depth - 1, seg, leaves);
        }
    }
    let (mut seg, mut leaves) = (0, 0);
    walk(s, s.depth, s.trunk_depth, &mut seg, &mut leaves);
    (
        seg * slices * slices + leaves * 16,
        seg * (slices - 1) * (slices - 1) * 6 + leaves * 24,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_species_matches_the_legacy_vertex_counts() {
        for s in &TREE_SPECIES {
            for lod in 0..TREE_LODS {
                let mesh = generate_tree(s, lod);
                assert_eq!(mesh.faces.len(), 1);
                let f = &mesh.faces[0];
                let (verts, indices) = tree_counts(s, lod);
                assert_eq!(f.positions.len(), verts, "{} lod {lod}", s.name);
                assert_eq!(f.indices.len(), indices, "{} lod {lod}", s.name);
                assert_eq!(f.normals.len(), verts);
                assert_eq!(f.uvs.len(), verts);
                assert!(f.indices.iter().all(|&i| (i as usize) < verts));
                assert!(f.positions.iter().flatten().all(|v| v.is_finite()), "{}", s.name);
                assert!(
                    f.normals.iter().all(|n| (Vec3::from_array(*n).length() - 1.0).abs() < 1e-3),
                    "{}",
                    s.name
                );
            }
        }
    }

    #[test]
    fn known_counts() {
        // oak: depth 3, four branches, no extra trunk: 21 segments, 64 tips
        assert_eq!(tree_counts(&TREE_SPECIES[1], 0), (21 * 100 + 64 * 16, 21 * 81 * 6 + 64 * 24));
        // pine 1: six more trunk segments, five tips at each
        assert_eq!(tree_counts(&TREE_SPECIES[0], 3), (7 * 9 + 35 * 16, 7 * 4 * 6 + 35 * 24));
        // eucalyptus: 3.6 branches fork three times
        assert_eq!(tree_counts(&TREE_SPECIES[14], 1).0, 40 * 25 + 81 * 16);
    }

    #[test]
    fn detail_levels_only_change_the_trunk() {
        let s = &TREE_SPECIES[1];
        let fine = generate_tree(s, 0);
        let coarse = generate_tree(s, 3);
        assert!(fine.faces[0].positions.len() > coarse.faces[0].positions.len());
        let leaf_vertices = |m: &VolumeMesh| m.faces[0].uvs.iter().filter(|uv| uv[0] >= 0.52).count();
        assert_eq!(leaf_vertices(&fine), leaf_vertices(&coarse));
        assert_eq!(leaf_vertices(&fine), 64 * 16);
    }

    #[test]
    fn a_tree_stands_up_from_its_base() {
        for s in &TREE_SPECIES {
            let mesh = generate_tree(s, 0);
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for p in &mesh.faces[0].positions {
                lo = lo.min(Vec3::from_array(*p));
                hi = hi.max(Vec3::from_array(*p));
            }
            // grows upwards (bushes' drooping leaves dip a little under
            // their base), roughly centred on its trunk
            assert!(hi.z > 0.5 && lo.z > -0.3 * hi.z, "{}: {lo} {hi}", s.name);
            assert!((hi.x + lo.x).abs() < 0.6 * (hi.x - lo.x) + 0.5, "{}: {lo} {hi}", s.name);
            // the culling box of the Legacy Viewer holds most of it
            let extent = 2.0 * tree_extent(s);
            assert!(hi.z < 2.0 * extent, "{}: {} vs {extent}", s.name, hi.z);
        }
    }

    #[test]
    fn the_geometry_is_stable() {
        // the bark noise and the recursion are deterministic: a regression
        // in either moves these
        let mesh = generate_tree(&TREE_SPECIES[1], 0);
        let f = &mesh.faces[0];
        let sum = f.positions.iter().fold(Vec3::ZERO, |a, p| a + Vec3::from_array(*p));
        let again = generate_tree(&TREE_SPECIES[1], 0);
        assert_eq!(f, &again.faces[0]);
        assert!(sum.is_finite());
        // the trunk's first ring closes its base 0.1 under the ground, its
        // second is at the ground
        assert!((f.positions[0][2] + 0.1 * 3.8).abs() < 1e-5);
        assert!(f.positions[10][2].abs() < 1e-6);
        // (sin, cos and the noise: a few ulps apart between platforms)
        let want = Vec3::new(-161.479_84, -151.323_44, 15_511.423);
        assert!((sum - want).abs().max_element() < 0.05, "oak position sum {sum}");
    }

    #[test]
    fn leaves_use_the_right_half_and_bark_the_left() {
        let mesh = generate_tree(&TREE_SPECIES[4], 2);
        let f = &mesh.faces[0];
        for uv in &f.uvs {
            assert!(uv[0] < 0.5 || (0.52..=0.98).contains(&uv[0]));
        }
    }

    #[test]
    fn unknown_species_are_the_first() {
        assert_eq!(tree_species(Some(200)).name, "Pine 1");
        assert_eq!(tree_species(None).name, "Pine 1");
        assert_eq!(tree_species(Some(14)).name, "Eucalyptus");
    }
}
