// Order-independent transparency: resolve (oit.rs).
//
// fs_blend_oit (object.wgsl) stored each blended fragment: a pixel's first
// `slots` layers in its slots of a texture array (layer k: every pixel's
// slot k, so a tile's pixels read their layers side by side), the others
// appended to its linked list in the node pool. Here, in two steps:
//
// 1. cs_classify (compute, one 8 x 8 tile per workgroup): each pixel sorts
//    its list once and stores the composite in `out_color`, the depth of
//    its nearest layer in `out_depth`. The tiles holding layers are listed,
//    with the instance count of an indirect draw.
// 2. fs_composite (vs_tile over the listed tiles): the composite over the
//    scene, once per pixel, at the depth of its nearest layer: with MSAA
//    the depth test keeps it off the samples where an opaque edge is in
//    front of every layer of the pixel.
//
// Per pixel, not per sample: the layers carry no coverage (the capture
// reads no sample mask) and a layer counts where it is in front of at least
// half of its pixel's opaque samples (`dmed`, cs_classify_ms); the edges
// of the transparent faces themselves get their anti-aliasing from the
// post-process one (SMAA) over the final image. An opaque edge pixel seen
// through a layer is wrong on at most half of its samples.
//
// When the pool overflowed (heads[0], the nodes asked for, past node_cap),
// no tile is listed and `flags` turns the in-order replay of the blended
// faces on and the composite's glow off (renderer.rs: the gated draws read
// copies of their arguments that oit_gate.wgsl zeroes or keeps from them):
// the frame is then drawn in the drawing order, as without the lists.
//
// Sorting: the `max_layers` nearest layers of a pixel, whatever their draw
// order, back to front, by depth then by the tie-break the capture stored
// (object.wgsl `oit_tie`: two coplanar layers, which arrive in an order
// the GPU decides, sort the same way on every frame; nothing depends on
// the order of a list); the others folded first, behind them, as a weighted
// average whose coverage is exact (the product of their transparencies), so
// a deep stack fades instead of popping. The arrays of a sort must stay in
// registers: a pixel first tries the 8-layer sort (resolve_8) and only a
// deeper one sorts again with 24 (resolve_24), both generated from the
// template between the resolve_N markers below (oit.rs `resolve_source`).
//
// Output: premultiplied colour and coverage, blended One / OneMinusSrcAlpha
// on the colour and Zero / OneMinusSrcAlpha on the alpha: the glow behind is
// dimmed by the coverage, as each blended face dims it when drawn in order
// (LLDrawPoolAlpha glow suppression).

struct OitParams {
    node_cap: u32,
    width: u32,
    height: u32,
    max_layers: u32,
    samples: u32,
    tiles_x: u32,
    // tiles on screen
    tiles: u32,
    // layers per pixel in the slot array
    slots: u32,
    // the capture's frame stamp (its node claims)
    stamp: u32,
    _pad1: u32,
    _pad2: u32,
    _pad3: u32,
};
// heads[0] counts the pool's nodes; pixel p: heads[1 + 2p] its layers,
// heads[2 + 2p] the first node of its list plus one (0: none)
@group(0) @binding(0) var<storage, read> heads: array<u32>;
@group(0) @binding(1) var<storage, read> nodes: array<vec4<u32>>;
@group(0) @binding(2) var<uniform> params: OitParams;
// MSAA only: the scene depth, sample by sample
@group(0) @binding(3) var depth_ms: texture_depth_multisampled_2d;
// cs_classify only: the composite and the depth of its nearest layer
@group(0) @binding(4) var out_color: texture_storage_2d<rgba16float, write>;
@group(0) @binding(12) var out_depth: texture_storage_2d<r32float, write>;
// one DrawIndirectArgs: the listed tiles
@group(0) @binding(5) var<storage, read_write> args: array<atomic<u32>, 4>;
@group(0) @binding(6) var<storage, read_write> tiles_w: array<u32>;
// The DrawIndirectArgs of the replay's depth copy (one triangle, when the
// pool overflowed): flags[1], its instance count, is also what
// oit_gate.wgsl reads to keep or zero the gated draws' instances
@group(0) @binding(9) var<storage, read_write> flags: array<u32, 4>;
// the draws only
@group(0) @binding(7) var resolved: texture_2d<f32>;
@group(0) @binding(13) var resolved_depth: texture_2d<f32>;
@group(0) @binding(8) var<storage, read> tiles_r: array<u32>;
// single-sample only: the scene depth (the replay's depth copy)
@group(0) @binding(11) var depth_ss: texture_depth_2d;
// the slot array (same node layout, `w` unused)
@group(0) @binding(10) var slots_r: texture_2d_array<u32>;

