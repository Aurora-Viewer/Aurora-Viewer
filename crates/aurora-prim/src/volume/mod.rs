//! Procedural prim geometry (`LLVolume`).
//!
//! Port of `indra/llmath/llvolume.cpp` (LLProfile, LLPath, LLVolume,
//! LLVolumeFace) from the Second Life viewer, Copyright (C) Linden Research,
//! Inc., licensed under the GNU Lesser General Public License, version 2.1.
//! This port is distributed under the same license.
//!
//! The generation pipeline mirrors LL exactly:
//! 1. `LLPath::generate` produces the sweep path (points with position,
//!    rotation, scale and texture T coordinate).
//! 2. `LLProfile::generate` produces the 2D cross-section and the list of
//!    profile faces (which become texture-entry faces, in order).
//! 3. `LLVolume::generate` sweeps the profile along the path into a grid.
//! 4. `LLVolume::createVolumeFaces` turns every profile face into a
//!    `LLVolumeFace` via `createSide` / `createCap` / `createUnCutCubeCap`.

mod face;
mod path;
mod profile;

use glam::Vec3;

use crate::params::{
    LL_PCODE_HOLE_MASK, LL_PCODE_HOLE_SQUARE, LL_PCODE_PATH_CIRCLE, LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE,
    LL_PCODE_PROFILE_CIRCLE_HALF, LL_PCODE_PROFILE_EQUALTRI, LL_PCODE_PROFILE_ISOTRI, LL_PCODE_PROFILE_MASK, LL_PCODE_PROFILE_RIGHTTRI,
    LL_PCODE_PROFILE_SQUARE, LL_SCULPT_TYPE_NONE, PathParams, ProfileParams, VolumeParams,
};

pub(crate) use path::Path;
pub(crate) use profile::Profile;

/// `MIN_DETAIL_FACES` from llvolume.h.
pub(crate) const MIN_DETAIL_FACES: i32 = 6;

// LLFaceID bits (llvolume.h).
pub const LL_FACE_PATH_BEGIN: u16 = 0x1 << 0;
pub const LL_FACE_PATH_END: u16 = 0x1 << 1;
pub const LL_FACE_INNER_SIDE: u16 = 0x1 << 2;
pub const LL_FACE_PROFILE_BEGIN: u16 = 0x1 << 3;
pub const LL_FACE_PROFILE_END: u16 = 0x1 << 4;
pub const LL_FACE_OUTER_SIDE_0: u16 = 0x1 << 5;
pub const LL_FACE_OUTER_SIDE_1: u16 = 0x1 << 6;
pub const LL_FACE_OUTER_SIDE_2: u16 = 0x1 << 7;
pub const LL_FACE_OUTER_SIDE_3: u16 = 0x1 << 8;

/// Maximum number of vertices in a single face (16-bit indices).
pub(crate) const MAX_FACE_VERTICES: usize = 65535;

/// LOD detail scales used by `LLVolumeLODGroup::mDetailScales`.
pub const DETAIL_SCALES: [f32; 4] = [1.0, 1.5, 2.5, 4.0];

/// One renderable face of a prim (`LLVolumeFace`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VolumeFace {
    /// LL face id bits of the profile face this was built from
    /// (`LL_FACE_PATH_BEGIN`, `LL_FACE_OUTER_SIDE_0 << n`, ...). The
    /// texture-entry index is the face's position in [`VolumeMesh::faces`].
    pub face_id: u16,
    /// Object-space positions for a unit-scale prim (box spans -0.5..0.5).
    pub positions: Vec<[f32; 3]>,
    /// Unit-length vertex normals.
    pub normals: Vec<[f32; 3]>,
    /// LL texture coordinates as produced by `LLVolumeFace`.
    pub uvs: Vec<[f32; 2]>,
    /// Triangle list, counter-clockwise front faces (LL winding).
    pub indices: Vec<u16>,
}

/// All faces of a prim; `faces[i]` corresponds to texture-entry face `i`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VolumeMesh {
    pub faces: Vec<VolumeFace>,
    /// `LLVolume::getSurfaceArea`: 1 for every prim, the swept grid's area
    /// for sculpts (attachment surface area limit of avatars).
    pub surface_area: f32,
}

