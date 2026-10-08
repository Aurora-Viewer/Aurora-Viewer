//! Sculpted prim geometry (`LLVolume::sculpt` and helpers).
//!
//! Port of the sculpt functions in `indra/llmath/llvolume.cpp` from the
//! Second Life viewer, Copyright (C) Linden Research, Inc., licensed under
//! the GNU Lesser General Public License, version 2.1.

use std::f32::consts::PI;

use glam::Vec3;

use crate::params::{
    LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE, LL_SCULPT_FLAG_INVERT, LL_SCULPT_FLAG_MIRROR, LL_SCULPT_TYPE_CYLINDER,
    LL_SCULPT_TYPE_MASK, LL_SCULPT_TYPE_PLANE, LL_SCULPT_TYPE_SPHERE, LL_SCULPT_TYPE_TORUS, PathParams, ProfileParams, VolumeParams,
};
use crate::volume::{Volume, VolumeMesh, sanitize_detail};

const SCULPT_MIN_AREA: f32 = 0.002;
const SCULPT_MAX_AREA: f32 = 384.0;
const SCULPT_MIN_AREA_DETAIL: f32 = 1.0;

const SCULPT_REZ_1: i32 = 6; // changed from 4 to 6 - 6 looks round whereas 4 looks square
const SCULPT_REZ_2: i32 = 8;
const SCULPT_REZ_3: i32 = 16;
const SCULPT_REZ_4: i32 = 32;

/// `sculpt_sides`: max sculpt mesh resolution (per side) for an LOD detail.
pub fn sculpt_sides(detail: f32) -> i32 {
    // detail is usually one of: 1, 1.5, 2.5, 4.0.
    if detail <= 1.0 {
        SCULPT_REZ_1
    } else if detail <= 2.0 {
        SCULPT_REZ_2
    } else if detail <= 3.0 {
        SCULPT_REZ_3
    } else {
        SCULPT_REZ_4
    }
}

/// `sculpt_calc_mesh_resolution`: number of vertices in the s (path) and
/// t (profile) directions for a sculpt map of the given size.
pub fn sculpt_calc_mesh_resolution(width: u32, height: u32, detail: f32) -> (i32, i32) {
    // 1) the aspect ratio of the mesh is as close as possible to the ratio of the map
    //    while still using all available verts
    // 2) the mesh cannot have more verts than is allowed by LOD
    // 3) the mesh cannot have more verts than is allowed by the map
    let max_vertices_lod = sculpt_sides(detail).pow(2);
    let max_vertices_map = ((width as u64 * height as u64) / 4).min(i32::MAX as u64) as i32;

    let vertices = if max_vertices_map > 0 {
        max_vertices_lod.min(max_vertices_map)
    } else {
        max_vertices_lod
    };

    let ratio = if width == 0 || height == 0 {
        1.0f32
    } else {
        width as f32 / height as f32
    };

    let mut s = (vertices as f32 / ratio).sqrt() as i32;
    s = s.max(4); // no degenerate sizes, please
    let mut t = vertices / s;
    t = t.max(4); // no degenerate sizes, please
    s = vertices / t;
    (s, t)
}

/// The sculpt type byte LL would use: the params' type, with non-sculpt
/// kinds mapped to sphere stitching.
fn effective_sculpt_type(params: &VolumeParams) -> u8 {
    let raw = params.sculpt.map_or(LL_SCULPT_TYPE_SPHERE, |s| s.sculpt_type);
    let kind = raw & LL_SCULPT_TYPE_MASK;
    let flags = raw & (LL_SCULPT_FLAG_INVERT | LL_SCULPT_FLAG_MIRROR);
    match kind {
        LL_SCULPT_TYPE_SPHERE | LL_SCULPT_TYPE_TORUS | LL_SCULPT_TYPE_PLANE | LL_SCULPT_TYPE_CYLINDER => raw,
        _ => LL_SCULPT_TYPE_SPHERE | flags,
    }
}

