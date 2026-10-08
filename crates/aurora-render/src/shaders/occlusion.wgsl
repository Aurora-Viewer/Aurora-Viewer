// GPU occlusion culling, two phases (no popping):
// 1. cs_prepare: the depth prepass first draws what was visible last frame;
// 2. the depth of that prepass is reduced into a Hi-Z pyramid (cs_hiz_*),
//    farthest depth per texel (reverse-Z: the minimum);
// 3. cs_cull: every tested draw's bounding sphere is checked against the
//    pyramid; hidden draws get instance_count 0 in the main indirect buffer,
//    and those visible now but not drawn in phase 1 go to a second prepass.
// Bounding spheres enclose the whole object (center of its geometry, not its
// origin), and the test projects their whole box: large objects (floors,
// buildings) are only culled when every pixel they could cover is in front.

// ---- Hi-Z pyramid

@group(0) @binding(0) var hiz_src_depth: texture_depth_2d;
@group(0) @binding(1) var hiz_src: texture_2d<f32>;
@group(0) @binding(2) var hiz_dst: texture_storage_2d<r32float, write>;

// Level 0: half resolution, the farthest of 2x2 depth pixels (clamped at the
// edges, so odd sizes are fully covered by ceil-sized levels).
@compute @workgroup_size(8, 8)
fn cs_hiz_first(@builtin(global_invocation_id) id: vec3<u32>) {
    let dst = textureDimensions(hiz_dst);
    if (id.x >= dst.x || id.y >= dst.y) {
        return;
    }
    let lim = vec2<i32>(textureDimensions(hiz_src_depth)) - vec2<i32>(1);
    let p = vec2<i32>(id.xy) * 2;
    let a = textureLoad(hiz_src_depth, min(p, lim), 0);
    let b = textureLoad(hiz_src_depth, min(p + vec2<i32>(1, 0), lim), 0);
    let c = textureLoad(hiz_src_depth, min(p + vec2<i32>(0, 1), lim), 0);
    let d = textureLoad(hiz_src_depth, min(p + vec2<i32>(1, 1), lim), 0);
    textureStore(hiz_dst, vec2<i32>(id.xy), vec4<f32>(min(min(a, b), min(c, d)), 0.0, 0.0, 1.0));
}

@compute @workgroup_size(8, 8)
fn cs_hiz_down(@builtin(global_invocation_id) id: vec3<u32>) {
    let dst = textureDimensions(hiz_dst);
    if (id.x >= dst.x || id.y >= dst.y) {
        return;
    }
    let lim = vec2<i32>(textureDimensions(hiz_src)) - vec2<i32>(1);
    let p = vec2<i32>(id.xy) * 2;
    let a = textureLoad(hiz_src, min(p, lim), 0).r;
    let b = textureLoad(hiz_src, min(p + vec2<i32>(1, 0), lim), 0).r;
    let c = textureLoad(hiz_src, min(p + vec2<i32>(0, 1), lim), 0).r;
    let d = textureLoad(hiz_src, min(p + vec2<i32>(1, 1), lim), 0).r;
    textureStore(hiz_dst, vec2<i32>(id.xy), vec4<f32>(min(min(a, b), min(c, d)), 0.0, 0.0, 1.0));
}

// ---- culling

struct CullParams {
    view_proj: mat4x4<f32>,
    // screen size in pixels, Hi-Z level count, draw count
    screen: vec2<f32>,
    levels: u32,
    count: u32,
}

// kind: 0 = never tested, 1 = depth prepass draw (two phases),
// 2 = tested only (blended, glow)
struct CullDraw {
    sphere: vec4<f32>,
    kind: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
}

@group(0) @binding(10) var<uniform> cull: CullParams;
@group(0) @binding(11) var<storage, read> draws: array<CullDraw>;
// DrawIndexedIndirect: index_count, instance_count, first_index, base_vertex, first_instance
@group(0) @binding(12) var<storage, read_write> main_args: array<u32>;
@group(0) @binding(13) var<storage, read_write> pre1_args: array<u32>;
@group(0) @binding(14) var<storage, read_write> pre2_args: array<u32>;
// visible last frame, by draw record
@group(0) @binding(15) var<storage, read_write> visibility: array<u32>;
@group(0) @binding(16) var hiz: texture_2d<f32>;
// hidden draws this frame: [prepass draws, tested only] (statistics)
@group(0) @binding(17) var<storage, read_write> hidden: array<atomic<u32>, 2>;

