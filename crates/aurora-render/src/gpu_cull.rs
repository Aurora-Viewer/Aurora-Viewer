//! GPU-driven draw lists (shaders/cull.wgsl).
//!
//! The scene keeps two tables on the GPU, updated only when something
//! changes: one entry per drawable face (indexed by its draw record, kept in
//! `RecordStore`) and one per object (bounding sphere, flags, avatar state).
//! Each frame a compute pass tests every face against every view of the
//! frame (main view, shadow cascades, water reflection, mirror, reflection
//! probe face) and writes the indirect draw arguments of each pass, in record
//! order, into fixed-size bins drawn with `multi_draw_indexed_indirect_count`.
//! Blended faces (sorted back to front), glow and the debug lists stay on
//! the CPU (`Scene::build_lists`), which only visits the objects that have
//! some (`ObjGpu::special`).
//!
//! The CPU path (all lists built by `Scene::build_lists`) remains when the
//! device lacks `MULTI_DRAW_INDIRECT_COUNT`, and with AURORA_CPU_CULL=1.
//!
//! Two owners: the tables are the scene's, kept by the main thread
//! ([`GpuTable`], [`CullTables`]) and written through the frame's journal;
//! the compute culling and its bins ([`GpuCull`]) are the render thread's,
//! which gets the tables' buffers and sizes with each frame packet.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};

use crate::gpu_alloc::{Backoff, try_buffer};
use crate::shadow_cull::{SHADOW_PLANES, casts_into, merged_from_finer};
use crate::types::{DrawIndexedIndirect, SKELETON_ANCHOR, SkinBind};
use crate::writes::GpuWrites;

/// `CullFace::object` / `CullObject::avatar` of nothing.
pub const NO_OBJECT: u32 = u32::MAX;

/// Pass of a face (`CullFace::bits` low bits), decided on the CPU by the
/// scene's alpha classification.
pub mod pass {
    pub const HIDDEN: u32 = 0;
    pub const OPAQUE: u32 = 1;
    pub const OPAQUE_2S: u32 = 2;
    pub const MASK: u32 = 3;
    pub const MASK_2S: u32 = 4;
    pub const BLEND: u32 = 5;
    pub const WATER_EXCLUSION: u32 = 6;
    pub const BITS: u32 = 7;
}

/// `CullFace::bits`: the object's first face (counts the visible objects).
pub const FACE_FIRST: u32 = 8;
/// System head, eyes, hair or eyelashes: hidden only in the main first-person view.
pub const FACE_FIRST_PERSON_HIDDEN: u32 = 16;

/// `CullObject::flags`.
pub mod object_flags {
    pub const ACTIVE: u32 = 1;
    /// An avatar's own body.
    pub const AVATAR: u32 = 2;
    pub const RIGGED: u32 = 4;
    pub const HUD: u32 = 8;
    /// Avatars, their attachments and animesh: not in reflection probes.
    pub const NO_PROBE: u32 = 16;
    /// Rigged mesh posed by the GPU: its sphere is computed each frame from
    /// its joint boxes (`CullObject::boxes`) and its skeleton's palette.
    pub const POSED: u32 = 32;
    /// Attachment point marked invisible in first person by avatar_lad.xml.
    pub const FIRST_PERSON_HIDDEN: u32 = 64;
    /// Left out of the photo by its isolation (the viewer's
    /// `snapshot::isolate`): not drawn, casting no shadow, in no reflection.
    pub const PHOTO_HIDDEN: u32 = 128;
}

/// `CullObject::state` of an avatar entry, set every frame by the scene.
pub mod avatar_state {
    /// Among the nearest avatars drawn in full (else an impostor).
    pub const FULL: u32 = 1;
    /// Over the complexity limit: silhouette, attachments hidden.
    pub const TOO_COMPLEX: u32 = 2;
    /// Still loading and hidden until complete.
    pub const LOADING: u32 = 4;
}

pub const BINS: usize = 16;
pub const BIN_WATER_EXCLUSION: usize = 15;
pub const MAIN_BINS: usize = 4;
/// Shadow casters of each cascade drawn depth only: opaque faces, which
/// `fs_shadow` never discards.
pub const BIN_SHADOW: usize = 4;
pub const BIN_REFL_WATER: usize = 7;
pub const BIN_REFL_MIRROR: usize = 8;
pub const BIN_PROBE: usize = 9;
/// Shadow casters of each cascade drawn with the alpha test of `fs_shadow`
/// (masked faces, blended rigged faces).
pub const BIN_SHADOW_ALPHA: usize = 10;
/// Shadow casters of the two projectors that cast shadows, drawn with the
/// alpha test.
pub const BIN_SPOT: usize = 13;
/// Projector shadow maps of a frame.
pub const SPOT_SHADOWS: usize = 2;
/// Argument regions: the bins, then the occlusion's phase 1 and phase 2
/// prepass lists of the main bins (depth only, in no particular order).
pub const REGION_PRE1: usize = 16;
pub const REGION_PRE2: usize = 20;
const REGIONS: usize = 24;

const COUNT_VISIBLE: usize = 16;
const COUNT_TRIANGLES: usize = 17;
const COUNT_HIDDEN: usize = 18;
/// Counts of the phase 1 and phase 2 prepass lists of each main bin.
pub const COUNT_PRE1: usize = 19;
pub const COUNT_PRE2: usize = 23;
/// Profile (`frame_flags::SHADOW_STATS`), per cascade: triangles of its
/// casters, then its rigged casters, then their triangles.
const COUNT_SHADOW: usize = 27;
#[cfg(test)]
const SHADOW_STATS: usize = 9;
const COUNTS: usize = 40;

/// `CullFrame::sizes.w`.
pub mod frame_flags {
    pub const SHADOWS: u32 = 1;
    pub const REFLECTIONS: u32 = 2;
    pub const WATER: u32 = 4;
    pub const MIRROR: u32 = 8;
    pub const PROBE: u32 = 16;
    pub const OCCLUSION: u32 = 32;
    /// Count the casters' triangles and the rigged casters of each cascade
    /// (AURORA_PROFILE).
    pub const SHADOW_STATS: u32 = 64;
    /// Every caster drawn with the alpha test, none depth only
    /// (AURORA_SHADOW_LEGACY).
    pub const SHADOW_ALPHA_ALL: u32 = 128;
    /// Projector shadow map 0 is drawn (map 1: the next bit).
    pub const SPOT: u32 = 256;
    /// Exterior refraction shares the water bin, with a union of frustums.
    pub const WATER_AIR: u32 = 1024;
    /// Blended faces that are not rigged cast too
    /// (`shadow_cull::blend_casts`).
    pub const BLEND_CASTS: u32 = 2048;
    pub const FIRST_PERSON: u32 = 4096;
    pub const SHOW_SELF: u32 = 8192;
}

const CLASSIFY_GROUP: u32 = 128;
const SPHERE_GROUP: u32 = 64;

/// One drawable face, indexed by its draw record (must match `CullFace` in
/// cull.wgsl).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct CullFace {
    pub index_count: u32,
    pub first_index: u32,
    pub base_vertex: i32,
    /// Object slot in `GpuCull::objects`, `NO_OBJECT` for records drawn
    /// from the CPU lists only (terrain, water, ban lines...).
    pub object: u32,
    /// `pass` | `FACE_FIRST`.
    pub bits: u32,
    pub _pad: [u32; 3],
}

impl CullFace {
    pub const NONE: CullFace = CullFace {
        index_count: 0,
        first_index: 0,
        base_vertex: 0,
        object: NO_OBJECT,
        bits: 0,
        _pad: [0; 3],
    };
}

/// One object of the scene, indexed like the scene's object slab (must
/// match `CullObject` in cull.wgsl).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct CullObject {
    /// Bounding sphere (center, radius), in the world or in the space of
    /// `anchor`.
    pub sphere: [f32; 4],
    pub flags: u32,
    /// Avatar whose state applies: itself, the wearer of an attachment, or
    /// `NO_OBJECT`.
    pub avatar: u32,
    /// `avatar_state` of an avatar entry.
    pub state: u32,
    /// Worn object placed by the GPU: index + 1 of the anchor matrix its
    /// sphere is relative to (joint palettes buffer, `flags::ANCHORED`
    /// records); 0: the sphere is in the world. A posed object's anchor is
    /// its skeleton's (`SKELETON_ANCHOR` after the palette's first joint).
    pub anchor: u32,
    /// Posed objects: first joint box of its geometry, their count, and
    /// the first skin binding of its mesh.
    pub boxes: u32,
    pub box_count: u32,
    pub binds: u32,
    pub _pad: u32,
}

/// Bind-space box of the vertices one mesh joint influences (must match
/// `JointBox` in cull.wgsl): the posed boxes of a rigged mesh enclose it.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct JointBox {
    pub min: [f32; 3],
    /// Mesh joint (index into the mesh's skin bindings).
    pub joint: u32,
    pub max: [f32; 3],
    pub _pad: u32,
}

impl JointBox {
    pub const NONE: JointBox = JointBox {
        min: [0.0; 3],
        joint: 0,
        max: [0.0; 3],
        _pad: 0,
    };
}

impl CullObject {
    pub const NONE: CullObject = CullObject {
        sphere: [0.0; 4],
        flags: 0,
        avatar: NO_OBJECT,
        state: 0,
        anchor: 0,
        boxes: 0,
        box_count: 0,
        binds: 0,
        _pad: 0,
    };