/// The volume parameters used for sculpt topology: the standard sculpt
/// shape (circle profile swept along a circle path, uncut).
fn sculpt_shape_params(params: &VolumeParams) -> VolumeParams {
    VolumeParams {
        profile: ProfileParams {
            curve_type: LL_PCODE_PROFILE_CIRCLE,
            begin: 0.0,
            end: 1.0,
            hollow: 0.0,
        },
        path: PathParams {
            curve_type: LL_PCODE_PATH_CIRCLE,
            ..PathParams::default()
        },
        sculpt: params.sculpt,
    }
}

/// Build the sculpt topology (path/profile) with an all-zero vertex grid.
fn sculpt_volume(params: &VolumeParams, detail: f32, width: u32, height: u32) -> Option<Volume> {
    let shape = sculpt_shape_params(params);
    let (req_s, req_t) = sculpt_calc_mesh_resolution(width, height, detail);
    let (path, profile) = Volume::generate_shape(&shape, detail, Some((req_s, req_t)))?;
    let n = path.points.len() * profile.points.len();
    Some(Volume {
        params: shape,
        profile,
        path,
        mesh: vec![Vec3::ZERO; n],
    })
}

/// `LLVolume::sculptGenerateSpherePlaceholder`.
fn generate_sphere_placeholder(vol: &mut Volume) {
    let size_s = vol.path.points.len();
    let size_t = vol.profile.points.len();
    let ds = (size_s.max(2) - 1) as f32;
    let dt = (size_t.max(2) - 1) as f32;
    const RADIUS: f32 = 0.3;
    for s in 0..size_s {
        for t in 0..size_t {
            let u = s as f32 / ds;
            let v = t as f32 / dt;
            let (sv, cv) = (PI * v).sin_cos();
            let (su, cu) = (2.0 * PI * u).sin_cos();
            vol.mesh[s * size_t + t] = Vec3::new(sv * cu * RADIUS, sv * su * RADIUS, cv * RADIUS);
        }
    }
}

/// `sculpt_rgb_to_vector`: maps RGB values [0..255] -> [-0.5..0.5].
#[inline]
fn sculpt_rgb_to_vector(r: u8, g: u8, b: u8) -> Vec3 {
    Vec3::new(r as f32, g as f32, b as f32) * (1.0 / 255.0) - Vec3::splat(0.5)
}

/// `LLVolume::sculptGenerateMapVertices`.
fn generate_map_vertices(vol: &mut Volume, width: u32, height: u32, components: usize, data: &[u8], sculpt_type: u8) {
    let stitching = sculpt_type & LL_SCULPT_TYPE_MASK;
    let invert = sculpt_type & LL_SCULPT_FLAG_INVERT != 0;
    let mirror = sculpt_type & LL_SCULPT_FLAG_MIRROR != 0;
    let reverse_horizontal = invert != mirror; // XOR

    let size_s = vol.path.points.len();
    let size_t = vol.profile.points.len();
    let ds = (size_s.max(2) - 1) as f32;
    let dt = (size_t.max(2) - 1) as f32;

    for s in 0..size_s {
        for t in 0..size_t {
            let reversed_t = if reverse_horizontal { size_t - t - 1 } else { t };

            let mut x = (reversed_t as f32 / dt * width as f32) as u32;
            let mut y = (s as f32 / ds * height as f32) as u32;

            if y == 0 && stitching == LL_SCULPT_TYPE_SPHERE {
                // top row stitching: pinch
                x = width / 2;
            }

            if y >= height {
                // bottom row stitching: wrap?
                y = if stitching == LL_SCULPT_TYPE_TORUS { 0 } else { height - 1 };
                // pinch?
                if stitching == LL_SCULPT_TYPE_SPHERE {
                    x = width / 2;
                }
            }

            if x >= width {
                // side stitching: wrap?
                x = if matches!(stitching, LL_SCULPT_TYPE_SPHERE | LL_SCULPT_TYPE_TORUS | LL_SCULPT_TYPE_CYLINDER) {
                    0
                } else {
                    width - 1
                };
            }

            let index = (x as usize + y as usize * width as usize) * components;
            let mut pt = match data.get(index..index + 3) {
                Some(px) => sculpt_rgb_to_vector(px[0], px[1], px[2]),
                None => Vec3::ZERO,
            };
            if mirror {
                pt.x = -pt.x;
            }
            vol.mesh[s * size_t + t] = pt;
        }
    }
}

