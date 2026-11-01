// Order-independent transparency: resolve (oit.rs).
//
// fs_blend_oit (object.wgsl) appended each blended fragment to its pixel's
// list. Here each pixel (each sample with MSAA) keeps its layers in front of
// the opaque scene, sorts `max_layers` of them back to front and
// composites them "over" the scene. Layers beyond them are not dropped: they
// are folded first, as a weighted average whose coverage is exact (the
// product of their transparencies), so a deep stack fades instead of
// popping. The sorted ones are the first of the list: the last drawn. Output: premultiplied colour and coverage, blended One /
// OneMinusSrcAlpha on the colour and Zero / OneMinusSrcAlpha on the alpha:
// the glow behind is dimmed by the coverage, as each blended face dims it
// when drawn in order (LLDrawPoolAlpha glow suppression).

struct OitParams {
    node_cap: u32,
    width: u32,
    height: u32,
    max_layers: u32,
};
@group(0) @binding(0) var<storage, read> heads: array<u32>;
@group(0) @binding(1) var<storage, read> nodes: array<vec4<u32>>;
@group(0) @binding(2) var<uniform> params: OitParams;
// MSAA only: the scene depth, sample by sample
@group(0) @binding(3) var depth_ms: texture_depth_multisampled_2d;

// sorted exactly; the arrays are this long (oit.rs MAX_SORTED_LAYERS). Kept
// small, two words a layer: larger ones leave the registers and cost more
// than the whole capture
const MAX_LAYERS: u32 = 24u;
// a guard against a broken list (never reached by a valid one)
const MAX_TRAVERSAL: u32 = 1024u;

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn node_color(n: vec4<u32>) -> vec4<f32> {
    let rg = unpack2x16float(n.x);
    let b = unpack2x16float(n.y & 0xffffu).x;
    return vec4<f32>(rg, b, f32((n.y >> 16u) & 0xffu) / 255.0);
}

// `bit`: the sample's bit in the nodes' coverage; `opaque`, `test`: the
// sample's scene depth, and whether to test it (MSAA)
fn resolve(px: vec2<u32>, bit: u32, opaque: f32, test: bool) -> vec4<f32> {
    if (px.x >= params.width || px.y >= params.height) {
        discard;
    }
    var idx = heads[1u + px.y * params.width + px.x];
    if (idx == 0u) {
        discard;
    }
    let keep = min(params.max_layers, MAX_LAYERS);
    // the kept layers' nodes and depths only (two words each): the arrays
    // must stay in registers, the colours are read again once sorted
    var ids: array<u32, MAX_LAYERS>;
    var dep: array<f32, MAX_LAYERS>;
    var cnt = 0u;
    // beyond `keep`: the weighted tail
    var tail_sum = vec3<f32>(0.0);
    var tail_a = 0.0;
    var tail_trans = 1.0;
    var steps = 0u;
    loop {
        if (idx == 0u || steps >= MAX_TRAVERSAL) {
            break;
        }
        let id = idx - 1u;
        let n = nodes[id];
        idx = n.w;
        steps += 1u;
        let z = bitcast<f32>(n.z);
        if ((n.y >> 24u) & bit) == 0u || (test && z <= opaque) {
            continue;
        }
        // the first `keep` of the list (the last drawn: the faces come back
        // to front, so mostly the nearest) are sorted, the rest averaged;
        // one pass over the list whatever its length
        if (cnt < keep) {
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
        discard;
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
    return vec4<f32>(accum, cov);
}

@fragment
fn fs_resolve(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return resolve(vec2<u32>(p.xy), 1u, 0.0, false);
}

// MSAA: run per sample (sample_index), each sample keeping the layers that
// cover it and lie in front of its own depth
@fragment
fn fs_resolve_ms(@builtin(position) p: vec4<f32>, @builtin(sample_index) s: u32) -> @location(0) vec4<f32> {
    let px = vec2<u32>(p.xy);
    let opaque = textureLoad(depth_ms, vec2<i32>(px), i32(s));
    return resolve(px, 1u << s, opaque, true);
}
