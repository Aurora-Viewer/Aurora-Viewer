//! `LLVolumeFace` construction: `createSide`, `createCap`,
//! `createUnCutCubeCap`.
//!
//! Ported from `indra/llmath/llvolume.cpp` of the Second Life viewer,
//! Copyright (C) Linden Research, Inc., originally LGPL 2.1.

use glam::{Vec2, Vec3};

use super::{MAX_FACE_VERTICES, Volume, VolumeFace};
use crate::params::{
    LL_PCODE_PATH_CIRCLE, LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE_HALF, LL_PCODE_PROFILE_MASK, LL_PCODE_PROFILE_SQUARE,
    LL_SCULPT_FLAG_INVERT, LL_SCULPT_FLAG_MIRROR, LL_SCULPT_TYPE_CYLINDER, LL_SCULPT_TYPE_MASK, LL_SCULPT_TYPE_NONE, LL_SCULPT_TYPE_SPHERE,
    LL_SCULPT_TYPE_TORUS,
};

// LLVolumeFace type mask bits.
pub(crate) const CAP_MASK: u16 = 0x0002;
pub(crate) const END_MASK: u16 = 0x0004;
pub(crate) const SIDE_MASK: u16 = 0x0008;
pub(crate) const INNER_MASK: u16 = 0x0010;
pub(crate) const OUTER_MASK: u16 = 0x0020;
pub(crate) const HOLLOW_MASK: u16 = 0x0040;
pub(crate) const OPEN_MASK: u16 = 0x0080;
pub(crate) const FLAT_MASK: u16 = 0x0100;
pub(crate) const TOP_MASK: u16 = 0x0200;
pub(crate) const BOTTOM_MASK: u16 = 0x0400;

const F_APPROXIMATELY_ZERO: f32 = 0.00001;

/// The per-face parameters LL stores in `LLVolumeFace` before `create()`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FaceDesc {
    pub begin_s: i32,
    pub num_s: i32,
    pub num_t: i32,
    pub type_mask: u16,
    pub face_id: u16,
}

#[inline]
fn at(v: &[Vec3], i: i32) -> Vec3 {
    if i < 0 {
        return Vec3::ZERO;
    }
    v.get(i as usize).copied().unwrap_or(Vec3::ZERO)
}

#[inline]
fn normalized_or(n: Vec3, fallback: Vec3) -> Vec3 {
    let len2 = n.length_squared();
    if len2 > 0.0 && len2.is_finite() {
        n / len2.sqrt()
    } else {
        fallback
    }
}

fn finish(face_id: u16, positions: Vec<Vec3>, normals: Vec<Vec3>, uvs: Vec<[f32; 2]>, indices: Vec<u16>) -> VolumeFace {
    VolumeFace {
        face_id,
        positions: positions.into_iter().map(Into::into).collect(),
        normals: normals.into_iter().map(Into::into).collect(),
        uvs,
        indices,
    }
}

fn empty(face_id: u16) -> VolumeFace {
    VolumeFace {
        face_id,
        ..Default::default()
    }
}

