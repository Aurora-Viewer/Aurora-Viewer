// GPU-driven draw lists (gpu_cull.rs). Every drawable face (one per draw
// record, table kept up to date by the scene) is tested against each view
// of the frame and appended to the bins it belongs to:
//   0..3  main view: opaque, opaque two-sided, masked, masked two-sided
//   4..6  shadow cascades
//   7..9  water reflection, mirror, reflection probe face
// Bins keep the record order (no atomics on the slots): coplanar faces drawn
// in the same pass keep the same winner from one frame to the next.
//   cs_classify: bin mask of each face, face count of each bin per workgroup;
//   cs_scan:     offset of each workgroup in each bin, bin sizes;
//   cs_scatter:  indirect draw arguments at their slot, and the occlusion's
//                phase 1 prepass list (what was visible last frame);
//   cs_occlude:  after the Hi-Z pyramid, the occlusion test of the main bins
//                (instance_count 0 when hidden) and the phase 2 prepass list.
// The prepass lists are depth only: their order does not matter, they are
// appended with atomics and hold only the draws to make.
// The tests mirror Scene::build_lists (CPU path) and `gpu_cull::face_bins`.
// The Hi-Z sphere test (hiz_test.wgsl) is prepended to this module.

const BINS: u32 = 10u;
const MAIN_BINS: u32 = 4u;
const BIN_SHADOW: u32 = 4u;
const BIN_REFL_WATER: u32 = 7u;
const BIN_REFL_MIRROR: u32 = 8u;
const BIN_PROBE: u32 = 9u;
// argument regions after the bins: occlusion phase 1 and phase 2 prepass
// lists of each main bin (own counts)
const REGION_PRE1: u32 = 10u;
const REGION_PRE2: u32 = 14u;

// counts: bin sizes, then statistics
const COUNT_VISIBLE: u32 = 10u;
const COUNT_TRIANGLES: u32 = 11u;
const COUNT_HIDDEN: u32 = 12u;
const COUNT_PRE1: u32 = 16u;
const COUNT_PRE2: u32 = 20u;

// CullFace.bits
const PASS_BITS: u32 = 7u;
const PASS_HIDDEN: u32 = 0u;
const PASS_OPAQUE: u32 = 1u;
const PASS_MASK_2S: u32 = 4u;
const PASS_BLEND: u32 = 5u;
const FACE_FIRST: u32 = 8u;

// CullObject.flags
const OBJ_ACTIVE: u32 = 1u;
const OBJ_AVATAR: u32 = 2u;
const OBJ_RIGGED: u32 = 4u;
const OBJ_HUD: u32 = 8u;
const OBJ_NO_PROBE: u32 = 16u;

// CullObject.state (avatars)
const AV_FULL: u32 = 1u;
const AV_TOO_COMPLEX: u32 = 2u;
const AV_LOADING: u32 = 4u;

// CullFrame.sizes.w
const F_SHADOWS: u32 = 1u;
const F_REFLECTIONS: u32 = 2u;
const F_WATER: u32 = 4u;
const F_MIRROR: u32 = 8u;
const F_PROBE: u32 = 16u;
const F_OCCLUSION: u32 = 32u;

struct CullFace {
    index_count: u32,
    first_index: u32,
    base_vertex: i32,
    object: u32,
    bits: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
}

struct CullObject {
    // bounding sphere (avatars: their posed body)
    sphere: vec4<f32>,
    flags: u32,
    // avatar whose state applies (itself, or the wearer), or none
    avatar: u32,
    state: u32,
    _p: u32,
}

struct CullFrame {
    // main view (6th plane accepts all: reverse-Z infinite projection)
    planes: array<vec4<f32>, 6>,
    water: array<vec4<f32>, 6>,
    mirror: array<vec4<f32>, 6>,
    probe: array<vec4<f32>, 6>,
    cascades: array<mat4x4<f32>, 3>,
    // xyz camera, w draw distance
    eye: vec4<f32>,
    // x shadow distance, y reflection distance,
    // z pixel scale (screen height / tan(fov / 2))
    dist: vec4<f32>,
    // x faces, y objects, z slots per bin, w flags
    sizes: vec4<u32>,
    // x classify workgroups
    groups: vec4<u32>,
}

