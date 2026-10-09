enable wgpu_binding_array;

// Shared declarations for Aurora Viewer shaders.
//
// The atmospheric sky and water fog functions are ports of the Second Life
// viewer shaders (skyV.glsl, cloudsV/F.glsl, waterFogF.glsl), Copyright (C)
// Linden Research, Inc., LGPL 2.1.

const MAX_LIGHTS: u32 = 64u;
const CASCADES: u32 = 3u;
const PI: f32 = 3.14159265359;

struct Light {
    pos_radius: vec4<f32>,   // xyz position, w radius
    color_falloff: vec4<f32>, // rgb linear color * intensity, w falloff
};

struct Frame {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,      // xyz, w = time (s)
    sun_dir: vec4<f32>,         // xyz towards the light, w = sun visible (0..1)
    sun_color: vec4<f32>,       // rgb radiance
    sky_zenith: vec4<f32>,      // ambient from above
    sky_horizon: vec4<f32>,     // ambient from the horizon
    ground_color: vec4<f32>,    // ambient from below
    fog: vec4<f32>,             // rgb color, w density
    params: vec4<f32>,          // x exposure, y water height, z shadows enabled, w light count
    cascade_splits: vec4<f32>,  // view-space distances
    screen: vec4<f32>,          // width, height, 1/w, 1/h
    misc: vec4<f32>,            // x ssao, y ssr, z water reflection texture, w reflection pass
    clip_plane: vec4<f32>,      // reflection passes: keep dot(n, p) + w >= 0
    cam_right: vec4<f32>,       // camera right axis, w = particle soft fade distance
    cam_up: vec4<f32>,          // camera up axis
    water_fog: vec4<f32>,       // linear fog color, w density (kd)
    water_params: vec4<f32>,    // x ks, y fresnel scale, z fresnel offset, w blur multiplier
    water_waves: vec4<f32>,     // wave1 dir xy, wave2 dir xy
    water_normal: vec4<f32>,    // normal scale xyz, w underwater fog density
    mirror_plane: vec4<f32>,    // n.xyz, d (0 normal = no mirror)
    mirror_box: mat4x4<f32>,    // world -> mirror probe box ([-0.5, 0.5]^3)
    // EEP sky (legacy haze model)
    sky_light: vec4<f32>,       // light norm xyz (z >= -0.1), sun/moon glow factor
    sky_sunlight: vec4<f32>,    // sunlight rgb, cloud shadow
    sky_ambient: vec4<f32>,     // ambient rgb, density multiplier
    sky_blue_horizon: vec4<f32>,// rgb, haze horizon
    sky_blue_density: vec4<f32>,// rgb, haze density
    sky_glow: vec4<f32>,        // glow xyz, max_y
    sky_cloud_color: vec4<f32>, // rgb, cloud scale
    sky_cloud_pd1: vec4<f32>,   // cloud pos/density 1, w cloud variance
    sky_cloud_pd2: vec4<f32>,   // cloud pos/density 2, w hdr scale
    sky_sun: vec4<f32>,         // true sun dir, sun scale
    sky_moon: vec4<f32>,        // moon dir, moon scale
    sky_misc: vec4<f32>,        // dome offset, dome radius, moon brightness, star brightness
    tex_slots: vec4<u32>,       // cloud, sun, moon, water normal map
    sky_ll: vec4<f32>,          // x classic sky (1 = no reflection probe ambiance), y distance multiplier
    sky_obj_light: vec4<f32>,   // rgb sun / moon color lighting objects (mSunDiffuse, max component <= 1)
    cascade_vp: array<mat4x4<f32>, 3>,
    lights: array<Light, 64>,
};

struct DrawRecord {
    model: mat4x4<f32>,
    base_color: vec4<f32>,
    emissive: vec4<f32>,      // rgb emissive, a glow
    uv_st: vec4<f32>,         // scale s, scale t, offset s, offset t
    params: vec4<f32>,        // x rotation, y metallic, z roughness, w alpha cutoff
    tex: vec4<u32>,           // base, normal, metallic-roughness, emissive
    flags: vec4<u32>,         // x flags
    mat_uv: vec4<f32>,        // normal map: scale s, t, offset s, t
    spec_uv: vec4<f32>,       // legacy specular / PBR metallic-roughness map: scale s, t, offset s, t
    legacy: vec4<f32>,        // x normal rot, y spec / MR rot, z glossiness (PBR: emissive rot), w env intensity
    spec_color: vec4<f32>,    // legacy specular light color (PBR: emissive map scale s, t, offset s, t)
};

const FLAG_FULLBRIGHT: u32 = 1u;
const FLAG_ALPHA_MASK: u32 = 2u;
const FLAG_ALPHA_BLEND: u32 = 4u;
const FLAG_PBR: u32 = 8u;
const FLAG_HIGHLIGHT: u32 = 16u;
const FLAG_PLANAR: u32 = 32u;
const FLAG_UNLIT_SHADOWLESS: u32 = 64u;
const FLAG_SKINNED: u32 = 128u;
const FLAG_LEGACY_MAT: u32 = 256u;
const FLAG_EMISSIVE_MASK: u32 = 512u;

@group(0) @binding(0) var<uniform> frame: Frame;
// atlas of 2x2 tiles, cascade i in tile (i % 2, i / 2)
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(0) @binding(3) var ao_tex: texture_2d<f32>;
@group(0) @binding(4) var lin_clamp: sampler;
@group(0) @binding(5) var water_refl_tex: texture_2d<f32>;
@group(0) @binding(6) var mirror_tex: texture_2d<f32>;
@group(0) @binding(7) var scene_tex: texture_2d<f32>;
@group(0) @binding(8) var depth_tex: texture_depth_2d;

// Reflection probes (probes.rs, reflectionProbeF.glsl).
struct Probe {
    center_radius: vec4<f32>,
    // x ambiance scale, y radiance scale, z fade in, w kind (0 auto, 1 sphere, -1 box)
    params: vec4<f32>,
    slot: vec4<u32>,
    box_inv: mat4x4<f32>,
};
struct ProbeSet {
    count: u32,
    max_lod: f32,
    enabled: u32,
    _pad: u32,
    probes: array<Probe, 64>,
};
@group(0) @binding(9) var<storage, read> probe_set: ProbeSet;
@group(0) @binding(10) var probe_tex: texture_cube_array<f32>;
@group(0) @binding(11) var probe_sampler: sampler;
@group(0) @binding(12) var probe_irr_tex: texture_cube_array<f32>;

