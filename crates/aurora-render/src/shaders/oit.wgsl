// Order-independent transparency: resolve (oit.rs).
//
// The capture (object.wgsl, fs_blend_keys then fs_blend_payloads) keeps,
// for each pixel, the `layers` nearest blended fragments of the frame:
// their depths as keys sorted nearest first (`keys`, one slice of the
// screen per layer), their colour, alpha and MSAA coverage beside them
// (`payloads`), and the fragments farther than the kept ones summed in the
// tail (`tail`: premultiplied colour, alpha and -log transparency, 16.16
// fixed point). Here, in three steps:
//
// 1. cs_classify (compute, one 8 x 8 tile per workgroup, the tiles the
//    capture flagged only): each pixel composited from its tail, then its
//    layers farthest first, into `out_color`; with MSAA a pixel where the
//    samples disagree (a layer covering only some of them, or hidden by the
//    opaque scene behind some of them only) is marked as an edge instead.
//    The tiles holding layers are listed, plain and edge apart, with the
//    instance counts of two indirect draws.
// 2. fs_composite (vs_tile over the plain tiles): the composite over the
//    scene, once per pixel, MSAA or not.
// 3. fs_edge (vs_tile over the edge tiles, MSAA only): the edge pixels
//    sample by sample, each sample over the layers that cover it and lie in
//    front of its depth. Nothing is sorted here: the keys are.
//
// The tail is the weighted average of the fragments beyond the kept
// layers, their colour weighted by their alpha over a coverage that is the
// product of their transparencies, composited behind the layers; it has no
// coverage of its own, so the samples of an edge pixel share it.
//
// Output: premultiplied colour and coverage, blended One / OneMinusSrcAlpha
// on the colour and Zero / OneMinusSrcAlpha on the alpha: the glow behind is
// dimmed by the coverage, as each blended face dims it when drawn in order
// (LLDrawPoolAlpha glow suppression).

struct OitParams {
    width: u32,
    height: u32,
    // the screen's pixels: the stride between the layers' slices
    pixels: u32,
    // layers kept per pixel
    layers: u32,
    samples: u32,
    tiles_x: u32,
    // tiles on screen: the edge tiles are listed from here, and the tiles'
    // flags follow the tail in its buffer (`tail[pixels * 5 + t]`)
    tiles: u32,
    _pad: u32,
};
// layer i of pixel p: keys[i * pixels + p] holds the bits of its depth
// (0: no layer; the top bit, CLAIMED, once its payload is stored),
// payloads[i * pixels + p] its premultiplied colour as rgb9e5, then its
// alpha (8 bits) and the samples it covers (4 bits)
@group(0) @binding(0) var<storage, read> keys: array<u32>;
@group(0) @binding(1) var<storage, read> payloads: array<vec2<u32>>;
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
// per pixel five words, the tail's premultiplied r, g, b, its alpha and its
// -log transparency in 16.16 fixed point; then one word per tile, 1 when
// the capture kept a layer in it
@group(0) @binding(9) var<storage, read> tail: array<u32>;

const TILE: u32 = 8u;
// `out_color` alpha of an edge pixel
const EDGE: f32 = -1.0;
const FIXED: f32 = 65536.0;

// rgb9e5: three 9-bit mantissas under a shared 5-bit exponent (packed by
// object.wgsl `pack_rgb9e5`)
fn unpack_rgb9e5(v: u32) -> vec3<f32> {
    let s = exp2(f32(v >> 27u) - 24.0);
    return vec3<f32>(f32(v & 0x1ffu), f32((v >> 9u) & 0x1ffu), f32((v >> 18u) & 0x1ffu)) * s;
}

struct Layer {
    z: f32,
    // premultiplied
    color: vec4<f32>,
    mask: u32,
};

// Set on a key by the capture's payload pass when the slot got its payload
// (object.wgsl `OIT_CLAIMED`): a key without it covers nothing.
const CLAIMED: u32 = 0x80000000u;

fn layer(i: u32, p: u32) -> Layer {
    let at = i * params.pixels + p;
    let pl = payloads[at];
    let k = keys[at];
    let mask = select(0u, (pl.y >> 8u) & 0xfu, (k & CLAIMED) != 0u);
    return Layer(bitcast<f32>(k & ~CLAIMED), vec4<f32>(unpack_rgb9e5(pl.x), f32(pl.y & 0xffu) / 255.0), mask);
}