struct OcclParams {
    view_proj: mat4x4<f32>,
    screen: vec2<f32>,
    levels: u32,
    count: u32,
}

@group(0) @binding(0) var<uniform> frame: CullFrame;
@group(0) @binding(1) var<storage, read> faces: array<CullFace>;
@group(0) @binding(2) var<storage, read> objects: array<CullObject>;
// visible last frame, by record (occlusion.rs)
@group(0) @binding(3) var<storage, read_write> visibility: array<u32>;
@group(0) @binding(4) var<storage, read_write> masks: array<u32>;
// per workgroup and bin: face count, then (after cs_scan) offset
@group(0) @binding(5) var<storage, read_write> group_data: array<u32>;
@group(0) @binding(6) var<storage, read_write> counts: array<atomic<u32>, 32>;
// DrawIndexedIndirect: index_count, instance_count, first_index, base_vertex, first_instance
@group(0) @binding(7) var<storage, read_write> args: array<u32>;
@group(0) @binding(8) var<uniform> occl: OcclParams;
@group(0) @binding(9) var hiz: texture_2d<f32>;

fn sphere_in(planes: array<vec4<f32>, 6>, c: vec3<f32>, r: f32) -> bool {
    var p = planes;
    for (var k = 0u; k < 6u; k++) {
        if (dot(p[k].xyz, c) + p[k].w < -r) {
            return false;
        }
    }
    return true;
}

// Renderer cascade test: orthographic, radius in clip units.
fn in_cascade(m: mat4x4<f32>, c: vec3<f32>, r: f32) -> bool {
    let p = m * vec4<f32>(c, 1.0);
    let rr = r * length(m[0].xyz);
    return !(p.x + rr < -1.0 || p.x - rr > 1.0 || p.y + rr < -1.0 || p.y - rr > 1.0 || p.z - rr > 1.0);
}

var<workgroup> wg_bins: array<atomic<u32>, BINS>;
var<workgroup> wg_stats: array<atomic<u32>, 2>;