@group(1) @binding(0) var<storage, read> records: array<DrawRecord>;
@group(1) @binding(1) var<storage, read> palettes: array<mat4x4<f32>>;
struct SkinBind {
    inverse_bind: mat4x4<f32>,
    joint: vec4<u32>,
};
@group(1) @binding(2) var<storage, read> skin_binds: array<SkinBind>;

@group(2) @binding(0) var textures: binding_array<texture_2d<f32>>;
@group(2) @binding(1) var tex_sampler: sampler;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3<f32>(0.0)) + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn sample_tex(idx: u32, uv: vec2<f32>) -> vec4<f32> {
    return textureSample(textures[idx], tex_sampler, uv);
}

fn sample_tex_level(idx: u32, uv: vec2<f32>, lod: f32) -> vec4<f32> {
    return textureSampleLevel(textures[idx], tex_sampler, uv, lod);
}

fn in_reflection_pass() -> bool {
    return frame.misc.w > 0.5;
}

// Probes are sampled in the main view only (captures see the sky instead,
// a single bounce).
fn probes_on() -> bool {
    return probe_set.enabled != 0u && !in_reflection_pass();
}

fn probe_max_lod() -> f32 {
    return probe_set.max_lod;
}

// reflectionProbeF.glsl sampleProbes (+ shouldSampleProbe, tapRefMap,
// sphereWeight, boxIntersect), in world space: radiance seen along `dir`
// from `pos` at mip `lod`. Probe 0 is the default probe, sampled whenever
// automatic probes are. w = 0 when no probe covers `pos`.
fn sample_probes(pos: vec3<f32>, dir: vec3<f32>, lod: f32) -> vec4<f32> {
    return sample_probes_ex(pos, dir, lod, false, vec3<f32>(0.0), false);
}

// sampleReflectionProbesWater: the default probe and manual probes only
// (automatic probes are not sampled for water).
fn sample_probes_water(pos: vec3<f32>, dir: vec3<f32>, lod: f32) -> vec4<f32> {
    return sample_probes_ex(pos, dir, lod, false, vec3<f32>(0.0), true);
}

// sampleProbeAmbient: irradiance along the normal `n`, each probe mixed with
// the sky ambient `amblit` by its ambiance (tapIrradianceMap).
fn sample_probe_ambient(pos: vec3<f32>, n: vec3<f32>, amblit: vec3<f32>) -> vec4<f32> {
    return sample_probes_ex(pos, n, 0.0, true, amblit, false);
}

fn sample_probes_ex(pos: vec3<f32>, dir: vec3<f32>, lod: f32, irradiance: bool, amblit: vec3<f32>, water: bool) -> vec4<f32> {
    let count = min(probe_set.count, 64u);
    // never let automatic probes encroach on box probes
    var sample_automatic = true;
    for (var i = 1u; i < count; i++) {
        let p = probe_set.probes[i];
        if (p.params.w < 0.0) {
            let lp = (p.box_inv * vec4<f32>(pos, 1.0)).xyz;
            if (all(abs(lp) <= vec3<f32>(1.0))) {
                sample_automatic = false;
            }
        }
    }
    var col0 = vec3<f32>(0.0);
    var col1 = vec3<f32>(0.0);
    var w0 = 0.0;
    var w1 = 0.0;
    var dw1 = 0.0;
    for (var i = 0u; i < count; i++) {
        let p = probe_set.probes[i];
        let kind = p.params.w;
        let manual = kind != 0.0;
        if (!manual && (!sample_automatic || (water && i != 0u))) {
            continue;
        }
        let c = p.center_radius.xyz;
        var v: vec3<f32>;
        var w: f32;
        var dw = 0.0;
        if (kind < 0.0) {
            // box influence volume with parallax correction
            let lp = (p.box_inv * vec4<f32>(pos, 1.0)).xyz;
            if (any(abs(lp) > vec3<f32>(1.0))) {
                continue;
            }
            let ray = (p.box_inv * vec4<f32>(dir, 0.0)).xyz;
            // the irradiance lookup pads the box (boxIntersect scale 3)
            let scale = select(1.0, 3.0, irradiance);
            let first = (vec3<f32>(scale) - lp) / ray;
            let second = (vec3<f32>(-scale) - lp) / ray;
            let far = max(first, second);
            let dist = min(far.x, min(far.y, far.z));
            v = pos + dir * dist - c;
            w = max(1.0 - max(max(abs(lp.x), abs(lp.y)), abs(lp.z)), 0.001);
        } else {
            let r = p.center_radius.w;
            let delta = pos - c;
            if (i != 0u && dot(delta, delta) > r * r) {
                continue;
            }
            if (kind >= 1.0) {
                // sphereIntersect: parallax for manual sphere probes only
                let l = c - pos;
                let tca = dot(l, dir);
                let d2 = dot(l, l) - tca * tca;
                v = pos + dir * (tca + sqrt(max(r * r - d2, 0.0))) - c;
            } else {
                v = dir;
            }
            // sphereWeight
            let r1 = r * 0.5;
            let d = max(length(delta), 0.001);
            let atten = 1.0 - max(d - r1, 0.0) / max(r - r1, 0.001);
            let base = p.params.z / d;
            dw = base * atten * max(r, 1.0) * 4.0;
            w = base * atten;
        }
        var refcol: vec3<f32>;
        if (irradiance) {
            let irr = textureSampleLevel(probe_irr_tex, probe_sampler, v, i32(p.slot.x), 0.0).rgb * p.params.x;
            refcol = mix(amblit, irr, min(p.params.x, 1.0));
        } else {
            refcol = textureSampleLevel(probe_tex, probe_sampler, v, i32(p.slot.x), lod).rgb * p.params.y;
        }
        if (manual) {
            col1 += refcol * w;
            w1 += w;
            dw1 += dw;
        } else {
            col0 += refcol * w;
            w0 += w;
        }
    }
    // mix automatic and manual probes
    if (sample_automatic && w0 > 0.0) {
        col0 /= w0;
        if (w1 > 0.0) {
            col1 = mix(col0, col1 / w1, min(dw1, 1.0));
            col0 = vec3<f32>(0.0);
        }
    } else if (w1 > 0.0) {
        col1 /= w1;
        col0 = vec3<f32>(0.0);
    }
    return vec4<f32>(col0 + col1, select(0.0, 1.0, w0 + w1 > 0.0));
}

