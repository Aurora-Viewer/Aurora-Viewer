use super::*;
use crate::scene::cloth::sim::{BindData, ClothMesh, ClothState, NodeSkin};
use crate::scene::cloth::weights::{self, Body, FaceInput, Params};
use glam::{Mat4, Vec3};
use std::f32::consts::TAU;

/// A sheet of `w` × `h` nodes `step` apart, in the x-z plane at `y`, its
/// edges along both sides and one diagonal.
fn sheet(w: u32, h: u32, step: f32, y: f32) -> (Vec<Vec3>, Vec<(u32, u32, f32)>) {
    let mut pos = Vec::new();
    for j in 0..h {
        for i in 0..w {
            pos.push(Vec3::new(i as f32 * step, y, 1.0 - j as f32 * step));
        }
    }
    let mut edges = Vec::new();
    for j in 0..h {
        for i in 0..w {
            let a = j * w + i;
            if i + 1 < w {
                edges.push((a, a + 1, step));
            }
            if j + 1 < h {
                edges.push((a, a + w, step));
                if i + 1 < w {
                    edges.push((a, a + w + 1, step * 2f32.sqrt()));
                }
            }
        }
    }
    (pos, edges)
}

/// A dense sheet (5 mm apart, as a detailed dress) is a few points per
/// cell's width; pinned and moving nodes never share a cell; every moving
/// node follows points whose weights sum to 1, a pinned one none.
#[test]
fn a_dense_sheet_is_a_few_points() {
    let (pos, edges) = sheet(121, 121, 0.005, 0.0);
    // its top row pinned, the rest free
    let free: Vec<f32> = pos.iter().map(|p| if p.z > 0.999 { 0.0 } else { 1.0 }).collect();
    let l = build(&pos, &free, &free, &edges, &[], 0.03);
    assert!(l.rep.len() < pos.len() / 25, "{} cells for {} nodes", l.rep.len(), pos.len());
    for (u, &c) in l.cell_of.iter().enumerate() {
        assert_eq!(free[u] > 0.0, l.free[c as usize] > 0.0, "node {u} in a cell of the other kind");
        let links = l.links_of(u);
        if free[u] > 0.0 {
            let sum: f32 = links.iter().map(|l| l.1).sum();
            assert!((sum - 1.0).abs() < 1e-4, "node {u}'s weights sum to {sum}");
        } else {
            assert!(links.is_empty(), "pinned node {u} follows {links:?}");
        }
    }
}

/// Two hair cards crossing the same voxels, 5 mm apart, not joined: never
/// one cell, and no node follows a point of the other card.
#[test]
fn cards_crossing_one_voxel_stay_apart() {
    let (a, ea) = sheet(4, 40, 0.01, 0.0);
    let (b, eb) = sheet(4, 40, 0.01, 0.005);
    let n = a.len() as u32;
    let mut pos = a;
    pos.extend(b);
    let mut edges = ea;
    edges.extend(eb.iter().map(|&(x, y, d)| (x + n, y + n, d)));
    let free = vec![1.0; pos.len()];
    let l = build(&pos, &free, &free, &edges, &[], 0.03);
    let card = |u: u32| u >= n;
    for u in 0..pos.len() as u32 {
        let rep = l.rep[l.cell_of[u as usize] as usize];
        assert_eq!(card(rep), card(u), "node {u} in a cell of the other card");
    }
    for u in 0..pos.len() as u32 {
        for &(c, _) in l.links_of(u as usize) {
            assert_eq!(card(l.rep[c as usize]), card(u), "node {u} follows the other card");
        }
    }
}