    /// World bounding sphere (reference of `cs_spheres` in cull.wgsl). An
    /// anchored object's follows its anchor (a rigid matrix: the radius is
    /// kept). A posed object's encloses its joint boxes as its skeleton
    /// poses them (as `LLVOAvatar::calculateSpatialExtents` stretches the
    /// extents by the rigged attachments), or is the anchored sphere when
    /// that gives nothing sane.
    pub fn world_sphere(&self, palettes: &[[[f32; 4]; 4]], binds: &[SkinBind], boxes: &[JointBox]) -> (Vec3, f32) {
        let c = Vec3::new(self.sphere[0], self.sphere[1], self.sphere[2]);
        let Some(anchor) = self.anchor.checked_sub(1).and_then(|a| palettes.get(a as usize)) else {
            return (c, self.sphere[3]);
        };
        let anchor = Mat4::from_cols_array_2d(anchor);
        let rest = (anchor.transform_point3(c), self.sphere[3]);
        let Some(base) = (self.anchor - 1)
            .checked_sub(SKELETON_ANCHOR)
            .filter(|_| self.flags & object_flags::POSED != 0)
        else {
            return rest;
        };
        let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let mut sane = self.box_count > 0;
        for k in 0..self.box_count {
            let Some(b) = boxes.get((self.boxes + k) as usize) else {
                return rest;
            };
            let Some(bind) = binds.get((self.binds + b.joint) as usize) else {
                return rest;
            };
            let Some(joint) = palettes.get((base + bind.joint[0]) as usize) else {
                return rest;
            };
            let m = anchor * Mat4::from_cols_array_2d(joint) * Mat4::from_cols_array_2d(&bind.inverse_bind);
            for corner in 0..8u32 {
                let pick = |bit: u32, lo: f32, hi: f32| if corner & bit != 0 { hi } else { lo };
                let p = m.transform_point3(Vec3::new(
                    pick(1, b.min[0], b.max[0]),
                    pick(2, b.min[1], b.max[1]),
                    pick(4, b.min[2], b.max[2]),
                ));
                // (a NaN fails the comparison)
                sane &= p.abs().cmplt(Vec3::splat(POSED_COORD_MAX)).all();
                min = min.min(p);
                max = max.max(p);
            }
        }
        if !sane {
            return rest;
        }
        let center = (min + max) * 0.5;
        let radius = ((max - min) * 0.5).length().max(0.05);
        if radius < POSED_RADIUS_MAX && center.distance(rest.0) < POSED_RADIUS_MAX {
            (center, radius)
        } else {
            rest
        }
    }
}

/// A posed sphere larger than this, or this far from its skeleton, comes
/// from a palette that is not a pose (not sized yet, garbage): the anchored
/// sphere is kept.
pub const POSED_RADIUS_MAX: f32 = 64.0;
/// Posed corners beyond this (or NaN) are no pose either.
pub const POSED_COORD_MAX: f32 = 1.0e6;

/// World sphere of every object (reference of `cs_spheres`).
pub fn world_spheres(objects: &[CullObject], palettes: &[[[f32; 4]; 4]], binds: &[SkinBind], boxes: &[JointBox]) -> Vec<(Vec3, f32)> {
    objects.iter().map(|o| o.world_sphere(palettes, binds, boxes)).collect()
}

/// Per-frame culling parameters (must match `CullFrame` in cull.wgsl).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct CullFrame {
    pub planes: [[f32; 4]; 6],
    pub water: [[f32; 4]; 6],
    pub water_air: [[f32; 4]; 6],
    pub mirror: [[f32; 4]; 6],
    pub probe: [[f32; 4]; 6],
    /// Frustums of the projector shadow maps (`frame_flags::SPOT`).
    pub spots: [[[f32; 4]; 6]; SPOT_SHADOWS],
    pub cascades: [[[f32; 4]; 4]; 3],
    /// Per cascade: planes around what can cast a shadow the main view
    /// samples in it (`caster_planes`).
    pub shadow_planes: [[[f32; 4]; SHADOW_PLANES]; 3],
    /// Camera, draw distance.
    pub eye: [f32; 4],
    /// Shadow distance, reflection distance, pixel scale, side of a cascade
    /// in texels when a cascade takes its opaque casters from the map of
    /// the cascade before (0: each draws its own;
    /// `shadow_cull::merged_from_finer`).
    pub dist: [f32; 4],
    /// Faces, objects, slots per bin, `frame_flags`.
    pub sizes: [u32; 4],
    /// Classify workgroups; y is first-person avatar index + 1 (0: none).
    pub groups: [u32; 4],
}

impl CullFrame {
    /// Frame parameters from the scene's view, the cascade matrices with
    /// their caster planes and the water reflection, mirror and probe face
    /// view-projections (`flags` says which are drawn); sizes are filled by
    /// `GpuCull::prepare`. `spots`: view-projections of the projector shadow
    /// maps that are drawn. `merge_size`: side of a cascade in texels when
    /// the cascades are merged (`CullFrame::dist`), 0 otherwise.
    pub fn new(
        view: &GpuCullView,
        cascades: &[Mat4; 3],
        shadow_planes: &[[[f32; 4]; SHADOW_PLANES]; 3],
        merge_size: f32,
        [water, mirror, probe]: [Mat4; 3],
        spots: [Option<Mat4>; SPOT_SHADOWS],
        mut flags: u32,
    ) -> CullFrame {
        let planes = |vp: Mat4| view_planes(vp).map(|p| p.to_array());
        if let Some((_, show)) = view.first_person {
            flags |= frame_flags::FIRST_PERSON;
            if show {
                flags |= frame_flags::SHOW_SELF;
            }
        }
        for (k, s) in spots.iter().enumerate() {
            if s.is_some() {
                flags |= frame_flags::SPOT << k;
            }
        }
        CullFrame {
            planes: view.planes.map(|p| p.to_array()),
            water: planes(water),
            water_air: planes(Mat4::IDENTITY),
            mirror: planes(mirror),
            probe: planes(probe),
            // (a depth of 0 to 1: the fifth plane is the far one, and
            // nothing is left out for being before the near one)
            spots: spots.map(|s| planes(s.unwrap_or(Mat4::IDENTITY))),
            cascades: cascades.map(|m| m.to_cols_array_2d()),
            shadow_planes: *shadow_planes,
            eye: view.eye.extend(view.draw_distance).to_array(),
            dist: [view.shadow_distance, view.reflection_distance, view.pixel_scale, merge_size],
            sizes: [0, 0, 0, flags],
            groups: [0, view.first_person.and_then(|(id, _)| id.checked_add(1)).unwrap_or(0), 0, 0],
        }
    }
}

/// What the scene gives the renderer for the GPU culling of a frame (the
/// renderer adds the views it computes itself: cascades, reflections).
#[derive(Debug, Clone, Copy)]
pub struct GpuCullView {
    /// Own avatar slot and FirstPersonAvatarVisible; None outside mouselook.
    pub first_person: Option<(u32, bool)>,
    /// Main view frustum (`CullView` planes; may be the frozen debug view).
    pub planes: [Vec4; 6],
    pub eye: Vec3,
    pub draw_distance: f32,
    pub shadow_distance: f32,
    pub reflection_distance: f32,
    /// Screen height / tan(fov / 2): projected size of a sphere in pixels
    /// is radius * pixel_scale / distance.
    pub pixel_scale: f32,
    pub shadows: bool,
    pub reflections: bool,
}

/// Side and near planes of a reverse-Z infinite view (the sixth accepts all).
pub fn view_planes(view_proj: Mat4) -> [Vec4; 6] {
    let m = view_proj.transpose();
    let norm = |p: Vec4| p / p.truncate().length().max(1e-6);
    [
        norm(m.w_axis + m.x_axis),
        norm(m.w_axis - m.x_axis),
        norm(m.w_axis + m.y_axis),
        norm(m.w_axis - m.y_axis),
        // reverse-Z: z_clip <= w_clip in front of the near plane
        norm(m.w_axis - m.z_axis),
        Vec4::new(0.0, 0.0, 0.0, 1.0),
    ]
}

pub fn sphere_in(planes: &[[f32; 4]; 6], c: Vec3, r: f32) -> bool {
    planes.iter().all(|p| Vec4::from_array(*p).truncate().dot(c) + p[3] >= -r)
}

/// Renderer cascade test (orthographic, radius in clip units).
pub fn in_cascade(m: &Mat4, c: Vec3, r: f32) -> bool {
    let p = *m * c.extend(1.0);
    let rr = r * m.x_axis.truncate().length();
    !(p.x + rr < -1.0 || p.x - rr > 1.0 || p.y + rr < -1.0 || p.y - rr > 1.0 || p.z - rr > 1.0)
}

/// Reference of `face_mask` in cull.wgsl (kept line for line): the bins of
/// face `i`, and whether it counts as a visible object. `spheres`: the
/// world sphere of each object (`world_spheres`).
pub fn face_bins(frame: &CullFrame, faces: &[CullFace], objects: &[CullObject], spheres: &[(Vec3, f32)], i: usize) -> (u32, bool) {
    let Some(f) = faces.get(i) else {
        return (0, false);
    };
    let n = frame.sizes[1];
    if f.object >= n {
        return (0, false);
    }
    let o = objects[f.object as usize];
    if o.flags & object_flags::ACTIVE == 0 || o.flags & (object_flags::HUD | object_flags::PHOTO_HIDDEN) != 0 {
        return (0, false);
    }
    let is_avatar = o.flags & object_flags::AVATAR != 0;
    let own_first_person = frame.sizes[3] & frame_flags::FIRST_PERSON != 0 && frame.groups[1] != 0 && o.avatar == frame.groups[1] - 1;
    if own_first_person && frame.sizes[3] & frame_flags::SHOW_SELF == 0 {
        return (0, false);
    }
    if o.avatar < n {
        let st = objects[o.avatar as usize].state;
        if st & avatar_state::FULL == 0 || (st & avatar_state::TOO_COMPLEX != 0 && !is_avatar) || st & avatar_state::LOADING != 0 {
            return (0, false);
        }
    }
    let (c, r) = spheres[f.object as usize];
    let d = (c - Vec3::new(frame.eye[0], frame.eye[1], frame.eye[2])).length();
    if d - r > frame.eye[3] && !is_avatar {
        return (0, false);
    }
    let fl = frame.sizes[3];
    let in_view = sphere_in(&frame.planes, c, r)
        && !(own_first_person && (o.flags & object_flags::FIRST_PERSON_HIDDEN != 0 || f.bits & FACE_FIRST_PERSON_HIDDEN != 0));
    let casts = fl & frame_flags::SHADOWS != 0 && d - r < frame.dist[0];
    let reflects = fl & frame_flags::REFLECTIONS != 0 && d - r < frame.dist[1];
    if !in_view && !casts && !reflects {
        return (0, false);
    }
    let px = r * frame.dist[2] / d.max(0.1);
    if px < 1.5 && !is_avatar {
        return (0, false);
    }
    let visible = in_view && f.bits & FACE_FIRST != 0;
    let p = f.bits & pass::BITS;
    if p == pass::HIDDEN {
        return (0, visible);
    }
    if p == pass::WATER_EXCLUSION {
        let allowed = in_view && o.flags & (object_flags::AVATAR | object_flags::RIGGED) == 0 && o.avatar >= n;
        return (if allowed { 1 << BIN_WATER_EXCLUSION } else { 0 }, visible);
    }
    let mut m = 0u32;
    let rigged = o.flags & (object_flags::RIGGED | object_flags::AVATAR) != 0;
    let caster = casts && (p != pass::BLEND || rigged || fl & frame_flags::BLEND_CASTS != 0);
    if caster {
        let depth_only = (p == pass::OPAQUE || p == pass::OPAQUE_2S) && fl & frame_flags::SHADOW_ALPHA_ALL == 0;
        let bin = if depth_only { BIN_SHADOW } else { BIN_SHADOW_ALPHA };
        let cascades = frame.cascades.map(|m| Mat4::from_cols_array_2d(&m));
        for (k, cm) in cascades.iter().enumerate() {
            if in_cascade(cm, c, r)
                && casts_into(&frame.shadow_planes[k], c, r)
                && !(depth_only && merged_from_finer(&cascades, &frame.shadow_planes, frame.dist[3], k, c, r))
            {
                m |= 1 << (bin + k);
            }
        }
    }
    if caster {
        for (k, planes) in frame.spots.iter().enumerate() {
            if fl & (frame_flags::SPOT << k) != 0 && sphere_in(planes, c, r) {
                m |= 1 << (BIN_SPOT + k);
            }
        }
    }
    let solid = (pass::OPAQUE..=pass::MASK_2S).contains(&p);
    if reflects && solid {
        if (fl & frame_flags::WATER != 0 && sphere_in(&frame.water, c, r))
            || (fl & frame_flags::WATER_AIR != 0 && sphere_in(&frame.water_air, c, r))
        {
            m |= 1 << BIN_REFL_WATER;
        }
        if fl & frame_flags::MIRROR != 0 && sphere_in(&frame.mirror, c, r) {
            m |= 1 << BIN_REFL_MIRROR;
        }
        if fl & frame_flags::PROBE != 0 && o.flags & object_flags::NO_PROBE == 0 && sphere_in(&frame.probe, c, r) {
            m |= 1 << BIN_PROBE;
        }
    }
    if in_view && solid {
        m |= 1 << (p - pass::OPAQUE);
    }
    (m, visible)
}

