// Sky, terrain and water. Concatenated after common.wgsl and object.wgsl.
//
// Water wave layering and fresnel follow the Second Life viewer's water
// shaders (class3/environment/waterF.glsl, class1/environment/waterV.glsl),
// Copyright (C) Linden Research, Inc., LGPL 2.1.

// ------------------------------------------------------------------- sky

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) vi: u32) -> SkyOut {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var out: SkyOut;
    let p = vec2<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0);
    // reverse-Z: far plane is 0
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.ndc = p;
    return out;
}

fn hash3(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453);
}

// Textured (or analytic) disc of a heavenly body; rgb premultiplied, a coverage.
fn body_disc(dir: vec3<f32>, body: vec3<f32>, half_size: f32, slot: u32) -> vec4<f32> {
    let b = normalize(body);
    let mu = dot(dir, b);
    if (mu < 0.0) {
        return vec4<f32>(0.0);
    }
    var right = cross(b, vec3<f32>(0.0, 0.0, 1.0));
    if (dot(right, right) < 1e-6) {
        right = vec3<f32>(1.0, 0.0, 0.0);
    }
    right = normalize(right);
    let up = cross(right, b);
    let p = dir / mu - b;
    let uv = vec2<f32>(dot(p, right), -dot(p, up)) / half_size * 0.5 + vec2<f32>(0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return vec4<f32>(0.0);
    }
    let t = sample_tex_level(slot, uv, 0.0);
    return vec4<f32>(srgb_to_linear(t.rgb) * t.a, t.a);
}

fn sky_full(dir: vec3<f32>) -> vec3<f32> {
    let d = normalize(dir);
    let a = atmos_eval(d, false);
    var c = clamp(a.color * 2.0, vec3<f32>(0.0), vec3<f32>(5.0));
    let clouds = sky_clouds(d);
    c = mix(c, clouds.rgb, clouds.a);
    var col = srgb_to_linear(c) * frame.sky_cloud_pd2.w;
    let cover = 1.0 - clouds.a;
    // stars
    let sun_up = frame.sky_sun.z;
    let night = clamp(-sun_up * 4.0, 0.0, 1.0);
    if (night > 0.0 && d.z > 0.0) {
        let cell = floor(d * 300.0);
        let h = hash3(cell);
        let star = step(0.9975, h) * night * d.z * (frame.sky_misc.w / 250.0);
        col += vec3<f32>(star * 2.0) * cover;
    }
    // moon
    if (frame.tex_slots.z != 0u) {
        let m = body_disc(d, frame.sky_moon.xyz, 0.035 * frame.sky_moon.w, frame.tex_slots.z);
        col = col * (1.0 - m.a * 0.9 * cover) + m.rgb * frame.sky_misc.z * 2.0 * cover;
    }
    // sun
    let sd = normalize(frame.sky_sun.xyz);
    if (frame.tex_slots.y != 0u) {
        let s = body_disc(d, sd, 0.025 * frame.sky_sun.w, frame.tex_slots.y);
        col += s.rgb * 6.0 * cover;
    } else {
        let mu = dot(d, sd);
        let disk = smoothstep(0.99955, 0.9997, mu) * clamp(sd.z * 10.0 + 1.0, 0.0, 1.0);
        col += frame.sun_color.rgb * disk * 6.0 * cover;
    }
    return col;
}

fn sky_pixel(in: SkyOut) -> vec4<f32> {
    let w = frame.inv_view_proj * vec4<f32>(in.ndc, 0.5, 1.0);
    let dir = normalize(w.xyz / w.w - frame.camera_pos.xyz);
    var col = sky_full(dir);
    if (camera_underwater()) {
        let wf = water_fog_view(dir * 512.0, frame.water_normal.w);
        col = col * wf.a + wf.rgb;
    }
    return vec4<f32>(col, 1.0);
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    return sky_pixel(in);
}

@fragment
fn fs_sky_g(in: SkyOut) -> GOut {
    var o: GOut;
    o.color = sky_pixel(in);
    o.gbuf = vec4<f32>(0.5, 0.5, 1.0, 0.0);
    return o;
}

// --------------------------------------------------------------- terrain

struct TerrainOut {
    @invariant @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) comp: f32,
    @location(4) @interpolate(flat) record: u32,
};

@vertex
fn vs_terrain(in: VsIn) -> TerrainOut {
    let rec = records[in.instance];
    let wp = rec.model * vec4<f32>(in.pos, 1.0);
    var out: TerrainOut;
    out.clip = frame.view_proj * wp;
    out.world_pos = wp.xyz;
    out.normal = in.normal.xyz;
    out.uv = in.uv;
    // composition 0..3 packed in normal.w (-1..1)
    out.comp = (in.normal.w * 0.5 + 0.5) * 3.0;
    out.record = in.instance;
    return out;
}