// Radiance of the environment along `r` for a surface of the given
// perceptual roughness (glossiness = 1 - roughness): the probes when they
// cover `pos`, else `fallback` (sky reflection).
fn probe_radiance(pos: vec3<f32>, r: vec3<f32>, roughness: f32, fallback: vec3<f32>) -> vec3<f32> {
    if (!probes_on()) {
        return fallback;
    }
    let s = sample_probes(pos, r, clamp(roughness, 0.0, 1.0) * probe_max_lod());
    return select(fallback, s.rgb, s.w > 0.0);
}

/// Reflection passes: drop geometry behind the mirror / below the water.
fn reflection_clip(world_pos: vec3<f32>) -> bool {
    return in_reflection_pass() && dot(frame.clip_plane.xyz, world_pos) + frame.clip_plane.w < 0.0;
}

fn screen_uv(frag: vec2<f32>) -> vec2<f32> {
    return frag * frame.screen.zw;
}

/// World position from a reverse-Z depth value at a screen uv.
fn world_from_depth(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let w = frame.inv_view_proj * vec4<f32>(ndc, max(depth, 1e-7), 1.0);
    return w.xyz / w.w;
}

fn scene_depth_at(uv: vec2<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(depth_tex));
    let px = clamp(vec2<i32>(uv * vec2<f32>(size)), vec2<i32>(0), size - vec2<i32>(1));
    return textureLoad(depth_tex, px, 0);
}

// ------------------------------------------------------- EEP atmospherics

struct Atmos {
    color: vec3<f32>,          // haze color (sky)
    sunlight: vec3<f32>,       // attenuated sunlight
    tmp_ambient: vec3<f32>,
    haze_glow: f32,
    combined_haze: vec3<f32>,
    below: vec3<f32>,          // haze color below the clouds
    light_atten: vec3<f32>,
    rel_z: f32,                // dome point height relative to the camera (+50)
};

/// Point of the sky dome seen in direction `d` (camera-relative), as in
/// LLVOWLSky: a sphere cap of `dome_radius` lowered by `dome_offset`.
fn dome_point(d: vec3<f32>) -> vec3<f32> {
    let r = max(frame.sky_misc.y, 100.0);
    let h = frame.sky_misc.x * r;
    let b = h * d.z;
    let t = -b + sqrt(max(b * b - (h * h - r * r), 0.0));
    return d * t;
}

fn atmos_eval(dir: vec3<f32>, for_clouds: bool) -> Atmos {
    let d = normalize(dir);
    var rel = dome_point(d) + vec3<f32>(0.0, 0.0, 50.0);
    var a: Atmos;
    a.rel_z = rel.z;
    let max_y = frame.sky_glow.w;
    if (rel.z > 0.0) {
        rel *= max_y / rel.z;
    }
    if (rel.z < 0.0) {
        rel *= -32000.0 / rel.z;
    }
    let rel_norm = normalize(rel);
    let rel_len = length(rel);
    let lightnorm = frame.sky_light.xyz;
    let blue_density = frame.sky_blue_density.rgb;
    let haze_density = frame.sky_blue_density.w;
    let blue_horizon = frame.sky_blue_horizon.rgb;
    let haze_horizon = frame.sky_blue_horizon.w;
    let density_multiplier = frame.sky_ambient.w;
    let cloud_shadow = frame.sky_sunlight.w;
    let ambient_color = frame.sky_ambient.rgb;
    let glow = frame.sky_glow.xyz;
    let glow_factor = frame.sky_light.w;

    var sunlight = frame.sky_sunlight.rgb;
    let light_atten = (blue_density + vec3<f32>(haze_density * 0.25)) * (density_multiplier * max_y);
    a.light_atten = light_atten;
    var combined_haze = max(abs(blue_density) + vec3<f32>(abs(haze_density)), vec3<f32>(1e-6));
    let blue_weight = blue_density / combined_haze;
    let haze_weight = haze_density / combined_haze;
    let off_axis = 1.0 / max(1e-6, max(0.0, rel_norm.z) + lightnorm.z);
    sunlight *= exp(-light_atten * off_axis);
    let density_dist = rel_len * density_multiplier;
    combined_haze = exp(-combined_haze * density_dist);

    var haze_glow = max(1.0 - dot(rel_norm, lightnorm), 0.001);
    haze_glow *= glow.x;
    haze_glow = pow(haze_glow, glow.z);
    if (for_clouds) {
        haze_glow *= glow_factor;
        haze_glow = select(haze_glow + 0.25, 0.0, glow_factor < 1.0);
    } else {
        haze_glow = select(glow_factor * (haze_glow + 0.25), 0.0, glow_factor < 1.0);
    }
    a.haze_glow = haze_glow;

    var color = blue_horizon * blue_weight * (sunlight + ambient_color)
              + (haze_horizon * haze_weight) * (sunlight * haze_glow + ambient_color);
    color *= (vec3<f32>(1.0) - combined_haze);

    let ambient = ambient_color + max(vec3<f32>(0.0), vec3<f32>(1.0) - ambient_color) * cloud_shadow * 0.5;
    a.tmp_ambient = ambient;
    sunlight *= max(0.0, 1.0 - cloud_shadow);
    a.sunlight = sunlight;
    let add_below_cloud = blue_horizon * blue_weight * (sunlight + ambient)
                        + (haze_horizon * haze_weight) * (sunlight * haze_glow + ambient);
    a.below = add_below_cloud;
    combined_haze = sqrt(combined_haze);
    a.combined_haze = combined_haze;
    color += (add_below_cloud - color) * (vec3<f32>(1.0) - sqrt(combined_haze));
    a.color = color;
    return a;
}

/// Linear HDR sky radiance for a direction, without clouds (reflections).
fn sky_color(dir: vec3<f32>) -> vec3<f32> {
    let a = atmos_eval(dir, false);
    let c = clamp(a.color * 2.0, vec3<f32>(0.0), vec3<f32>(5.0));
    return srgb_to_linear(c) * frame.sky_cloud_pd2.w;
}

fn cloud_noise(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(textures[frame.tex_slots.x], tex_sampler, uv, 0.0).x;
}

