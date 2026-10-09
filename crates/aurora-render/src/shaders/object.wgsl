// Object (prim / mesh / avatar) shading. Concatenated after common.wgsl.

// Alpha-masked faces use alpha-to-coverage with MSAA, an alpha test otherwise.
override ALPHA_TO_COVERAGE: bool = true;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
    @builtin(instance_index) instance: u32,
};

// Skinning matrix of one mesh joint: owner joint world matrix times the
// mesh's inverse bind matrix (LLSkinningUtil::initSkinningMatrixPalette).
fn joint_matrix(rec: DrawRecord, j: u32) -> mat4x4<f32> {
    let b = skin_binds[rec.flags.z + j];
    return palettes[rec.flags.y + b.joint.x] * b.inverse_bind;
}

// Linear blend skinning.
fn skin_matrix(rec: DrawRecord, j: vec4<u32>, w: vec4<f32>) -> mat4x4<f32> {
    let ws = max(w.x + w.y + w.z + w.w, 1e-4);
    var m = joint_matrix(rec, j.x) * w.x;
    if (w.y > 0.0) { m += joint_matrix(rec, j.y) * w.y; }
    if (w.z > 0.0) { m += joint_matrix(rec, j.z) * w.z; }
    if (w.w > 0.0) { m += joint_matrix(rec, j.w) * w.w; }
    return m * (1.0 / ws);
}

fn skinned_model(rec: DrawRecord, in: VsIn) -> mat4x4<f32> {
    if ((rec.flags.x & FLAG_SKINNED) != 0u) {
        return rec.model * skin_matrix(rec, in.joints, in.weights);
    }
    return rec.model;
}

struct VsOut {
    @invariant @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) record: u32,
    @location(4) view_depth: f32,
    // normal / specular (legacy) or metallic-roughness (PBR) map coordinates
    @location(5) uv_n: vec2<f32>,
    @location(6) uv_s: vec2<f32>,
    // PBR emissive map coordinates
    @location(7) uv_e: vec2<f32>,
};

fn cofactor(m: mat4x4<f32>) -> mat3x3<f32> {
    let a = m[0].xyz;
    let b = m[1].xyz;
    let c = m[2].xyz;
    return mat3x3<f32>(cross(b, c), cross(c, a), cross(a, b));
}

// Planar texgen (planarProjection, indra/newview/llface.cpp): the scaled
// volume position projected on a binormal / tangent pair picked from the
// unit-volume normal, two repeats per metre. `pos` is the unit-volume
// position; the object scale comes from the model matrix (rigged meshes
// have none, like LL's global volumes).
fn planar_st(pos: vec3<f32>, normal: vec3<f32>, model: mat4x4<f32>) -> vec2<f32> {
    let n = normalize(normal);
    var b = vec3<f32>(1.0, 0.0, 0.0);
    if (abs(n.x) >= 0.5) {
        b = vec3<f32>(0.0, select(1.0, -1.0, n.x < 0.0), 0.0);
    } else if (n.y > 0.0) {
        b = vec3<f32>(-1.0, 0.0, 0.0);
    }
    let t = cross(b, n);
    let scale = vec3<f32>(length(model[0].xyz), length(model[1].xyz), length(model[2].xyz));
    let p = pos * scale;
    return vec2<f32>(1.0 + (dot(b, p) * 2.0 - 0.5), -(dot(t, p) * 2.0 - 0.5));
}

// Face texture coordinates before any transform: the vertex's own, or the
// planar projection when the texture entry asks for it.
fn face_st(in: VsIn, rec: DrawRecord) -> vec2<f32> {
    if ((rec.flags.x & FLAG_PLANAR) != 0u) {
        return planar_st(in.pos, in.normal.xyz, rec.model);
    }
    return in.uv;
}

// LL texture-entry transform about the face center (LLFace xform), in GL
// texture space (t up). `sto`, `rot`: scale s, t, offset s, t and rotation.
fn ll_xform(st: vec2<f32>, sto: vec4<f32>, rot: f32) -> vec2<f32> {
    var s = st.x - 0.5;
    var t = st.y - 0.5;
    let ca = cos(rot);
    let sa = sin(rot);
    let tmp = s;
    s = s * ca + t * sa;
    t = -tmp * sa + t * ca;
    s = s * sto.x + sto.z + 0.5;
    t = t * sto.y + sto.w + 0.5;
    return vec2<f32>(s, t);
}

// LL texture-entry transform (diffuse, or a legacy material's normal /
// specular map), then GL->wgpu V flip.
fn ll_uv(st: vec2<f32>, sto: vec4<f32>, rot: f32) -> vec2<f32> {
    let r = ll_xform(st, sto, rot);
    return vec2<f32>(r.x, 1.0 - r.y);
}

