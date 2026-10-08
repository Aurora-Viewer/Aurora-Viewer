// Temporal anti-aliasing: reprojection through depth, 3x3 neighbourhood clamp.

@group(0) @binding(0) var cur_tex: texture_2d<f32>;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var hist_tex: texture_2d<f32>;
@group(0) @binding(3) var lin: sampler;

struct TaaParams {
    inv_view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
    params: vec4<f32>, // x reset, y current weight, z width, w height
};
@group(0) @binding(4) var<uniform> taa: TaaParams;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_taa(@builtin(vertex_index) vi: u32) -> Out {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var o: Out;
    o.clip = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, 1.0 - y);
    return o;
}

// Reduce HDR fireflies in the blend.
fn tm(c: vec3<f32>) -> vec3<f32> {
    return c / (1.0 + max(c.r, max(c.g, c.b)));
}

fn itm(c: vec3<f32>) -> vec3<f32> {
    return c / max(1.0 - max(c.r, max(c.g, c.b)), 1e-4);
}

@fragment
fn fs_taa(in: Out) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(cur_tex));
    let px = clamp(vec2<i32>(in.clip.xy), vec2<i32>(0), size - vec2<i32>(1));
    let cur = textureLoad(cur_tex, px, 0);
    // alpha: glow amount of the current frame, not reprojected
    let glow = cur.a;
    let c = tm(cur.rgb);
    var mn = c;
    var mx = c;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let q = clamp(px + vec2<i32>(dx, dy), vec2<i32>(0), size - vec2<i32>(1));
            let s = tm(textureLoad(cur_tex, q, 0).rgb);
            mn = min(mn, s);
            mx = max(mx, s);
        }
    }
    if (taa.params.x > 0.5) {
        return vec4<f32>(itm(c), glow);
    }
    // reverse-Z: 0 is infinitely far
    let d = max(textureLoad(depth_tex, px, 0), 1e-7);
    let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let wp = taa.inv_view_proj * vec4<f32>(ndc, d, 1.0);
    let world = wp.xyz / wp.w;
    let pp = taa.prev_view_proj * vec4<f32>(world, 1.0);
    if (pp.w <= 0.0) {
        return vec4<f32>(itm(c), glow);
    }
    let pn = pp.xy / pp.w;
    let puv = vec2<f32>(pn.x * 0.5 + 0.5, 0.5 - pn.y * 0.5);
    if (puv.x < 0.0 || puv.y < 0.0 || puv.x > 1.0 || puv.y > 1.0) {
        return vec4<f32>(itm(c), glow);
    }
    var h = tm(textureSampleLevel(hist_tex, lin, puv, 0.0).rgb);
    h = clamp(h, mn, mx);
    let blended = mix(h, c, taa.params.y);
    return vec4<f32>(itm(blended), glow);
}