/// Cloud layer over the sky (cloudsV/F.glsl), in the sky's sRGB-like space.
/// Returns rgb and coverage.
fn sky_clouds(dir: vec3<f32>) -> vec4<f32> {
    let cloud_scale = frame.sky_cloud_color.w;
    if (frame.tex_slots.x == 0u || cloud_scale < 0.001) {
        return vec4<f32>(0.0);
    }
    let d = normalize(dir);
    // planar dome texture coordinates (LLVOWLSky::buildStripsBuffer)
    let r = max(frame.sky_misc.y, 100.0);
    let local = (dome_point(d) + vec3<f32>(0.0, 0.0, frame.sky_misc.x * r)) / r;
    var tc0 = vec2<f32>((1.0 - local.x) * 0.5, (1.0 - local.y) * 0.5);
    tc0 = vec2<f32>(-tc0.x, tc0.y);
    tc0 = (tc0 - vec2<f32>(0.5)) / cloud_scale + vec2<f32>(0.5);
    let ln = frame.sky_light.xyz;
    let tc1 = tc0 + vec2<f32>(ln.y * 0.0125, ln.x * 0.0125);
    let tc2 = tc0 * 16.0;
    let tc3 = tc1 * 16.0;

    let a = atmos_eval(d, true);
    let max_y = frame.sky_glow.w;
    var altitude_blend = clamp((a.rel_z + 512.0) / max_y, 0.0, 1.0);
    if (a.rel_z < 0.0) {
        altitude_blend = 0.0;
    }
    // cloud lighting
    let cloud_color = frame.sky_cloud_color.rgb;
    let off_axis = 1.0 / max(1e-6, ln.z * 2.0);
    let sun_c = frame.sky_sunlight.rgb * exp(-a.light_atten * off_axis);
    let col_sun = (sun_c * a.haze_glow) * cloud_color * a.combined_haze;
    let col_amb = a.tmp_ambient * cloud_color * a.combined_haze + a.below * (vec3<f32>(1.0) - a.combined_haze);
    let cloud_shadow = frame.sky_sunlight.w;
    var density = 2.0 * (cloud_shadow - 0.25);

    let pd1 = frame.sky_cloud_pd1.xyz;
    let pd2 = frame.sky_cloud_pd2.xyz;
    let variance = frame.sky_cloud_pd1.w;
    var uv1 = tc0;
    var uv2 = tc1;
    var uv3 = tc2;
    var uv4 = tc3;
    let dist1 = vec2<f32>(cloud_noise(uv1 / 8.0), cloud_noise((uv3 + uv1) / 16.0)) * variance * (1.0 - cloud_scale * 0.25);
    let dist2 = vec2<f32>(cloud_noise((uv1 + uv3) / 4.0), cloud_noise((uv4 + uv2) / 8.0)) * variance * (1.0 - cloud_scale * 0.25);
    uv1 += pd1.xy + dist1 * 0.2;
    uv2 += pd1.xy;
    uv3 += pd2.xy;
    uv4 += pd2.xy;
    let dv = min(1.0, (dist1.x * 2.0 + dist1.y * 2.0 + dist2.x + dist2.y) * 4.0);
    density *= 1.0 - dv * dv;
    var alpha1 = (cloud_noise(uv1) - 0.5) + (cloud_noise(uv3) - 0.5) * pd2.z;
    alpha1 = min(max(alpha1 + density, 0.0) * 10.0 * pd1.z, 1.0);
    alpha1 = 1.0 - alpha1 * alpha1;
    alpha1 = 1.0 - alpha1 * alpha1;
    alpha1 = clamp(alpha1 * altitude_blend, 0.0, 1.0);
    var alpha2 = cloud_noise(uv2) - 0.5;
    alpha2 = min(max(alpha2 + density, 0.0) * 2.5 * pd1.z, 1.0);
    alpha2 = 1.0 - alpha2;
    alpha2 = 1.0 - alpha2 * alpha2;
    var color = col_sun * (1.0 - alpha2) + col_amb;
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)) * 2.0;
    return vec4<f32>(color, alpha1);
}

// ------------------------------------------------------------- water fog

/// SL getWaterFogViewNoClip: in-scattered color (rgb) and transmittance (a)
/// along the camera-relative segment to `pos_rel`.
fn water_fog_view(pos_rel: vec3<f32>, kd: f32) -> vec4<f32> {
    let view = normalize(pos_rel);
    let w = frame.camera_pos.z - frame.params.y;
    let es = max(-view.z, 1e-4);
    let e0 = max(-w, 0.0);
    var int_v = vec3<f32>(0.0);
    if (w > 0.0) {
        int_v = view * w / es;
    }
    let l = max(length(pos_rel - int_v), 0.1);
    let ks = frame.water_params.x;
    let f = 0.98;
    let t1 = -kd * pow(f, ks * e0);
    let t2 = kd + ks * es;
    let t3 = pow(f, t2 * l) - 1.0;
    let lum = pow(max(min(t1 / t2 * t3, 1.0), 0.0), 1.0 / 1.7);
    let dt = pow(0.98, l * kd);
    return vec4<f32>(frame.water_fog.rgb * lum, dt);
}

fn camera_underwater() -> bool {
    return frame.camera_pos.z < frame.params.y && !in_reflection_pass();
}

fn apply_fog(col: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    if (camera_underwater()) {
        let wf = water_fog_view(world_pos - frame.camera_pos.xyz, frame.water_normal.w);
        return col * wf.a + wf.rgb;
    }
    let dist = distance(world_pos, frame.camera_pos.xyz);
    let height_factor = exp(-max(world_pos.z - frame.params.y, 0.0) * 0.004);
    let f = 1.0 - exp(-dist * frame.fog.w * height_factor);
    return mix(col, frame.fog.rgb, clamp(f, 0.0, 1.0));
}

// ------------------------------------------------------------- shadows

// Half-width of the shadow filter in metres, the same in every cascade:
// with a filter sized in texels, each cascade (texels from ~6 mm to ~7 cm)
// had its own softness and the cascade borders showed as steps. Firestorm
// softens its shadows in screen space (RenderShadowBlurSize).
const SHADOW_PENUMBRA: f32 = 0.03;