/// `LLVolume::sculptGetSurfaceArea`.
fn surface_area(vol: &Volume) -> f32 {
    let size_s = vol.path.points.len();
    let size_t = vol.profile.points.len();
    let m = &vol.mesh;
    let mut area = 0.0f32;
    for s in 0..size_s.saturating_sub(1) {
        for t in 0..size_t.saturating_sub(1) {
            // get four corners of quad
            let p1 = m[s * size_t + t];
            let p2 = m[(s + 1) * size_t + t];
            let p3 = m[s * size_t + t + 1];
            let p4 = m[(s + 1) * size_t + t + 1];
            let cross1 = (p1 - p2).cross(p1 - p3);
            let cross2 = (p4 - p2).cross(p4 - p3);
            area += (cross1.length() + cross2.length()) / 2.0;
        }
    }
    area
}

/// Sculpted prim from a decoded sculpt map (RGB or RGBA bytes, row-major,
/// `components` bytes per pixel). Produces a single face; honors the sculpt
/// type (sphere/torus/plane/cylinder stitching), invert & mirror flags and
/// LL's mesh resolution / degenerate-area logic (falling back to the sphere
/// placeholder like LL does).
pub fn generate_sculpt(params: &VolumeParams, detail: f32, width: u32, height: u32, components: u8, pixels: &[u8]) -> VolumeMesh {
    let detail = sanitize_detail(detail);
    let sculpt_type = effective_sculpt_type(params);
    let comps = components as usize;

    let needed = (width as u64) * (height as u64) * (comps as u64);
    let mut data_is_empty = width == 0
        || height == 0
        || components < 3
        || (pixels.len() as u64) < needed
        || width > u16::MAX as u32
        || height > u16::MAX as u32;

    let (w, h) = (width.min(u16::MAX as u32), height.min(u16::MAX as u32));
    let Some(mut vol) = sculpt_volume(params, detail, w, h) else {
        return VolumeMesh::default();
    };

    // LLVolume::mSurfaceArea: 1 until measured
    let mut measured = 1.0;
    if !data_is_empty {
        generate_map_vertices(&mut vol, width, height, comps, pixels, sculpt_type);

        // don't test lowest LOD to support legacy content DEV-33670
        if detail > SCULPT_MIN_AREA_DETAIL {
            let area = surface_area(&vol);
            measured = area;
            if !(SCULPT_MIN_AREA..=SCULPT_MAX_AREA).contains(&area) {
                data_is_empty = true;
            }
        }
    }

    if data_is_empty {
        generate_sphere_placeholder(&mut vol);
    }

    VolumeMesh {
        faces: vol.create_faces(sculpt_type),
        surface_area: measured,
    }
}

/// LL's sphere placeholder (radius 0.3) shown while the sculpt map loads
/// (`LLVolume::sculpt` with no data and a visible placeholder).
pub fn sculpt_placeholder(params: &VolumeParams, detail: f32) -> VolumeMesh {
    let detail = sanitize_detail(detail);
    let sculpt_type = effective_sculpt_type(params);
    let Some(mut vol) = sculpt_volume(params, detail, 0, 0) else {
        return VolumeMesh::default();
    };
    generate_sphere_placeholder(&mut vol);
    VolumeMesh {
        faces: vol.create_faces(sculpt_type),
        surface_area: 1.0,
    }
}