// textureUtilV.glsl texture_transform: SL (GL) t -> glTF v = 1 - t, then
// KHR_texture_transform (`so`: scale, offset; `r`: rotation). LL flips back
// to GL; our top-left-origin textures are sampled in glTF space directly.
fn pbr_uv(uv: vec2<f32>, so: vec4<f32>, r: f32) -> vec2<f32> {
    let st = vec2<f32>(uv.x, 1.0 - uv.y);
    let c = cos(r);
    let sn = sin(r);
    let v = st * so.xy;
    let rv = vec2<f32>(c * v.x + sn * v.y, -sn * v.x + c * v.y);
    return rv + so.zw;
}

fn te_uv(uv: vec2<f32>, rec: DrawRecord) -> vec2<f32> {
    if ((rec.flags.x & FLAG_PBR) != 0u) {
        return pbr_uv(uv, rec.uv_st, rec.params.x);
    }
    return ll_uv(uv, rec.uv_st, rec.params.x);
}

// ------------------------------------------------------ texture animation
// llSetTextureAnim, evaluated per vertex from the renderer clock (`clock`:
// ms, sub-ms fraction bits; Frame / ShadowParams anim_clock). A line-by-line
// copy of aurora-render/src/tex_anim.rs (clock_counter, frame_from_counter),
// the port of LLViewerTextureAnim::animateTextures
// (indra/newview/llviewertextureanim.cpp, LGPL 2.1) and of the texture matrix
// of LLVOVolume::animateTextures: the animation replaces the rotation
// (ROTATE), the repeats (SCALE) or the offsets and repeats (frame grid) of the
// face's texture transform, `anim_xf` holds the texture entry's other parts.

const TA_LOOP: u32 = 2u;
const TA_REVERSE: u32 = 4u;
const TA_PING_PONG: u32 = 8u;
const TA_SMOOTH: u32 = 16u;
const TA_ROTATE: u32 = 32u;
const TA_SCALE: u32 = 64u;

struct TexAnimDef {
    mode: u32,
    size_x: f32,
    size_y: f32,
    start: f32,
    rate: f32,
    num: f32,
    full: f32,
};

fn tex_anim_def(rec: DrawRecord) -> TexAnimDef {
    var d: TexAnimDef;
    let packed = rec.flags.w;
    d.mode = packed & 0xffu;
    d.size_x = f32((packed >> 8u) & 0xffu);
    d.size_y = f32((packed >> 16u) & 0xffu);
    d.start = bitcast<f32>(rec.anim.y);
    let length = bitcast<f32>(rec.anim.z);
    d.rate = bitcast<f32>(rec.anim.w);
    d.num = select(max(d.size_x * d.size_y, 1.0), length, length != 0.0);
    d.full = d.num;
    if ((d.mode & TA_PING_PONG) != 0u) {
        if ((d.mode & TA_SMOOTH) != 0u) {
            d.full = 2.0 * d.num;
        } else if ((d.mode & TA_LOOP) != 0u) {
            d.full = max(2.0 * d.num - 2.0, 1.0);
        } else {
            d.full = max(2.0 * d.num - 1.0, 1.0);
        }
    }
    return d;
}

// LOOP wraps (C fmod), one-shot stops on the last frame.
fn tex_anim_wrap(d: TexAnimDef, raw: f32) -> f32 {
    if ((d.mode & TA_LOOP) != 0u) {
        return raw % d.full;
    }
    return min(d.full - 1.0, raw);
}

// Wrapped frame counter at the current time (tex_anim.rs clock_counter): a
// looping animation wraps the whole seconds before adding the sub-second
// part, so the motion stays smooth after hours.
fn tex_anim_counter(d: TexAnimDef, rec: DrawRecord, clock: vec2<u32>) -> f32 {
    if (d.rate == 0.0) {
        return tex_anim_wrap(d, bitcast<f32>(rec.anim.x));
    }
    let e_ms = bitcast<i32>(clock.x - rec.anim.x);
    let frac = bitcast<f32>(clock.y);
    if (e_ms >= 0) {
        let whole = f32(e_ms / 1000);
        let part = (f32(e_ms % 1000) + frac) * 0.001;
        if ((d.mode & TA_LOOP) != 0u) {
            let full = abs(d.full);
            let a = abs(d.rate);
            let m = ((a * whole) % full + a * part) % full;
            return select(m, -m, d.rate < 0.0);
        }
        return tex_anim_wrap(d, d.rate * whole + d.rate * part);
    }
    return tex_anim_wrap(d, d.rate * (f32(e_ms) + frac) * 0.001);
}

