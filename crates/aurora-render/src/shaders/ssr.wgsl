// Screen-space reflections on glossy surfaces. Concatenated after
// common.wgsl; drawn as a full-screen pass blended over the lit scene.

@group(3) @binding(0) var gbuf_tex: texture_2d<f32>;
// The ray-traced reflections (rt_reflect.wgsl), read by `fs_ssr_rt`; black
// textures without them. The mirror ray's: rgb and 1 on a hit. The glossy
// ones: rgb the light of the rays that met something, a their share (the
// cover). The base colour of a metal: rgb its square root, a 1 where found.
@group(3) @binding(1) var rt_reflect_tex: texture_2d<f32>;
@group(3) @binding(2) var rt_gloss_tex: texture_2d<f32>;
@group(3) @binding(3) var rt_base_tex: texture_2d<f32>;

struct SsrOut {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_ssr(@builtin(vertex_index) vi: u32) -> SsrOut {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var o: SsrOut;
    o.clip = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

fn oct_decode(e_in: vec2<f32>) -> vec3<f32> {
    let e = e_in * 2.0 - vec2<f32>(1.0);
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>((vec2<f32>(1.0) - abs(n.yx)) * s, n.z);
    }
    return normalize(n);
}

// The share of a reflection over the lit colour: SSR's Fresnel, gloss and
// hit weights, under the planar mirror's.
fn reflection_weight(p: vec3<f32>, n: vec3<f32>, v: vec3<f32>, r: vec3<f32>, roughness: f32, metallic: f32,
                     hit: f32) -> f32 {
    let nv = max(dot(n, v), 1e-3);
    let f0 = mix(0.04, 1.0, metallic);
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - nv, 5.0);
    let smooth_k = clamp(1.0 - roughness * 2.0, 0.0, 1.0);
    // the planar mirror wins over SSR. The Legacy Viewer applies tapHeroProbe
    // after the SSR mix (reflectionProbeF.glsl doProbeSample), leaving
    // (1 - w) of SSR; w falls off near the probe's plane (0.74 on the face of
    // a 5 cm panel centred on it), so SSR's copy of the scene behind the
    // camera still showed through. Aurora drops SSR once the mirror takes a
    // quarter of the surface, fading over the box's and the image's edges.
    let mirror_w = mirror_tap(p, r, 1.0 - roughness).a;
    let ssr_w = 1.0 - smoothstep(0.0, 0.25, mirror_w);
    return clamp(fresnel * smooth_k * smooth_k * hit * ssr_w, 0.0, 1.0);
}

@fragment
fn fs_ssr(in: SsrOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.clip.xy);
    let g = textureLoad(gbuf_tex, px, 0);
    let roughness = g.z;
    let uv = screen_uv(in.clip.xy);
    let d = scene_depth_at(uv);
    if (roughness > 0.5 || d <= 0.0) {
        discard;
    }
    let n = oct_decode(g.xy);
    let p = world_from_depth(uv, d);
    let v = normalize(frame.camera_pos.xyz - p);
    let r = reflect(-v, n);
    let hit = ssr_trace(p + n * 0.03, r, 48);
    if (hit.a <= 0.0) {
        discard;
    }
    let k = reflection_weight(p, n, v, r, roughness, g.w, hit.a);
    // premultiplied: mix the reflection over the lit color
    return vec4<f32>(hit.rgb * k, k);
}

// The share of a mirror ray's reflection a surface of this roughness
// takes. One ray per pixel is a mirror's image, right only for a surface
// close to a mirror: on a satin metal (a roughness of 0.25 to 0.5) it showed
// a sharp picture cut up by the normal map. The Legacy Viewer gives a PBR
// face its screen-space reflections from a glossiness of 0.9 only
// (reflectionProbeF.glsl `doProbeSample`), the probes' image beyond. Aurora
// fades them out over a roughness of 0.1 to 0.15 rather than cutting at
// 0.1, which would draw the contour of a roughness map (rt_reflect.wgsl
// `TRACED_ROUGH_MAX` and `hit_sent` follow it), and the glossy rays take
// over (`gloss_share`).
fn traced_share(roughness: f32) -> f32 {
    return 1.0 - smoothstep(0.1, 0.15, roughness);
}