/// `LLVolumeFace::createSide`.
pub(crate) fn create_side(vol: &Volume, d: &FaceDesc, sculpt_type: u8) -> VolumeFace {
    let mask = d.type_mask;
    let flat = mask & FLAT_MASK != 0;

    let sculpt_stitching = sculpt_type & LL_SCULPT_TYPE_MASK;
    let sculpt_invert = sculpt_type & LL_SCULPT_FLAG_INVERT != 0;
    let sculpt_mirror = sculpt_type & LL_SCULPT_FLAG_MIRROR != 0;
    let sculpt_reverse_horizontal = sculpt_invert != sculpt_mirror; // XOR

    let mesh = &vol.mesh;
    let profile = &vol.profile.points;
    let path = &vol.path.points;
    let max_s = vol.profile.total;

    let num_s = d.num_s;
    let num_t = d.num_t;
    if num_s < 1 || num_t < 1 {
        return empty(d.face_id);
    }
    let num_vertices = num_s as usize * num_t as usize;
    if num_vertices > MAX_FACE_VERTICES {
        return empty(d.face_id);
    }
    let begin_s = d.begin_s;

    let begin_stex = at(profile, begin_s).z.floor();
    let inner_flat = (mask & INNER_MASK != 0) && (mask & FLAT_MASK != 0) && num_s > 2;
    let ns = if inner_flat { num_s / 2 } else { num_s };

    let mut pos: Vec<Vec3> = Vec::with_capacity(num_vertices);
    let mut tc: Vec<[f32; 2]> = Vec::with_capacity(num_vertices);

    // Copy the vertices into the array
    for t in 0..num_t {
        let tt = path.get(t as usize).map_or(0.0, |p| p.tex_t);
        for s in 0..ns {
            let mut ss = if mask & END_MASK != 0 {
                if s != 0 { 1.0 } else { 0.0 }
            } else {
                // Get s value for tex-coord.
                let index = begin_s + s;
                if index < 0 || index as usize >= profile.len() {
                    // edge?
                    if flat { 1.0 - begin_stex } else { 1.0 }
                } else if !flat {
                    profile[index as usize].z
                } else {
                    profile[index as usize].z - begin_stex
                }
            };

            if sculpt_reverse_horizontal {
                ss = 1.0 - ss;
            }

            // Check to see if this triangle wraps around the array.
            let i = if begin_s + s >= max_s {
                // We're wrapping
                begin_s + s + max_s * (t - 1)
            } else {
                begin_s + s + max_s * t
            };

            let v = at(mesh, i);
            pos.push(v);
            tc.push([ss, tt]);

            if inner_flat && s > 0 {
                pos.push(v);
                tc.push([ss, tt]);
            }
        }

        if inner_flat {
            let s = if mask & OPEN_MASK != 0 { ns - 1 } else { 0 };
            let i = begin_s + s + max_s * t;
            let ss = at(profile, begin_s + s).z - begin_stex;
            pos.push(at(mesh, i));
            tc.push([ss, tt]);
        }
    }
    // Should already be exact; guards index validity regardless.
    pos.resize(num_vertices, Vec3::ZERO);
    tc.resize(num_vertices, [0.0, 0.0]);

    // Now we generate the indices.
    let mut indices: Vec<u16> = Vec::with_capacity(((num_s - 1).max(0) * (num_t - 1).max(0) * 6) as usize);
    for t in 0..(num_t - 1) {
        for s in 0..(num_s - 1) {
            let bl = (s + num_s * t) as u16;
            let br = (s + 1 + num_s * t) as u16;
            let tl = (s + num_s * (t + 1)) as u16;
            let tr = (s + 1 + num_s * (t + 1)) as u16;
            indices.extend_from_slice(&[bl, tr, tl, bl, br, tr]);
        }
    }

    // generate normals (area weighted, LL quad balancing)
    let mut norm = vec![Vec3::ZERO; num_vertices];
    for (i, tri) in indices.as_chunks::<3>().0.iter().enumerate() {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let p0 = pos[i0];
        let a = p0 - pos[i1];
        let b = p0 - pos[i2];
        let c = a.cross(b);
        norm[i0] += c;
        norm[i1] += c;
        norm[i2] += c;
        // even out quad contributions
        if i % 2 == 0 {
            norm[i1] += c;
        } else {
            norm[i2] += c;
        }
    }

    // adjust normals based on wrapping and stitching
    let nsu = num_s as usize;
    let ntu = num_t as usize;
    let (s_bottom_converges, s_top_converges) = if ntu >= 2 {
        let top = pos[0] - pos[nsu * (ntu - 2)];
        let b = top.dot(top) < 0.000001;
        let top = pos[nsu - 1] - pos[nsu * (ntu - 2) + nsu - 1];
        (b, top.dot(top) < 0.000001)
    } else {
        (false, false)
    };

    let wrap_t = |norm: &mut [Vec3]| {
        for i in 0..nsu {
            let n = norm[i] + norm[nsu * (ntu - 1) + i];
            norm[i] = n;
            norm[nsu * (ntu - 1) + i] = n;
        }
    };
    let wrap_s = |norm: &mut [Vec3]| {
        for i in 0..ntu {
            let n = norm[nsu * i] + norm[nsu * i + nsu - 1];
            norm[nsu * i] = n;
            norm[nsu * i + nsu - 1] = n;
        }
    };

    if sculpt_stitching == LL_SCULPT_TYPE_NONE {
        // logic for non-sculpt volumes
        if !vol.path.open {
            wrap_t(&mut norm);
        }
        if !vol.profile.open && !s_bottom_converges {
            wrap_s(&mut norm);
        }
        if vol.params.path.curve_type == LL_PCODE_PATH_CIRCLE
            && (vol.params.profile.curve_type & LL_PCODE_PROFILE_MASK) == LL_PCODE_PROFILE_CIRCLE_HALF
        {
            if s_bottom_converges {
                // all lower S have same normal
                for i in 0..ntu {
                    norm[nsu * i] = Vec3::X;
                }
            }
            if s_top_converges {
                // all upper S have same normal
                for i in 0..ntu {
                    norm[nsu * i + nsu - 1] = Vec3::NEG_X;
                }
            }
        }
    } else {
        // logic for sculpt volumes
        let average_poles = sculpt_stitching == LL_SCULPT_TYPE_SPHERE;
        let do_wrap_s = matches!(
            sculpt_stitching,
            LL_SCULPT_TYPE_SPHERE | LL_SCULPT_TYPE_TORUS | LL_SCULPT_TYPE_CYLINDER
        );
        let do_wrap_t = sculpt_stitching == LL_SCULPT_TYPE_TORUS;

        if average_poles {
            // average normals for north pole
            let average: Vec3 = norm[..nsu].iter().copied().sum();
            for n in &mut norm[..nsu] {
                *n = average;
            }
            // average normals for south pole
            let south = nsu * (ntu - 1);
            let average: Vec3 = norm[south..south + nsu].iter().copied().sum();
            for n in &mut norm[south..south + nsu] {
                *n = average;
            }
        }
        if do_wrap_s {
            wrap_s(&mut norm);
        }
        if do_wrap_t {
            wrap_t(&mut norm);
        }
    }

    // LL leaves side normals unnormalized (the renderer normalizes); we
    // deliver unit normals.
    for n in &mut norm {
        *n = normalized_or(*n, Vec3::Z);
    }

    finish(d.face_id, pos, norm, tc, indices)
}