/// An open tube through rings (radius, height), top to bottom.
fn lathe(rings: &[(f32, f32)], seg: u32) -> (Vec<Vec3>, Vec<u32>) {
    let mut pos = Vec::new();
    for &(r, z) in rings {
        for i in 0..seg {
            let a = TAU * i as f32 / seg as f32;
            pos.push(Vec3::new(r * a.cos(), r * a.sin(), z));
        }
    }
    let mut idx = Vec::new();
    for ring in 0..rings.len() as u32 - 1 {
        for i in 0..seg {
            let (a, b) = (ring * seg + i, ring * seg + (i + 1) % seg);
            let (c, d) = (a + seg, b + seg);
            idx.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }
    (pos, idx)
}

/// A dense flared skirt (1 cm between rings, 192 round), simulated on its
/// lattice and on every node, swung the same way: the hem trails about as
/// far (a little further), and neighbouring vertices move together (no steps between cells).
#[test]
fn a_skirt_on_its_lattice_swings_as_it_does_whole_and_smoothly() {
    let rings: Vec<(f32, f32)> = (0..=50)
        .map(|t| (0.155 + 0.25 * (t as f32 / 50.0).powf(1.5), 1.0 - 0.011 * t as f32))
        .collect();
    let seg = 192;
    let (pos, idx) = lathe(&rings, seg);
    let rigid = vec![false; pos.len()];
    let body = Body::new([
        Mat4::from_scale_rotation_translation(Vec3::new(0.15, 0.15, 0.75), glam::Quat::IDENTITY, Vec3::new(0.0, 0.0, 0.75)),
        Mat4::from_scale_rotation_translation(Vec3::splat(0.1), glam::Quat::IDENTITY, Vec3::new(0.0, 0.0, 1.6)),
    ]);
    let face = [FaceInput {
        positions: &pos,
        indices: &idx,
        rigid: &rigid,
    }];
    let weigh = || weights::compute(&face, Some(&body), &Params::default());
    let bind = BindData {
        skin: vec![vec![
            NodeSkin {
                joints: [0; 4],
                weights: [1.0, 0.0, 0.0, 0.0],
            };
            pos.len()
        ]],
        positions: vec![pos.clone()],
        normals: vec![pos.iter().map(|p| Vec3::new(p.x, p.y, 0.0).normalize_or_zero()).collect()],
    };
    let whole = ClothMesh::new(weigh(), &bind);
    let w = weigh();
    let still = w.node_free.clone();
    let lattice = ClothMesh::coarse(w, &bind, still, 0.03);
    assert!(
        lattice.nodes.len() * 5 < whole.nodes.len(),
        "{} points for {} nodes",
        lattice.nodes.len(),
        whole.nodes.len()
    );
    let swing = |mesh: &ClothMesh| {
        let mut s = ClothState::default();
        let mut at = Mat4::IDENTITY;
        for f in 0..40 {
            let x = if f < 20 { 0.05 } else { 0.0 };
            at = Mat4::from_translation(at.w_axis.truncate() + Vec3::X * x);
            s.step(mesh, &[at], &[], at, Vec3::ZERO, 1.0 / 60.0);
        }
        s.deltas(mesh, &mut Vec::new());
        s
    };
    let (sw, sl) = (swing(&whole), swing(&lattice));
    let hem = |s: &ClothState, mesh: &ClothMesh| {
        let last = pos.len() - seg as usize..pos.len();
        last.map(|v| s.offset_of(mesh, mesh.node_of[0][v] as usize).length()).sum::<f32>() / seg as f32
    };
    let (hw, hl) = (hem(&sw, &whole), hem(&sl, &lattice));
    assert!(hw > 0.01, "the hem trails: {hw}");
    // (a little freer on the lattice: about a third further)
    assert!((hl - hw).abs() < 0.5 * hw, "the hem trails {hl} m on the lattice, {hw} m whole");
    // neighbours along a ring and down the skirt: no step between cells (no
    // more apart in their motion than on the whole mesh)
    let worst = |s: &ClothState, mesh: &ClothMesh| {
        let off = |v: u32| s.offset_of(mesh, mesh.node_of[0][v as usize] as usize);
        let mut worst: f32 = 0.0;
        for t in idx.as_chunks::<3>().0 {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                worst = worst.max((off(a) - off(b)).length());
            }
        }
        worst
    };
    let (ww, wl) = (worst(&sw, &whole), worst(&sl, &lattice));
    assert!(
        wl <= 1.5 * ww + 0.001,
        "neighbours {wl} m apart in their motion on the lattice, {ww} m whole"
    );
}

/// A buckle (a small piece of its own) stitched to a sheet: one cell of its
/// own, never shared with the fabric, its nodes following that cell alone:
/// it moves as one and never bends.
#[test]
fn a_solid_piece_is_one_cell_and_never_bends() {
    let (mut pos, mut edges) = sheet(20, 20, 0.01, 0.0);
    let n = pos.len() as u32;
    // a 2 cm square 5 mm in front of the sheet's middle, stitched to it
    let buckle: Vec<u32> = (0..9).map(|k| n + k).collect();
    for j in 0..3 {
        for i in 0..3 {
            pos.push(Vec3::new(0.09 + 0.01 * i as f32, -0.005, 0.91 - 0.01 * j as f32));
        }
    }
    for j in 0..3u32 {
        for i in 0..3u32 {
            let a = n + j * 3 + i;
            if i < 2 {
                edges.push((a, a + 1, 0.01));
            }
            if j < 2 {
                edges.push((a, a + 3, 0.01));
            }
        }
    }
    // stitched where it sits (the sheet's node behind its corner)
    edges.push((n, 9 * 20 + 9, 0.005));
    let free = vec![1.0; pos.len()];
    let l = build(&pos, &free, &free, &edges, std::slice::from_ref(&buckle), 0.03);
    let cell = l.cell_of[buckle[0] as usize];
    for &u in &buckle {
        assert_eq!(l.cell_of[u as usize], cell, "the buckle in one cell");
        assert_eq!(l.links_of(u as usize), &[(cell, 1.0)], "following its cell alone");
    }
    for u in 0..n {
        assert_ne!(l.cell_of[u as usize], cell, "sheet node {u} in the buckle's cell");
    }
    assert!(l.edges.iter().any(|&(a, b, _)| a == cell || b == cell), "still tied to the sheet");
}