// The share of the glossy rays' reflection (rt_reflect.wgsl `cs_gloss`):
// what the mirror ray's leaves, fading into the probes' towards the
// roughness the rays stop at (`GLOSS_ROUGH_MAX` there), where a probe's
// blur is as right and costs no ray.
fn gloss_share(roughness: f32) -> f32 {
    return (1.0 - traced_share(roughness)) * (1.0 - smoothstep(0.45, 0.6, roughness));
}

// The ray-traced reflections in SSR's place. They take the place of the
// reflection the shading gave the pixel (common.wgsl `shade_pbr`: the
// probes' radiance by the environment's share of the light), and of that
// alone: the pixel's own light, its highlights and its colour stay, where
// a plain mix over the lit colour dimmed a lamp's highlight on a metal as
// much as it showed the reflection. The shading's reflection is worked out
// again here, as a share of the lit colour (the scene copy's): what the
// G-buffer does not tell (a material's own occlusion) can only make that
// share too large, which a share of 1 bounds, the plain mix of before.
@fragment
fn fs_ssr_rt(in: SsrOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.clip.xy);
    let g = textureLoad(gbuf_tex, px, 0);
    let roughness = g.z;
    let uv = screen_uv(in.clip.xy);
    let d = scene_depth_at(uv);
    if (roughness > 0.6 || d <= 0.0) {
        discard;
    }
    let sharp = textureLoad(rt_reflect_tex, px, 0);
    let gloss = textureLoad(rt_gloss_tex, px, 0);
    let smooth_k = clamp(1.0 - roughness * 2.0, 0.0, 1.0);
    let k_sharp = traced_share(roughness) * smooth_k * smooth_k * sharp.a;
    let k_gloss = gloss_share(roughness);
    let traced = k_sharp + k_gloss * gloss.a;
    if (traced <= 0.0) {
        discard;
    }
    let n = oct_decode(g.xy);
    let p = world_from_depth(uv, d);
    let v = normalize(frame.camera_pos.xyz - p);
    let r = reflect(-v, n);
    let nv = max(dot(n, v), 1e-4);
    // the planar mirror wins over the traced reflections as over SSR
    // (`reflection_weight`)
    let ssr_w = 1.0 - smoothstep(0.0, 0.25, mirror_tap(p, r, 1.0 - roughness).a);
    // the environment's share of the light, as `shade_pbr` has it; a metal
    // whose base colour was not found mirrors as a white one
    let base = textureLoad(rt_base_tex, px, 0);
    let f0 = mix(vec3<f32>(0.04), mix(vec3<f32>(1.0), base.rgb * base.rgb, base.a), g.w);
    let rough = clamp(roughness, 0.04, 1.0);
    let share = env_brdf(f0, rough, nv);
    // the reflection the shading gave, at the most
    let up = n.z * 0.5 + 0.5;
    let hemi = mix(mix(frame.ground_color.rgb, frame.sky_horizon.rgb, clamp(up * 2.0, 0.0, 1.0)),
                   frame.sky_zenith.rgb, clamp(up * 2.0 - 1.0, 0.0, 1.0));
    let spec_ao = specular_occlusion(in.clip.xy, r, rough, screen_ao(in.clip.xy));
    // (the sky's, darkened where the traced probes do not see the sky, as
    // `shade_pbr` darkens it)
    var env = mix(sky_color(vec3<f32>(r.x, r.y, max(r.z, -0.2))), hemi, rough) * (1.0 - 0.75 * rough) * screen_sky(in.clip.xy);
    if (probes_on()) {
        let s = sample_probes(p, in.clip.xy, r, rough * probe_max_lod());
        if (s.w > 0.0) {
            env = s.rgb * probe_pass_radiance();
        }
    }
    // (the air between: what `apply_fog` lets through of a colour)
    let through = apply_fog(vec3<f32>(1.0), p) - apply_fog(vec3<f32>(0.0), p);
    let given = env * share * spec_ao * through;
    let lit = textureSampleLevel(scene_tex, lin_clamp, uv, 0.0).rgb;
    let luma = vec3<f32>(0.2126, 0.7152, 0.0722);
    let given_share = clamp(dot(given, luma) / max(dot(lit, luma), 1e-6), 0.0, 1.0);
    let brought = (sharp.rgb * k_sharp + gloss.rgb * k_gloss) * share * through * ssr_w;
    // premultiplied: the lit colour less the reflection it had, and the
    // traced one
    return vec4<f32>(brought, clamp(traced * given_share * ssr_w, 0.0, 1.0));
}