/// `LLVolumeFace::createUnCutCubeCap`.
fn create_uncut_cube_cap(vol: &Volume, d: &FaceDesc) -> VolumeFace {
    let mesh = &vol.mesh;
    let profile = &vol.profile.points;
    let max_s = vol.profile.total;
    let max_t = vol.path.points.len() as i32;
    let top = d.type_mask & TOP_MASK != 0;

    let grid_size = (profile.len() as i32 - 1) / 4;
    if grid_size < 1 {
        return empty(d.face_id);
    }
    let gs1 = (grid_size + 1) as usize;
    let size = gs1 * gs1;
    if size > MAX_FACE_VERTICES {
        return empty(d.face_id);
    }

    let offset = if top { (max_t - 1) * max_s } else { d.begin_s };

    let mut cpos = [Vec3::ZERO; 4];
    let mut ctc = [Vec2::ZERO; 4];
    for t in 0..4 {
        cpos[t] = at(mesh, offset + grid_size * t as i32);
        let p = at(profile, grid_size * t as i32);
        ctc[t] = Vec2::new(p.x + 0.5, 0.5 - p.y);
    }

    let lhs = cpos[1] - cpos[0];
    let rhs = cpos[2] - cpos[1];
    let mut normal = normalized_or(lhs.cross(rhs), Vec3::Z);

    if !top {
        normal = -normal;
    } else {
        // Swap the UVs on the U(X) axis for top face
        ctc.swap(0, 3);
        ctc.swap(1, 2);
    }

    let mut pos = Vec::with_capacity(size);
    let mut tc = Vec::with_capacity(size);
    let gsf = grid_size as f32;
    for gx in 0..=grid_size {
        for gy in 0..=grid_size {
            // LerpPlanarVertex(corners[0], corners[1], corners[3], ...)
            let c01 = gx as f32 / gsf;
            let c02 = gy as f32 / gsf;
            let l = (cpos[1] - cpos[0]) * c01;
            let r = (cpos[3] - cpos[0]) * c02;
            pos.push(r + l + cpos[0]);
            let uv = ctc[0] + (ctc[1] - ctc[0]) * c01 + (ctc[3] - ctc[0]) * c02;
            tc.push([uv.x, uv.y]);
        }
    }
    let norm = vec![normal; size];

    let g = grid_size;
    let mut indices = Vec::with_capacity((g * g * 6) as usize);
    let idxs = [0, 1, (g + 1) + 1, (g + 1) + 1, g + 1, 0];
    for gx in 0..g {
        for gy in 0..g {
            let base = gy * (g + 1) + gx;
            if top {
                for i in (0..6).rev() {
                    indices.push((base + idxs[i]) as u16);
                }
            } else {
                for &o in &idxs {
                    indices.push((base + o) as u16);
                }
            }
        }
    }

    finish(d.face_id, pos, norm, tc, indices)
}