// The pixel's layers: its keys are sorted nearest first, 0 past the last.
fn count_of(p: u32) -> u32 {
    var n = 0u;
    for (var i = 0u; i < params.layers; i += 1u) {
        if (keys[i * params.pixels + p] == 0u) {
            break;
        }
        n += 1u;
    }
    return n;
}

// The tail's premultiplied colour and coverage.
fn tail_of(p: u32) -> vec4<f32> {
    let t = p * 5u;
    let a = f32(tail[t + 3u]) / FIXED;
    if (a <= 0.0) {
        return vec4<f32>(0.0);
    }
    let rgb = vec3<f32>(f32(tail[t]), f32(tail[t + 1u]), f32(tail[t + 2u])) / FIXED;
    let cov = 1.0 - exp(-f32(tail[t + 4u]) / FIXED);
    return vec4<f32>(rgb / a * cov, cov);
}

// The pixel's `count` layers that cover `mask` and lie in front of `depth`
// (reverse-Z: nearer is larger), composited farthest first over the tail.
fn composite(p: u32, count: u32, mask: u32, depth: f32) -> vec4<f32> {
    var accum = tail_of(p);
    for (var j = count; j > 0u; j -= 1u) {
        let l = layer(j - 1u, p);
        if ((l.mask & mask) != 0u && l.z > depth) {
            accum = vec4<f32>(l.color.rgb + accum.rgb * (1.0 - l.color.a), l.color.a + accum.a * (1.0 - l.color.a));
        }
    }
    return accum;
}

// ---- 1. classify, composite and list the tiles

var<workgroup> tile_flags: atomic<u32>;

fn tile_busy(wg: vec2<u32>) -> bool {
    return tail[params.pixels * 5u + wg.y * params.tiles_x + wg.x] != 0u;
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
    // (an empty tile, most of the screen most of the time, reads and
    // stores nothing per pixel; the barriers stay)
    let busy = tile_busy(wg.xy);
    if (li == 0u) {
        atomicStore(&tile_flags, 0u);
    }
    workgroupBarrier();
    var flag = 0u;
    if (busy && g.x < params.width && g.y < params.height) {
        let p = g.y * params.width + g.x;
        let n = count_of(p);
        var out = vec4<f32>(0.0);
        if (n > 0u) {
            // the capture kept only what is in front of the opaque scene
            out = composite(p, n, 1u, -1.0);
            flag = 1u;
        }
        textureStore(out_color, g.xy, out);
    }
    list_tile(flag, li, wg.xy);
}

@compute @workgroup_size(8, 8)
fn cs_classify_ms(@builtin(global_invocation_id) g: vec3<u32>, @builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>) {
    let busy = tile_busy(wg.xy);
    if (li == 0u) {
        atomicStore(&tile_flags, 0u);
    }
    workgroupBarrier();
    var flag = 0u;
    if (busy && g.x < params.width && g.y < params.height) {
        let p = g.y * params.width + g.x;
        let n = count_of(p);
        var out = vec4<f32>(0.0);
        if (n > 0u) {
            // a layer in front of all the samples' depths and covering them
            // all is the same for each: composited once; otherwise an edge,
            // composited sample by sample by fs_edge
            var dmax = -1.0;
            for (var s = 0u; s < params.samples; s += 1u) {
                dmax = max(dmax, textureLoad(depth_ms, vec2<i32>(g.xy), i32(s)));
            }
            let full = (1u << params.samples) - 1u;
            var edge = false;
            for (var i = 0u; i < n; i += 1u) {
                let l = layer(i, p);
                if ((l.mask & full) != full || l.z <= dmax) {
                    edge = true;
                    break;
                }
            }
            if (edge) {
                out = vec4<f32>(0.0, 0.0, 0.0, EDGE);
                flag = 2u;
            } else {
                out = composite(p, n, full, -1.0);
                flag = 1u;
            }
        }
        textureStore(out_color, g.xy, out);
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
fn fs_edge(@builtin(position) pos: vec4<f32>, @builtin(sample_index) s: u32) -> @location(0) vec4<f32> {
    let px = vec2<u32>(pos.xy);
    if (textureLoad(resolved, vec2<i32>(px), 0).a != EDGE) {
        discard;
    }
    let p = px.y * params.width + px.x;
    let opaque = textureLoad(depth_ms, vec2<i32>(px), i32(s));
    let c = composite(p, count_of(p), 1u << s, opaque);
    if (c.a <= 0.0) {
        discard;
    }
    return c;
}
