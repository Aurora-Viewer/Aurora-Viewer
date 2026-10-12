// Reflection probe filtering, ported from the Second Life / Firestorm viewer
// sources (LGPL 2.1, Copyright (C) 2022-2024 Linden Research, Inc. and the
// Firestorm project): interface/reflectionmipF.glsl (mip chain of the
// captured cube) and interface/radianceGenF.glsl (GGX prefiltered radiance,
// parts (c) 2018 Sascha Willems, MIT licence).
//
// Faces follow the Vulkan cube map convention (+X, -X, +Y, -Y, +Z, -Z, texel
// row 0 at the top), the world being z-up.
//
// Compute: one dispatch writes the six faces of one mip (layer = face), a
// texel per invocation, sampled at its center. The whole filtering of a probe
// is 15 dispatches of one pass (it was 90 render passes, one per face and
// mip, each paid for in wgpu's CommandEncoder::finish).

@group(0) @binding(0) var src_faces: texture_2d_array<f32>;
@group(0) @binding(1) var src_cube: texture_cube<f32>;
@group(0) @binding(2) var lin: sampler;
@group(0) @binding(3) var dst: texture_storage_2d_array<rgba16float, write>;

/// World direction of a texel of cube face `face` (Vulkan face selection).
fn face_dir(face: u32, uv: vec2<f32>) -> vec3<f32> {
    let sc = uv.x * 2.0 - 1.0;
    let tc = uv.y * 2.0 - 1.0;
    switch face {
        case 0u: { return normalize(vec3<f32>(1.0, -tc, -sc)); }
        case 1u: { return normalize(vec3<f32>(-1.0, -tc, sc)); }
        case 2u: { return normalize(vec3<f32>(sc, 1.0, tc)); }
        case 3u: { return normalize(vec3<f32>(sc, -1.0, -tc)); }
        case 4u: { return normalize(vec3<f32>(sc, -tc, 1.0)); }
        default: { return normalize(vec3<f32>(-sc, -tc, -1.0)); }
    }
}

// reflectionmipF.glsl: each mip is the bilinear downsample of the previous
// one (the source view holds that single mip, all six faces).
fn mip_texel(face: u32, uv: vec2<f32>) -> vec4<f32> {
    let c = textureSampleLevel(src_faces, lin, uv, i32(face), 0.0).rgb;
    return vec4<f32>(c, 1.0);
}

const PI: f32 = 3.1415926536;
// RenderReflectionProbeResolution 128: mips 128 .. 2, max_probe_lod = log2(128) - 1
const MAX_PROBE_LOD: f32 = 6.0;
// PROBE_RES of probes.rs (the radiance mip being written is known by its size)
const PROBE_RES: u32 = 128u;
// The capture is twice the probe resolution: its mip 1 matches probe mip 0.
const CAPTURE_LOD_OFFSET: f32 = 1.0;
// gRadianceGenProgram PROBE_FILTER_SAMPLES
const FILTER_SAMPLES: f32 = 32.0;

fn random(co: vec2<f32>) -> f32 {
    let dt = dot(co, vec2<f32>(12.9898, 78.233));
    let sn = dt - 3.14 * floor(dt / 3.14);
    return fract(sin(sn) * 43758.5453);
}

fn hammersley2d(i: u32, n: u32) -> vec2<f32> {
    var bits = (i << 16u) | (i >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return vec2<f32>(f32(i) / f32(n), f32(bits) * 2.3283064365386963e-10);
}

fn importance_sample_ggx(xi: vec2<f32>, roughness: f32, normal: vec3<f32>) -> vec3<f32> {
    let alpha = roughness * roughness;
    let phi = 2.0 * PI * xi.x + random(normal.xz) * 0.1;
    let cos_theta = sqrt((1.0 - xi.y) / (1.0 + (alpha * alpha - 1.0) * xi.y));
    let sin_theta = sqrt(1.0 - cos_theta * cos_theta);
    let h = vec3<f32>(sin_theta * cos(phi), sin_theta * sin(phi), cos_theta);
    let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(normal.z) < 0.999);
    let tx = normalize(cross(up, normal));
    let ty = normalize(cross(normal, tx));
    return normalize(tx * h.x + ty * h.y + normal * h.z);
}

fn d_ggx(nh: f32, roughness: f32) -> f32 {
    let alpha = roughness * roughness;
    let a2 = alpha * alpha;
    let denom = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * denom * denom);
}