const TILE: u32 = 8u;
// The most nodes a walk of one list visits (object.wgsl `OIT_MAX_WALK`):
// a list is at most as long as its pixel's count and never a loop, but no
// walk may hang the GPU whatever the data.
const MAX_WALK: u32 = 1024u;
// a sorted layer held in a slot (the rest of the id: the slot), not a node
const SLOT: u32 = 0x80000000u;

struct Resolved {
    // 0 nothing in front, 1 `color` at `depth`, 3 more layers to sort than
    // the arrays hold
    status: u32,
    color: vec4<f32>,
    // the nearest layer's depth (reverse-Z: the largest)
    depth: f32,
};

// A layer by its id: slot `id & !SLOT` of the pixel, or node `id`.
fn layer(px: vec2<u32>, id: u32) -> vec4<u32> {
    if ((id & SLOT) != 0u) {
        return textureLoad(slots_r, px, id & ~SLOT, 0);
    }
    return nodes[id];
}

// rgb9e5: three 9-bit mantissas under a shared 5-bit exponent (packed by
// object.wgsl `pack_rgb9e5`)
fn unpack_rgb9e5(v: u32) -> vec3<f32> {
    let s = exp2(f32(v >> 27u) - 24.0);
    return vec3<f32>(f32(v & 0x1ffu), f32((v >> 9u) & 0x1ffu), f32((v >> 18u) & 0x1ffu)) * s;
}

fn node_color(n: vec4<u32>) -> vec4<f32> {
    return vec4<f32>(unpack_rgb9e5(n.x), f32(n.y & 0xffu) / 255.0);
}

// Layer (k, t) is farther than (k2, t2): a smaller depth (reverse-Z), or
// the same depth and a smaller tie-break. A total order: the sort and
// the choice of the farthest kept layer never depend on the list's order.
fn before(k: u32, t: u32, k2: u32, t2: u32) -> bool {
    return k < k2 || (k == k2 && t < t2);
}