// Animated GL texture coordinates of a face (`st` before any transform).
fn tex_anim_st(st: vec2<f32>, rec: DrawRecord, clock: vec2<u32>) -> vec2<f32> {
    let d = tex_anim_def(rec);
    let smooth_mode = (d.mode & TA_SMOOTH) != 0u;
    var fc = tex_anim_counter(d, rec, clock);
    if (!smooth_mode) {
        fc = floor(fc + 0.01);
        fc = min(d.full - 1.0, fc);
    }
    if ((d.mode & TA_PING_PONG) != 0u && fc >= d.num) {
        fc = select((d.num - 1.99) - (fc - d.num), d.num - (fc - d.num), smooth_mode);
    }
    if ((d.mode & TA_REVERSE) != 0u) {
        fc = select((d.num - 0.99) - fc, d.num - fc, smooth_mode);
    }
    fc += d.start;
    if (!smooth_mode) {
        fc = floor(fc + 0.5);
    }
    let c = rec.anim_xf;
    var sto: vec4<f32>;
    var rot: f32;
    if ((d.mode & TA_ROTATE) != 0u) {
        rot = fc;
        sto = c;
    } else if ((d.mode & TA_SCALE) != 0u) {
        rot = c.z;
        sto = vec4<f32>(fc, fc, c.x, c.y);
    } else {
        rot = c.x;
        if (d.size_x > 0.0 && d.size_y > 0.0) {
            // c.yz = 1 / grid size
            let x_frame = fc % d.size_x;
            let y_frame = trunc(fc / d.size_x);
            sto = vec4<f32>(c.y, c.z, (-0.5 + 0.5 * c.y) + x_frame * c.y, (0.5 - 0.5 * c.z) - y_frame * c.z);
        } else {
            // no grid: offset only, the texture entry's repeats (c.yz)
            sto = vec4<f32>(c.y, c.z, fc, 0.0);
        }
    }
    return ll_xform(st, sto, rot);
}

fn has_tex_anim(rec: DrawRecord) -> bool {
    return (rec.flags.x & FLAG_TEX_ANIM) != 0u;
}

// Diffuse map coordinates of a face: texture entry (or PBR base color)
// transform, after the texture animation if any. An animated legacy face's
// animation already includes its texture entry transform.
fn diffuse_uv(st: vec2<f32>, rec: DrawRecord, clock: vec2<u32>) -> vec2<f32> {
    if (has_tex_anim(rec)) {
        let a = tex_anim_st(st, rec, clock);
        if ((rec.flags.x & FLAG_PBR) != 0u) {
            return pbr_uv(a, rec.uv_st, rec.params.x);
        }
        return vec2<f32>(a.x, 1.0 - a.y);
    }
    return te_uv(st, rec);
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let rec = records[in.instance];
    let model = skinned_model(rec, in);
    let wp = model * vec4<f32>(in.pos, 1.0);
    var out: VsOut;
    out.clip = frame.view_proj * wp;
    out.world_pos = wp.xyz;
    out.normal = cofactor(model) * in.normal.xyz;
    var st = face_st(in, rec);
    let animated = has_tex_anim(rec);
    if (animated) {
        // texture animation first (PBR: before each map's KHR transform,
        // textureUtilV.glsl texture_transform)
        st = tex_anim_st(st, rec, frame.anim_clock.xy);
    }
    if (animated && (rec.flags.x & FLAG_PBR) == 0u) {
        // the animation includes the texture entry transform; normal and
        // specular maps follow the diffuse one (LLFace::getGeometryVolume)
        out.uv = vec2<f32>(st.x, 1.0 - st.y);
    } else {
        out.uv = te_uv(st, rec);
    }
    out.uv_n = out.uv;
    out.uv_s = out.uv;
    out.uv_e = out.uv;
    if ((rec.flags.x & FLAG_LEGACY_MAT) != 0u && !animated) {
        out.uv_n = ll_uv(st, rec.mat_uv, rec.legacy.x);
        out.uv_s = ll_uv(st, rec.spec_uv, rec.legacy.y);
    } else if ((rec.flags.x & FLAG_PBR) != 0u) {
        // one KHR transform per map (LLFetchedGLTFMaterial::bind), packed in
        // the legacy fields: normal, metallic-roughness, emissive
        out.uv_n = pbr_uv(st, rec.mat_uv, rec.legacy.x);
        out.uv_s = pbr_uv(st, rec.spec_uv, rec.legacy.y);
        out.uv_e = pbr_uv(st, rec.spec_color, rec.legacy.z);
    }
    out.record = in.instance;
    out.view_depth = distance(wp.xyz, frame.camera_pos.xyz);
    return out;
}

struct DepthOut {
    @invariant @builtin(position) clip: vec4<f32>,
};

// Depth prepass (opaque geometry and terrain): position only.
@vertex
fn vs_depth(in: VsIn) -> DepthOut {
    let rec = records[in.instance];
    let model = skinned_model(rec, in);
    let wp = model * vec4<f32>(in.pos, 1.0);
    var out: DepthOut;
    out.clip = frame.view_proj * wp;
    return out;
}