impl VolumeMesh {
    /// Total vertex count over all faces.
    pub fn vertex_count(&self) -> usize {
        self.faces.iter().map(|f| f.positions.len()).sum()
    }

    /// Total triangle count over all faces.
    pub fn triangle_count(&self) -> usize {
        self.faces.iter().map(|f| f.indices.len() / 3).sum()
    }
}

/// Generate the geometry of a (non-sculpted, non-mesh) prim.
///
/// `detail` is the LOD detail scale (`LLVolumeLODGroup::mDetailScales`,
/// i.e. one of [`DETAIL_SCALES`]). Sculpt/mesh parameters are ignored; use
/// [`crate::sculpt::generate_sculpt`] for sculpted prims.
pub fn generate_volume(params: &VolumeParams, detail: f32) -> VolumeMesh {
    let p = ll_constrain(params);
    let detail = sanitize_detail(detail);
    match Volume::generate(&p, detail) {
        Some(vol) => VolumeMesh {
            faces: vol.create_faces(LL_SCULPT_TYPE_NONE),
            surface_area: 1.0,
        },
        None => VolumeMesh::default(),
    }
}

/// `LLVolume::getLoDTriangleCounts`: triangles of the four LODs estimated
/// from the path and profile point counts (render cost of prims, which LL
/// feeds to the mesh cost as LOD sizes of 10 bytes per triangle).
pub fn lod_triangle_counts(params: &VolumeParams) -> [u32; 4] {
    let p = ll_constrain(params);
    DETAIL_SCALES.map(|detail| {
        let mut path = Path::default();
        path.generate(&p.path, detail, 0, None);
        let mut profile = Profile::default();
        if !profile.generate(&p.profile, false, detail, 0, None) {
            return 0;
        }
        let (path_points, profile_points) = (path.points.len() as i64, profile.points.len() as i64);
        let count = (profile_points - 1) * 2 * (path_points - 1) + profile_points * 2;
        count.max(0) as u32
    })
}

/// Number of texture-entry faces LL generates for these parameters
/// (ordinary prims; sculpt/mesh parameters are ignored).
pub fn num_faces(params: &VolumeParams) -> usize {
    let p = ll_constrain(params);
    match Volume::generate_shape(&p, 1.0, None) {
        Some((_, profile)) => profile.faces.len(),
        None => 0,
    }
}

/// Clamp the LOD detail to a sane range (LL only uses 1.0 ..= 4.0).
pub(crate) fn sanitize_detail(detail: f32) -> f32 {
    if detail.is_finite() { detail.clamp(0.5, 8.0) } else { 1.0 }
}

#[inline]
pub(crate) fn lerp(a: f32, b: f32, u: f32) -> f32 {
    a + (b - a) * u
}

#[inline]
pub(crate) fn llfloor(f: f32) -> i32 {
    f.floor() as i32
}

#[inline]
pub(crate) fn llceil(f: f32) -> i32 {
    f.ceil() as i32
}

#[inline]
pub(crate) fn ll_round(f: f32) -> i32 {
    llfloor(f + 0.5)
}

// ---------------------------------------------------------------------------
// Parameter validation (LLVolumeMessage::constrainVolumeParams)
// ---------------------------------------------------------------------------

const MIN_CUT_DELTA: f32 = 0.02;
const HOLLOW_MIN: f32 = 0.0;
const HOLLOW_MAX: f32 = 0.99;
const HOLLOW_MAX_SQUARE: f32 = 0.7;
const TWIST_MIN: f32 = -1.0;
const TWIST_MAX: f32 = 1.0;
const RATIO_MIN: f32 = 0.0;
const RATIO_MAX: f32 = 2.0;
const HOLE_X_MIN: f32 = 0.01;
const HOLE_X_MAX: f32 = 1.0;
const HOLE_Y_MIN: f32 = 0.01;
const HOLE_Y_MAX: f32 = 0.5;
const SHEAR_MIN: f32 = -0.5;
const SHEAR_MAX: f32 = 0.5;
const REV_MIN: f32 = 1.0;
const REV_MAX: f32 = 4.0;
const TAPER_MIN: f32 = -1.0;
const TAPER_MAX: f32 = 1.0;
const SKEW_MIN: f32 = -0.95;
const SKEW_MAX: f32 = 0.95;
const LL_PCODE_PROFILE_MAX: u8 = 0x05;
const LL_PCODE_HOLE_MAX: u8 = 0x03;
const LL_PCODE_PATH_MIN: u8 = 0x01;
const LL_PCODE_PATH_MAX: u8 = 0x08;