/// Signed-area tests used by the hollow cap triangulation. Returns whether
/// to use triangle (p1, pa, p2).
fn use_tri1a2(p1: Vec3, p2: Vec3, pa: Vec3, pb: Vec3) -> bool {
    let area_1a2 = (p1.x * pa.y - pa.x * p1.y) + (pa.x * p2.y - p2.x * pa.y) + (p2.x * p1.y - p1.x * p2.y);
    let area_1ba = (p1.x * pb.y - pb.x * p1.y) + (pb.x * pa.y - pa.x * pb.y) + (pa.x * p1.y - p1.x * pa.y);
    let area_21b = (p2.x * p1.y - p1.x * p2.y) + (p1.x * pb.y - pb.x * p1.y) + (pb.x * p2.y - p2.x * pb.y);
    let area_2ab = (p2.x * pa.y - pa.x * p2.y) + (pa.x * pb.y - pb.x * pa.y) + (pb.x * p2.y - p2.x * pb.y);

    let mut tri_1a2 = true;
    let mut tri_21b = true;
    if area_1a2 < 0.0 {
        tri_1a2 = false;
    }
    if area_2ab < 0.0 {
        // Can't use, because it contains point b
        tri_1a2 = false;
    }
    if area_21b < 0.0 {
        tri_21b = false;
    }
    if area_1ba < 0.0 {
        // Can't use, because it contains point b
        tri_21b = false;
    }

    if !tri_1a2 {
        false
    } else if !tri_21b {
        true
    } else {
        let d1 = p1 - pa;
        let d2 = p2 - pb;
        d1.dot(d1) < d2.dot(d2)
    }
}