fn copy_args(i: u32, instances_1: u32, instances_2: u32) {
    let b = i * 5u;
    for (var k = 0u; k < 5u; k++) {
        pre1_args[b + k] = main_args[b + k];
        pre2_args[b + k] = main_args[b + k];
    }
    pre1_args[b + 1u] = instances_1;
    pre2_args[b + 1u] = instances_2;
}

@compute @workgroup_size(64)
fn cs_prepare(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= cull.count || draws[i].kind != 1u) {
        return;
    }
    let rec = main_args[i * 5u + 4u];
    var was = 1u;
    if (rec < arrayLength(&visibility)) {
        was = visibility[rec];
    }
    copy_args(i, was, 0u);
}

// Is the sphere possibly visible against the Hi-Z of the phase 1 depth?
fn sphere_visible(s: vec4<f32>) -> bool {
    if (s.w <= 0.0) {
        return true;
    }
    var lo = vec2<f32>(1e9);
    var hi = vec2<f32>(-1e9);
    var nearest = 0.0;
    for (var k = 0u; k < 8u; k++) {
        let o = vec3<f32>(
            select(-s.w, s.w, (k & 1u) != 0u),
            select(-s.w, s.w, (k & 2u) != 0u),
            select(-s.w, s.w, (k & 4u) != 0u),
        );
        let c = cull.view_proj * vec4<f32>(s.xyz + o, 1.0);
        // a corner at or behind the camera plane: keep it
        if (c.w <= 1e-3) {
            return true;
        }
        let ndc = c.xyz / c.w;
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        lo = min(lo, uv);
        hi = max(hi, uv);
        nearest = max(nearest, ndc.z);
    }
    // off screen: left to the frustum culling
    if (hi.x < 0.0 || hi.y < 0.0 || lo.x > 1.0 || lo.y > 1.0) {
        return true;
    }
    // pixels, one pixel of margin (TAA jitter, rasterization rules)
    let pmin = clamp(lo * cull.screen - vec2<f32>(1.0), vec2<f32>(0.0), cull.screen - vec2<f32>(1.0));
    let pmax = clamp(hi * cull.screen + vec2<f32>(1.0), vec2<f32>(0.0), cull.screen - vec2<f32>(1.0));
    let size = max(pmax.x - pmin.x, pmax.y - pmin.y);
    // level whose texels (2^(level+1) pixels) make the box span 2x2 at most
    var level = 0u;
    if (size > 2.0) {
        level = u32(ceil(log2(size))) - 1u;
    }
    level = min(level, cull.levels - 1u);
    let texel = f32(1u << (level + 1u));
    let dims = vec2<i32>(textureDimensions(hiz, i32(level))) - vec2<i32>(1);
    let t0 = min(vec2<i32>(floor(pmin / texel)), dims);
    let t1 = min(vec2<i32>(floor(pmax / texel)), dims);
    var farthest = 1.0;
    for (var y = t0.y; y <= t1.y; y++) {
        for (var x = t0.x; x <= t1.x; x++) {
            farthest = min(farthest, textureLoad(hiz, vec2<i32>(x, y), i32(level)).r);
        }
    }
    // reverse-Z: hidden when even its nearest point is behind the farthest
    // depth over the whole box
    return nearest >= farthest;
}

@compute @workgroup_size(64)
fn cs_cull(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= cull.count) {
        return;
    }
    let d = draws[i];
    if (d.kind == 0u) {
        return;
    }
    let vis = sphere_visible(d.sphere);
    if (!vis) {
        atomicAdd(&hidden[min(d.kind, 2u) - 1u], 1u);
    }
    let b = i * 5u;
    main_args[b + 1u] = select(0u, 1u, vis);
    if (d.kind == 1u) {
        // second prepass: visible now, not drawn by the first one
        pre2_args[b + 1u] = select(0u, 1u, vis && pre1_args[b + 1u] == 0u);
        let rec = main_args[b + 4u];
        if (rec < arrayLength(&visibility)) {
            visibility[rec] = select(0u, 1u, vis);
        }
    }
}