#[inline]
fn finite_or(v: f32, d: f32) -> f32 {
    if v.is_finite() { v } else { d }
}

#[inline]
fn limit_range(v: f32, min: f32, max: f32) -> f32 {
    let mut v = v;
    if v - min < 0.0 {
        v = min;
    }
    if max - v < 0.0 {
        v = max;
    }
    v
}

/// Replace non-finite values with defaults and run the parameters through
/// LL's checked setters, in the same order as
/// `LLVolumeMessage::constrainVolumeParams`.
pub(crate) fn ll_constrain(params: &VolumeParams) -> VolumeParams {
    let dp = PathParams::default();
    let dpr = ProfileParams::default();
    let mut out = *params;

    // --- NaN / inf sanitization ---
    {
        let pr = &mut out.profile;
        pr.begin = finite_or(pr.begin, dpr.begin);
        pr.end = finite_or(pr.end, dpr.end);
        pr.hollow = finite_or(pr.hollow, dpr.hollow);
        let pa = &mut out.path;
        pa.begin = finite_or(pa.begin, dp.begin);
        pa.end = finite_or(pa.end, dp.end);
        for k in 0..2 {
            pa.scale[k] = finite_or(pa.scale[k], dp.scale[k]);
            pa.shear[k] = finite_or(pa.shear[k], dp.shear[k]);
            pa.taper[k] = finite_or(pa.taper[k], dp.taper[k]);
        }
        pa.twist_begin = finite_or(pa.twist_begin, dp.twist_begin);
        pa.twist_end = finite_or(pa.twist_end, dp.twist_end);
        pa.radius_offset = finite_or(pa.radius_offset, dp.radius_offset);
        pa.revolutions = finite_or(pa.revolutions, dp.revolutions);
        pa.skew = finite_or(pa.skew, dp.skew);
    }

    // --- setType ---
    {
        let profile = out.profile.curve_type;
        let profile_type = profile & LL_PCODE_PROFILE_MASK;
        let hole_type = (profile & LL_PCODE_HOLE_MASK) >> 4;
        if profile_type > LL_PCODE_PROFILE_MAX {
            out.profile.curve_type = LL_PCODE_PROFILE_SQUARE;
        } else if hole_type > LL_PCODE_HOLE_MAX {
            out.profile.curve_type = profile_type;
        }
        let path_type = out.path.curve_type >> 4;
        if !(LL_PCODE_PATH_MIN..=LL_PCODE_PATH_MAX).contains(&path_type) {
            out.path.curve_type = LL_PCODE_PATH_LINE;
        }
    }

    // --- setBeginAndEndS ---
    {
        let mut begin = limit_range(out.profile.begin, 0.0, 1.0 - MIN_CUT_DELTA);
        let mut end = out.profile.end;
        if (0.0149..MIN_CUT_DELTA).contains(&end) {
            end = MIN_CUT_DELTA;
        }
        end = limit_range(end, MIN_CUT_DELTA, 1.0);
        begin = limit_range(begin, 0.0, end - MIN_CUT_DELTA);
        out.profile.begin = begin;
        out.profile.end = end;
    }
    // --- setBeginAndEndT ---
    {
        let mut begin = limit_range(out.path.begin, 0.0, 1.0 - MIN_CUT_DELTA);
        let end = limit_range(out.path.end, MIN_CUT_DELTA, 1.0);
        begin = limit_range(begin, 0.0, end - MIN_CUT_DELTA);
        out.path.begin = begin;
        out.path.end = end;
    }
    // --- setHollow ---
    {
        let profile = out.profile.curve_type & LL_PCODE_PROFILE_MASK;
        let hole_type = out.profile.curve_type & LL_PCODE_HOLE_MASK;
        let mut max_hollow = HOLLOW_MAX;
        if hole_type == LL_PCODE_HOLE_SQUARE
            && matches!(
                profile,
                LL_PCODE_PROFILE_CIRCLE | LL_PCODE_PROFILE_CIRCLE_HALF | LL_PCODE_PROFILE_EQUALTRI
            )
        {
            max_hollow = HOLLOW_MAX_SQUARE;
        }
        out.profile.hollow = limit_range(out.profile.hollow, HOLLOW_MIN, max_hollow);
    }
    // --- twist ---
    out.path.twist_begin = limit_range(out.path.twist_begin, TWIST_MIN, TWIST_MAX);
    out.path.twist_end = limit_range(out.path.twist_end, TWIST_MIN, TWIST_MAX);
    // --- setRatio ---
    {
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (RATIO_MIN, RATIO_MAX, RATIO_MIN, RATIO_MAX);
        let path_type = out.path.curve_type;
        let profile_type = out.profile.curve_type & LL_PCODE_PROFILE_MASK;
        if path_type == LL_PCODE_PATH_CIRCLE && profile_type != LL_PCODE_PROFILE_CIRCLE_HALF {
            min_x = HOLE_X_MIN;
            max_x = HOLE_X_MAX;
            min_y = HOLE_Y_MIN;
            max_y = HOLE_Y_MAX;
        }
        out.path.scale[0] = limit_range(out.path.scale[0], min_x, max_x);
        out.path.scale[1] = limit_range(out.path.scale[1], min_y, max_y);
    }
    // --- shear / taper / revolutions ---
    for k in 0..2 {
        out.path.shear[k] = limit_range(out.path.shear[k], SHEAR_MIN, SHEAR_MAX);
        out.path.taper[k] = limit_range(out.path.taper[k], TAPER_MIN, TAPER_MAX);
    }
    out.path.revolutions = limit_range(out.path.revolutions, REV_MIN, REV_MAX);
    // --- setRadiusOffset ---
    {
        let path_type = out.path.curve_type;
        let profile_type = out.profile.curve_type & LL_PCODE_PROFILE_MASK;
        if profile_type == LL_PCODE_PROFILE_CIRCLE_HALF || path_type != LL_PCODE_PATH_CIRCLE {
            out.path.radius_offset = 0.0;
        } else {
            let mut radius_offset = out.path.radius_offset;
            let taper_y = out.path.taper[1];
            let radius_mag = radius_offset.abs();
            let hole_y_mag = out.path.scale[1].abs();
            let mut taper_y_mag = taper_y.abs();
            if (radius_offset > 0.0 && taper_y < 0.0) || (radius_offset < 0.0 && taper_y > 0.0) {
                taper_y_mag = 0.0;
            }
            let max_radius_mag = 1.0 - hole_y_mag * (1.0 - taper_y_mag) / (1.0 - hole_y_mag);
            let delta = max_radius_mag - radius_mag;
            if delta < 0.0 {
                radius_offset = if radius_offset < 0.0 { -max_radius_mag } else { max_radius_mag };
            }
            out.path.radius_offset = finite_or(radius_offset, 0.0);
        }
    }
    // --- setSkew ---
    {
        let mut skew = out.path.skew.clamp(SKEW_MIN, SKEW_MAX);
        let skew_mag = skew.abs();
        let revolutions = out.path.revolutions;
        let scale_x = out.path.scale[0];
        let mut min_skew_mag = 1.0 - 1.0 / (revolutions * scale_x + 1.0);
        if (revolutions - 1.0).abs() < 0.001 {
            min_skew_mag = 0.0;
        }
        if skew_mag - min_skew_mag < 0.0 {
            skew = if skew < 0.0 { -min_skew_mag } else { min_skew_mag };
        }
        out.path.skew = finite_or(skew, 0.0);
    }
    out
}