// Depth prepass for alpha-masked faces: alpha test only.
@fragment
fn fs_depth_mask(in: VsOut) {
    let rec = records[in.record];
    let a = sample_tex(rec.tex.x, in.uv).a * rec.base_color.a;
    if (a < max(rec.params.w, 0.01)) {
        discard;
    }
}

struct Material {
    color: vec4<f32>,
    n: vec3<f32>,
    metallic: f32,
    roughness: f32,
    emissive: vec3<f32>,
    // legacy material (LLMaterial): specular color (linear) and glossiness,
    // environment reflection intensity
    legacy_spec: vec4<f32>,
    legacy_env: f32,
};

fn eval_material(in: VsOut, front: bool) -> Material {
    let rec = records[in.record];
    var m: Material;
    let base_s = sample_tex(rec.tex.x, in.uv);
    // each map with its own transform (legacy and PBR materials)
    let nrm_s = sample_tex(rec.tex.y, in.uv_n);
    let mr_s = sample_tex(rec.tex.z, in.uv_s);
    let em_s = sample_tex(rec.tex.w, in.uv_e);
    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    if ((rec.flags.x & FLAG_PBR) != 0u) {
        m.color = vec4<f32>(srgb_to_linear(base_s.rgb), base_s.a) * rec.base_color;
    } else {
        // SL multiplies texture and face color in sRGB, then linearizes
        m.color = vec4<f32>(srgb_to_linear(base_s.rgb * rec.base_color.rgb), base_s.a * rec.base_color.a);
    }
    m.n = perturb_normal(n, in.world_pos, in.uv_n, nrm_s.xyz);
    if (rec.tex.y == 1u) {
        m.n = n;
    }
    // glTF: G = roughness, B = metallic
    m.metallic = clamp(rec.params.y * mr_s.b, 0.0, 1.0);
    m.roughness = clamp(rec.params.z * mr_s.g, 0.03, 1.0);
    // TE glow is not emissive: it only feeds the glow pass (fs_glow)
    m.emissive = rec.emissive.rgb * srgb_to_linear(em_s.rgb);
    m.legacy_spec = vec4<f32>(0.0);
    m.legacy_env = 0.0;
    if ((rec.flags.x & FLAG_LEGACY_MAT) != 0u) {
        // LLMaterial as materialF.glsl: specular = specular map x specular
        // color, glossiness = exponent x normal map alpha, environment =
        // intensity x specular map alpha; lit with Blinn-Phong (legacy_light)
        // on a diffuse base
        let gloss = rec.legacy.z * select(nrm_s.a, 1.0, rec.tex.y == 1u);
        m.legacy_spec = vec4<f32>(srgb_to_linear(mr_s.rgb) * rec.spec_color.rgb, gloss);
        m.legacy_env = rec.legacy.w * mr_s.a;
        m.metallic = 0.0;
        m.roughness = 1.0;
    }
    if ((rec.flags.x & FLAG_EMISSIVE_MASK) != 0u) {
        // "emissive mask": the texture alpha says how much of the color glows
        m.emissive += m.color.rgb * base_s.a;
        m.color.a = rec.base_color.a;
    }
    return m;
}

// LLPipeline::createLUTBuffers "lightFunc": normalized Blinn-Phong.
fn ll_light_func(nh: f32, gloss: f32) -> f32 {
    let e = gloss * gloss * 368.0;
    let norm = ((e + 2.0) * (e + 4.0)) / (8.0 * PI * (pow(2.0, -e / 2.0) + e));
    return pow(max(nh, 0.0), e) * norm;
}

// Specular of one light for legacy materials (materialF.glsl).
fn ll_spec(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, gloss: f32) -> f32 {
    let nl = dot(n, l);
    let h = normalize(l + v);
    let nh = dot(n, h);
    if (nl <= 0.0 || nh <= 0.0) {
        return 0.0;
    }
    let nv = max(dot(n, v), 1e-4);
    let vh = max(dot(v, h), 1e-4);
    let lit = min(nl * 6.0, 1.0);
    let fres = pow(1.0 - vh, 5.0) * 0.4 + 0.5;
    let gtdenom = 2.0 * nh;
    let gt = max(0.0, min(gtdenom * nv / vh, gtdenom * nl / vh));
    return lit * fres * ll_light_func(nh, gloss) * gt / (nh * nl);
}