// Shadow of one cascade (1 lit, 0 shadowed); -1 when the point is outside
// its tile, so that a blend never takes light from a cascade that does not
// see the point.
fn cascade_shadow(world_pos: vec3<f32>, n: vec3<f32>, cascade: u32) -> f32 {
    let bias_n = n * (0.02 + 0.04 * f32(cascade));
    let m = frame.cascade_vp[cascade];
    let lp = m * vec4<f32>(world_pos + bias_n, 1.0);
    let ndc = lp.xyz / lp.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0) {
        return -1.0;
    }
    // texel of a tile, in tile coordinates
    let texel = 2.0 / f32(textureDimensions(shadow_map).x);
    // half extent of the cascade in metres: orthographic, so the first row
    // of its matrix is the light's right axis divided by it
    let radius = 1.0 / max(length(vec3<f32>(m[0].x, m[1].x, m[2].x)), 1e-6);
    let half_uv = SHADOW_PENUMBRA * 0.5 / radius;
    // 4x4 taps spread over the penumbra, kept between half a texel and two
    // texels apart so the bilinear compare taps blend without gaps
    let spacing = clamp(half_uv * (2.0 / 3.0), 0.5 * texel, 2.0 * texel);
    let tile = vec2<f32>(f32(cascade % 2u), f32(cascade / 2u));
    var sum = 0.0;
    for (var y = 0; y < 4; y++) {
        for (var x = 0; x < 4; x++) {
            let o = (vec2<f32>(f32(x), f32(y)) - 1.5) * spacing;
            // clamped to the centers of the tile's edge texels: the same
            // result as the clamp-to-edge of a separate texture, never a
            // texel of the neighbouring tile
            let t = clamp(uv + o, vec2<f32>(0.5 * texel), vec2<f32>(1.0 - 0.5 * texel));
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, (t + tile) * 0.5, ndc.z - 0.0005);
        }
    }
    return sum / 16.0;
}

fn shadow_factor(world_pos: vec3<f32>, n: vec3<f32>, view_depth: f32) -> f32 {
    if (frame.params.z < 0.5) {
        return 1.0;
    }
    let splits = frame.cascade_splits;
    var cascade = 0u;
    if (view_depth > splits.x) { cascade = 1u; }
    if (view_depth > splits.y) { cascade = 2u; }
    if (view_depth > splits.z) {
        return 1.0;
    }
    var s = cascade_shadow(world_pos, n, cascade);
    // blend into the next cascade over the last 15 % of this one: no visible
    // border between cascades
    if (cascade < 2u) {
        let start = select(0.0, splits.x, cascade == 1u);
        let end = select(splits.x, splits.y, cascade == 1u);
        let band = (end - start) * 0.15;
        let t = clamp((view_depth - (end - band)) / max(band, 1e-4), 0.0, 1.0);
        if (t > 0.0) {
            let next = cascade_shadow(world_pos, n, cascade + 1u);
            if (s < 0.0) {
                s = next;
            } else if (next >= 0.0) {
                s = mix(s, next, t);
            }
        }
    }
    if (s < 0.0) {
        return 1.0;
    }
    // fade out at the far end of the last cascade
    let fade = clamp((splits.z - view_depth) / (splits.z * 0.1), 0.0, 1.0);
    return mix(1.0, s, fade);
}

// ---------------------------------------------------------------- BRDF

fn d_ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d + 1e-6);
}

fn v_smith(nv: f32, nl: f32, a: f32) -> f32 {
    let k = a * 0.5;
    let gv = nv / (nv * (1.0 - k) + k);
    let gl = nl / (nl * (1.0 - k) + k);
    return gv * gl / max(4.0 * nv * nl, 1e-4);
}

fn f_schlick(f0: vec3<f32>, vh: f32) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vh, 5.0);
}