// radianceGenF.glsl prefilterEnvMap
fn radiance_texel(face: u32, mip: f32, uv: vec2<f32>) -> vec4<f32> {
    let n = face_dir(face, uv);
    let v = n;
    let env_dim = f32(textureDimensions(src_cube).x) * 0.5;
    let roughness = mip / MAX_PROBE_LOD;
    let num = u32(max(FILTER_SAMPLES * roughness, 1.0));
    var color = vec3<f32>(0.0);
    var total = 0.0;
    for (var i = 0u; i < num; i++) {
        let xi = hammersley2d(i, num);
        let h = importance_sample_ggx(xi, roughness, n);
        let l = 2.0 * dot(v, h) * h - v;
        let nl = clamp(dot(n, l), 0.0, 1.0);
        if (nl > 0.0) {
            let nh = clamp(dot(n, h), 0.0, 1.0);
            let vh = clamp(dot(v, h), 0.0, 1.0);
            let pdf = d_ggx(nh, roughness) * nh / (4.0 * vh) + 0.0001;
            let omega_s = 1.0 / (f32(num) * pdf);
            let omega_p = 4.0 * PI / (6.0 * env_dim * env_dim);
            var lod = 0.0;
            if (roughness > 0.0) {
                lod = clamp(0.5 * log2(omega_s / omega_p) + 1.0, 0.0, MAX_PROBE_LOD);
            }
            color += textureSampleLevel(src_cube, lin, l, lod + CAPTURE_LOD_OFFSET).rgb * nl;
            total += nl;
        }
    }
    return vec4<f32>(max(color / max(total, 1e-4), vec3<f32>(0.0)), 1.0);
}

// irradianceGenF.glsl (from the Khronos glTF Sample Viewer ibl_filtering):
// cosine weighted hemisphere, mip chosen from the sample's solid angle,
// written into the 16x16 irradiance cube of the slot.
const IRR_SAMPLES: u32 = 32u;
// u_width / u_lodBias of irradianceGenF
const IRR_WIDTH: f32 = 64.0;
const IRR_LOD_BIAS: f32 = 2.0;

fn irradiance_texel(face: u32, uv: vec2<f32>) -> vec4<f32> {
    let n = face_dir(face, uv);
    // generateTBN (with the robust bitangent near +-Y LL leaves commented out)
    let up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n.y) > 0.999);
    let t = normalize(cross(up, n));
    let b = cross(n, t);
    var color = vec3<f32>(0.0);
    for (var i = 0u; i < IRR_SAMPLES; i++) {
        let xi = hammersley2d(i, IRR_SAMPLES);
        let cos_t = sqrt(1.0 - xi.y);
        let sin_t = sqrt(xi.y);
        let phi = 2.0 * PI * xi.x;
        let pdf = cos_t / PI;
        let dir = normalize(t * (sin_t * cos(phi)) + b * (sin_t * sin(phi)) + n * cos_t);
        var lod = 0.5 * log2(6.0 * IRR_WIDTH * IRR_WIDTH / (f32(IRR_SAMPLES) * pdf)) + IRR_LOD_BIAS;
        lod = clamp(lod, 0.0, MAX_PROBE_LOD);
        color += textureSampleLevel(src_cube, lin, dir, lod + CAPTURE_LOD_OFFSET).rgb;
    }
    return vec4<f32>(max(color / f32(IRR_SAMPLES), vec3<f32>(0.0)), 1.0);
}

fn texel_uv(id: vec3<u32>, dims: vec2<u32>) -> vec2<f32> {
    return (vec2<f32>(id.xy) + 0.5) / vec2<f32>(dims);
}

@compute @workgroup_size(8, 8, 1)
fn cs_mip(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(dst);
    if (id.x >= dims.x || id.y >= dims.y) {
        return;
    }
    textureStore(dst, vec2<i32>(id.xy), i32(id.z), mip_texel(id.z, texel_uv(id, dims)));
}

// The mip being written gives the roughness: its size against PROBE_RES.
@compute @workgroup_size(8, 8, 1)
fn cs_radiance(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(dst);
    if (id.x >= dims.x || id.y >= dims.y) {
        return;
    }
    let mip = f32(firstTrailingBit(PROBE_RES / dims.x));
    textureStore(dst, vec2<i32>(id.xy), i32(id.z), radiance_texel(id.z, mip, texel_uv(id, dims)));
}

@compute @workgroup_size(8, 8, 1)
fn cs_irradiance(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(dst);
    if (id.x >= dims.x || id.y >= dims.y) {
        return;
    }
    textureStore(dst, vec2<i32>(id.xy), i32(id.z), irradiance_texel(id.z, texel_uv(id, dims)));
}
