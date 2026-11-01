// Order-independent transparency: resolve (oit.rs).
//
// fs_blend_oit (object.wgsl) appended each blended fragment to its pixel's
// list. Here, in three steps:
//
// 1. cs_classify (compute, one 8 x 8 tile per workgroup): each pixel sorts
//    its list once and stores the composite in `out_color`; a pixel where
//    the MSAA samples disagree (a layer covering only some of them, or one
//    the opaque scene hides behind some of them only) is marked as an edge
//    instead. The tiles holding layers are listed, plain and edge apart,
//    with the instance counts of two indirect draws.
// 2. fs_composite (vs_tile over the plain tiles): the composite over the
//    scene, once per pixel, MSAA or not.
// 3. fs_edge (vs_tile over the edge tiles, MSAA only): the edge pixels again,
//    sample by sample, against each sample's depth.
//
// Sorting: the first `max_layers` layers of a list (the last drawn: the faces
// come back to front, so mostly the nearest) back to front; the others
// folded first as a weighted average whose coverage is exact (the product
// of their transparencies), so a deep stack fades instead of popping. The
// arrays of a sort must stay in registers: a pixel first tries the 8-layer
// sort (resolve_8) and only a deeper one sorts again with 24 (resolve_24),
// both generated from the template between the resolve_N markers below
// (oit.rs `resolve_source`).
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
    // tiles on screen: the edge tiles are listed from here
    tiles: u32,
    _pad: u32,
};
@group(0) @binding(0) var<storage, read> heads: array<u32>;
@group(0) @binding(1) var<storage, read> nodes: array<vec4<u32>>;
@group(0) @binding(2) var<uniform> params: OitParams;
// MSAA only: the scene depth, sample by sample
@group(0) @binding(3) var depth_ms: texture_depth_multisampled_2d;
// cs_classify only
@group(0) @binding(4) var out_color: texture_storage_2d<rgba16float, write>;
// two DrawIndirectArgs: the plain tiles, then the edge tiles
@group(0) @binding(5) var<storage, read_write> args: array<atomic<u32>, 8>;
@group(0) @binding(6) var<storage, read_write> tiles_w: array<u32>;
// the draws only
@group(0) @binding(7) var resolved: texture_2d<f32>;
@group(0) @binding(8) var<storage, read> tiles_r: array<u32>;

const TILE: u32 = 8u;
// a guard against a broken list (never reached by a valid one)
const MAX_TRAVERSAL: u32 = 1024u;
// `out_color` alpha of an edge pixel
const EDGE: f32 = -1.0;

struct Resolved {
    // 0 nothing in front, 1 `color`, 2 an edge (the samples disagree),
    // 3 more layers to sort than the arrays hold
    status: u32,
    color: vec4<f32>,
};

fn node_color(n: vec4<u32>) -> vec4<f32> {
    let rg = unpack2x16float(n.x);
    let b = unpack2x16float(n.y & 0xffffu).x;
    return vec4<f32>(rg, b, f32((n.y >> 16u) & 0xffu) / 255.0);
}

// <resolve_N>
// The list from `first`: `full`, the samples a layer must cover; a layer at
// or behind `dmin` is hidden everywhere, one in front of `dmax` nowhere
// (reverse-Z). Per sample (fs_edge): `full` that sample's bit and
// dmin = dmax its depth. Without MSAA: 1 and -1, every layer kept.
fn resolve___N__(first: u32, full: u32, dmin: f32, dmax: f32, keep: u32) -> Resolved {
    // the kept layers' nodes and depths only (two words each), the colours
    // read again once sorted: the arrays stay in registers
    var ids: array<u32, __N__>;
    var dep: array<f32, __N__>;
    var cnt = 0u;
    var tail_sum = vec3<f32>(0.0);
    var tail_a = 0.0;
    var tail_trans = 1.0;
    var idx = first;
    var steps = 0u;
    loop {
        if (idx == 0u || steps >= MAX_TRAVERSAL) {
            break;
        }
        let id = idx - 1u;
        let n = nodes[id];
        idx = n.w;
        steps += 1u;
        let m = (n.y >> 24u) & full;
        let z = bitcast<f32>(n.z);
        if (m == 0u || z <= dmin) {
            continue;
        }
        if (m != full || z <= dmax) {
            return Resolved(2u, vec4<f32>(0.0));
        }
        if (cnt < keep) {
            if (cnt == __N__u) {
                return Resolved(3u, vec4<f32>(0.0));
            }
            ids[cnt] = id;
            dep[cnt] = z;
            cnt += 1u;
            continue;
        }
        let c = node_color(n);
        tail_sum += c.rgb;
        tail_a += c.a;
        tail_trans *= 1.0 - c.a;
    }
    if (cnt == 0u && tail_a <= 0.0) {
        return Resolved(0u, vec4<f32>(0.0));
    }
    // insertion sort, farthest first (reverse-Z: ascending depth)
    for (var i = 1u; i < cnt; i += 1u) {
        let c = ids[i];
        let d = dep[i];
        var j = i;
        loop {
            if (j == 0u || dep[j - 1u] <= d) {
                break;
            }
            ids[j] = ids[j - 1u];
            dep[j] = dep[j - 1u];
            j -= 1u;
        }
        ids[j] = c;
        dep[j] = d;
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
        let c = node_color(nodes[ids[i]]);
        accum = c.rgb + accum * (1.0 - c.a);
        cov = c.a + cov * (1.0 - c.a);
    }
    return Resolved(1u, vec4<f32>(accum, cov));
}
// </resolve_N>