// Legacy material lighting applied to the sun + sky result `col`, as
// softenLightF.glsl: sun highlight, applyGlossEnv and applyLegacyEnv. The
// local light highlights are separate (legacy_local_spec), as LL adds them
// in another pass (multiPointLightF).
fn legacy_light(col_in: vec3<f32>, spec: vec4<f32>, env: f32, n: vec3<f32>, v: vec3<f32>, world_pos: vec3<f32>, shadow: f32,
                sun_lin: vec3<f32>, sky_scale: f32) -> vec3<f32> {
    var col = col_in;
    let r = reflect(-v, n);
    let sky = sky_color(vec3<f32>(r.x, r.y, max(r.z, -0.2))) * sky_scale * (0.35 + 0.65 * shadow);
    // sampleReflectionProbesLegacy: glossy environment at the glossiness mip,
    // legacy environment at the sharpest one
    var gloss_sky = sky;
    var env_sky = sky;
    if (probes_on()) {
        let g = sample_probes(world_pos, r, (1.0 - spec.a) * probe_max_lod());
        if (g.w > 0.0) {
            gloss_sky = g.rgb;
        }
        if (env > 0.0) {
            let e = sample_probes(world_pos, r, 0.0);
            if (e.w > 0.0) {
                env_sky = e.rgb;
            }
        }
    }
    if (spec.a > 0.0) {
        let l = normalize(frame.sun_dir.xyz);
        col += ll_spec(n, v, l, spec.a) * shadow * sun_lin * spec.rgb;
        // applyGlossEnv
        var fresnel = clamp(1.0 - dot(v, n), 0.3, 1.0);
        fresnel = fresnel * fresnel * spec.a;
        let glossenv = gloss_sky * 0.5 * spec.rgb * fresnel * max(vec3<f32>(1.0) - col, vec3<f32>(0.0));
        col += glossenv * 0.5;
    }
    if (env > 0.0) {
        // applyLegacyEnv
        var fresnel = 1.0 - dot(v, n);
        fresnel = min(fresnel * fresnel + env, 1.0);
        col = mix(col, env_sky * env * fresnel * 0.5, env);
    }
    return col;
}