/// `pack_bins` of cull.wgsl: one 8-bit counter per bin, four per lane.
pub fn pack_bins(m: u32) -> [u32; 4] {
    let mut v = [0u32; 4];
    for b in 0..BINS {
        if (m >> b) & 1 != 0 {
            v[b >> 2] += 1 << ((b & 3) * 8);
        }
    }
    v
}

/// Rank of each face in each of its bins inside one workgroup of the
/// scatter pass, as cull.wgsl computes it (inclusive prefix sum of the
/// packed counters minus the face's own): reference for the tests.
pub fn workgroup_ranks(masks: &[u32]) -> Vec<[u32; BINS]> {
    let mut acc = [0u32; 4];
    masks
        .iter()
        .map(|&m| {
            let own = pack_bins(m);
            for k in 0..4 {
                acc[k] += own[k];
            }
            let mut r = [0u32; BINS];
            for (b, rb) in r.iter_mut().enumerate() {
                let excl = acc[b >> 2] - own[b >> 2];
                *rb = (excl >> ((b & 3) * 8)) & 0xff;
            }
            r
        })
        .collect()
}

/// Elements per dirty chunk of a mirrored table: small enough that
/// scattered updates (worn attachments, avatars) do not drag big untouched
/// ranges along; neighbouring chunks are still uploaded in one write.
pub(crate) const CHUNK: usize = 64;

/// AURORA_SYNC_SLICE: whole chunks are uploaded, as before the uploads
/// were cut to the changed elements (comparison).
static WHOLE_CHUNKS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Upload whole dirty chunks of the mirrored tables (draw records, culling
/// tables) instead of their changed elements only.
pub fn set_whole_chunk_uploads(on: bool) {
    WHOLE_CHUNKS.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Changed elements of a mirrored table, by chunk: the span they cover in
/// each chunk, coalesced into upload ranges.
///
/// A range runs from the first changed element of a run of dirty chunks to
/// the last one: as many writes as with whole chunks, without the untouched
/// elements before and after (one record that moves alone in its chunk was
/// sent with its 63 neighbours, 16 KB for 256 bytes).
#[derive(Default, Debug)]
pub(crate) struct DirtyChunks {
    /// First changed element of each chunk and the one after the last,
    /// within the chunk; equal: nothing changed.
    spans: Vec<(u8, u8)>,
    any: bool,
}

impl DirtyChunks {
    pub(crate) fn mark(&mut self, i: usize) {
        let (c, at) = (i / CHUNK, (i % CHUNK) as u8);
        if self.spans.len() <= c {
            self.spans.resize(c + 1, (0, 0));
        }
        let s = &mut self.spans[c];
        *s = if s.0 == s.1 { (at, at + 1) } else { (s.0.min(at), s.1.max(at + 1)) };
        self.any = true;
    }

    pub(crate) fn clear(&mut self) {
        self.spans.iter_mut().for_each(|s| *s = (0, 0));
        self.any = false;
    }

    pub(crate) fn any(&self) -> bool {
        self.any
    }

    /// Element ranges to upload (clipped to `len`); clears the marks.
    pub(crate) fn take_ranges(&mut self, len: usize) -> Vec<std::ops::Range<usize>> {
        self.take_ranges_as(len, WHOLE_CHUNKS.load(std::sync::atomic::Ordering::Relaxed))
    }

    fn take_ranges_as(&mut self, len: usize, whole: bool) -> Vec<std::ops::Range<usize>> {
        let mut out = Vec::new();
        if !self.any {
            return out;
        }
        let mut c = 0;
        while c < self.spans.len() {
            if self.spans[c].0 == self.spans[c].1 {
                c += 1;
                continue;
            }
            let start = c;
            let first = self.spans[c].0 as usize;
            let mut last = 0;
            while c < self.spans.len() && self.spans[c].0 != self.spans[c].1 {
                last = self.spans[c].1 as usize;
                self.spans[c] = (0, 0);
                c += 1;
            }
            let (a, b) = if whole {
                (start * CHUNK, c * CHUNK)
            } else {
                (start * CHUNK + first, (c - 1) * CHUNK + last)
            };
            let b = b.min(len);
            if a < b {
                out.push(a..b);
            }
        }
        self.any = false;
        out
    }
}

/// Storage buffer mirrored on the CPU; only changed chunks are uploaded.
pub struct GpuTable<T: Pod + PartialEq> {
    label: &'static str,
    buffer: wgpu::Buffer,
    mirror: Vec<T>,
    fill: T,
    dirty: DirtyChunks,
    /// Bumped when the buffer is reallocated (bind groups must follow).
    pub generation: u64,
    /// Bytes sent by the last `flush`.
    pub uploaded: u64,
    /// The buffer could not grow (video memory full): when to try again.
    backoff: Backoff,
}

impl<T: Pod + PartialEq> GpuTable<T> {
    pub fn new(device: &wgpu::Device, label: &'static str, cap: usize, fill: T) -> Self {
        GpuTable {
            label,
            buffer: device.create_buffer(&Self::desc(label, cap.max(1))),
            mirror: Vec::with_capacity(cap),
            fill,
            dirty: DirtyChunks::default(),
            generation: 0,
            uploaded: 0,
            backoff: Backoff::default(),
        }
    }

    fn desc(label: &str, cap: usize) -> wgpu::BufferDescriptor<'_> {
        wgpu::BufferDescriptor {
            label: Some(label),
            size: (cap * std::mem::size_of::<T>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }
    }

    pub fn len(&self) -> usize {
        self.mirror.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mirror.is_empty()
    }

    pub fn get(&self, i: usize) -> Option<&T> {
        self.mirror.get(i)
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// Set entry `i` (the table grows with the fill value); unchanged
    /// entries are not uploaded again.
    pub fn set(&mut self, i: usize, v: T) {
        if i >= self.mirror.len() {
            if v == self.fill {
                return;
            }
            let from = self.mirror.len();
            self.mirror.resize(i + 1, self.fill);
            for k in from..=i {
                self.dirty.mark(k);
            }
        }
        if self.mirror[i] != v {
            self.mirror[i] = v;
            self.dirty.mark(i);
        }
    }

    /// At least `len` entries (new ones hold the fill value).
    pub fn ensure_len(&mut self, len: usize) {
        let from = self.mirror.len();
        if len > from {
            self.mirror.resize(len, self.fill);
            for k in from..len {
                self.dirty.mark(k);
            }
        }
    }

    /// Every entry back to the fill value.
    pub fn reset(&mut self) {
        let fill = self.fill;
        for i in 0..self.mirror.len() {
            if self.mirror[i] != fill {
                self.mirror[i] = fill;
                self.dirty.mark(i);
            }
        }
    }

    /// Record the upload of the changed chunks in the frame's journal.
    pub fn flush(&mut self, device: &wgpu::Device, writes: &mut GpuWrites) {
        let size = std::mem::size_of::<T>();
        self.uploaded = 0;
        let needed = (self.mirror.len() * size) as u64;
        let now = std::time::Instant::now();
        if needed > self.buffer.size() && self.backoff.ready(now) {
            let cap = self.mirror.len().next_power_of_two();
            match try_buffer(device, &Self::desc(self.label, cap)) {
                Ok(buffer) => {
                    self.backoff.succeeded();
                    self.buffer = buffer;
                    self.generation += 1;
                    writes.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.mirror));
                    self.uploaded = needed;
                    self.dirty.clear();
                    return;
                }
                Err(e) => {
                    if !self.backoff.waiting() {
                        log::warn!(
                            "{} not grown to {} MB ({e}): video memory is full, the entries past {} wait for it",
                            self.label,
                            (cap * size) >> 20,
                            self.buffer.size() as usize / size
                        );
                    }
                    self.backoff.failed(now);
                }
            }
        }
        // (while the buffer could not grow, what it has no room for waits:
        // the growth writes the whole table)
        let fits = self.buffer.size() as usize / size;
        for r in self.dirty.take_ranges(self.mirror.len()) {
            let r = r.start.min(fits)..r.end.min(fits);
            if r.is_empty() {
                continue;
            }
            writes.write_buffer(&self.buffer, (r.start * size) as u64, bytemuck::cast_slice(&self.mirror[r.clone()]));
            self.uploaded += (r.len() * size) as u64;
        }
    }
}

/// Bin sizes and statistics of a frame, read back a frame or two later.
#[derive(Debug, Clone, Copy, Default)]
pub struct CullCounts {
    pub bins: [u32; BINS],
    pub visible_objects: u32,
    pub triangles: u32,
    /// Main-bin draws hidden by the occlusion.
    pub hidden: u32,
    /// With `frame_flags::SHADOW_STATS`, per cascade: triangles of its
    /// casters, its rigged casters (avatars and rigged meshes) and their
    /// triangles.
    pub shadow_triangles: [u32; 3],
    pub shadow_rigged: [u32; 3],
    pub shadow_rigged_triangles: [u32; 3],
}

impl CullCounts {
    pub fn main_draws(&self) -> u32 {
        self.bins[..MAIN_BINS].iter().sum()
    }

    /// Casters drawn in cascade `k`, depth only and alpha tested.
    pub fn cascade_draws(&self, k: usize) -> u32 {
        self.bins[BIN_SHADOW + k] + self.bins[BIN_SHADOW_ALPHA + k]
    }

    pub fn shadow_draws(&self) -> u32 {
        (0..3).map(|k| self.cascade_draws(k)).sum()
    }
}

fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn entry(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: b.as_entire_binding(),
    }
}

