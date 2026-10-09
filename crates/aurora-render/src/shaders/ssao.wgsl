// Screen-space ambient occlusion at half resolution, from the single-sample
// depth prepass, followed by a depth-aware blur. The occlusion is Second
// Life's calcAmbientOcclusion (aoUtil.glsl, Copyright (C) Linden Research,
// Inc., LGPL 2.1): screen-space taps around the pixel, each counting as an
// occluding sphere whose solid angle falls off with the squared distance.

@group(0) @binding(0) var depth_tex: texture_depth_2d;
@group(0) @binding(1) var ao_in: texture_2d<f32>;

struct SsaoParams {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    // x ssao_radius (RenderSSAOScale, pixels x meters), y samples (8 as LL,
    // or 16), z ssao_max_radius (RenderSSAOMaxScale, pixels), w ssao_factor
    params: vec4<f32>,
};
@group(0) @binding(2) var<uniform> ssao: SsaoParams;

struct Out {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vi: u32) -> Out {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var o: Out;
    o.clip = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

fn full_size() -> vec2<i32> {
    return vec2<i32>(textureDimensions(depth_tex));
}

fn depth_at(px: vec2<i32>) -> f32 {
    let s = full_size();
    return textureLoad(depth_tex, clamp(px, vec2<i32>(0), s - vec2<i32>(1)), 0);
}

fn world_at(px: vec2<i32>, d: f32) -> vec3<f32> {
    let s = vec2<f32>(full_size());
    let uv = (vec2<f32>(px) + vec2<f32>(0.5)) / s;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let w = ssao.inv_view_proj * vec4<f32>(ndc, max(d, 1e-7), 1.0);
    return w.xyz / w.w;
}

// PCG hash of a pixel, in [0, 1): white noise like LL's random noise map
// (a structured noise shows as stripes through the blur).
fn pixel_noise(px: vec2<i32>) -> f32 {
    var h = u32(px.x & 127) | (u32(px.y & 127) << 7u);
    h = h * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = (h >> 22u) ^ h;
    return f32(h) / 4294967296.0;
}

// getKern: exponentially (^2) distant taps spread around the pixel, in
// pixels of the full resolution target (times the scale). Taps 8..15 are
// Aurora's higher quality option: the same rings turned by 22.5 degrees,
// in between LL's distances.
fn ao_kernel(i: i32) -> vec2<f32> {
    let dirs = array<vec2<f32>, 8>(
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0), vec2<f32>(0.0, -1.0),
        vec2<f32>(0.7071, 0.7071), vec2<f32>(-0.7071, -0.7071), vec2<f32>(-0.7071, 0.7071), vec2<f32>(0.7071, -0.7071));
    let k = i & 7;
    var d = dirs[k];
    var r = f32(k + 1) * 0.125;
    if (i >= 8) {
        // rotate by 22.5 degrees, half a step closer
        d = vec2<f32>(d.x * 0.92388 - d.y * 0.38268, d.x * 0.38268 + d.y * 0.92388);
        r -= 0.0625;
    }
    return d * r * r;
}

@fragment
fn fs_ssao(in: Out) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.clip.xy) * 2;
    let d = depth_at(px);
    if (d <= 0.0) {
        return vec4<f32>(1.0);
    }
    let p = world_at(px, d);
    // normal from the closest neighbours (avoids smearing across edges)
    let pr = world_at(px + vec2<i32>(2, 0), depth_at(px + vec2<i32>(2, 0)));
    let pl = world_at(px - vec2<i32>(2, 0), depth_at(px - vec2<i32>(2, 0)));
    let pu = world_at(px + vec2<i32>(0, 2), depth_at(px + vec2<i32>(0, 2)));
    let pd = world_at(px - vec2<i32>(0, 2), depth_at(px - vec2<i32>(0, 2)));
    let dx = select(p - pl, pr - p, distance(pr, p) < distance(pl, p));
    let dy = select(p - pd, pu - p, distance(pu, p) < distance(pd, p));
    var n = normalize(cross(dx, dy));
    let to_cam = ssao.camera_pos.xyz - p;
    if (dot(n, to_cam) < 0.0) {
        n = -n;
    }
    // view depth (-z in eye space) of the pixel
    let view_z = (ssao.view_proj * vec4<f32>(p, 1.0)).w;
    let full = vec2<f32>(full_size());
    let pos_screen = (vec2<f32>(px) + vec2<f32>(0.5)) / full;
    // LL reflects the kernel about a random unit vector from a 128 x 128
    // noise texture (LLPipeline mNoiseMap), tiled over the screen
    let a = pixel_noise(px) * 6.2831853;
    let noise_reflect = vec2<f32>(cos(a), sin(a));
    let scale = min(ssao.params.x / max(view_z, 1e-3), ssao.params.z);
    let factor = ssao.params.w;
    let count = i32(ssao.params.y);
    var angle_hidden = 0.0;
    var points = 0.0;
    for (var i = 0; i < count; i++) {
        let k = ao_kernel(i) / full;
        let samp_screen = pos_screen + scale * reflect(k, noise_reflect);
        let spx = vec2<i32>(floor(samp_screen * full));
        let q = world_at(spx, depth_at(spx));
        let diff = p - q;
        let dist2 = dot(diff, diff);
        // samples above the surface (offset 5 cm along the normal) occlude
        // with the solid angle of a sphere of constant radius, capped
        let above = select(0.0, 1.0, dot(q - 0.05 * n - p, n) > 0.0);
        angle_hidden += above * min(1.0 / max(dist2, 1e-6), 1.0 / factor);
        // samples more than 1 m in front of the pixel are "no data", not
        // "no occlusion"
        let q_view_z = (ssao.view_proj * vec4<f32>(q, 1.0)).w;
        points += select(0.0, 1.0, q_view_z - view_z > -1.0);
    }
    var ao = 1.0;
    if (points > 0.0) {
        ao = 1.0 - min(factor * angle_hidden / points, 1.0);
    }
    ao = clamp(ao, 0.0, 1.0);
    return vec4<f32>(ao, ao, ao, 1.0);
}

@fragment
fn fs_blur(in: Out) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.clip.xy);
    let size = vec2<i32>(textureDimensions(ao_in));
    let center_d = depth_at(px * 2);
    let cp = world_at(px * 2, center_d);
    let cdist = max(distance(cp, ssao.camera_pos.xyz), 0.1);
    var sum = 0.0;
    var wsum = 0.0;
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            let q = clamp(px + vec2<i32>(x, y), vec2<i32>(0), size - vec2<i32>(1));
            let qd = depth_at(q * 2);
            let qp = world_at(q * 2, qd);
            let rel = abs(distance(qp, ssao.camera_pos.xyz) - cdist) / cdist;
            let w = select(0.0, 1.0, rel < 0.05);
            sum += textureLoad(ao_in, q, 0).r * w;
            wsum += w;
        }
    }
    let ao = select(1.0, sum / max(wsum, 1e-4), wsum > 0.0);
    return vec4<f32>(ao, ao, ao, 1.0);
}