fn terrain_color(in: TerrainOut) -> vec3<f32> {
    let rec = records[in.record];
    // uv is region-space meters; detail textures repeat every uv_st.x meters
    let tc = in.uv / max(rec.uv_st.x, 0.1);
    let t0 = sample_tex(rec.tex.x, tc).rgb;
    let t1 = sample_tex(rec.tex.y, tc).rgb;
    let t2 = sample_tex(rec.tex.z, tc).rgb;
    let t3 = sample_tex(rec.tex.w, tc).rgb;
    let c = clamp(in.comp, 0.0, 3.0);
    let w0 = clamp(1.0 - c, 0.0, 1.0);
    let w1 = clamp(1.0 - abs(c - 1.0), 0.0, 1.0);
    let w2 = clamp(1.0 - abs(c - 2.0), 0.0, 1.0);
    let w3 = clamp(c - 2.0, 0.0, 1.0);
    var albedo = srgb_to_linear(t0 * w0 + t1 * w1 + t2 * w2 + t3 * w3);
    if (rec.tex.x == 0u && rec.tex.y == 0u) {
        // no detail textures: flat palette (sand, grass, earth, rock)
        let p0 = vec3<f32>(0.62, 0.55, 0.40);
        let p1 = vec3<f32>(0.20, 0.32, 0.14);
        let p2 = vec3<f32>(0.30, 0.25, 0.18);
        let p3 = vec3<f32>(0.42, 0.42, 0.44);
        albedo = srgb_to_linear(p0 * w0 + p1 * w1 + p2 * w2 + p3 * w3) * 1.6;
    }
    let n = normalize(in.normal);
    let v = normalize(frame.camera_pos.xyz - in.world_pos);
    let dist = distance(in.world_pos, frame.camera_pos.xyz);
    let sh = shadow_factor(in.world_pos, n, dist);
    if (is_classic_sky() && !camera_underwater()) {
        let a = ll_atmos(in.world_pos - frame.camera_pos.xyz);
        return ll_haze(ll_classic_diffuse(albedo, n, in.world_pos, sh, in.clip.xy, a), a);
    }
    let col = shade_pbr(albedo, 0.0, 0.92, n, v, in.world_pos, sh, in.clip.xy);
    return apply_fog(col, in.world_pos);
}

@fragment
fn fs_terrain(in: TerrainOut) -> @location(0) vec4<f32> {
    let col = terrain_color(in);
    if (reflection_clip(in.world_pos)) {
        discard;
    }
    return vec4<f32>(col, 1.0);
}

@fragment
fn fs_terrain_g(in: TerrainOut) -> GOut {
    var o: GOut;
    o.color = vec4<f32>(terrain_color(in), 1.0);
    o.gbuf = vec4<f32>(oct_encode(normalize(in.normal)), 1.0, 0.0);
    return o;
}

// ----------------------------------------------------------------- water

struct WaterOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
};

@vertex
fn vs_water(in: VsIn) -> WaterOut {
    let rec = records[in.instance];
    let wp = rec.model * vec4<f32>(in.pos, 1.0);
    var out: WaterOut;
    out.clip = frame.view_proj * wp;
    out.world_pos = wp.xyz;
    return out;
}

fn analytic_wave(p: vec2<f32>, t: f32) -> vec3<f32> {
    var g = vec2<f32>(0.0);
    let dirs = array<vec2<f32>, 4>(
        vec2<f32>(0.8, 0.6), vec2<f32>(-0.6, 0.8), vec2<f32>(0.3, -0.95), vec2<f32>(-0.9, -0.35));
    let freqs = array<f32, 4>(0.35, 0.6, 1.3, 2.1);
    let amps = array<f32, 4>(0.12, 0.07, 0.035, 0.02);
    let speeds = array<f32, 4>(0.9, 1.3, 1.9, 2.6);
    for (var i = 0; i < 4; i++) {
        let ph = dot(dirs[i], p) * freqs[i] + t * speeds[i];
        g += dirs[i] * cos(ph) * freqs[i] * amps[i];
    }
    return normalize(vec3<f32>(-g.x, -g.y, 1.0));
}

