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
    // legacy material normal / specular map coordinates
    @location(5) uv_n: vec2<f32>,
    @location(6) uv_s: vec2<f32>,
};

fn cofactor(m: mat4x4<f32>) -> mat3x3<f32> {
    let a = m[0].xyz;
    let b = m[1].xyz;
    let c = m[2].xyz;
    return mat3x3<f32>(cross(b, c), cross(c, a), cross(a, b));
}

// LL texture-entry transform about the face center (LLFace xform), then GL->wgpu V flip.
// `st_rot`: scale s, t, offset s, t and rotation (diffuse, or a legacy
// material's normal / specular map).
fn ll_uv(uv: vec2<f32>, flags: u32, pos: vec3<f32>, sto: vec4<f32>, rot: f32) -> vec2<f32> {
    var st = uv;
    if ((flags & FLAG_PLANAR) != 0u) {
        // planar mapping: project object-space position on the dominant plane
        st = vec2<f32>(pos.y + 0.5, pos.z + 0.5);
    }
    var s = st.x - 0.5;
    var t = st.y - 0.5;
    let ca = cos(rot);
    let sa = sin(rot);
    let tmp = s;
    s = s * ca + t * sa;
    t = -tmp * sa + t * ca;
    s = s * sto.x + sto.z + 0.5;
    t = t * sto.y + sto.w + 0.5;
    return vec2<f32>(s, 1.0 - t);
}

fn te_uv(uv: vec2<f32>, rec: DrawRecord, pos: vec3<f32>) -> vec2<f32> {
    if ((rec.flags.x & FLAG_PBR) != 0u) {
        var st = uv;
        if ((rec.flags.x & FLAG_PLANAR) != 0u) {
            st = vec2<f32>(pos.y + 0.5, pos.z + 0.5);
        }
        // textureUtilV.glsl texture_transform: SL (GL) t -> glTF v = 1 - t,
        // then KHR_texture_transform (offset, rotation, scale). LL flips back
        // to GL; our top-left-origin textures are sampled in glTF space directly.
        st.y = 1.0 - st.y;
        let s = rec.uv_st.xy;
        let o = rec.uv_st.zw;
        let r = rec.params.x;
        let c = cos(r);
        let sn = sin(r);
        let v = vec2<f32>(st.x * s.x, st.y * s.y);
        let rv = vec2<f32>(c * v.x + sn * v.y, -sn * v.x + c * v.y);
        st = rv + o;
        return vec2<f32>(st.x, st.y);
    }
    return ll_uv(uv, rec.flags.x, pos, rec.uv_st, rec.params.x);
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
    out.uv = te_uv(in.uv, rec, in.pos);
    out.uv_n = out.uv;
    out.uv_s = out.uv;
    if ((rec.flags.x & FLAG_LEGACY_MAT) != 0u) {
        out.uv_n = ll_uv(in.uv, rec.flags.x, in.pos, rec.mat_uv, rec.legacy.x);
        out.uv_s = ll_uv(in.uv, rec.flags.x, in.pos, rec.spec_uv, rec.legacy.y);
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
    // uv_n / uv_s equal uv except for legacy materials (own transforms)
    let nrm_s = sample_tex(rec.tex.y, in.uv_n);
    let mr_s = sample_tex(rec.tex.z, in.uv_s);
    let em_s = sample_tex(rec.tex.w, in.uv);
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

// Legacy material lighting applied to the diffuse result `col`, as
// materialF.glsl / softenLightF.glsl: sun and local light highlights,
// applyGlossEnv and applyLegacyEnv. Our light colors are PBR radiances
// (diffuse divided by pi): SL's classic light colors are those over pi.
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
        if ((rec.flags.x & FLAG_PBR) != 0u) {
            col = ll_classic_pbr(m.color.rgb, m.metallic, m.roughness, m.n, v, in.world_pos, sh, in.clip.xy, a);
        } else {
            col = ll_classic_diffuse(m.color.rgb, m.n, in.world_pos, sh, in.clip.xy, a);
            if (m.legacy_spec.a > 0.0 || m.legacy_env > 0.0) {
                col = legacy_light(col, m.legacy_spec, m.legacy_env, m.n, v, in.world_pos, sh, ll_sun_linear(a), 1.0);
            }
        }
        col = ll_haze(col + m.emissive, a);
        hazed = true;
    } else {
        let v = normalize(frame.camera_pos.xyz - in.world_pos);
        let sh = shadow_factor(in.world_pos, m.n, in.view_depth);
        col = shade_pbr(m.color.rgb, m.metallic, m.roughness, m.n, v, in.world_pos, sh, in.clip.xy) + m.emissive;
        if (m.legacy_spec.a > 0.0 || m.legacy_env > 0.0) {
            col = legacy_light(col - m.emissive, m.legacy_spec, m.legacy_env, m.n, v, in.world_pos, sh, frame.sun_color.rgb / PI, 1.0 / PI) + m.emissive;
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
        let e = rec.emissive.rgb * srgb_to_linear(sample_tex(rec.tex.w, in.uv).rgb);
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
};

struct ShadowParams {
    vp: mat4x4<f32>,
};

@group(3) @binding(0) var<uniform> shadow_params: ShadowParams;

@vertex
fn vs_shadow(in: VsIn) -> ShadowOut {
    let rec = records[in.instance];
    let wp = skinned_model(rec, in) * vec4<f32>(in.pos, 1.0);
    var out: ShadowOut;
    out.clip = shadow_params.vp * wp;
    return out;
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