// Local light highlights of legacy materials (multiPointLightF). Our light
// colors are PBR radiances (diffuse divided by pi): SL's classic light
// colors are those over pi.
fn legacy_local_spec(spec: vec4<f32>, n: vec3<f32>, v: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    var col = vec3<f32>(0.0);
    if (spec.a <= 0.0) {
        return col;
    }
    let count = u32(frame.params.w);
    for (var i = 0u; i < count; i++) {
        let lt = frame.lights[i];
        let to_l = lt.pos_radius.xyz - world_pos;
        let dist = length(to_l);
        let radius = lt.pos_radius.w;
        if (dist < radius) {
            let atten = ll_light_atten(dist / radius, lt.color_falloff.w);
            let s = ll_spec(n, v, to_l / max(dist, 1e-4), spec.a) * atten;
            col += clamp(s * lt.color_falloff.rgb / 3.25 * spec.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
        }
    }
    return col;
}

struct Shaded {
    color: vec4<f32>,
    n: vec3<f32>,
    roughness: f32,
    metallic: f32,
};

fn shade(in: VsOut, front: bool) -> Shaded {
    let rec = records[in.record];
    let m = eval_material(in, front);
    var col: vec3<f32>;
    let classic = is_classic_sky() && !camera_underwater();
    var hazed = false;
    if ((rec.flags.x & FLAG_FULLBRIGHT) != 0u) {
        col = m.color.rgb + m.emissive;
        // opaque fullbright faces are not hazed on classic skies
        hazed = classic;
    } else if (classic) {
        let v = normalize(frame.camera_pos.xyz - in.world_pos);
        let sh = shadow_factor(in.world_pos, m.n, in.view_depth);
        let a = ll_atmos(in.world_pos - frame.camera_pos.xyz);
        // softenLightF: (sun + sky + specular + environment + emissive) x
        // final_scale, then the local lights pass, then hazeF
        if ((rec.flags.x & FLAG_PBR) != 0u) {
            col = ll_classic_pbr(m.color.rgb, m.metallic, m.roughness, m.n, v, in.world_pos, sh, in.clip.xy, a);
            col = (col + m.emissive) * ll_final_scale()
                + ll_classic_pbr_local(m.color.rgb, m.metallic, m.roughness, m.n, v, in.world_pos);
        } else {
            col = ll_classic_sun_ambient(m.color.rgb, m.n, sh, in.clip.xy, a);
            if (m.legacy_spec.a > 0.0 || m.legacy_env > 0.0) {
                col = legacy_light(col, m.legacy_spec, m.legacy_env, m.n, v, in.world_pos, sh, ll_sun_linear(a), 1.0);
            }
            col = (col + m.emissive) * ll_final_scale() + ll_local_diffuse(m.color.rgb, m.n, in.world_pos)
                + legacy_local_spec(m.legacy_spec, m.n, v, in.world_pos) * ll_local_scale();
        }
        col = ll_haze(col, a);
        hazed = true;
    } else {
        let v = normalize(frame.camera_pos.xyz - in.world_pos);
        let sh = shadow_factor(in.world_pos, m.n, in.view_depth);
        col = shade_pbr(m.color.rgb, m.metallic, m.roughness, m.n, v, in.world_pos, sh, in.clip.xy) + m.emissive;
        if (m.legacy_spec.a > 0.0 || m.legacy_env > 0.0) {
            col = legacy_light(col - m.emissive, m.legacy_spec, m.legacy_env, m.n, v, in.world_pos, sh, frame.sun_color.rgb / PI, 1.0 / PI)
                + legacy_local_spec(m.legacy_spec, m.n, v, in.world_pos) + m.emissive;
        }
    }
    if ((rec.flags.x & FLAG_HIGHLIGHT) != 0u) {
        col = mix(col, vec3<f32>(0.55, 0.36, 0.96), 0.35);
    }
    if (!hazed) {
        col = apply_fog(col, in.world_pos);
    }
    var s: Shaded;
    s.color = vec4<f32>(col, m.color.a);
    s.n = m.n;
    s.roughness = m.roughness;
    s.metallic = m.metallic;
    if ((rec.flags.x & FLAG_FULLBRIGHT) != 0u) {
        s.roughness = 1.0;
    }
    return s;
}

// Octahedral normal encoding into [0, 1]^2.
fn oct_encode(n: vec3<f32>) -> vec2<f32> {
    let p = n.xy / (abs(n.x) + abs(n.y) + abs(n.z));
    var e = p;
    if (n.z < 0.0) {
        e = (vec2<f32>(1.0) - abs(p.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0));
    }
    return e * 0.5 + vec2<f32>(0.5);
}

fn gbuffer(s: Shaded) -> vec4<f32> {
    return vec4<f32>(oct_encode(normalize(s.n)), s.roughness, s.metallic);
}

fn mask_color(rec: DrawRecord, c: vec4<f32>) -> vec4<f32> {
    let cutoff = max(rec.params.w, 0.01);
    if (!ALPHA_TO_COVERAGE) {
        if (c.a < cutoff) {
            discard;
        }
        return vec4<f32>(c.rgb, 1.0);
    }
    // sharpen alpha around the cutoff for alpha-to-coverage
    let a = clamp((c.a - cutoff) / max(fwidth(c.a), 1e-4) + 0.5, 0.0, 1.0);
    return vec4<f32>(c.rgb, a);
}

@fragment
fn fs_opaque(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let c = shade(in, front);
    return vec4<f32>(c.color.rgb, 1.0);
}

@fragment
fn fs_mask(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let rec = records[in.record];
    return mask_color(rec, shade(in, front).color);
}

struct GOut {
    @location(0) color: vec4<f32>,
    @location(1) gbuf: vec4<f32>,
};

// Variants writing normal / roughness / metallic for screen-space reflections.
@fragment
fn fs_opaque_g(in: VsOut, @builtin(front_facing) front: bool) -> GOut {
    let s = shade(in, front);
    var o: GOut;
    o.color = vec4<f32>(s.color.rgb, 1.0);
    o.gbuf = gbuffer(s);
    return o;
}

@fragment
fn fs_mask_g(in: VsOut, @builtin(front_facing) front: bool) -> GOut {
    let rec = records[in.record];
    let s = shade(in, front);
    var o: GOut;
    o.color = mask_color(rec, s.color);
    o.gbuf = gbuffer(s);
    return o;
}

// Planar reflection passes (water, mirror): clip plane + alpha test.
@fragment
fn fs_reflect(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let rec = records[in.record];
    let s = shade(in, front);
    let masked = (rec.flags.x & FLAG_ALPHA_MASK) != 0u && s.color.a < max(rec.params.w, 0.01);
    if (reflection_clip(in.world_pos) || masked) {
        discard;
    }
    return vec4<f32>(s.color.rgb, 1.0);
}

@fragment
fn fs_blend(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let c = shade(in, front).color;
    if (c.a < 0.004) {
        discard;
    }
    // premultiplied alpha
    return vec4<f32>(c.rgb * c.a, c.a);
}

// Glow amount into the scene alpha (additive, alpha channel only), drawn
// for faces with glow after the scene: emissiveF.glsl (texture alpha x TE
// glow, the face alpha is ignored: invisible faces still glow) and
// pbrglowF.glsl (max emissive component x glow).
@fragment
fn fs_glow(in: VsOut) -> @location(0) vec4<f32> {
    let rec = records[in.record];
    let base = sample_tex(rec.tex.x, in.uv);
    var a: f32;
    if ((rec.flags.x & FLAG_PBR) != 0u) {
        if ((rec.flags.x & FLAG_ALPHA_MASK) != 0u && base.a * rec.base_color.a < max(rec.params.w, 0.01)) {
            discard;
        }
        let e = rec.emissive.rgb * srgb_to_linear(sample_tex(rec.tex.w, in.uv_e).rgb);
        a = max(e.r, max(e.g, e.b)) * rec.emissive.w;
    } else {
        a = base.a * rec.emissive.w;
    }
    if (a <= 0.0) {
        discard;
    }
    return vec4<f32>(0.0, 0.0, 0.0, a);
}

// Debug overlays (« Afficher la transparence », fil de fer): one flat
// translucent color per pipeline.
override debug_r: f32 = 1.0;
override debug_g: f32 = 0.0;
override debug_b: f32 = 0.0;
override debug_a: f32 = 0.35;

@fragment
fn fs_debug_flat(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(debug_r, debug_g, debug_b, debug_a);
}

// Glow suppression of a face LL draws in its alpha pool: the glow under it
// is scaled by (1 - its alpha) before it adds its own (fs_glow).
@fragment
fn fs_glow_suppress(in: VsOut) -> @location(0) vec4<f32> {
    let rec = records[in.record];
    let a = sample_tex(rec.tex.x, in.uv).a * rec.base_color.a;
    if (a <= 0.0) {
        discard;
    }
    return vec4<f32>(0.0, 0.0, 0.0, a);
}

// ------------------------------------------------------------- shadow pass

struct ShadowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) record: u32,
};