// ---------------------------------------------------------------------------
// LLVolume
// ---------------------------------------------------------------------------

/// A generated volume: path, profile and the swept vertex grid
/// (`mesh[s * profile_len + t]`).
pub(crate) struct Volume {
    pub params: VolumeParams,
    pub profile: Profile,
    pub path: Path,
    pub mesh: Vec<Vec3>,
}

impl Volume {
    /// `LLVolume::generate` split computation.
    fn split_for(params: &VolumeParams, detail: f32) -> i32 {
        let mut split = (detail * 0.66) as i32;
        let pc = params.profile.curve_type;
        if params.path.curve_type == LL_PCODE_PATH_LINE
            && (params.path.scale[0] != 1.0 || params.path.scale[1] != 1.0)
            && (pc == LL_PCODE_PROFILE_SQUARE
                || pc == LL_PCODE_PROFILE_ISOTRI
                || pc == LL_PCODE_PROFILE_EQUALTRI
                || pc == LL_PCODE_PROFILE_RIGHTTRI)
        {
            split = 0;
        }
        split
    }

    /// Generate path and profile only. `sculpt_size` = `Some((s, t))` for
    /// sculpted generation (`LLVolume::sculpt`), which uses split 0.
    pub(crate) fn generate_shape(params: &VolumeParams, detail: f32, sculpt_size: Option<(i32, i32)>) -> Option<(Path, Profile)> {
        let split = if sculpt_size.is_some() {
            0
        } else {
            Self::split_for(params, detail)
        };
        let mut path = Path::default();
        path.generate(&params.path, detail, split, sculpt_size.map(|s| s.0));
        let mut profile = Profile::default();
        if !profile.generate(&params.profile, path.open, detail, split, sculpt_size.map(|s| s.1)) {
            return None;
        }
        if path.points.is_empty() || profile.points.is_empty() {
            return None;
        }
        Some((path, profile))
    }