// Karis' analytic approximation of the split-sum environment BRDF.
fn env_brdf(f0: vec3<f32>, roughness: f32, nv: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

fn screen_ao(frag: vec2<f32>) -> f32 {
    if (frame.misc.x < 0.5 || in_reflection_pass()) {
        return 1.0;
    }
    return textureSampleLevel(ao_tex, lin_clamp, screen_uv(frag), 0.0).r;
}

/// How much a surface takes the planar mirror image (SL hero probe).
fn mirror_weight(world_pos: vec3<f32>, n: vec3<f32>) -> f32 {
    if (dot(frame.mirror_plane.xyz, frame.mirror_plane.xyz) < 0.5 || in_reflection_pass()) {
        return 0.0;
    }
    let lp = (frame.mirror_box * vec4<f32>(world_pos, 1.0)).xyz;
    if (any(abs(lp) > vec3<f32>(0.52))) {
        return 0.0;
    }
    return smoothstep(0.8, 0.95, dot(n, frame.mirror_plane.xyz));
}

fn shade_pbr(base: vec3<f32>, metallic: f32, roughness_in: f32, n: vec3<f32>, v: vec3<f32>,
             world_pos: vec3<f32>, shadow: f32, frag: vec2<f32>) -> vec3<f32> {
    let roughness = clamp(roughness_in, 0.04, 1.0);
    let a = roughness * roughness;
    let f0 = mix(vec3<f32>(0.04), base, metallic);
    let diffuse_color = base * (1.0 - metallic);
    let nv = max(dot(n, v), 1e-4);
    let ao = screen_ao(frag);

    var col = vec3<f32>(0.0);
    // sun / moon
    let l = normalize(frame.sun_dir.xyz);
    let nl = max(dot(n, l), 0.0);
    if (nl > 0.0) {
        let h = normalize(l + v);
        let nh = max(dot(n, h), 0.0);
        let vh = max(dot(v, h), 0.0);
        let f = f_schlick(f0, vh);
        let spec = d_ggx(nh, a) * v_smith(nv, nl, a) * f;
        let kd = (vec3<f32>(1.0) - f) * diffuse_color / PI;
        col += (kd + spec) * frame.sun_color.rgb * nl * shadow;
    }
    // local lights
    let count = u32(frame.params.w);
    for (var i = 0u; i < count; i++) {
        let lt = frame.lights[i];
        let to_l = lt.pos_radius.xyz - world_pos;
        let dist = length(to_l);
        let radius = lt.pos_radius.w;
        if (dist < radius) {
            let ll = to_l / max(dist, 1e-4);
            let lnl = max(dot(n, ll), 0.0);
            if (lnl > 0.0) {
                let atten = ll_light_atten(dist / radius, lt.color_falloff.w);
                let h = normalize(ll + v);
                let nh = max(dot(n, h), 0.0);
                let vh = max(dot(v, h), 0.0);
                let f = f_schlick(f0, vh);
                let spec = d_ggx(nh, a) * v_smith(nv, lnl, a) * f;
                let kd = (vec3<f32>(1.0) - f) * diffuse_color / PI;
                col += (kd + spec) * lt.color_falloff.rgb * lnl * atten;
            }
        }
    }
    // ambient: hemisphere diffuse + sky reflection
    let up = n.z * 0.5 + 0.5;
    let hemi = mix(mix(frame.ground_color.rgb, frame.sky_horizon.rgb, clamp(up * 2.0, 0.0, 1.0)),
                   frame.sky_zenith.rgb, clamp(up * 2.0 - 1.0, 0.0, 1.0));
    // non classic skies: ambient from the probes' irradiance (sampleProbeAmbient)
    var amb = hemi;
    if (probes_on()) {
        let s = sample_probe_ambient(world_pos, n, hemi);
        if (s.w > 0.0) {
            amb = s.rgb;
        }
    }
    col += diffuse_color * amb * ao;
    let r = reflect(-v, n);
    var env = mix(sky_color(vec3<f32>(r.x, r.y, max(r.z, -0.2))), hemi, roughness);
    var occl = (0.35 + 0.65 * shadow) * (1.0 - 0.75 * roughness) * ao;
    if (probes_on()) {
        // pbrIbl radiance from the reflection probes (ambient occlusion only)
        let s = sample_probes(world_pos, r, roughness * probe_max_lod());
        if (s.w > 0.0) {
            env = s.rgb;
            occl = ao;
        }
    }
    let mw = mirror_weight(world_pos, n) * clamp(1.0 - roughness * 1.5, 0.0, 1.0);
    if (mw > 0.0) {
        let m = textureSampleLevel(mirror_tex, lin_clamp, screen_uv(frag), 0.0).rgb;
        env = mix(env, m, mw);
        occl = mix(occl, 1.0, mw);
    }
    col += env * env_brdf(f0, roughness, nv) * occl;
    return col;
}

// Normal mapping without precomputed tangents (Mikkelsen 2010 / Schüler).
// `uv` is the sampling coordinate of the top-left-origin texture, so the
// derived bitangent points down the image; SL normal maps (materialF.glsl
// getNormal: B = sign * cross(N, T) along +t, GL bottom-left origin) put +Y
// up the image, hence the green channel is negated.
fn perturb_normal(n: vec3<f32>, p: vec3<f32>, uv: vec2<f32>, map: vec3<f32>) -> vec3<f32> {
    let dp1 = dpdx(p);
    let dp2 = dpdy(p);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let invmax = inverseSqrt(max(dot(t, t), dot(b, b)) + 1e-12);
    let tbn = mat3x3<f32>(t * invmax, b * invmax, n);
    let m = (map * 2.0 - vec3<f32>(1.0)) * vec3<f32>(1.0, -1.0, 1.0);
    let res = normalize(tbn * m);
    return select(n, res, dot(res, res) > 0.5);
}

// ------------------------------------------------------ screen-space rays

/// March a world-space ray through the depth buffer; returns the hit color
/// from the scene copy and a confidence (0 = miss).
fn ssr_trace(origin: vec3<f32>, dir: vec3<f32>, steps: i32) -> vec4<f32> {
    var t = 0.15;
    var prev_t = 0.0;
    for (var i = 0; i < steps; i++) {
        let p = origin + dir * t;
        let c = frame.view_proj * vec4<f32>(p, 1.0);
        if (c.w <= 0.01) {
            break;
        }
        let ndc = c.xyz / c.w;
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
            break;
        }
        let d = scene_depth_at(uv);
        // reverse-Z: the scene is in front of the ray point when its depth is larger
        if (d > ndc.z && d > 0.0) {
            let sp = world_from_depth(uv, d);
            let gap = distance(sp, frame.camera_pos.xyz) - distance(p, frame.camera_pos.xyz);
            if (abs(gap) < max(t - prev_t, 0.25) * 1.5) {
                // refine between the previous and current step
                var lo = prev_t;
                var hi = t;
                var huv = uv;
                for (var k = 0; k < 5; k++) {
                    let mid = (lo + hi) * 0.5;
                    let mp = origin + dir * mid;
                    let mc = frame.view_proj * vec4<f32>(mp, 1.0);
                    let mn = mc.xyz / mc.w;
                    let muv = vec2<f32>(mn.x * 0.5 + 0.5, 0.5 - mn.y * 0.5);
                    if (scene_depth_at(muv) > mn.z) {
                        hi = mid;
                        huv = muv;
                    } else {
                        lo = mid;
                    }
                }
                let edge = min(min(huv.x, 1.0 - huv.x), min(huv.y, 1.0 - huv.y));
                let conf = clamp(edge * 10.0, 0.0, 1.0) * (1.0 - f32(i) / f32(steps));
                let col = textureSampleLevel(scene_tex, lin_clamp, huv, 0.0).rgb;
                return vec4<f32>(col, conf);
            }
        }
        prev_t = t;
        t = t * 1.18 + 0.1;
    }
    return vec4<f32>(0.0);
}

// calcLegacyDistanceAttenuation (deferredUtil.glsl): `d` is the distance
// over the light radius, `falloff` the prim light's falloff (0..2).
fn ll_light_atten(d: f32, falloff: f32) -> f32 {
    let f = max(falloff, 0.0);
    let x = 1.0 - clamp((d + f) / (1.0 + f), 0.0, 1.0);
    return x * x * 2.0;
}

// ------------------------------------------------- classic skies (Firestorm)
// Skies without reflection probe ambiance ("classic" EEP skies): objects are
// lit and hazed as the Second Life viewer does (atmosphericsFuncs.glsl,
// softenLightF.glsl, deferredUtil.glsl pbrBaseLight, hazeF.glsl) and the
// frame is not tone mapped. Values are in SL's own units (1 = white).

// SL lighting (classic or linear) instead of our HDR model: skies whose
// reflection probe ambiance is 0 or absent.
fn is_classic_sky() -> bool {
    return frame.sky_ll.x > 0.5;
}

// Classic mode proper (no reflection_probe_ambiance key at all).
fn ll_classic_mode() -> bool {
    return frame.sky_ll.x < 1.5;
}

// Sunlight for specular terms (linear).
fn ll_sun_linear(a: LLAtmos) -> vec3<f32> {
    if (ll_classic_mode()) {
        return srgb_to_linear(a.sunlit * 1.35);
    }
    return srgb_to_linear(a.sunlit);
}