/// `LLVolumeFace::createCap`.
pub(crate) fn create_cap(vol: &Volume, d: &FaceDesc) -> VolumeFace {
    let mask = d.type_mask;
    let hollow = mask & HOLLOW_MASK != 0;
    let open = mask & OPEN_MASK != 0;
    let top = mask & TOP_MASK != 0;
    let pp = &vol.params;

    if !hollow
        && !open
        && pp.path.begin == 0.0
        && pp.path.end == 1.0
        && pp.profile.curve_type == LL_PCODE_PROFILE_SQUARE
        && pp.path.curve_type == LL_PCODE_PATH_LINE
    {
        return create_uncut_cube_cap(vol, d);
    }

    let mesh = &vol.mesh;
    let profile = &vol.profile.points;

    // All types of caps have the same number of vertices and indices
    let mut num_vertices = profile.len();
    if num_vertices < 3 || num_vertices + 1 > MAX_FACE_VERTICES {
        return empty(d.face_id);
    }

    let max_s = vol.profile.total;
    let max_t = vol.path.points.len() as i32;
    let offset = if top { (max_t - 1) * max_s } else { d.begin_s };

    let mut pos: Vec<Vec3> = Vec::with_capacity(num_vertices + 1);
    let mut tc: Vec<[f32; 2]> = Vec::with_capacity(num_vertices + 1);

    let first = at(mesh, offset);
    let mut min = first;
    let mut max = first;
    let mut min_uv = Vec2::splat(f32::MAX);
    let mut max_uv = Vec2::splat(f32::MIN);

    for (k, p) in profile.iter().enumerate() {
        let src = at(mesh, offset + k as i32);
        let uv = if top {
            Vec2::new(p.x + 0.5, p.y + 0.5)
        } else {
            // Mirror for underside.
            Vec2::new(p.x + 0.5, 0.5 - p.y)
        };
        min = min.min(src);
        max = max.max(src);
        min_uv = min_uv.min(uv);
        max_uv = max_uv.max(uv);
        pos.push(src);
        tc.push([uv.x, uv.y]);
    }

    let center = (min + max) * 0.5;
    let cuv = (min_uv + max_uv) * 0.5;

    if !hollow && !open {
        pos.push(center);
        tc.push([cuv.x, cuv.y]);
        num_vertices += 1;
    }

    let mut indices: Vec<u16> = Vec::with_capacity((num_vertices - 2) * 3);

    if hollow {
        // HOLLOW TOP / HOLLOW BOTTOM
        let mut pt1: i32 = 0;
        let mut pt2: i32 = num_vertices as i32 - 1;
        while pt2 - pt1 > 1 {
            // Use the profile points instead of the mesh, since you want
            // the un-transformed profile distances.
            let p1 = at(profile, pt1);
            let p2 = at(profile, pt2);
            let pa = at(profile, pt1 + 1);
            let pb = at(profile, pt2 - 1);

            let use_1a2 = use_tri1a2(p1, p2, pa, pb);
            if top {
                if use_1a2 {
                    indices.extend_from_slice(&[pt1 as u16, (pt1 + 1) as u16, pt2 as u16]);
                    pt1 += 1;
                } else {
                    indices.extend_from_slice(&[pt1 as u16, (pt2 - 1) as u16, pt2 as u16]);
                    pt2 -= 1;
                }
            } else {
                // Flipped backfacing from top
                if use_1a2 {
                    indices.extend_from_slice(&[pt1 as u16, pt2 as u16, (pt1 + 1) as u16]);
                    pt1 += 1;
                } else {
                    indices.extend_from_slice(&[pt1 as u16, pt2 as u16, (pt2 - 1) as u16]);
                    pt2 -= 1;
                }
            }
        }
    } else {
        // Not hollow, generate the triangle fan.
        let (v1, v2) = if top { (1usize, 2usize) } else { (2usize, 1usize) };
        let center_idx = (num_vertices - 1) as u16;
        for i in 0..(num_vertices - 2) {
            let mut tri = [center_idx, 0, 0];
            tri[v1] = i as u16;
            tri[v2] = (i + 1) as u16;
            indices.extend_from_slice(&tri);
        }
    }

    let normal = if indices.len() >= 3 {
        let d0 = pos[indices[1] as usize] - pos[indices[0] as usize];
        let d1 = pos[indices[2] as usize] - pos[indices[0] as usize];
        let n = d0.cross(d1);
        if n.dot(n) > F_APPROXIMATELY_ZERO {
            normalized_or(n, Vec3::Z)
        } else if n.z >= 0.0 {
            // degenerate, make up a value
            Vec3::Z
        } else {
            Vec3::NEG_Z
        }
    } else {
        Vec3::Z
    };
    let norm = vec![normal; num_vertices];

    finish(d.face_id, pos, norm, tc, indices)
}