/// The scene's side of the GPU culling, on the main thread.
pub struct CullTables {
    /// Objects table (indexed like the scene's object slab).
    pub objects: GpuTable<CullObject>,
    /// Joint boxes of the rigged geometries the GPU poses (a range each,
    /// `CullObject::boxes`).
    pub boxes: GpuTable<JointBox>,
    /// Last counts read back by the render thread (a few frames old).
    pub last: CullCounts,
}

impl CullTables {
    pub fn new(device: &wgpu::Device) -> Self {
        CullTables {
            objects: GpuTable::new(device, "cull objects", 4096, CullObject::NONE),
            boxes: GpuTable::new(device, "cull joint boxes", 1024, JointBox::NONE),
            last: CullCounts::default(),
        }
    }
}

/// The scene's buffers the culling reads (handles of the frame's packet).
pub struct CullInputs<'a> {
    pub faces: &'a wgpu::Buffer,
    pub objects: &'a wgpu::Buffer,
    pub palettes: &'a wgpu::Buffer,
    pub skin_binds: &'a wgpu::Buffer,
    pub boxes: &'a wgpu::Buffer,
}

pub struct GpuCull {
    spheres_layout: wgpu::BindGroupLayout,
    lists_layout: wgpu::BindGroupLayout,
    occlude_layout: wgpu::BindGroupLayout,
    sphere_pass: wgpu::ComputePipeline,
    classify: wgpu::ComputePipeline,
    scan: wgpu::ComputePipeline,
    scatter: wgpu::ComputePipeline,
    occlude: wgpu::ComputePipeline,
    frame: wgpu::Buffer,
    args: wgpu::Buffer,
    /// Slots per bin (argument region).
    cap: u32,
    counts: wgpu::Buffer,
    masks: wgpu::Buffer,
    group_data: wgpu::Buffer,
    /// This frame: faces tested and classify workgroups.
    faces: u32,
    groups: u32,
    readback: Vec<(wgpu::Buffer, Arc<AtomicBool>, bool)>,
    /// Readback slot of this frame's copy (set while the frame's draw
    /// closures borrow the bins).
    copied: std::cell::Cell<Option<usize>>,
    /// Last counts read back.
    pub last: CullCounts,
    max_groups: u32,
    /// World sphere of each object this frame (`cs_spheres`), and the
    /// objects it has room for.
    spheres: wgpu::Buffer,
    sphere_cap: u32,
    /// This frame: objects of the scene's table.
    objects: u32,
}