fn ll_linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let cl = max(c, vec3<f32>(0.0));
    let lo = cl * 12.92;
    let hi = 1.055 * pow(cl, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, cl <= vec3<f32>(0.0031308));
}

struct LLAtmos {
    sunlit: vec3<f32>,   // sRGB-like
    amblit: vec3<f32>,   // sRGB-like, before ambientLighting
    additive: vec3<f32>,
    atten: vec3<f32>,
};

// calcAtmosphericVars: `rel` = point minus camera (world, z up).
fn ll_atmos(rel_in: vec3<f32>) -> LLAtmos {
    var rel_pos = rel_in;
    let max_y = frame.sky_glow.w;
    if (abs(rel_pos.z) > max_y) {
        rel_pos *= max_y / abs(rel_pos.z);
    }
    let rel_pos_norm = normalize(rel_pos + vec3<f32>(0.0, 0.0, 1e-6));
    let rel_pos_len = length(rel_pos);
    let blue_density = frame.sky_blue_density.rgb;
    let haze_density = frame.sky_blue_density.w;
    let density_multiplier = frame.sky_ambient.w;
    let lightnorm = frame.sky_light.xyz;
    let light_dir = frame.sun_dir.xyz;
    // sunlight_color / moonlight_color as LLPipeline::bindDeferredShader sets
    // them: normalized, and the moon without skyV's 0.7
    var sunlight = frame.sky_obj_light.rgb;
    let cloud_shadow = frame.sky_sunlight.w;
    let light_atten = (blue_density + vec3<f32>(haze_density * 0.25)) * (density_multiplier * max_y);
    let combined = max(blue_density + vec3<f32>(haze_density), vec3<f32>(1e-6));
    let blue_weight = blue_density / combined;
    let haze_weight = vec3<f32>(haze_density) / combined;
    sunlight *= exp(-light_atten * (1.0 / max(1e-6, lightnorm.z)));
    let density_dist = rel_pos_len * density_multiplier;
    let atten = exp(-combined * density_dist * frame.sky_ll.y);
    var haze_glow = dot(rel_pos_norm, lightnorm);
    haze_glow *= max(0.0, dot(light_dir, rel_pos_norm));
    haze_glow = max(1.0 - haze_glow, 0.001) * frame.sky_glow.x;
    haze_glow = clamp(pow(haze_glow, frame.sky_glow.z), -100000.0, 100000.0) + 0.25;
    haze_glow *= frame.sky_light.w;
    let amb = frame.sky_ambient.rgb;
    let tmp_ambient = amb + (vec3<f32>(1.0) - amb) * cloud_shadow * 0.5;
    let cs = sunlight * (1.0 - cloud_shadow);
    var additive = frame.sky_blue_horizon.rgb * blue_weight * (cs + tmp_ambient)
        + frame.sky_blue_horizon.w * haze_weight * (cs * haze_glow + tmp_ambient);
    var o: LLAtmos;
    o.sunlit = sunlight;
    o.amblit = pow(max(tmp_ambient, vec3<f32>(0.0)), vec3<f32>(0.9)) * 0.57;
    o.additive = min(additive * (vec3<f32>(1.0) - atten), vec3<f32>(10.0));
    o.atten = atten;
    return o;
}

// ambientLighting: a touch of light opposite the sun.
fn ll_ambient_lighting(n: vec3<f32>) -> f32 {
    let a = min(abs(dot(n, frame.sun_dir.xyz)), 1.0) * 0.5;
    return 1.0 - a * a;
}

// hazeF.glsl applied to a lit color.
fn ll_haze(col: vec3<f32>, a: LLAtmos) -> vec3<f32> {
    return col * a.atten + srgb_to_linear(a.additive * 2.0) * frame.sky_cloud_pd2.w;
}

// adjustIrradiance (SSAO only darkens the ambient): RenderSSAOIrradianceScale
// 0.6, RenderSSAOIrradianceMax 0.18, then ssao_effect_mat from
// RenderSSAOEffect (0.8, 1.0): value x 0.8, saturation x 1, i.e. minus 0.2 x
// the mean of the channels.
fn ll_ssao(irr: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    let ao = screen_ao(frag);
    let occluded = min(irr * 0.6, vec3<f32>(0.18));
    let effect = occluded - vec3<f32>(0.2 * (occluded.r + occluded.g + occluded.b) / 3.0);
    return mix(effect, irr, ao);
}

// softenLightF.glsl final_scale: 1.1 on classic skies, applied to the whole
// sun + sky result of every surface (legacy and PBR, specular, environment
// and emissive included), but not to the local lights, which LL adds in
// another pass (multiPointLightF, its own final_scale 0.9).
fn ll_final_scale() -> f32 {
    return select(1.0, 1.1, ll_classic_mode());
}

// multiPointLightF.glsl final_scale.
fn ll_local_scale() -> f32 {
    return select(1.0, 0.9, ll_classic_mode());
}

// Sun and sky lighting of a classic (Blinn-Phong) surface: softenLightF
// legacy branch before its final_scale. `base` is the linear albedo.
fn ll_classic_sun_ambient(base: vec3<f32>, n: vec3<f32>, shadow: f32, frag: vec2<f32>, a: LLAtmos) -> vec3<f32> {
    let nl_sun = max(dot(n, frame.sun_dir.xyz), 0.0);
    if (ll_classic_mode()) {
        let sunlit = a.sunlit * 1.35;
        let irr = ll_ssao(a.amblit * ll_ambient_lighting(n), frag);
        let sun_contrib = vec3<f32>(min(pow(nl_sun, 1.2), shadow));
        return srgb_to_linear(irr * 0.9 + ll_linear_to_srgb(sun_contrib) * sunlit * 0.7) * base;
    }
    // linear mode: grey sky ambient, linear sunlight, plain lambert
    let amb_lin = srgb_to_linear(a.amblit * ll_ambient_lighting(n));
    let irr = ll_ssao(vec3<f32>(dot(amb_lin, vec3<f32>(0.2126, 0.7152, 0.0722))), frag);
    return (irr + min(nl_sun, shadow) * srgb_to_linear(a.sunlit)) * base;
}