@fragment
fn fs_water(in: WaterOut) -> @location(0) vec4<f32> {
    let uv = screen_uv(in.clip.xy);
    let t = frame.camera_pos.w * 0.5;
    let rel = in.world_pos - frame.camera_pos.xyz;
    let dist = length(rel);
    let view_vec = rel / max(dist, 1e-4);

    // SL wave layers (big wave + two little waves) from the normal map
    var v = in.world_pos.xy;
    v.x += (cos(v.x * 0.08) + sin(v.y * 0.02)) * 6.0;
    let w1d = frame.water_waves.xy;
    let w2d = frame.water_waves.zw;
    let big = v * 0.04 + w1d * t * 0.055;
    let little1 = v * vec2<f32>(0.45, 0.9) + w2d * t * 0.13;
    let little2 = v * vec2<f32>(0.1, 0.2) + w1d * t * 0.1;
    let slot = frame.tex_slots.w;
    var wave1 = sample_tex(slot, big).xyz * 2.0 - vec3<f32>(1.0);
    var wave2 = sample_tex(slot, little1).xyz * 2.0 - vec3<f32>(1.0);
    var wave3 = sample_tex(slot, little2).xyz * 2.0 - vec3<f32>(1.0);
    if (slot == 0u) {
        wave1 = analytic_wave(in.world_pos.xy, t * 2.0);
        wave2 = wave1;
        wave3 = wave1;
    }
    let wavef = (wave1 + wave2 * 0.4 + wave3 * 0.6) * 0.5;
    var n = wavef * frame.water_normal.xyz;
    n.z *= 2.0;
    n = normalize(n);

    // fresnel (calculateFresnelFactors)
    let fs = frame.water_params.y;
    let fo = frame.water_params.z;
    var df3 = max(vec3<f32>(0.0), vec3<f32>(
        dot(view_vec, wave1), dot(view_vec, (wave2 + wave3) * 0.5), dot(view_vec, wave3)) * fs + vec3<f32>(fo));
    df3 *= df3;
    let df2 = max(vec2<f32>(0.0), vec2<f32>(df3.x + df3.y + df3.z, dot(view_vec, normalize(wavef)) * fs + fo));

    // refraction of the scene behind the surface
    let dmod = sqrt(dist);
    let waver = wavef * 3.0;
    let ref_scale = 0.018;
    var distort2 = clamp(uv + waver.xy * ref_scale / max(dmod, 1.0) * 2.0, vec2<f32>(0.0), vec2<f32>(0.999));
    let d0 = scene_depth_at(uv);
    let surf_dist = dist;
    var refr_dist = 1e5;
    if (d0 > 0.0) {
        refr_dist = distance(world_from_depth(uv, d0), frame.camera_pos.xyz);
    }
    // shoreline: no distortion where the water is shallow
    let fade = clamp((refr_dist - surf_dist) / 10.0, 0.0, 1.0);
    distort2 = mix(uv, distort2, min(1.0, fade * 10.0));
    let d1 = scene_depth_at(distort2);
    var ref_pos = frame.camera_pos.xyz + view_vec * 1e5;
    if (d1 > 0.0) {
        ref_pos = world_from_depth(distort2, d1);
    }
    if (distance(ref_pos, frame.camera_pos.xyz) < surf_dist - 0.05) {
        // the distorted sample is in front of the water: don't refract it
        distort2 = uv;
        ref_pos = frame.camera_pos.xyz + view_vec * refr_dist;
    }
    var fb = textureSampleLevel(scene_tex, lin_clamp, distort2, 0.0).rgb;
    let underwater = frame.camera_pos.z < frame.params.y;
    if (!underwater) {
        // fog of the water volume between the surface and the refracted point
        let wf = water_fog_view(ref_pos - frame.camera_pos.xyz, frame.water_fog.w);
        fb = fb * wf.a + wf.rgb;
    }

    // reflection: planar texture, screen-space rays, or the sky
    let r = reflect(view_vec, normalize(mix(vec3<f32>(0.0, 0.0, 1.0), n, 0.6)));
    var radiance = sky_color(vec3<f32>(r.x, r.y, abs(r.z)));
    if (probes_on() && !underwater) {
        // without a planar reflection the water reflects its probes
        let s = sample_probes_water(in.world_pos, vec3<f32>(r.x, r.y, abs(r.z)), 0.0);
        if (s.w > 0.0) {
            radiance = s.rgb;
        }
    }
    if (frame.misc.z > 0.5 && !underwater) {
        let ruv = clamp(uv + n.xy * 0.035 / max(dmod * 0.25, 1.0), vec2<f32>(0.001), vec2<f32>(0.999));
        radiance = textureSampleLevel(water_refl_tex, lin_clamp, ruv, 0.0).rgb;
    } else if (frame.misc.y > 0.5 && !underwater) {
        let hit = ssr_trace(in.world_pos + vec3<f32>(0.0, 0.0, 0.02), r, 40);
        radiance = mix(radiance, hit.rgb, hit.a);
    }
    if (underwater) {
        radiance = frame.water_fog.rgb;
    }
    radiance *= df2.y;

    // sun glint (punctual light, roughness = blur multiplier)
    let l = normalize(frame.sun_dir.xyz);
    let vv = -view_vec;
    let h = normalize(l + vv);
    let rough = clamp(frame.water_params.w, 0.02, 1.0);
    let nh = max(dot(n, h), 0.0);
    let nl = max(dot(n, l), 0.0);
    let spec = d_ggx(nh, rough * rough) * v_smith(max(dot(n, vv), 1e-3), nl, rough * rough) * nl;
    let shadow = shadow_factor(in.world_pos, vec3<f32>(0.0, 0.0, 1.0), dist);
    let punctual = clamp(spec * frame.sun_color.rgb * shadow, vec3<f32>(0.0), vec3<f32>(10.0));

    var color = mix(fb, radiance, min(1.0, df2.x)) + punctual;
    color = mix(fb, color, min(1.0, fade * 60.0));
    if (underwater) {
        let wf = water_fog_view(rel, frame.water_normal.w);
        color = color * wf.a + wf.rgb;
    } else {
        color = apply_fog(color, in.world_pos);
    }
    return vec4<f32>(color, 1.0);
}