impl GpuCull {
    /// The GPU lists need indirect draw counts and enough storage buffers;
    /// AURORA_CPU_CULL=1 keeps the CPU lists (comparison).
    pub fn supported(device: &wgpu::Device) -> bool {
        static FORCE_CPU: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let force_cpu = *FORCE_CPU.get_or_init(|| std::env::var_os("AURORA_CPU_CULL").is_some_and(|v| v != "0"));
        !force_cpu
            && device.features().contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT)
            && device.limits().max_storage_buffers_per_shader_stage >= 8
    }

    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cull"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{}\n{}", include_str!("shaders/hiz_test.wgsl"), include_str!("shaders/cull.wgsl")).into(),
            ),
        });
        let spheres_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cull spheres"),
            entries: &[
                uniform(0),
                storage(2, true),
                storage(10, true),
                storage(11, true),
                storage(12, true),
                storage(13, false),
            ],
        });
        let lists_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cull lists"),
            entries: &[
                uniform(0),
                storage(1, true),
                storage(2, true),
                storage(3, false),
                storage(4, false),
                storage(5, false),
                storage(6, false),
                storage(7, false),
                storage(13, false),
            ],
        });
        let occlude_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cull occlusion"),
            entries: &[
                uniform(0),
                storage(1, true),
                storage(2, true),
                storage(3, false),
                storage(6, false),
                storage(7, false),
                uniform(8),
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                storage(13, false),
            ],
        });
        let pipeline = |layout: &wgpu::BindGroupLayout, entry: &str| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let sphere_pass = pipeline(&spheres_layout, "cs_spheres");
        let classify = pipeline(&lists_layout, "cs_classify");
        let scan = pipeline(&lists_layout, "cs_scan");
        let scatter = pipeline(&lists_layout, "cs_scatter");
        let occlude = pipeline(&occlude_layout, "cs_occlude");
        let frame = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cull frame"),
            size: std::mem::size_of::<CullFrame>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let counts = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cull counts"),
            size: (COUNTS * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cap = 4096;
        let mut s = GpuCull {
            spheres_layout,
            lists_layout,
            occlude_layout,
            sphere_pass,
            classify,
            scan,
            scatter,
            occlude,
            frame,
            args: Self::make_args(device, cap),
            cap,
            counts,
            masks: Self::make_storage(device, "cull masks", cap as u64 * 4),
            group_data: Self::make_storage(device, "cull groups", (cap / CLASSIFY_GROUP) as u64 * BINS as u64 * 4),
            faces: 0,
            groups: 0,
            readback: Vec::new(),
            copied: std::cell::Cell::new(None),
            last: CullCounts::default(),
            max_groups: device.limits().max_compute_workgroups_per_dimension,
            spheres: Self::make_storage(device, "cull spheres", 4096 * 16),
            sphere_cap: 4096,
            objects: 0,
        };
        s.readback = (0..3)
            .map(|_| {
                let b = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("cull readback"),
                    size: (COUNTS * 4) as u64,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                (b, Arc::new(AtomicBool::new(false)), false)
            })
            .collect();
        s
    }

    fn make_storage(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: size.max(16),
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        })
    }

    fn make_args(device: &wgpu::Device, cap: u32) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cull args"),
            size: REGIONS as u64 * cap as u64 * std::mem::size_of::<DrawIndexedIndirect>() as u64,
            // copy source: GPU test read-back
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    /// Size the buffers for `faces` face slots and upload the frame
    /// parameters (`frame.sizes` and `groups` are filled in here); `objects`:
    /// entries of the scene's object table this frame.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, faces: usize, objects: usize, mut frame: CullFrame) {
        let faces = (faces as u32).min(self.max_groups.saturating_mul(CLASSIFY_GROUP));
        if faces > self.cap {
            self.cap = faces.next_power_of_two();
            self.args = Self::make_args(device, self.cap);
            self.masks = Self::make_storage(device, "cull masks", self.cap as u64 * 4);
            self.group_data = Self::make_storage(device, "cull groups", self.cap.div_ceil(CLASSIFY_GROUP) as u64 * BINS as u64 * 4);
            log::info!("GPU culling: {} slots per bin", self.cap);
        }
        let objects = (objects as u32).min(self.max_groups.saturating_mul(SPHERE_GROUP));
        if objects > self.sphere_cap {
            self.sphere_cap = objects.next_power_of_two();
            self.spheres = Self::make_storage(device, "cull spheres", self.sphere_cap as u64 * 16);
        }
        self.objects = objects;
        self.faces = faces;
        self.groups = faces.div_ceil(CLASSIFY_GROUP).max(1);
        frame.sizes[0] = faces;
        frame.sizes[1] = objects;
        frame.sizes[2] = self.cap;
        frame.groups[0] = self.groups;
        queue.write_buffer(&self.frame, 0, bytemuck::bytes_of(&frame));
    }

    /// The objects' world spheres, then classify, scan and scatter: this
    /// frame's bins.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene: &CullInputs,
        visibility: &wgpu::Buffer,
        timestamps: Option<wgpu::ComputePassTimestampWrites>,
    ) {
        let spheres_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cull spheres"),
            layout: &self.spheres_layout,
            entries: &[
                entry(0, &self.frame),
                entry(2, scene.objects),
                entry(10, scene.palettes),
                entry(11, scene.skin_binds),
                entry(12, scene.boxes),
                entry(13, &self.spheres),
            ],
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cull lists"),
            layout: &self.lists_layout,
            entries: &[
                entry(0, &self.frame),
                entry(1, scene.faces),
                entry(2, scene.objects),
                entry(3, visibility),
                entry(4, &self.masks),
                entry(5, &self.group_data),
                entry(6, &self.counts),
                entry(7, &self.args),
                entry(13, &self.spheres),
            ],
        });
        encoder.clear_buffer(&self.counts, 0, None);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gpu culling"),
            timestamp_writes: timestamps,
        });
        pass.set_pipeline(&self.sphere_pass);
        pass.set_bind_group(0, &spheres_bg, &[]);
        pass.dispatch_workgroups(self.objects.div_ceil(SPHERE_GROUP).max(1), 1, 1);
        pass.set_bind_group(0, &bg, &[]);
        pass.set_pipeline(&self.classify);
        pass.dispatch_workgroups(self.groups, 1, 1);
        pass.set_pipeline(&self.scan);
        pass.dispatch_workgroups(BINS as u32, 1, 1);
        pass.set_pipeline(&self.scatter);
        pass.dispatch_workgroups(self.groups, 1, 1);
    }

    /// Occlusion test of the main bins (after the Hi-Z pyramid of the phase
    /// 1 prepass): hidden draws get instance_count 0, phase 2 arguments.
    pub fn encode_occlusion(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        faces: &wgpu::Buffer,
        objects: &wgpu::Buffer,
        visibility: &wgpu::Buffer,
        occl_params: &wgpu::Buffer,
        hiz: &wgpu::TextureView,
    ) {
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cull occlusion"),
            layout: &self.occlude_layout,
            entries: &[
                entry(0, &self.frame),
                entry(1, faces),
                entry(2, objects),
                entry(3, visibility),
                entry(6, &self.counts),
                entry(7, &self.args),
                entry(8, occl_params),
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(hiz),
                },
                entry(13, &self.spheres),
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gpu culling occlusion"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.occlude);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(self.cap.div_ceil(64).min(self.max_groups), MAIN_BINS as u32, 1);
    }

    /// Copy this frame's counts for the statistics (mapped after the submit).
    pub fn copy_counts(&self, encoder: &mut wgpu::CommandEncoder) {
        if let Some(slot) = self.readback.iter().position(|r| !r.2) {
            encoder.copy_buffer_to_buffer(&self.counts, 0, &self.readback[slot].0, 0, (COUNTS * 4) as u64);
            self.copied.set(Some(slot));
        }
    }

    pub fn after_submit(&mut self) {
        let Some(slot) = self.copied.take() else {
            return;
        };
        let (buf, ready, pending) = &mut self.readback[slot];
        let ready = ready.clone();
        *pending = true;
        buf.map_async(wgpu::MapMode::Read, .., move |r| {
            if r.is_ok() {
                ready.store(true, Ordering::Release);
            }
        });
    }

    /// Collect the finished count readbacks (never waits).
    pub fn harvest(&mut self) {
        for (buf, ready, pending) in self.readback.iter_mut() {
            if *pending && ready.load(Ordering::Acquire) {
                if let Ok(view) = buf.get_mapped_range(..) {
                    let n: &[u32] = bytemuck::cast_slice(&view[..COUNTS * 4]);
                    let mut c = CullCounts::default();
                    c.bins.copy_from_slice(&n[..BINS]);
                    c.visible_objects = n[COUNT_VISIBLE];
                    c.triangles = n[COUNT_TRIANGLES];
                    c.hidden = n[COUNT_HIDDEN];
                    let stat = |i: usize| std::array::from_fn(|k| n[COUNT_SHADOW + i * 3 + k]);
                    c.shadow_triangles = stat(0);
                    c.shadow_rigged = stat(1);
                    c.shadow_rigged_triangles = stat(2);
                    self.last = c;
                }
                buf.unmap();
                *pending = false;
                ready.store(false, Ordering::Release);
            }
        }
    }

    pub fn args(&self) -> &wgpu::Buffer {
        &self.args
    }

    pub fn counts(&self) -> &wgpu::Buffer {
        &self.counts
    }

    /// Slots per bin: the `max_count` of the indirect count draws.
    pub fn cap(&self) -> u32 {
        self.cap
    }

    /// Byte offset of argument region `r` (a bin or a prepass copy).
    pub fn region_offset(&self, r: usize) -> u64 {
        r as u64 * self.cap as u64 * std::mem::size_of::<DrawIndexedIndirect>() as u64
    }

    /// Byte offset of count `c`: a bin, or `COUNT_PRE1` / `COUNT_PRE2` + bin.
    pub fn count_offset(c: usize) -> u64 {
        c as u64 * 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wgsl_fields(src: &str, name: &str) -> Vec<String> {
        let start = src.find(&format!("struct {name} {{")).expect("struct in cull.wgsl");
        let body = &src[start..start + src[start..].find('}').expect("end of struct")];
        body.lines()
            .skip(1)
            .filter_map(|l| l.trim().split(':').next())
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty() && !n.starts_with("//"))
            .collect()
    }

    #[test]
    fn layouts_match_wgsl() {
        let src = include_str!("shaders/cull.wgsl");
        assert_eq!(std::mem::size_of::<CullFace>(), 32);
        assert_eq!(
            wgsl_fields(src, "CullFace"),
            ["index_count", "first_index", "base_vertex", "object", "bits", "_p0", "_p1", "_p2"]
        );
        assert_eq!(std::mem::size_of::<CullObject>(), 48);
        assert_eq!(
            wgsl_fields(src, "CullObject"),
            ["sphere", "flags", "avatar", "state", "anchor", "boxes", "box_count", "binds", "_p"]
        );
        assert_eq!(std::mem::size_of::<JointBox>(), 32);
        assert_eq!(wgsl_fields(src, "JointBox"), ["min", "joint", "max", "_p"]);
        assert_eq!(std::mem::size_of::<SkinBind>(), 80);
        assert_eq!(wgsl_fields(src, "SkinBind"), ["inverse_bind", "joint"]);
        assert!(src.contains(&format!("const POSED_RADIUS_MAX: f32 = {POSED_RADIUS_MAX:?};")));
        assert!(src.contains("const POSED_COORD_MAX: f32 = 1.0e6;") && POSED_COORD_MAX == 1.0e6);
        // Main, water reflection/exterior, mirror, probe and spot frusta;
        // then 3 matrices, 3 x 12 shadow planes and 4 vectors.
        assert_eq!(
            std::mem::size_of::<CullFrame>(),
            (5 + SPOT_SHADOWS) * 6 * 16 + 3 * 64 + 3 * SHADOW_PLANES * 16 + 4 * 16
        );
        assert_eq!(
            wgsl_fields(src, "CullFrame"),
            [
                "planes",
                "water",
                "water_air",
                "mirror",
                "probe",
                "spots",
                "cascades",
                "shadow_planes",
                "eye",
                "dist",
                "sizes",
                "groups"
            ]
        );
        assert!(src.contains(&format!("shadow_planes: array<vec4<f32>, {}>,", 3 * SHADOW_PLANES)));
        assert!(src.contains(&format!("counts: array<atomic<u32>, {COUNTS}>;")));
        const { assert!(COUNT_SHADOW + SHADOW_STATS <= COUNTS && COUNT_PRE2 + MAIN_BINS <= COUNT_SHADOW) };
    }

    #[test]
    fn first_person_visibility_hides_own_head_only_in_the_main_view() {
        let mut frame = frame_for_tests();
        frame.sizes[1] = 3;
        frame.sizes[3] = frame_flags::SHADOWS | frame_flags::REFLECTIONS | frame_flags::WATER | frame_flags::MIRROR;
        let mut objects = vec![
            CullObject {
                flags: object_flags::ACTIVE | object_flags::AVATAR,
                avatar: 0,
                state: avatar_state::FULL,
                ..CullObject::NONE
            };
            3
        ];
        objects[1].flags = object_flags::ACTIVE | object_flags::RIGGED | object_flags::FIRST_PERSON_HIDDEN;
        objects[1].avatar = 0;
        objects[2].avatar = 2;
        let spheres = vec![(Vec3::ZERO, 1.0); 3];
        let faces = [
            CullFace {
                object: 0,
                bits: pass::OPAQUE | FACE_FIRST,
                ..CullFace::NONE
            },
            CullFace {
                object: 0,
                bits: pass::OPAQUE | FACE_FIRST_PERSON_HIDDEN,
                ..CullFace::NONE
            },
            CullFace {
                object: 1,
                bits: pass::OPAQUE | FACE_FIRST,
                ..CullFace::NONE
            },
            CullFace {
                object: 2,
                bits: pass::OPAQUE | FACE_FIRST_PERSON_HIDDEN,
                ..CullFace::NONE
            },
        ];
        let original: Vec<_> = (0..faces.len()).map(|i| face_bins(&frame, &faces, &objects, &spheres, i)).collect();
        assert!(original.iter().all(|v| v.0 & 1 != 0));
        frame.groups[1] = 1;
        frame.sizes[3] |= frame_flags::FIRST_PERSON | frame_flags::SHOW_SELF;
        for (i, original_mask) in original.iter().enumerate() {
            let (mask, _) = face_bins(&frame, &faces, &objects, &spheres, i);
            if i == 1 || i == 2 {
                assert_eq!(mask & 15, 0, "own head/attachment must not enter depth or colour bins");
                assert_eq!(
                    mask & !15,
                    original_mask.0 & !15,
                    "the enabled body's secondary views remain complete"
                );
            } else {
                assert_eq!(mask, original_mask.0, "body and other avatars stay visible");
            }
        }
        frame.sizes[3] &= !frame_flags::SHOW_SELF;
        for i in 0..3 {
            assert_eq!(face_bins(&frame, &faces, &objects, &spheres, i), (0, false));
        }
        assert_eq!(face_bins(&frame, &faces, &objects, &spheres, 3), original[3]);
        frame.sizes[3] &= !frame_flags::FIRST_PERSON;
        for (i, expected) in original.into_iter().enumerate() {
            assert_eq!(
                face_bins(&frame, &faces, &objects, &spheres, i),
                expected,
                "leaving mouselook restores immediately"
            );
        }
    }

    /// The constants shared with the shader have the same values.
    #[test]
    fn constants_match_wgsl() {
        let src = include_str!("shaders/cull.wgsl");
        let value = |name: &str| -> u32 {
            let l = src
                .lines()
                .find(|l| l.starts_with(&format!("const {name}:")))
                .unwrap_or_else(|| panic!("const {name}"));
            let v = l
                .split('=')
                .nth(1)
                .expect("value")
                .trim()
                .trim_end_matches(';')
                .trim_end_matches('u');
            v.parse().unwrap_or_else(|_| panic!("{name}: {v}"))
        };
        let pairs: [(&str, u32); 47] = [
            ("F_FIRST_PERSON", frame_flags::FIRST_PERSON),
            ("F_SHOW_SELF", frame_flags::SHOW_SELF),
            ("OBJ_FIRST_PERSON_HIDDEN", object_flags::FIRST_PERSON_HIDDEN),
            ("FACE_FIRST_PERSON_HIDDEN", FACE_FIRST_PERSON_HIDDEN),
            ("PASS_WATER_EXCLUSION", pass::WATER_EXCLUSION),
            ("BIN_WATER_EXCLUSION", BIN_WATER_EXCLUSION as u32),
            ("BIN_SHADOW_ALPHA", BIN_SHADOW_ALPHA as u32),
            ("BIN_SPOT", BIN_SPOT as u32),
            ("F_SPOT", frame_flags::SPOT),
            ("COUNT_SHADOW", COUNT_SHADOW as u32),
            ("SHADOW_STATS", SHADOW_STATS as u32),
            ("SHADOW_PLANES", SHADOW_PLANES as u32),
            ("PASS_OPAQUE_2S", pass::OPAQUE_2S),
            ("F_SHADOW_STATS", frame_flags::SHADOW_STATS),
            ("F_SHADOW_ALPHA_ALL", frame_flags::SHADOW_ALPHA_ALL),
            ("F_BLEND_CASTS", frame_flags::BLEND_CASTS),
            ("BINS", BINS as u32),
            ("MAIN_BINS", MAIN_BINS as u32),
            ("BIN_SHADOW", BIN_SHADOW as u32),
            ("BIN_REFL_WATER", BIN_REFL_WATER as u32),
            ("BIN_REFL_MIRROR", BIN_REFL_MIRROR as u32),
            ("BIN_PROBE", BIN_PROBE as u32),
            ("REGION_PRE1", REGION_PRE1 as u32),
            ("REGION_PRE2", REGION_PRE2 as u32),
            ("COUNT_VISIBLE", COUNT_VISIBLE as u32),
            ("COUNT_TRIANGLES", COUNT_TRIANGLES as u32),
            ("COUNT_HIDDEN", COUNT_HIDDEN as u32),
            ("COUNT_PRE1", COUNT_PRE1 as u32),
            ("COUNT_PRE2", COUNT_PRE2 as u32),
            ("PASS_BITS", pass::BITS),
            ("PASS_HIDDEN", pass::HIDDEN),
            ("PASS_OPAQUE", pass::OPAQUE),
            ("PASS_MASK_2S", pass::MASK_2S),
            ("PASS_BLEND", pass::BLEND),
            ("FACE_FIRST", FACE_FIRST),
            ("OBJ_ACTIVE", object_flags::ACTIVE),
            ("OBJ_AVATAR", object_flags::AVATAR),
            ("OBJ_RIGGED", object_flags::RIGGED),
            ("OBJ_HUD", object_flags::HUD),
            ("OBJ_NO_PROBE", object_flags::NO_PROBE),
            ("OBJ_POSED", object_flags::POSED),
            ("OBJ_PHOTO_HIDDEN", object_flags::PHOTO_HIDDEN),
            ("SKELETON_ANCHOR", SKELETON_ANCHOR),
            ("AV_FULL", avatar_state::FULL),
            ("AV_TOO_COMPLEX", avatar_state::TOO_COMPLEX),
            ("AV_LOADING", avatar_state::LOADING),
            ("F_OCCLUSION", frame_flags::OCCLUSION),
        ];
        for (name, v) in pairs {
            assert_eq!(value(name), v, "{name}");
        }
        for (name, v) in [
            ("F_SHADOWS", frame_flags::SHADOWS),
            ("F_REFLECTIONS", frame_flags::REFLECTIONS),
            ("F_WATER", frame_flags::WATER),
            ("F_WATER_AIR", frame_flags::WATER_AIR),
            ("F_MIRROR", frame_flags::MIRROR),
            ("F_PROBE", frame_flags::PROBE),
        ] {
            assert_eq!(value(name), v, "{name}");
        }
        const { assert!(REGION_PRE2 + MAIN_BINS == REGIONS && REGION_PRE1 == BINS) };
    }

    #[test]
    fn dirty_chunks_coalesce_and_clip() {
        let mut d = DirtyChunks::default();
        assert!(d.take_ranges_as(1000, true).is_empty());
        d.mark(3);
        d.mark(70);
        d.mark(300);
        assert_eq!(d.take_ranges_as(310, true), vec![0..128, 256..310]);
        assert!(d.take_ranges_as(310, true).is_empty());
    }

    /// The same writes as with whole chunks, cut to the changed elements.
    #[test]
    fn dirty_ranges_stop_at_the_changed_elements() {
        let mut d = DirtyChunks::default();
        assert!(d.take_ranges_as(1000, false).is_empty() && !d.any());
        // one element alone, the last of a chunk, the first of the table
        d.mark(300);
        assert_eq!(d.take_ranges_as(1000, false), vec![300..301]);
        d.mark(63);
        d.mark(0);
        assert_eq!(d.take_ranges_as(1000, false), vec![0..64]);
        // neighbouring chunks in one write, from the first changed element
        // of the first to the last changed one of the last; marked in any
        // order, and twice
        for i in [200, 70, 75, 70, 3, 5, 600, 640] {
            d.mark(i);
        }
        assert!(d.any());
        assert_eq!(d.take_ranges_as(1000, false), vec![3..76, 200..201, 600..641]);
        assert!(d.take_ranges_as(1000, false).is_empty());
        // clipped to the table
        d.mark(310);
        d.mark(319);
        assert_eq!(d.take_ranges_as(315, false), vec![310..315]);
        d.mark(400);
        assert!(d.take_ranges_as(315, false).is_empty());
        // forgotten without an upload (the table was sent whole)
        d.mark(9);
        d.clear();
        assert!(d.take_ranges_as(1000, false).is_empty() && !d.any());
        // never more writes, never an element less than the changed ones
        let mut seed = 12345u32;
        let mut rand = |n: u32| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) % n
        };
        for _ in 0..200 {
            let (mut exact, mut whole) = (DirtyChunks::default(), DirtyChunks::default());
            let marks: Vec<usize> = (0..rand(40)).map(|_| rand(2000) as usize).collect();
            for &i in &marks {
                exact.mark(i);
                whole.mark(i);
            }
            let (e, w) = (exact.take_ranges_as(2000, false), whole.take_ranges_as(2000, true));
            assert_eq!(e.len(), w.len());
            for (e, w) in e.iter().zip(&w) {
                assert!(w.start <= e.start && e.end <= w.end);
                assert!(marks.contains(&e.start) && marks.contains(&(e.end - 1)));
            }
            assert!(marks.iter().all(|i| e.iter().any(|r| r.contains(i))));
        }
    }

    /// Packed 8-bit counters never carry into the next bin for a full
    /// workgroup, and give each face its rank among the earlier faces.
    #[test]
    fn packed_ranks_match_naive_ranks() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for round in 0..20 {
            let masks: Vec<u32> = (0..CLASSIFY_GROUP)
                .map(|_| {
                    if round == 0 {
                        (1 << BINS) - 1
                    } else {
                        (rand() as u32) & ((1 << BINS) - 1)
                    }
                })
                .collect();
            let ranks = workgroup_ranks(&masks);
            let mut seen = [0u32; BINS];
            for (m, r) in masks.iter().zip(&ranks) {
                for b in 0..BINS {
                    if (m >> b) & 1 != 0 {
                        assert_eq!(r[b], seen[b]);
                        seen[b] += 1;
                    }
                }
            }
        }
    }

    fn frame_for_tests() -> CullFrame {
        let view = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, Vec3::X, Vec3::Z);
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.5, 0.1);
        let planes = view_planes(proj * view).map(|p| p.to_array());
        // orthographic box of 100 m around the origin
        let cascade = Mat4::from_scale(Vec3::new(0.02, 0.02, 0.01));
        CullFrame {
            planes,
            water: planes,
            water_air: planes,
            mirror: planes,
            probe: planes,
            spots: [planes; SPOT_SHADOWS],
            cascades: [cascade.to_cols_array_2d(); 3],
            shadow_planes: [crate::shadow_cull::CASTER_PLANES_ALL; 3],
            eye: [0.0, 0.0, 0.0, 64.0],
            dist: [64.0, 32.0, 1000.0 / 0.5, 0.0],
            sizes: [0, 4, 0, frame_flags::SHADOWS | frame_flags::REFLECTIONS | frame_flags::WATER],
            groups: [0; 4],
        }
    }

    #[test]
    fn water_exclusion_has_no_scene_shadow_or_reflection_bins() {
        let frame = frame_for_tests();
        let mut objects = [CullObject {
            sphere: [10.0, 0.0, 0.0, 1.0],
            flags: object_flags::ACTIVE,
            ..CullObject::NONE
        }];
        let faces = [CullFace {
            object: 0,
            bits: pass::WATER_EXCLUSION | FACE_FIRST,
            index_count: 6,
            ..CullFace::NONE
        }];
        let spheres = world_spheres(&objects, &[], &[], &[]);
        assert_eq!(face_bins(&frame, &faces, &objects, &spheres, 0), (1 << BIN_WATER_EXCLUSION, true));
        for flag in [object_flags::HUD, object_flags::AVATAR, object_flags::RIGGED] {
            objects[0].flags = object_flags::ACTIVE | flag;
            assert_eq!(face_bins(&frame, &faces, &objects, &spheres, 0).0, 0);
        }
        objects[0].flags = object_flags::ACTIVE;
        objects[0].avatar = 0;
        objects[0].state = avatar_state::FULL;
        assert_eq!(
            face_bins(&frame, &faces, &objects, &spheres, 0).0,
            0,
            "attachments do not exclude water"
        );
    }

    #[test]
    fn water_bin_includes_the_exterior_frustum_without_main_visibility() {
        let mut frame = frame_for_tests();
        let objects = [CullObject {
            sphere: [-10.0, 0.0, 0.0, 1.0],
            flags: object_flags::ACTIVE,
            ..CullObject::NONE
        }];
        let faces = [CullFace {
            index_count: 30,
            object: 0,
            bits: pass::OPAQUE | FACE_FIRST,
            ..CullFace::NONE
        }];
        let spheres = world_spheres(&objects, &[], &[], &[]);
        assert_eq!(face_bins(&frame, &faces, &objects, &spheres, 0).0 & (1 << BIN_REFL_WATER), 0);
        let view = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, -Vec3::X, Vec3::Z);
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.5, 0.1);
        frame.water_air = view_planes(proj * view).map(|p| p.to_array());
        frame.sizes[3] |= frame_flags::WATER_AIR;
        let (mask, visible) = face_bins(&frame, &faces, &objects, &spheres, 0);
        assert_ne!(mask & (1 << BIN_REFL_WATER), 0, "offscreen exterior must reach its render pass");
        assert!(!visible, "exterior visibility must not make the object visible in the main view");
    }

    #[test]
    fn reference_bins_follow_the_cpu_rules() {
        let frame = frame_for_tests();
        let obj = |c: [f32; 3], r: f32| CullObject {
            sphere: [c[0], c[1], c[2], r],
            flags: object_flags::ACTIVE,
            ..CullObject::NONE
        };
        let mut objects = vec![
            obj([10.0, 0.0, 0.0], 1.0),  // in view, near
            obj([-10.0, 0.0, 0.0], 1.0), // behind: shadows only
            obj([100.0, 0.0, 0.0], 1.0), // beyond the draw distance
            CullObject {
                flags: object_flags::ACTIVE | object_flags::AVATAR,
                avatar: 3,
                state: 0,
                ..obj([5.0, 0.0, 0.0], 1.0)
            },
        ];
        let face = |object: u32, p: u32| CullFace {
            index_count: 30,
            object,
            bits: p | FACE_FIRST,
            ..CullFace::NONE
        };
        let faces = [
            face(0, pass::OPAQUE),
            face(0, pass::BLEND),
            face(1, pass::MASK),
            face(2, pass::OPAQUE),
            face(3, pass::OPAQUE),
            face(0, pass::HIDDEN),
        ];
        let shadows = 0b111 << BIN_SHADOW;
        let spheres = world_spheres(&objects, &[], &[], &[]);
        assert_eq!(
            face_bins(&frame, &faces, &objects, &spheres, 0),
            (1 | shadows | 1 << BIN_REFL_WATER, true)
        );
        // blended: the CPU draws it; a caster with the alpha test, rigged or
        // not (renderAlphaObjects), unless AURORA_BLEND_SHADOWS=0
        let mut blend = frame;
        blend.sizes[3] |= frame_flags::BLEND_CASTS;
        assert_eq!(face_bins(&blend, &faces, &objects, &spheres, 1), (0b111 << BIN_SHADOW_ALPHA, true));
        assert_eq!(face_bins(&frame, &faces, &objects, &spheres, 1), (0, true));
        objects[0].flags |= object_flags::RIGGED;
        assert_eq!(face_bins(&frame, &faces, &objects, &spheres, 1), (0b111 << BIN_SHADOW_ALPHA, true));
        objects[0].flags &= !object_flags::RIGGED;
        // behind the camera: casts only; masked: with the alpha test
        assert_eq!(face_bins(&frame, &faces, &objects, &spheres, 2).0, 0b111 << BIN_SHADOW_ALPHA);
        // the old path draws every caster with the alpha test
        let mut legacy = frame;
        legacy.sizes[3] |= frame_flags::SHADOW_ALPHA_ALL;
        assert_eq!(
            face_bins(&legacy, &faces, &objects, &spheres, 0).0,
            1 | 0b111 << BIN_SHADOW_ALPHA | 1 << BIN_REFL_WATER
        );
        // a cascade whose receivers the caster cannot shadow leaves it out
        let mut tight = frame;
        tight.shadow_planes[1][0] = [-1.0, 0.0, 0.0, 5.0];
        assert_eq!(face_bins(&tight, &faces, &objects, &spheres, 0).0 >> BIN_SHADOW & 7, 0b101);
        assert_eq!(face_bins(&tight, &faces, &objects, &spheres, 2).0, 0b111 << BIN_SHADOW_ALPHA);
        assert_eq!(face_bins(&frame, &faces, &objects, &spheres, 3), (0, false));
        // an avatar not among the nearest: impostor, nothing on the GPU
        assert_eq!(face_bins(&frame, &faces, &objects, &spheres, 4), (0, false));
        objects[3].state = avatar_state::FULL;
        assert_eq!(face_bins(&frame, &faces, &objects, &spheres, 4).0 & 1, 1);
        objects[3].state = avatar_state::FULL | avatar_state::LOADING;
        assert_eq!(face_bins(&frame, &faces, &objects, &spheres, 4).0, 0);
        assert_eq!(face_bins(&frame, &faces, &objects, &spheres, 5), (0, true));
    }

    /// A worn object is tested where its anchor puts it: the sphere of the
    /// table is relative to the anchor matrix.
    #[test]
    fn anchored_spheres_follow_their_anchor() {
        let frame = frame_for_tests();
        let worn = CullObject {
            sphere: [0.0, 1.0, 0.0, 1.0],
            flags: object_flags::ACTIVE,
            anchor: 2,
            ..CullObject::NONE
        };
        let faces = [CullFace {
            index_count: 30,
            object: 0,
            bits: pass::OPAQUE | FACE_FIRST,
            ..CullFace::NONE
        }];
        // a quarter turn about Z, then 10 m ahead of the camera
        let ahead = Mat4::from_rotation_translation(glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), Vec3::new(11.0, 0.0, 0.0));
        let behind = Mat4::from_translation(Vec3::new(-10.0, 0.0, 0.0));
        let (c, r) = worn.world_sphere(&[behind.to_cols_array_2d(), ahead.to_cols_array_2d()], &[], &[]);
        assert!(c.distance(Vec3::new(10.0, 0.0, 0.0)) < 1e-5 && r == 1.0);
        let shadows = 0b111 << BIN_SHADOW;
        let palettes = [behind.to_cols_array_2d(), ahead.to_cols_array_2d()];
        assert_eq!(
            face_bins(&frame, &faces, &[worn], &world_spheres(&[worn], &palettes, &[], &[]), 0),
            (1 | shadows | 1 << BIN_REFL_WATER, true)
        );
        // the wearer turned its back to the camera: casts only
        let palettes = [behind.to_cols_array_2d(), behind.to_cols_array_2d()];
        assert_eq!(
            face_bins(&frame, &faces, &[worn], &world_spheres(&[worn], &palettes, &[], &[]), 0).0,
            shadows
        );
        // an anchor the buffer does not hold: the sphere as it is
        assert_eq!(worn.world_sphere(&[], &[], &[]).0, Vec3::new(0.0, 1.0, 0.0));
    }

    /// A rigged mesh posed by the GPU is enclosed where its skeleton puts
    /// its joint boxes; without a sane pose, its anchored sphere stands.
    #[test]
    fn posed_spheres_enclose_the_joint_boxes() {
        let skeleton = SKELETON_ANCHOR as usize;
        let mut palettes = vec![Mat4::IDENTITY.to_cols_array_2d(); skeleton + 1];
        // the skeleton stands 20 m east; joint 3 raises its box by 2 m and
        // turns it a quarter turn about Z
        palettes[skeleton] = Mat4::from_translation(Vec3::new(20.0, 0.0, 0.0)).to_cols_array_2d();
        palettes[3] = Mat4::from_rotation_translation(glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), Vec3::new(0.0, 0.0, 2.0))
            .to_cols_array_2d();
        let binds = [
            SkinBind {
                inverse_bind: Mat4::IDENTITY.to_cols_array_2d(),
                joint: [0; 4],
            },
            // (bind pose: the joint sits 1 m up)
            SkinBind {
                inverse_bind: Mat4::from_translation(Vec3::new(0.0, 0.0, -1.0)).to_cols_array_2d(),
                joint: [3, 0, 0, 0],
            },
        ];
        let boxes = [
            JointBox::NONE,
            JointBox {
                min: [0.0, -0.1, 1.0],
                joint: 1,
                max: [1.0, 0.1, 1.2],
                _pad: 0,
            },
        ];
        let mesh = CullObject {
            // (rest: around the skeleton's root)
            sphere: [0.0, 0.0, 1.0, 2.5],
            flags: object_flags::ACTIVE | object_flags::RIGGED | object_flags::POSED,
            anchor: skeleton as u32 + 1,
            boxes: 1,
            box_count: 1,
            binds: 0,
            ..CullObject::NONE
        };
        // box x 0..1 turned onto y 0..1, z 1..1.2 lowered by the bind and
        // raised by the joint to 2..2.2, then moved with the skeleton
        let (c, r) = mesh.world_sphere(&palettes, &binds, &boxes);
        assert!(c.distance(Vec3::new(20.0, 0.5, 2.1)) < 1e-4, "{c}");
        assert!((r - Vec3::new(0.1, 0.5, 0.1).length()).abs() < 1e-4, "{r}");
        // not posed: the anchored sphere
        let rest = (Vec3::new(20.0, 0.0, 1.0), 2.5);
        let plain = CullObject {
            flags: object_flags::ACTIVE | object_flags::RIGGED,
            ..mesh
        };
        assert_eq!(plain.world_sphere(&palettes, &binds, &boxes), rest);
        // no box, boxes or bindings out of the tables, a palette of NaN
        assert_eq!(CullObject { box_count: 0, ..mesh }.world_sphere(&palettes, &binds, &boxes), rest);
        assert_eq!(CullObject { boxes: 2, ..mesh }.world_sphere(&palettes, &binds, &boxes), rest);
        assert_eq!(CullObject { binds: 1, ..mesh }.world_sphere(&palettes, &binds, &boxes), rest);
        palettes[3] = [[f32::NAN; 4]; 4];
        assert_eq!(mesh.world_sphere(&palettes, &binds, &boxes), rest);
        // a pose thrown far away is no pose
        palettes[3] = Mat4::from_translation(Vec3::new(0.0, 500.0, 0.0)).to_cols_array_2d();
        assert_eq!(mesh.world_sphere(&palettes, &binds, &boxes), rest);
    }

    /// The compute passes give the reference bins, in record order, for a
    /// random scene larger than one workgroup and one scan chunk. Needs a
    /// GPU: `cargo test -p aurora-render gpu_bins -- --ignored`.
    #[test]
    #[ignore]
    fn gpu_bins_match_the_reference() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .expect("GPU device");
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rand = move |n: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as u32
        };
        let mut cull = GpuCull::new(&device);
        let mut tables = CullTables::new(&device);
        let n_obj = 3000u32;
        // two skeletons: joints, then anchors. Whole-metre translations,
        // which the GPU and the reference apply to the same bit
        let per = SKELETON_ANCHOR as usize + 48;
        let palettes: Vec<[[f32; 4]; 4]> = (0..2 * per)
            .map(|k| {
                let range = if k % per < SKELETON_ANCHOR as usize { 3 } else { 60 };
                let mut at = |n: u32| rand(n) as f32 - (n / 2) as f32;
                Mat4::from_translation(Vec3::new(at(range), at(range), at(range.min(10)))).to_cols_array_2d()
            })
            .collect();
        let binds: Vec<SkinBind> = (0..200)
            .map(|_| SkinBind {
                inverse_bind: Mat4::from_translation(Vec3::new(rand(3) as f32 - 1.0, rand(3) as f32 - 1.0, rand(3) as f32 - 1.0))
                    .to_cols_array_2d(),
                joint: [rand(SKELETON_ANCHOR), 0, 0, 0],
            })
            .collect();
        let bind_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: std::mem::size_of_val(&binds[..]) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&bind_buffer, 0, bytemuck::cast_slice(&binds));
        let boxes: Vec<JointBox> = (0..300)
            .map(|_| {
                let min = [rand(3) as f32 - 2.0, rand(3) as f32 - 2.0, rand(3) as f32 - 2.0];
                JointBox {
                    min,
                    joint: rand(40),
                    max: [min[0] + 1.0 + rand(2) as f32, min[1] + 1.0, min[2] + rand(3) as f32],
                    _pad: 0,
                }
            })
            .collect();
        for (i, b) in boxes.iter().enumerate() {
            tables.boxes.set(i, *b);
        }
        let palette_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (palettes.len() * 64) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&palette_buffer, 0, bytemuck::cast_slice(&palettes));
        let mut objects = Vec::new();
        for i in 0..n_obj {
            let avatar = rand(10) == 0;
            let o = CullObject {
                sphere: [
                    rand(200) as f32 - 100.0,
                    rand(200) as f32 - 100.0,
                    rand(40) as f32 - 20.0,
                    0.1 + rand(50) as f32 * 0.1,
                ],
                flags: object_flags::ACTIVE
                    | if avatar { object_flags::AVATAR } else { 0 }
                    | if rand(8) == 0 { object_flags::RIGGED } else { 0 }
                    | if rand(30) == 0 { object_flags::HUD } else { 0 }
                    | if rand(5) == 0 { object_flags::NO_PROBE } else { 0 },
                avatar: if avatar {
                    i
                } else if rand(6) == 0 {
                    rand(n_obj)
                } else {
                    NO_OBJECT
                },
                state: rand(8),
                anchor: if !avatar && rand(4) == 0 { 1 + rand(2 * per as u32) } else { 0 },
                ..CullObject::NONE
            };
            // a rigged mesh posed by one of the skeletons: a few joint
            // boxes (now and then out of the tables: its rest sphere)
            let bind_range = if rand(20) == 0 { 400 } else { 160 };
            let o = if !avatar && rand(5) == 0 {
                CullObject {
                    sphere: [0.0, 0.0, 1.0, 3.0],
                    flags: o.flags | object_flags::RIGGED | object_flags::POSED,
                    anchor: 1 + rand(2) * per as u32 + SKELETON_ANCHOR,
                    boxes: rand(290),
                    box_count: rand(7),
                    binds: rand(bind_range),
                    ..o
                }
            } else {
                o
            };
            let o = if i < 32 {
                CullObject {
                    sphere: [0.0, i as f32 * 0.1, 0.0, 2.0],
                    flags: object_flags::ACTIVE
                        | if i == 0 { object_flags::AVATAR } else { object_flags::RIGGED }
                        | if i.is_multiple_of(2) {
                            object_flags::FIRST_PERSON_HIDDEN
                        } else {
                            0
                        },
                    avatar: 0,
                    state: avatar_state::FULL,
                    ..CullObject::NONE
                }
            } else {
                o
            };
            tables.objects.set(i as usize, o);
            objects.push(o);
        }
        assert!(objects.iter().any(|o| o.flags & object_flags::POSED != 0));
        let spheres = world_spheres(&objects, &palettes, &binds, &boxes);
        assert!(
            objects
                .iter()
                .zip(&spheres)
                .any(|(o, s)| o.flags & object_flags::POSED != 0 && s.1 != 3.0),
            "some object is posed"
        );
        let n_faces = 70_000usize;
        let faces: Vec<CullFace> = (0..n_faces)
            .map(|_| {
                if rand(20) == 0 {
                    return CullFace::NONE;
                }
                CullFace {
                    index_count: 3 + 3 * rand(100),
                    first_index: rand(100_000),
                    base_vertex: rand(100_000) as i32,
                    object: rand(n_obj),
                    bits: rand(7) | if rand(4) == 0 { FACE_FIRST } else { 0 } | if rand(4) == 0 { FACE_FIRST_PERSON_HIDDEN } else { 0 },
                    _pad: [0; 3],
                }
            })
            .collect();
        let mut table = GpuTable::new(&device, "faces", 16, CullFace::NONE);
        for (i, f) in faces.iter().enumerate() {
            table.set(i, *f);
        }
        let mut writes = GpuWrites::default();
        table.flush(&device, &mut writes);
        tables.objects.flush(&device, &mut writes);
        tables.boxes.flush(&device, &mut writes);
        writes.replay(&mut crate::writes::QueueSink(&queue));
        // visible last frame (occlusion phase 1 list)
        let was: Vec<u32> = (0..n_faces).map(|_| rand(2)).collect();
        let visibility = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: n_faces as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&visibility, 0, bytemuck::cast_slice(&was));
        let mut frame = frame_for_tests();
        frame.groups[1] = 1;
        frame.sizes[3] = frame_flags::FIRST_PERSON
            | frame_flags::SHOW_SELF
            | frame_flags::SHADOWS
            | frame_flags::REFLECTIONS
            | frame_flags::WATER
            | frame_flags::WATER_AIR
            | frame_flags::PROBE
            | frame_flags::OCCLUSION
            | frame_flags::SHADOW_STATS
            | frame_flags::BLEND_CASTS;
        // caster volumes: the east half in the second cascade, a slanted slab in the third
        frame.shadow_planes[1][0] = [-1.0, 0.0, 0.0, 5.0];
        frame.shadow_planes[2][3] = [0.6, 0.0, 0.8, 10.0];
        frame.shadow_planes[2][SHADOW_PLANES - 1] = [-0.6, 0.0, -0.8, 30.0];
        // merged cascades: squares of 10, 25 and 50 m, the opaque casters
        // of a finer one left out of the next
        frame.cascades = [0.1, 0.04, 0.02].map(|s| Mat4::from_scale(Vec3::new(s, s, 0.01)).to_cols_array_2d());
        frame.dist[3] = 2048.0;
        // Exterior and reflection share one draw bin but retain independent
        // frusta. Include objects behind the main/reflection camera as well.
        frame.water_air = view_planes(
            glam::camera::rh::proj::directx::perspective_infinite_reverse(1.8, 1.6, 0.01)
                * glam::camera::rh::view::look_at_mat4(Vec3::ZERO, -Vec3::Y, Vec3::Z),
        )
        .map(|p| p.to_array());
        frame.probe = view_planes(
            glam::camera::rh::proj::directx::perspective_infinite_reverse(1.5, 1.0, 0.1)
                * glam::camera::rh::view::look_at_mat4(Vec3::ZERO, Vec3::Y, Vec3::Z),
        )
        .map(|p| p.to_array());
        cull.prepare(&device, &queue, n_faces, tables.objects.len(), frame);
        let mut f = frame;
        f.sizes[0] = n_faces as u32;
        f.sizes[1] = n_obj;
        let mut encoder = device.create_command_encoder(&Default::default());
        cull.encode(
            &device,
            &mut encoder,
            &CullInputs {
                faces: table.buffer(),
                objects: tables.objects.buffer(),
                palettes: &palette_buffer,
                skin_binds: &bind_buffer,
                boxes: tables.boxes.buffer(),
            },
            &visibility,
            None,
        );
        let size = cull.args().size();
        let read = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let args_read = read("args");
        let counts_read = read("counts");
        encoder.copy_buffer_to_buffer(cull.args(), 0, &args_read, 0, size);
        encoder.copy_buffer_to_buffer(cull.counts(), 0, &counts_read, 0, (COUNTS * 4) as u64);
        queue.submit([encoder.finish()]);
        args_read.map_async(wgpu::MapMode::Read, .., |_| {});
        counts_read.map_async(wgpu::MapMode::Read, .., |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let args = args_read.get_mapped_range(..).expect("args mapped");
        let args: &[u32] = bytemuck::cast_slice(&args);
        let counts = counts_read.get_mapped_range(..).expect("counts mapped");
        let counts: &[u32] = bytemuck::cast_slice(&counts[..COUNTS * 4]);
        let mut expect: Vec<Vec<u32>> = vec![Vec::new(); BINS];
        let (mut visible, mut tris) = (0, 0);
        for i in 0..n_faces {
            let (m, v) = face_bins(&f, &faces, &objects, &spheres, i);
            visible += v as u32;
            for (b, e) in expect.iter_mut().enumerate() {
                if (m >> b) & 1 != 0 {
                    e.push(i as u32);
                    if b < MAIN_BINS {
                        tris += faces[i].index_count / 3;
                    }
                }
            }
        }
        assert!(expect.iter().take(MAIN_BINS + 3).all(|e| !e.is_empty()), "every bin used");
        assert!(
            expect[BIN_SHADOW_ALPHA..BIN_SHADOW_ALPHA + 3].iter().all(|e| !e.is_empty()),
            "alpha tested casters in every cascade"
        );
        let cascades = frame.cascades.map(|m| Mat4::from_cols_array_2d(&m));
        let left_out = (0..n_obj as usize)
            .filter(|&o| merged_from_finer(&cascades, &frame.shadow_planes, frame.dist[3], 2, spheres[o].0, spheres[o].1))
            .count();
        assert!(
            left_out > 50 && left_out < n_obj as usize - 50,
            "{left_out} objects left out of the third cascade"
        );
        // the casters' statistics of each cascade
        for k in 0..3 {
            let casters = || expect[BIN_SHADOW + k].iter().chain(&expect[BIN_SHADOW_ALPHA + k]);
            let rigged =
                |rec: &&u32| objects[faces[**rec as usize].object as usize].flags & (object_flags::RIGGED | object_flags::AVATAR) != 0;
            let tris = |rec: &u32| faces[*rec as usize].index_count / 3;
            assert_eq!(counts[COUNT_SHADOW + k], casters().map(tris).sum::<u32>(), "cascade {k} triangles");
            assert_eq!(
                counts[COUNT_SHADOW + 3 + k],
                casters().filter(rigged).count() as u32,
                "cascade {k} rigged"
            );
            assert_eq!(counts[COUNT_SHADOW + 6 + k], casters().filter(rigged).map(tris).sum::<u32>());
        }
        assert_eq!(counts[COUNT_VISIBLE], visible);
        assert_eq!(counts[COUNT_TRIANGLES], tris);
        let cap = cull.cap() as usize;
        for (b, e) in expect.iter().enumerate() {
            assert_eq!(counts[b] as usize, e.len(), "bin {b} size");
            for (j, &rec) in e.iter().enumerate() {
                let a = &args[(b * cap + j) * 5..(b * cap + j) * 5 + 5];
                let fc = &faces[rec as usize];
                assert_eq!(
                    a,
                    [fc.index_count, 1, fc.first_index, fc.base_vertex as u32, rec],
                    "bin {b} slot {j}"
                );
            }
            if b < MAIN_BINS {
                // phase 1 prepass list: the bin's draws visible last frame, any order
                let mut want: Vec<u32> = e.iter().copied().filter(|&r| was[r as usize] != 0).collect();
                let n = counts[COUNT_PRE1 + b] as usize;
                let mut got: Vec<u32> = (0..n).map(|j| args[((REGION_PRE1 + b) * cap + j) * 5 + 4]).collect();
                want.sort_unstable();
                got.sort_unstable();
                assert_eq!(got, want, "phase 1 list of bin {b}");
            }
        }
    }
}