    /// `LLVolume::generate`: path, profile and swept vertex grid.
    pub(crate) fn generate(params: &VolumeParams, detail: f32) -> Option<Volume> {
        let (path, profile) = Self::generate_shape(params, detail, None)?;
        let size_s = path.points.len();
        let size_t = profile.points.len();
        let mut mesh = Vec::with_capacity(size_s * size_t);
        for pp in &path.points {
            let c0 = pp.rot.x_axis * pp.scale[0];
            let c1 = pp.rot.y_axis * pp.scale[1];
            // MAINT-5660 workaround: non-finite path offsets are cleared.
            let offset = if pp.pos.is_finite() { pp.pos } else { Vec3::ZERO };
            for pt in &profile.points {
                mesh.push(c0 * pt.x + c1 * pt.y + offset);
            }
        }
        Some(Volume {
            params: *params,
            profile,
            path,
            mesh,
        })
    }

    /// `LLVolume::createVolumeFaces`.
    pub(crate) fn create_faces(&self, sculpt_type: u8) -> Vec<VolumeFace> {
        let num_t = self.path.points.len() as i32;
        let hollow = self.params.profile.hollow > 0.0;
        let mut out = Vec::with_capacity(self.profile.faces.len());
        for pf in &self.profile.faces {
            let mut desc = face::FaceDesc {
                begin_s: pf.index,
                num_s: pf.count,
                num_t,
                type_mask: 0,
                face_id: pf.face_id,
            };
            if hollow {
                desc.type_mask |= face::HOLLOW_MASK;
            }
            if self.profile.open {
                desc.type_mask |= face::OPEN_MASK;
            }
            if pf.cap {
                desc.type_mask |= face::CAP_MASK;
                if pf.face_id == LL_FACE_PATH_BEGIN {
                    desc.type_mask |= face::TOP_MASK;
                } else {
                    desc.type_mask |= face::BOTTOM_MASK;
                }
            } else if pf.face_id & (LL_FACE_PROFILE_BEGIN | LL_FACE_PROFILE_END) != 0 {
                desc.type_mask |= face::FLAT_MASK | face::END_MASK;
            } else {
                desc.type_mask |= face::SIDE_MASK;
                if pf.flat {
                    desc.type_mask |= face::FLAT_MASK;
                }
                if pf.face_id & LL_FACE_INNER_SIDE != 0 {
                    desc.type_mask |= face::INNER_MASK;
                    if pf.flat && desc.num_s > 2 {
                        // flat inner faces have to copy vert normals
                        desc.num_s *= 2;
                    }
                } else {
                    desc.type_mask |= face::OUTER_MASK;
                }
            }
            let vf = if desc.type_mask & face::CAP_MASK != 0 {
                face::create_cap(self, &desc)
            } else {
                face::create_side(self, &desc, sculpt_type)
            };
            out.push(vf);
        }
        out
    }
}

#[cfg(test)]
mod tests;