struct ShadowParams {
    vp: mat4x4<f32>,
    anim_clock: vec4<u32>,  // texture animation clock, as Frame's
};

@group(3) @binding(0) var<uniform> shadow_params: ShadowParams;

@vertex
fn vs_shadow(in: VsIn) -> ShadowOut {
    let rec = records[in.instance];
    let wp = skinned_model(rec, in) * vec4<f32>(in.pos, 1.0);
    var out: ShadowOut;
    out.clip = shadow_params.vp * wp;
    out.uv = diffuse_uv(face_st(in, rec), rec, shadow_params.anim_clock.xy);
    out.record = in.instance;
    return out;
}

// LLDrawPoolAvatar::renderShadow / deferred/avatarAlphaShadowF.glsl (LGPL 2.1):
// transparent texels cast no shadow; semi-transparent texels use alternating
// shadow-map columns. Opaque material modes ignore the texture alpha.
@fragment
fn fs_shadow(in: ShadowOut) {
    let rec = records[in.record];
    let alpha = sample_tex(rec.tex.x, in.uv).a * rec.base_color.a;
    if ((rec.flags.x & (FLAG_ALPHA_MASK | FLAG_ALPHA_BLEND)) != 0u) {
        if ((rec.flags.x & FLAG_ALPHA_MASK) != 0u) {
            if (alpha < rec.params.w) { discard; }
        } else {
            if (alpha < 0.05) { discard; }
            let cutoff = select(0.598, 0.88, (rec.flags.x & FLAG_PBR) != 0u);
            if (alpha < cutoff && (u32(in.clip.x) % 2u) == 0u) { discard; }
        }
    }
}

// --------------------------------------------------------------- particles

const PART_EMISSIVE: u32 = 1u;
const PART_ADDITIVE: u32 = 2u;
const PART_AXIS: u32 = 4u;

struct PartIn {
    @location(0) pos: vec3<f32>,
    @location(1) texture: u32,
    @location(2) size: vec2<f32>,
    @location(3) flags: u32,
    @location(4) color: vec4<f32>,
    @location(5) axis: vec3<f32>,
    @location(6) glow: f32,
    @builtin(vertex_index) vi: u32,
};

struct PartOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) texture: u32,
    @location(3) @interpolate(flat) flags: u32,
    @location(4) world_pos: vec3<f32>,
};

@vertex
fn vs_particle(in: PartIn) -> PartOut {
    // two triangles: corners (-1,-1) (1,-1) (1,1) / (-1,-1) (1,1) (-1,1)
    let cx = array<f32, 6>(-1.0, 1.0, 1.0, -1.0, 1.0, -1.0);
    let cy = array<f32, 6>(-1.0, -1.0, 1.0, -1.0, 1.0, 1.0);
    let c = vec2<f32>(cx[in.vi % 6u], cy[in.vi % 6u]);
    var right = frame.cam_right.xyz;
    var up = frame.cam_up.xyz;
    var offset: vec3<f32>;
    if ((in.flags & PART_AXIS) != 0u && dot(in.axis, in.axis) > 1e-8) {
        // stretched along the axis (ribbon segment / velocity), facing the camera
        let to_cam = normalize(frame.camera_pos.xyz - in.pos);
        up = in.axis * 0.5;
        let side = cross(to_cam, normalize(in.axis));
        right = select(frame.cam_right.xyz, normalize(side), dot(side, side) > 1e-6);
        offset = right * c.x * in.size.x * 0.5 + up * c.y;
    } else {
        offset = right * c.x * in.size.x * 0.5 + up * c.y * in.size.y * 0.5;
    }
    let wp = in.pos + offset;
    var out: PartOut;
    out.clip = frame.view_proj * vec4<f32>(wp, 1.0);
    out.uv = vec2<f32>(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5);
    out.color = vec4<f32>(srgb_to_linear(in.color.rgb), in.color.a);
    // glow brightens the particle (no bloom pass)
    out.color = vec4<f32>(out.color.rgb * (1.0 + in.glow * 2.0), out.color.a);
    out.texture = in.texture;
    out.flags = in.flags;
    out.world_pos = wp;
    return out;
}