// Bins of face `i` this frame (0: not drawn anywhere).
fn face_mask(i: u32) -> u32 {
    let f = faces[i];
    let n = frame.sizes.y;
    if (f.object >= n) {
        return 0u;
    }
    let o = objects[f.object];
    if ((o.flags & OBJ_ACTIVE) == 0u || (o.flags & OBJ_HUD) != 0u) {
        return 0u;
    }
    let is_avatar = (o.flags & OBJ_AVATAR) != 0u;
    // avatar limit (impostors are pictured by the CPU), too complex
    // (silhouette without attachments), still loading (cloud)
    if (o.avatar < n) {
        let st = objects[o.avatar].state;
        if ((st & AV_FULL) == 0u || ((st & AV_TOO_COMPLEX) != 0u && !is_avatar) || (st & AV_LOADING) != 0u) {
            return 0u;
        }
    }
    let c = o.sphere.xyz;
    let r = o.sphere.w;
    let d = length(c - frame.eye.xyz);
    if (d - r > frame.eye.w && !is_avatar) {
        return 0u;
    }
    let fl = frame.sizes.w;
    let in_view = sphere_in(frame.planes, c, r);
    let casts = (fl & F_SHADOWS) != 0u && d - r < frame.dist.x;
    let reflects = (fl & F_REFLECTIONS) != 0u && d - r < frame.dist.y;
    if (!in_view && !casts && !reflects) {
        return 0u;
    }
    // tiny objects far away (small object culling)
    let px = r * frame.dist.z / max(d, 0.1);
    if (px < 1.5 && !is_avatar) {
        return 0u;
    }
    if (in_view && (f.bits & FACE_FIRST) != 0u) {
        atomicAdd(&wg_stats[0], 1u);
    }
    let fp = f.bits & PASS_BITS;
    if (fp == PASS_HIDDEN) {
        return 0u;
    }
    var m = 0u;
    // LLDrawPoolAvatar::renderShadow: blended rigged faces cast too
    if (casts && (fp != PASS_BLEND || (o.flags & (OBJ_RIGGED | OBJ_AVATAR)) != 0u)) {
        for (var k = 0u; k < 3u; k++) {
            if (in_cascade(frame.cascades[k], c, r)) {
                m |= 1u << (BIN_SHADOW + k);
            }
        }
    }
    let solid = fp >= PASS_OPAQUE && fp <= PASS_MASK_2S;
    if (reflects && solid) {
        if ((fl & F_WATER) != 0u && sphere_in(frame.water, c, r)) {
            m |= 1u << BIN_REFL_WATER;
        }
        if ((fl & F_MIRROR) != 0u && sphere_in(frame.mirror, c, r)) {
            m |= 1u << BIN_REFL_MIRROR;
        }
        if ((fl & F_PROBE) != 0u && (o.flags & OBJ_NO_PROBE) == 0u && sphere_in(frame.probe, c, r)) {
            m |= 1u << BIN_PROBE;
        }
    }
    if (in_view && solid) {
        m |= 1u << (fp - PASS_OPAQUE);
        atomicAdd(&wg_stats[1], f.index_count / 3u);
    }
    return m;
}

@compute @workgroup_size(128)
fn cs_classify(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
    @builtin(workgroup_id) wid: vec3<u32>,
) {
    if (lid < BINS) {
        atomicStore(&wg_bins[lid], 0u);
    }
    if (lid < 2u) {
        atomicStore(&wg_stats[lid], 0u);
    }
    workgroupBarrier();
    let i = gid.x;
    var m = 0u;
    if (i < frame.sizes.x) {
        m = face_mask(i);
        masks[i] = m;
    }
    for (var b = 0u; b < BINS; b++) {
        if (((m >> b) & 1u) != 0u) {
            atomicAdd(&wg_bins[b], 1u);
        }
    }
    workgroupBarrier();
    if (lid < BINS) {
        group_data[wid.x * BINS + lid] = atomicLoad(&wg_bins[lid]);
    }
    if (lid == 0u) {
        atomicAdd(&counts[COUNT_VISIBLE], atomicLoad(&wg_stats[0]));
        atomicAdd(&counts[COUNT_TRIANGLES], atomicLoad(&wg_stats[1]));
    }
}

var<workgroup> scan_tmp: array<u32, 256>;

// One workgroup per bin: exclusive prefix sum of the workgroup counts, in
// place, 256 at a time.
@compute @workgroup_size(256)
fn cs_scan(@builtin(local_invocation_index) lid: u32, @builtin(workgroup_id) wid: vec3<u32>) {
    let b = wid.x;
    let n = frame.groups.x;
    var carry = 0u;
    for (var base = 0u; base < n; base += 256u) {
        let g = base + lid;
        var v = 0u;
        if (g < n) {
            v = group_data[g * BINS + b];
        }
        scan_tmp[lid] = v;
        workgroupBarrier();
        for (var off = 1u; off < 256u; off <<= 1u) {
            var add = 0u;
            if (lid >= off) {
                add = scan_tmp[lid - off];
            }
            workgroupBarrier();
            scan_tmp[lid] += add;
            workgroupBarrier();
        }
        if (g < n) {
            group_data[g * BINS + b] = carry + scan_tmp[lid] - v;
        }
        carry += scan_tmp[255];
        workgroupBarrier();
    }
    if (lid == 0u) {
        atomicStore(&counts[b], carry);
    }
}