// <resolve_N>
// The layers of pixel `px`: `count` stored, the slots' then the list from
// `first`; the `keep` nearest are sorted, the others go to the tail. A layer
// at or behind `dmed` is hidden (reverse-Z): the opaque depth that half of
// the pixel's samples are at or behind (-1 without MSAA: every layer kept).
fn resolve___N__(px: vec2<u32>, count: u32, first: u32, dmed: f32, keep: u32) -> Resolved {
    // the kept layers' nodes, depths and tie-breaks only (three words
    // each), the colours read again once sorted: the arrays stay in
    // registers
    var ids: array<u32, __N__>;
    var key: array<u32, __N__>;
    var tie: array<u32, __N__>;
    var cnt = 0u;
    // the farthest kept layer: its order and index (valid once cnt > 0)
    var far_key = 0xffffffffu;
    var far_tie = 0xffffffffu;
    var far_i = 0u;
    // the nearest layer of all, kept or folded
    var near_z = -1.0;
    var tail_sum = vec3<f32>(0.0);
    var tail_a = 0.0;
    var tail_trans = 1.0;
    let in_slots = min(count, params.slots);
    var k = 0u;
    var idx = first;
    var steps = 0u;
    loop {
        var n: vec4<u32>;
        var id: u32;
        if (k < in_slots) {
            n = textureLoad(slots_r, px, k, 0);
            id = SLOT | k;
            k += 1u;
        } else {
            // (a list holds at most `count` nodes: the bound only guards
            // against a broken one, never cuts a valid list)
            if (idx == 0u || steps >= min(count, MAX_WALK)) {
                break;
            }
            id = idx - 1u;
            n = nodes[id];
            idx = n.w;
            steps += 1u;
        }
        let z = bitcast<f32>(n.z);
        if (z <= dmed) {
            continue;
        }
        near_z = max(near_z, z);
        // (a visible depth is positive: its bits order like it)
        let k = n.z;
        let t = n.y >> 16u;
        if (cnt < keep) {
            if (cnt == __N__u) {
                return Resolved(3u, vec4<f32>(0.0), 0.0);
            }
            ids[cnt] = id;
            key[cnt] = k;
            tie[cnt] = t;
            if (before(k, t, far_key, far_tie)) {
                far_key = k;
                far_tie = t;
                far_i = cnt;
            }
            cnt += 1u;
            continue;
        }
        // full: keep the nearest layers whatever the draw order (reverse-Z:
        // the nearest has the largest depth). A nearer layer takes the place
        // of the farthest kept, which is folded into the tail instead.
        var c = node_color(n);
        if (before(far_key, far_tie, k, t)) {
            let evicted = layer(px, ids[far_i]);
            ids[far_i] = id;
            key[far_i] = k;
            tie[far_i] = t;
            c = node_color(evicted);
            far_key = key[0];
            far_tie = tie[0];
            far_i = 0u;
            for (var i = 1u; i < cnt; i += 1u) {
                if (before(key[i], tie[i], far_key, far_tie)) {
                    far_key = key[i];
                    far_tie = tie[i];
                    far_i = i;
                }
            }
        }
        tail_sum += c.rgb;
        tail_a += c.a;
        tail_trans *= 1.0 - c.a;
    }
    if (cnt == 0u && tail_a <= 0.0) {
        return Resolved(0u, vec4<f32>(0.0), 0.0);
    }
    // insertion sort, farthest first (reverse-Z: ascending depth, then the
    // tie-break)
    for (var i = 1u; i < cnt; i += 1u) {
        let c = ids[i];
        let d = key[i];
        let t = tie[i];
        var j = i;
        loop {
            if (j == 0u || !before(d, t, key[j - 1u], tie[j - 1u])) {
                break;
            }
            ids[j] = ids[j - 1u];
            key[j] = key[j - 1u];
            tie[j] = tie[j - 1u];
            j -= 1u;
        }
        ids[j] = c;
        key[j] = d;
        tie[j] = t;
    }
    var accum = vec3<f32>(0.0);
    var cov = 0.0;
    if (tail_a > 0.0) {
        // colours are premultiplied: their sum over the alphas' sum is the
        // coverage-weighted average colour
        cov = 1.0 - tail_trans;
        accum = tail_sum / tail_a * cov;
    }
    for (var i = 0u; i < cnt; i += 1u) {
        let c = node_color(layer(px, ids[i]));
        accum = c.rgb + accum * (1.0 - c.a);
        cov = c.a + cov * (1.0 - c.a);
    }
    return Resolved(1u, vec4<f32>(accum, cov), near_z);
}
// </resolve_N>

fn resolve_list(px: vec2<u32>, dmed: f32) -> Resolved {
    let keep = min(params.max_layers, 24u);
    let p = pixel(px);
    let r = resolve_8(px, p.x, p.y, dmed, keep);
    if (r.status == 3u) {
        return resolve_24(px, p.x, p.y, dmed, keep);
    }
    return r;
}

// The pixel's layer count and the first node of its list plus one.
fn pixel(px: vec2<u32>) -> vec2<u32> {
    let p = 1u + 2u * (px.y * params.width + px.x);
    return vec2<u32>(heads[p], heads[p + 1u]);
}

fn overflowed() -> bool {
    return heads[0] > params.node_cap;
}