@fragment
fn fs_particle(in: PartOut) -> @location(0) vec4<f32> {
    // slot 0 is white: the default particle is a soft round dot
    let ts = sample_tex(in.texture, in.uv);
    var tex = vec4<f32>(srgb_to_linear(ts.rgb), ts.a);
    if (in.texture == 0u) {
        let d = length(in.uv * 2.0 - vec2<f32>(1.0));
        let a = clamp(1.0 - d, 0.0, 1.0);
        tex = vec4<f32>(1.0, 1.0, 1.0, a * a);
    }
    var col = tex.rgb * in.color.rgb;
    var alpha = tex.a * in.color.a;
    if ((in.flags & PART_EMISSIVE) == 0u) {
        // lit by the environment: ambient plus a share of the sun
        let light = frame.sky_zenith.rgb * 0.6 + frame.sky_horizon.rgb * 0.4
                  + frame.sun_color.rgb * max(frame.sun_dir.z, 0.0) * 0.25;
        col *= light;
    }
    // soft particles: fade where they cut into the scene
    let d = scene_depth_at(screen_uv(in.clip.xy));
    if (d > 0.0) {
        let sp = world_from_depth(screen_uv(in.clip.xy), d);
        let gap = distance(sp, frame.camera_pos.xyz) - distance(in.world_pos, frame.camera_pos.xyz);
        alpha *= clamp(gap / max(frame.cam_right.w, 0.01), 0.0, 1.0);
    }
    col = apply_fog(col, in.world_pos);
    if (alpha < 0.002) {
        discard;
    }
    if ((in.flags & PART_ADDITIVE) != 0u) {
        // premultiplied blend with zero alpha = additive
        return vec4<f32>(col * alpha, 0.0);
    }
    return vec4<f32>(col * alpha, alpha);
}

// ------------------------------------------------------------- impostors
// Avatars beyond RenderAvatarMaxNonImpostors are drawn as a card showing
// their picture, refreshed now and then (LLVOAvatar impostors). The pictures
// live in an atlas of square tiles.

@group(3) @binding(4) var impostor_atlas: texture_2d<f32>;
@group(3) @binding(5) var impostor_sampler: sampler;

override impostor_tiles_per_row: f32 = 8.0;

struct SpriteIn {
    @location(0) center: vec3<f32>,
    @location(1) tile: u32,
    @location(2) right: vec3<f32>,
    @location(3) up: vec3<f32>,
    @builtin(vertex_index) vid: u32,
};

struct SpriteOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world_pos: vec3<f32>,
    @location(2) normal: vec3<f32>,
};

@vertex
fn vs_impostor(in: SpriteIn) -> SpriteOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let c = corners[in.vid % 6u];
    let p = in.center + in.right * c.x + in.up * c.y;
    let per = u32(impostor_tiles_per_row);
    let origin = vec2<f32>(f32(in.tile % per), f32(in.tile / per));
    var out: SpriteOut;
    out.clip = frame.view_proj * vec4<f32>(p, 1.0);
    out.uv = (origin + vec2<f32>(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5)) / impostor_tiles_per_row;
    out.world_pos = p;
    out.normal = normalize(cross(in.right, in.up));
    return out;
}

fn impostor_color(in: SpriteOut) -> vec4<f32> {
    let t = textureSample(impostor_atlas, impostor_sampler, in.uv);
    if (t.a < 0.5) {
        discard;
    }
    // the picture was taken lit and unfogged: fog it at its real distance
    return vec4<f32>(apply_fog(t.rgb / max(t.a, 1e-3), in.world_pos), 1.0);
}

@fragment
fn fs_impostor(in: SpriteOut) -> @location(0) vec4<f32> {
    return impostor_color(in);
}

@fragment
fn fs_impostor_g(in: SpriteOut) -> GOut {
    var o: GOut;
    o.color = impostor_color(in);
    // matte, not metallic: no screen-space reflection on the card
    o.gbuf = vec4<f32>(oct_encode(in.normal), 1.0, 0.0);
    return o;
}

// Picture of an impostor: opaque and alpha-masked faces (alpha 1 = covered).
@fragment
fn fs_impostor_capture(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let rec = records[in.record];
    let s = shade(in, front);
    if ((rec.flags.x & FLAG_ALPHA_MASK) != 0u && s.color.a < max(rec.params.w, 0.01)) {
        discard;
    }
    return vec4<f32>(s.color.rgb, 1.0);
}

// Blended faces (hair, lace): kept where mostly opaque.
@fragment
fn fs_impostor_capture_blend(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let s = shade(in, front);
    if (s.color.a < 0.5) {
        discard;
    }
    return vec4<f32>(s.color.rgb, 1.0);
}
