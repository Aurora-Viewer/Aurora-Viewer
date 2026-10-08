// Screen-space ambient occlusion at half resolution, from the single-sample
// depth prepass, followed by a depth-aware blur.

@group(0) @binding(0) var depth_tex: texture_depth_2d;
@group(0) @binding(1) var ao_in: texture_2d<f32>;

struct SsaoParams {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    params: vec4<f32>,  // x radius (m), y samples, z intensity, w frame
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

fn ign(p: vec2<f32>) -> f32 {
    // interleaved gradient noise
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
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
    let dist = length(to_cam);
    let radius = ssao.params.x * clamp(dist * 0.05, 0.6, 3.0);
    let count = i32(ssao.params.y);
    var t = normalize(cross(n, select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n.x) > 0.8)));
    let b = cross(n, t);
    let rot = ign(in.clip.xy + vec2<f32>(ssao.params.w * 5.588)) * 6.2831853;
    var occlusion = 0.0;
    for (var i = 0; i < count; i++) {
        let fi = (f32(i) + 0.5) / f32(count);
        let ang = f32(i) * 2.3999632 + rot;
        let z = sqrt(1.0 - fi);
        let r = sqrt(fi);
        let dir = t * (cos(ang) * r) + b * (sin(ang) * r) + n * z;
        let scale = mix(0.15, 1.0, fi * fi);
        let s = p + dir * radius * scale + n * 0.03;
        let c = ssao.view_proj * vec4<f32>(s, 1.0);
        if (c.w <= 0.01) {
            continue;
        }
        let ndc = c.xyz / c.w;
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        let spx = vec2<i32>(uv * vec2<f32>(full_size()));
        let sd = depth_at(spx);
        if (sd > ndc.z) {
            // occluder in front of the sample: count it if it is close to p
            let q = world_at(spx, sd);
            let range = clamp(radius / max(distance(q, p), 1e-3), 0.0, 1.0);
            occlusion += range * range;
        }
    }
    let ao = clamp(1.0 - occlusion / f32(count) * ssao.params.z, 0.0, 1.0);
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