fn resolve_list(first: u32, full: u32, dmin: f32, dmax: f32) -> Resolved {
    let keep = min(params.max_layers, 24u);
    let r = resolve_8(first, full, dmin, dmax, keep);
    if (r.status == 3u) {
        return resolve_24(first, full, dmin, dmax, keep);
    }
    return r;
}

fn head(px: vec2<u32>) -> u32 {
    return heads[1u + px.y * params.width + px.x];
}

// ---- 1. classify, sort and list the tiles

var<workgroup> tile_flags: atomic<u32>;

// 1 plain, 2 edge, 0 nothing; the composite into `out_color`
fn classify(px: vec2<u32>, full: u32, dmin: f32, dmax: f32) -> u32 {
    var flag = 0u;
    var out = vec4<f32>(0.0);
    let first = head(px);
    if (first != 0u) {
        let r = resolve_list(first, full, dmin, dmax);
        if (r.status == 1u) {
            out = r.color;
            flag = 1u;
        } else if (r.status == 2u) {
            out = vec4<f32>(0.0, 0.0, 0.0, EDGE);
            flag = 2u;
        }
    }
    textureStore(out_color, px, out);
    return flag;
}

fn list_tile(flag: u32, li: u32, wg: vec2<u32>) {
    if (flag != 0u) {
        atomicOr(&tile_flags, flag);
    }
    workgroupBarrier();
    if (li == 0u) {
        let f = atomicLoad(&tile_flags);
        let tile = wg.y * params.tiles_x + wg.x;
        if ((f & 1u) != 0u) {
            tiles_w[atomicAdd(&args[1], 1u)] = tile;
        }
        if ((f & 2u) != 0u) {
            tiles_w[params.tiles + atomicAdd(&args[5], 1u)] = tile;
        }
    }
}

@compute @workgroup_size(8, 8)
fn cs_classify(@builtin(global_invocation_id) g: vec3<u32>, @builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>) {
    if (li == 0u) {
        atomicStore(&tile_flags, 0u);
    }
    workgroupBarrier();
    var flag = 0u;
    if (g.x < params.width && g.y < params.height) {
        // the capture kept only what is in front of the opaque scene
        flag = classify(g.xy, 1u, -1.0, -1.0);
    }
    list_tile(flag, li, wg.xy);
}

@compute @workgroup_size(8, 8)
fn cs_classify_ms(@builtin(global_invocation_id) g: vec3<u32>, @builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>) {
    if (li == 0u) {
        atomicStore(&tile_flags, 0u);
    }
    workgroupBarrier();
    var flag = 0u;
    if (g.x < params.width && g.y < params.height) {
        // the scene's depth range in the pixel: a layer in front of all its
        // samples, or behind all, is the same for each
        var dmin = 2.0;
        var dmax = -1.0;
        if (head(g.xy) != 0u) {
            for (var s = 0u; s < params.samples; s += 1u) {
                let d = textureLoad(depth_ms, vec2<i32>(g.xy), i32(s));
                dmin = min(dmin, d);
                dmax = max(dmax, d);
            }
        }
        flag = classify(g.xy, (1u << params.samples) - 1u, dmin, dmax);
    }
    list_tile(flag, li, wg.xy);
}

// ---- 2 and 3. composite the listed tiles

@vertex
fn vs_tile(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> @builtin(position) vec4<f32> {
    // first_instance: 0 for the plain tiles, `tiles` for the edge ones
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

@fragment
fn fs_composite(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(resolved, vec2<i32>(p.xy), 0);
    // nothing, or an edge (fs_edge)
    if (c.a <= 0.0) {
        discard;
    }
    return c;
}

@fragment
fn fs_edge(@builtin(position) p: vec4<f32>, @builtin(sample_index) s: u32) -> @location(0) vec4<f32> {
    let px = vec2<u32>(p.xy);
    // (per sample: four threads walk the list at once, one per sample,
    // where one compute thread walking it for all of them was slower)
    if (textureLoad(resolved, vec2<i32>(px), 0).a != EDGE) {
        discard;
    }
    let opaque = textureLoad(depth_ms, vec2<i32>(px), i32(s));
    let r = resolve_list(head(px), 1u << s, opaque, opaque);
    if (r.status != 1u) {
        discard;
    }
    return r.color;
}