// Local lights on a classic surface (multiPointLightF): light color x n.l x
// attenuation x albedo, with its final_scale.
fn ll_local_diffuse(base: vec3<f32>, n: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    var local = vec3<f32>(0.0);
    // our colors carry the 3.25 PBR factor
    let count = u32(frame.params.w);
    for (var i = 0u; i < count; i++) {
        let lt = frame.lights[i];
        let to_l = lt.pos_radius.xyz - world_pos;
        let dist = length(to_l);
        let radius = lt.pos_radius.w;
        if (dist < radius) {
            let nl = dot(n, to_l / max(dist, 1e-4));
            if (nl > 0.0) {
                local += lt.color_falloff.rgb / 3.25 * nl * ll_light_atten(dist / radius, lt.color_falloff.w) * base;
            }
        }
    }
    return local * ll_local_scale();
}

// Diffuse lighting of a classic surface without specular or emissive (the
// terrain): sun and sky with the final scale, plus the local lights.
fn ll_classic_diffuse(base: vec3<f32>, n: vec3<f32>, world_pos: vec3<f32>, shadow: f32, frag: vec2<f32>, a: LLAtmos) -> vec3<f32> {
    return ll_classic_sun_ambient(base, n, shadow, frag, a) * ll_final_scale() + ll_local_diffuse(base, n, world_pos);
}

// calcDiffuseSpecular (deferredUtil.glsl): base x (1 - F0) x (1 - metallic).
fn ll_pbr_diffuse_color(base: vec3<f32>, metallic: f32) -> vec3<f32> {
    return base * 0.96 * (1.0 - metallic);
}

// pbrBaseLight classic mode (deferredUtil.glsl): metallic-roughness surfaces
// recombined like Blinn-Phong ones, plus the image based specular (pbrIbl),
// before softenLightF's final_scale. The local lights are separate
// (ll_classic_pbr_local), as LL adds them in another pass.
fn ll_classic_pbr(base: vec3<f32>, metallic: f32, roughness_in: f32, n: vec3<f32>, v: vec3<f32>, world_pos: vec3<f32>, shadow: f32, frag: vec2<f32>, a: LLAtmos) -> vec3<f32> {
    let roughness = clamp(roughness_in, 0.04, 1.0);
    let ra = roughness * roughness;
    let f0 = mix(vec3<f32>(0.04), base, metallic);
    let diffuse_color = ll_pbr_diffuse_color(base, metallic);
    let nv = max(dot(n, v), 1e-4);
    let sunlit = a.sunlit * 1.35;
    let irr = srgb_to_linear(ll_ssao(a.amblit * ll_ambient_lighting(n), frag) * 0.9);
    let l = normalize(frame.sun_dir.xyz);
    let nl = max(dot(n, l), 0.0);
    var final_sun = vec3<f32>(0.0);
    if (nl > 0.0) {
        let h = normalize(l + v);
        let nh = max(dot(n, h), 0.0);
        let vh = max(dot(v, h), 0.0);
        let f = f_schlick(f0, vh);
        let spec = d_ggx(nh, ra) * v_smith(nv, nl, ra) * f;
        let diff = (vec3<f32>(1.0) - f) * diffuse_color / PI;
        let sun_contrib = srgb_to_linear(ll_linear_to_srgb(vec3<f32>(min(pow(nl, 1.2), shadow))) * sunlit * 0.7) * PI;
        final_sun = clamp(sun_contrib * (diff + spec) * shadow, vec3<f32>(0.0), vec3<f32>(10.0));
    }
    var col = srgb_to_linear(ll_linear_to_srgb(irr * diffuse_color) + ll_linear_to_srgb(final_sun) * 1.1);
    // pbrBaseLight adds the image based specular (pbrIbl: radiance x
    // (F0 x brdf.x + brdf.y) x ao) on classic skies too. LL samples its
    // reflection probes; without them the radiance is the sky reflection,
    // blurred toward the hemisphere with roughness, or the mirror.
    let up = n.z * 0.5 + 0.5;
    let hemi = mix(mix(frame.ground_color.rgb, frame.sky_horizon.rgb, clamp(up * 2.0, 0.0, 1.0)),
                   frame.sky_zenith.rgb, clamp(up * 2.0 - 1.0, 0.0, 1.0));
    let r = reflect(-v, n);
    var env = probe_radiance(world_pos, r, roughness, mix(sky_color(vec3<f32>(r.x, r.y, max(r.z, -0.2))), hemi, roughness));
    let mw = mirror_weight(world_pos, n) * clamp(1.0 - roughness * 1.5, 0.0, 1.0);
    if (mw > 0.0) {
        let m = textureSampleLevel(mirror_tex, lin_clamp, screen_uv(frag), 0.0).rgb;
        env = mix(env, m, mw);
    }
    // pbrIbl scales the specular by the material's occlusion only (ORM red);
    // SSAO darkens the irradiance alone (adjustIrradiance)
    col += env * env_brdf(f0, roughness, nv) * screen_ao(frag);
    return col;
}

// Local lights on a PBR surface (multiPointLightF -> pbrCalcPointLightOrSpotLight),
// with multiPointLightF's final_scale.
fn ll_classic_pbr_local(base: vec3<f32>, metallic: f32, roughness_in: f32, n: vec3<f32>, v: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let roughness = clamp(roughness_in, 0.04, 1.0);
    let ra = roughness * roughness;
    let f0 = mix(vec3<f32>(0.04), base, metallic);
    let diffuse_color = ll_pbr_diffuse_color(base, metallic);
    let nv = max(dot(n, v), 1e-4);
    var col = vec3<f32>(0.0);
    let count = u32(frame.params.w);
    for (var i = 0u; i < count; i++) {
        let lt = frame.lights[i];
        let to_l = lt.pos_radius.xyz - world_pos;
        let dist = length(to_l);
        let radius = lt.pos_radius.w;
        if (dist < radius) {
            let ll = to_l / max(dist, 1e-4);
            let lnl = max(dot(n, ll), 0.0);
            if (lnl > 0.0) {
                let h = normalize(ll + v);
                let f = f_schlick(f0, max(dot(v, h), 0.0));
                let spec = d_ggx(max(dot(n, h), 0.0), ra) * v_smith(nv, lnl, ra) * f;
                let diff = (vec3<f32>(1.0) - f) * diffuse_color / PI;
                let intensity = ll_light_atten(dist / radius, lt.color_falloff.w) * lt.color_falloff.rgb;
                col += intensity * clamp(lnl * (diff + spec), vec3<f32>(0.0), vec3<f32>(10.0));
            }
        }
    }
    return col * ll_local_scale();
}