// One 8-bit counter per bin, four bins per lane (128 threads: at most 128).
fn pack_bins(m: u32) -> vec4<u32> {
    var v = vec4<u32>(0u);
    for (var b = 0u; b < BINS; b++) {
        if (((m >> b) & 1u) != 0u) {
            v[b >> 2u] += 1u << ((b & 3u) * 8u);
        }
    }
    return v;
}

fn write_args(slot: u32, f: CullFace, rec: u32, instances: u32) {
    let a = slot * 5u;
    args[a] = f.index_count;
    args[a + 1u] = instances;
    args[a + 2u] = f.first_index;
    args[a + 3u] = bitcast<u32>(f.base_vertex);
    args[a + 4u] = rec;
}

var<workgroup> ranks: array<vec4<u32>, 128>;

@compute @workgroup_size(128)
fn cs_scatter(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
    @builtin(workgroup_id) wid: vec3<u32>,
) {
    let i = gid.x;
    var m = 0u;
    if (i < frame.sizes.x) {
        m = masks[i];
    }
    let own = pack_bins(m);
    ranks[lid] = own;
    workgroupBarrier();
    // inclusive prefix sum of the packed counters (no carry between fields)
    for (var off = 1u; off < 128u; off <<= 1u) {
        var add = vec4<u32>(0u);
        if (lid >= off) {
            add = ranks[lid - off];
        }
        workgroupBarrier();
        ranks[lid] += add;
        workgroupBarrier();
    }
    if (m == 0u) {
        return;
    }
    let excl = ranks[lid] - own;
    let f = faces[i];
    let cap = frame.sizes.z;
    let occlusion = (frame.sizes.w & F_OCCLUSION) != 0u;
    for (var b = 0u; b < BINS; b++) {
        if (((m >> b) & 1u) == 0u) {
            continue;
        }
        let rank = (excl[b >> 2u] >> ((b & 3u) * 8u)) & 0xffu;
        let pos = group_data[wid.x * BINS + b] + rank;
        write_args(b * cap + pos, f, i, 1u);
        if (b < MAIN_BINS && occlusion) {
            // phase 1 prepass: what was visible last frame
            var was = 1u;
            if (i < arrayLength(&visibility)) {
                was = visibility[i];
            }
            if (was != 0u) {
                let k = atomicAdd(&counts[COUNT_PRE1 + b], 1u);
                write_args((REGION_PRE1 + b) * cap + k, f, i, 1u);
            }
        }
    }
}

// Occlusion of the main bins against the Hi-Z of the phase 1 depth. One
// row of workgroups per main bin.
@compute @workgroup_size(64)
fn cs_occlude(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>) {
    let b = wid.y;
    let j = gid.x;
    if (b >= MAIN_BINS || j >= atomicLoad(&counts[b])) {
        return;
    }
    let cap = frame.sizes.z;
    let a = (b * cap + j) * 5u;
    let rec = args[a + 4u];
    let f = faces[rec];
    var vis = true;
    if (f.object < frame.sizes.y) {
        vis = hiz_sphere_visible(hiz, objects[f.object].sphere, occl.view_proj, occl.screen, occl.levels);
    }
    args[a + 1u] = select(0u, 1u, vis);
    // drawn by phase 1 = visible last frame (cs_scatter)
    var was = 1u;
    if (rec < arrayLength(&visibility)) {
        was = visibility[rec];
        visibility[rec] = select(0u, 1u, vis);
    }
    // phase 2 prepass: visible now, not drawn by phase 1
    if (vis && was == 0u) {
        let k = atomicAdd(&counts[COUNT_PRE2 + b], 1u);
        let p2 = ((REGION_PRE2 + b) * cap + k) * 5u;
        for (var n = 0u; n < 5u; n++) {
            args[p2 + n] = args[a + n];
        }
    }
    if (!vis) {
        atomicAdd(&counts[COUNT_HIDDEN], 1u);
    }
}