// One thread of the dispatch sets the draw counts that follow.
fn set_flags(g: vec3<u32>) {
    if (g.x == 0u && g.y == 0u) {
        flags[0] = 3u;
        flags[1] = select(0u, 1u, overflowed());
        flags[2] = 0u;
        flags[3] = 0u;
    }
}

// ---- 1. classify, sort and list the tiles

var<workgroup> tile_flags: atomic<u32>;

// 1 composited, 0 nothing; the composite into `out_color`, the depth of its
// nearest layer into `out_depth`
fn classify(px: vec2<u32>, dmed: f32) -> u32 {
    var flag = 0u;
    var out = vec4<f32>(0.0);
    var depth = 0.0;
    // (overflowed: nothing composited, the faces are replayed in order)
    if (pixel(px).x != 0u && !overflowed()) {
        let r = resolve_list(px, dmed);
        if (r.status == 1u) {
            out = r.color;
            depth = r.depth;
            flag = 1u;
        }
    }
    textureStore(out_color, px, out);
    textureStore(out_depth, px, vec4<f32>(depth, 0.0, 0.0, 0.0));
    return flag;
}

fn list_tile(flag: u32, li: u32, wg: vec2<u32>) {
    if (flag != 0u) {
        atomicOr(&tile_flags, flag);
    }
    workgroupBarrier();
    if (li == 0u && atomicLoad(&tile_flags) != 0u) {
        tiles_w[atomicAdd(&args[1], 1u)] = wg.y * params.tiles_x + wg.x;
    }
}

@compute @workgroup_size(8, 8)
fn cs_classify(@builtin(global_invocation_id) g: vec3<u32>, @builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>) {
    set_flags(g);
    if (li == 0u) {
        atomicStore(&tile_flags, 0u);
    }
    workgroupBarrier();
    var flag = 0u;
    if (g.x < params.width && g.y < params.height) {
        // the capture kept only what is in front of the opaque scene
        flag = classify(g.xy, -1.0);
    }
    list_tile(flag, li, wg.xy);
}

// The opaque depth half of the pixel's samples are at or behind: a layer in
// front of it is in front of at least half of them (the samples' depths
// sorted, reverse-Z: the (samples / 2)th smallest).
fn median_depth(px: vec2<u32>) -> f32 {
    var d: array<f32, 8>;
    let n = min(params.samples, 8u);
    for (var s = 0u; s < n; s += 1u) {
        let v = textureLoad(depth_ms, vec2<i32>(px), i32(s));
        var j = s;
        loop {
            if (j == 0u || d[j - 1u] <= v) {
                break;
            }
            d[j] = d[j - 1u];
            j -= 1u;
        }
        d[j] = v;
    }
    return d[max(n / 2u, 1u) - 1u];
}

@compute @workgroup_size(8, 8)
fn cs_classify_ms(@builtin(global_invocation_id) g: vec3<u32>, @builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>) {
    set_flags(g);
    if (li == 0u) {
        atomicStore(&tile_flags, 0u);
    }
    workgroupBarrier();
    var flag = 0u;
    if (g.x < params.width && g.y < params.height) {
        var dmed = 2.0;
        if (pixel(g.xy).x != 0u && !overflowed()) {
            dmed = median_depth(g.xy);
        }
        flag = classify(g.xy, dmed);
    }
    list_tile(flag, li, wg.xy);
}

// ---- 2. composite the listed tiles

@vertex
fn vs_tile(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> @builtin(position) vec4<f32> {
    let tile = tiles_r[i];
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    let at = (vec2<f32>(f32(tile % params.tiles_x), f32(tile / params.tiles_x)) + corners[v]) * f32(TILE);
    let size = vec2<f32>(f32(params.width), f32(params.height));
    return vec4<f32>(at.x / size.x * 2.0 - 1.0, 1.0 - at.y / size.y * 2.0, 0.0, 1.0);
}

struct CompositeOut {
    @location(0) color: vec4<f32>,
    // the nearest layer's depth, tested (Greater) against the scene's: with
    // MSAA, an opaque edge in front of every layer keeps its samples
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_composite(@builtin(position) p: vec4<f32>) -> CompositeOut {
    let px = vec2<i32>(p.xy);
    let c = textureLoad(resolved, px, 0);
    if (c.a <= 0.0) {
        discard;
    }
    return CompositeOut(c, textureLoad(resolved_depth, px, 0).x);
}

// ---- the replay's depth copy (overflow only)
//
// The rigged faces' `Layered` and `Layers` modes draw on a copy of the scene
// depth (renderer.rs, `Targets::blend_layers`). When the pool overflowed the
// replay needs that copy, and a texture copy cannot wait on the GPU's
// decision: this full-screen triangle, drawn indirectly (one instance when
// overflowed, none otherwise), writes it instead.

@vertex
fn vs_full(@builtin(vertex_index) v: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((v << 1u) & 2u), f32(v & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_copy_depth(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
    return textureLoad(depth_ss, vec2<i32>(p.xy), 0);
}

@fragment
fn fs_copy_depth_ms(@builtin(position) p: vec4<f32>, @builtin(sample_index) s: u32) -> @builtin(frag_depth) f32 {
    return textureLoad(depth_ms, vec2<i32>(p.xy), i32(s));
}

// ---- The per-sample form of the resolve, kept for reference, not compiled.
//
// 1. What it did: per-sample OIT coverage for the MSAA edges. The capture
//    read each fragment's `@builtin(sample_mask)` into its node (8 bits),
//    and the resolve composited the pixels whose samples disagreed sample
//    by sample (`fs_edge`, a second indirect draw over the edge tiles), so
//    the transparent edges were anti-aliased by their own coverage.
// 2. Why it is off: naga 30.0.1's SPIR-V writer emits the SampleMask
//    built-in as a scalar where Vulkan wants an array of 32-bit integers,
//    so the validation layer reports VUID-SampleMask-SampleMask-04359 at
//    every start; and the per-sample edge pass cost about a third of OIT's
//    time on hair.
// 3. What replaces it: the resolve per pixel (a layer kept where it is in
//    front of at least half of the pixel's samples, the composite tested at
//    the depth of its nearest layer) and, in the MSAA modes while OIT is
//    on, the SMAA passes over the resolved image (renderer.rs
//    `smaa_wanted`).
// 4. When to bring it back: once naga emits SampleMask as an array, and
//    only if it measures faster or better than the per-pixel resolve with
//    SMAA.
// 5. The last working version: commit e19a2ab5 (the one before d5855ef5),
//    which the code below is taken from.
// (the bindings that differed, the sort for N layers with the coverage, the
// classify with its edge flag and two tile lists, and `fs_edge`)
// // two DrawIndirectArgs: the plain tiles, then the edge tiles
// @group(0) @binding(5) var<storage, read_write> args: array<atomic<u32>, 8>;
// @group(0) @binding(6) var<storage, read_write> tiles_w: array<u32>;
// // The DrawIndirectArgs of the replay's depth copy (one triangle, when the
// // pool overflowed): flags[1], its instance count, is also what
// // oit_gate.wgsl reads to keep or zero the gated draws' instances
// @group(0) @binding(9) var<storage, read_write> flags: array<u32, 4>;
// // the draws only
// @group(0) @binding(7) var resolved: texture_2d<f32>;
// @group(0) @binding(8) var<storage, read> tiles_r: array<u32>;
// // single-sample only: the scene depth (the replay's depth copy)
// @group(0) @binding(11) var depth_ss: texture_depth_2d;
// // the slot array (same node layout, `w` unused)
// @group(0) @binding(10) var slots_r: texture_2d_array<u32>;
//
// const TILE: u32 = 8u;
// // The most nodes a walk of one list visits (object.wgsl `OIT_MAX_WALK`):
// // a list is at most as long as its pixel's count and never a loop, but no
// // walk may hang the GPU whatever the data.
// const MAX_WALK: u32 = 1024u;
// // `out_color` alpha of an edge pixel
// const EDGE: f32 = -1.0;
// // a sorted layer held in a slot (the rest of the id: the slot), not a node
// const SLOT: u32 = 0x80000000u;
//
// struct Resolved {
//     // 0 nothing in front, 1 `color`, 2 an edge (the samples disagree),
//     // 3 more layers to sort than the arrays hold
//     status: u32,
//     color: vec4<f32>,
// };
//
// // A layer by its id: slot `id & !SLOT` of the pixel, or node `id`.
// fn layer(px: vec2<u32>, id: u32) -> vec4<u32> {
//     if ((id & SLOT) != 0u) {
//         return textureLoad(slots_r, px, id & ~SLOT, 0);
//     }
//     return nodes[id];
// }
//
// fn node_color(n: vec4<u32>) -> vec4<f32> {
//     let rg = unpack2x16float(n.x);
//     let b = unpack2x16float(n.y & 0xffffu).x;
//     return vec4<f32>(rg, b, f32((n.y >> 16u) & 0xffu) / 255.0);
// }
//
// // <resolve_N, per sample>
// // The layers of pixel `px`: `count` stored, the slots' then the list from
// // `first`; the `keep` nearest are sorted, the others go to the tail. `full`, the samples a layer must cover; a layer at or behind
// // `dmin` is hidden everywhere, one in front of `dmax` nowhere (reverse-Z).
// // Per sample (fs_edge): `full` that sample's bit and dmin = dmax its depth.
// // Without MSAA: 1 and -1, every layer kept.
// fn resolve_N(px: vec2<u32>, count: u32, first: u32, full: u32, dmin: f32, dmax: f32, keep: u32) -> Resolved {
//     // the kept layers' nodes and depths only (two words each), the colours
//     // read again once sorted: the arrays stay in registers
//     var ids: array<u32, N>;
//     var dep: array<f32, N>;
//     var cnt = 0u;
//     // the farthest kept layer: its depth and index (valid once cnt > 0)
//     var far_z = 2.0;
//     var far_i = 0u;
//     var tail_sum = vec3<f32>(0.0);
//     var tail_a = 0.0;
//     var tail_trans = 1.0;
//     let in_slots = min(count, params.slots);
//     var k = 0u;
//     var idx = first;
//     var steps = 0u;
//     loop {
//         var n: vec4<u32>;
//         var id: u32;
//         if (k < in_slots) {
//             n = textureLoad(slots_r, px, k, 0);
//             id = SLOT | k;
//             k += 1u;
//         } else {
//             // (a list holds at most `count` nodes: the bound only guards
//             // against a broken one, never cuts a valid list)
//             if (idx == 0u || steps >= min(count, MAX_WALK)) {
//                 break;
//             }
//             id = idx - 1u;
//             n = nodes[id];
//             idx = n.w;
//             steps += 1u;
//         }
//         let m = (n.y >> 24u) & full;
//         let z = bitcast<f32>(n.z);
//         if (m == 0u || z <= dmin) {
//             continue;
//         }
//         if (m != full || z <= dmax) {
//             return Resolved(2u, vec4<f32>(0.0));
//         }
//         if (cnt < keep) {
//             if (cnt == Nu) {
//                 return Resolved(3u, vec4<f32>(0.0));
//             }
//             ids[cnt] = id;
//             dep[cnt] = z;
//             if (z < far_z) {
//                 far_z = z;
//                 far_i = cnt;
//             }
//             cnt += 1u;
//             continue;
//         }
//         // full: keep the nearest layers whatever the draw order (reverse-Z:
//         // the nearest has the largest depth). A nearer layer takes the place
//         // of the farthest kept, which is folded into the tail instead.
//         var c = node_color(n);
//         if (z > far_z) {
//             let evicted = layer(px, ids[far_i]);
//             ids[far_i] = id;
//             dep[far_i] = z;
//             c = node_color(evicted);
//             far_z = dep[0];
//             far_i = 0u;
//             for (var i = 1u; i < cnt; i += 1u) {
//                 if (dep[i] < far_z) {
//                     far_z = dep[i];
//                     far_i = i;
//                 }
//             }
//         }
//         tail_sum += c.rgb;
//         tail_a += c.a;
//         tail_trans *= 1.0 - c.a;
//     }
//     if (cnt == 0u && tail_a <= 0.0) {
//         return Resolved(0u, vec4<f32>(0.0));
//     }
//     // insertion sort, farthest first (reverse-Z: ascending depth)
//     for (var i = 1u; i < cnt; i += 1u) {
//         let c = ids[i];
//         let d = dep[i];
//         var j = i;
//         loop {
//             if (j == 0u || dep[j - 1u] <= d) {
//                 break;
//             }
//             ids[j] = ids[j - 1u];
//             dep[j] = dep[j - 1u];
//             j -= 1u;
//         }
//         ids[j] = c;
//         dep[j] = d;
//     }
//     var accum = vec3<f32>(0.0);
//     var cov = 0.0;
//     if (tail_a > 0.0) {
//         // colours are premultiplied: their sum over the alphas' sum is the
//         // coverage-weighted average colour
//         cov = 1.0 - tail_trans;
//         accum = tail_sum / tail_a * cov;
//     }
//     for (var i = 0u; i < cnt; i += 1u) {
//         let c = node_color(layer(px, ids[i]));
//         accum = c.rgb + accum * (1.0 - c.a);
//         cov = c.a + cov * (1.0 - c.a);
//     }
//     return Resolved(1u, vec4<f32>(accum, cov));
// }
// // </resolve_N, per sample>
//
// fn resolve_list(px: vec2<u32>, full: u32, dmin: f32, dmax: f32) -> Resolved {
//     let keep = min(params.max_layers, 24u);
//     let p = pixel(px);
//     let r = resolve_8(px, p.x, p.y, full, dmin, dmax, keep);
//     if (r.status == 3u) {
//         return resolve_24(px, p.x, p.y, full, dmin, dmax, keep);
//     }
//     return r;
// }
//
// // The pixel's layer count and the first node of its list plus one.
// fn pixel(px: vec2<u32>) -> vec2<u32> {
//     let p = 1u + 2u * (px.y * params.width + px.x);
//     return vec2<u32>(heads[p], heads[p + 1u]);
// }
//
// fn overflowed() -> bool {
//     return heads[0] > params.node_cap;
// }
//
// // One thread of the dispatch sets the draw counts that follow.
// fn set_flags(g: vec3<u32>) {
//     if (g.x == 0u && g.y == 0u) {
//         flags[0] = 3u;
//         flags[1] = select(0u, 1u, overflowed());
//         flags[2] = 0u;
//         flags[3] = 0u;
//     }
// }
//
// // ---- 1. classify, sort and list the tiles
//
// var<workgroup> tile_flags: atomic<u32>;
//
// // 1 plain, 2 edge, 0 nothing; the composite into `out_color`
// fn classify(px: vec2<u32>, full: u32, dmin: f32, dmax: f32) -> u32 {
//     var flag = 0u;
//     var out = vec4<f32>(0.0);
//     // (overflowed: nothing composited, the faces are replayed in order)
//     if (pixel(px).x != 0u && !overflowed()) {
//         let r = resolve_list(px, full, dmin, dmax);
//         if (r.status == 1u) {
//             out = r.color;
//             flag = 1u;
//         } else if (r.status == 2u) {
//             out = vec4<f32>(0.0, 0.0, 0.0, EDGE);
//             flag = 2u;
//         }
//     }
//     textureStore(out_color, px, out);
//     return flag;
// }
//
// fn list_tile(flag: u32, li: u32, wg: vec2<u32>) {
//     if (flag != 0u) {
//         atomicOr(&tile_flags, flag);
//     }
//     workgroupBarrier();
//     if (li == 0u) {
//         let f = atomicLoad(&tile_flags);
//         let tile = wg.y * params.tiles_x + wg.x;
//         if ((f & 1u) != 0u) {
//             tiles_w[atomicAdd(&args[1], 1u)] = tile;
//         }
//         if ((f & 2u) != 0u) {
//             tiles_w[params.tiles + atomicAdd(&args[5], 1u)] = tile;
//         }
//     }
// }
//
// @compute @workgroup_size(8, 8)
// fn cs_classify(@builtin(global_invocation_id) g: vec3<u32>, @builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>) {
//     set_flags(g);
//     if (li == 0u) {
//         atomicStore(&tile_flags, 0u);
//     }
//     workgroupBarrier();
//     var flag = 0u;
//     if (g.x < params.width && g.y < params.height) {
//         // the capture kept only what is in front of the opaque scene
//         flag = classify(g.xy, 1u, -1.0, -1.0);
//     }
//     list_tile(flag, li, wg.xy);
// }
//
// @compute @workgroup_size(8, 8)
// fn cs_classify_ms(@builtin(global_invocation_id) g: vec3<u32>, @builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>) {
//     set_flags(g);
//     if (li == 0u) {
//         atomicStore(&tile_flags, 0u);
//     }
//     workgroupBarrier();
//     var flag = 0u;
//     if (g.x < params.width && g.y < params.height) {
//         // the scene's depth range in the pixel: a layer in front of all its
//         // samples, or behind all, is the same for each
//         var dmin = 2.0;
//         var dmax = -1.0;
//         if (pixel(g.xy).x != 0u && !overflowed()) {
//             for (var s = 0u; s < params.samples; s += 1u) {
//                 let d = textureLoad(depth_ms, vec2<i32>(g.xy), i32(s));
//                 dmin = min(dmin, d);
//                 dmax = max(dmax, d);
//             }
//         }
//         flag = classify(g.xy, (1u << params.samples) - 1u, dmin, dmax);
//     }
//     list_tile(flag, li, wg.xy);
// }
//
// // ---- 2 and 3. composite the listed tiles
//
// @vertex
// fn vs_tile(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> @builtin(position) vec4<f32> {
//     // first_instance: 0 for the plain tiles, `tiles` for the edge ones
//     let tile = tiles_r[i];
//     var corners = array<vec2<f32>, 6>(
//         vec2<f32>(0.0, 0.0),
//         vec2<f32>(1.0, 0.0),
//         vec2<f32>(0.0, 1.0),
//         vec2<f32>(0.0, 1.0),
//         vec2<f32>(1.0, 0.0),
//         vec2<f32>(1.0, 1.0),
//     );
//     let at = (vec2<f32>(f32(tile % params.tiles_x), f32(tile / params.tiles_x)) + corners[v]) * f32(TILE);
//     let size = vec2<f32>(f32(params.width), f32(params.height));
//     return vec4<f32>(at.x / size.x * 2.0 - 1.0, 1.0 - at.y / size.y * 2.0, 0.0, 1.0);
// }
//
// @fragment
// fn fs_composite(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
//     let c = textureLoad(resolved, vec2<i32>(p.xy), 0);
//     // nothing, or an edge (fs_edge)
//     if (c.a <= 0.0) {
//         discard;
//     }
//     return c;
// }
//
// @fragment
// fn fs_edge(@builtin(position) p: vec4<f32>, @builtin(sample_index) s: u32) -> @location(0) vec4<f32> {
//     let px = vec2<u32>(p.xy);
//     // (per sample: four threads walk the list at once, one per sample,
//     // where one compute thread walking it for all of them was slower)
//     if (textureLoad(resolved, vec2<i32>(px), 0).a != EDGE) {
//         discard;
//     }
//     let opaque = textureLoad(depth_ms, vec2<i32>(px), i32(s));
//     let r = resolve_list(px, 1u << s, opaque, opaque);
//     if (r.status != 1u) {
//         discard;
//     }
//     return r.color;
// }
