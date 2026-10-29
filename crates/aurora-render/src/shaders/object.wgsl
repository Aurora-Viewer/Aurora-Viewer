// Object (prim / mesh / avatar) shading. Concatenated after common.wgsl.

// Alpha-masked faces use alpha-to-coverage with MSAA, an alpha test otherwise.
override ALPHA_TO_COVERAGE: bool = true;

// Evaluated after skinning/attachment placement, in every surface/depth pass.
// Outside the actual ellipse, even geometry beyond the plane stays visible.
fn portal_clip(rec: DrawRecord, position: vec3<f32>) {
    if ((rec.flags.x & FLAG_PORTAL_CLIP) != 0u) {
        let code = rec.portal_clip.w;
        let y = select(code, sign(code) * (2.0 - abs(code)), abs(code) > 1.0);
        let normal = normalize(vec2<f32>(1.0 - abs(code), y));
        let local = position - rec.portal_clip.xyz;
        if (dot(local.xy, normal) < -0.0001) {
            let across = dot(local.xy, vec2<f32>(-normal.y, normal.x)) / PORTAL_HALF_WIDTH;
            let height = local.z / PORTAL_HALF_HEIGHT;
            if (across * across + height * height <= 1.0) { discard; }
        }
    }
}

// One analytically bounded Aurora surface, animated by the GPU clock. The
// navy/indigo depth, violet currents and teal rim come from the resolved theme
// in the record, not a baked image. Its outer corona is purely decorative.
struct PortalSurface { color: vec3<f32>, alpha: f32, glow: f32 };

fn portal_hash(p0: vec2<f32>) -> f32 {
    var p = fract(p0 * vec2<f32>(123.34, 456.21));
    p += vec2<f32>(dot(p, p + vec2<f32>(45.32)));
    return fract(p.x * p.y);
}

fn portal_noise(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let f = fract(p);
    let u = f * f * (vec2<f32>(3.0) - 2.0 * f);
    return mix(mix(portal_hash(cell), portal_hash(cell + vec2<f32>(1.0, 0.0)), u.x),
        mix(portal_hash(cell + vec2<f32>(0.0, 1.0)), portal_hash(cell + vec2<f32>(1.0)), u.x), u.y);
}

fn portal_surface(rec: DrawRecord, uv: vec2<f32>, aa: f32) -> PortalSurface {
    let q = (uv * 2.0 - vec2<f32>(1.0)) * PORTAL_SURFACE_SCALE;
    let r = length(q);
    let angle = atan2(q.y, q.x);
    let time = f32(frame.anim_clock.x) * 0.001 * rec.params.y;
    let turn = r * 2.1 - time * 0.09;
    let flow = mat2x2<f32>(vec2<f32>(cos(turn), sin(turn)), vec2<f32>(-sin(turn), cos(turn))) * q;
    let drift = vec2<f32>(time * 0.045, -time * 0.028);
    let cloud = portal_noise(flow * 3.4 + drift) * 0.55
        + portal_noise(flow * 7.3 - drift * 1.5) * 0.3
        + portal_noise(flow * 15.9 + drift * 0.6) * 0.15;
    let depth = smoothstep(0.08, 0.9, r);
    let mist = smoothstep(0.2, 0.88, cloud);
    let current = pow(0.5 + 0.5 * sin(angle * 3.0 + r * 10.0 + cloud * 4.0 - time * 0.8), 8.0) * depth;
    let braid = pow(0.5 + 0.5 * sin(angle * 5.0 - r * 14.0 + cloud * 5.0 + time * 0.5), 14.0) * depth;
    let violet = srgb_to_linear(rec.base_color.rgb);
    let teal = srgb_to_linear(rec.emissive.rgb);
    let indigo = srgb_to_linear(rec.spec_color.rgb);
    let navy = srgb_to_linear(rec.legacy.rgb);
    let energy = rec.params.z;
    var core = navy * (0.7 + 2.0 * mist) + mix(indigo, violet, cloud) * (0.035 + mist * 0.22);
    core += (violet * current * 0.34 + teal * braid * 0.095) * energy;
    let star_cell = floor(q * 42.0);
    let star_distance = length(fract(q * 42.0) - vec2<f32>(0.5));
    let stars = step(0.992, portal_hash(star_cell)) * (1.0 - smoothstep(0.025, 0.12 + aa * 42.0, star_distance))
        * (0.55 + 0.45 * sin(time * 1.3 + portal_hash(star_cell) * 37.0));
    core += mix(indigo, teal, 0.45) * stars * 1.2 * (1.0 - smoothstep(0.7, 0.96, r)) * energy;
    let ripple = sin(angle * 11.0 - time * 1.7 + cloud * 3.0) * 0.003;
    let rim = exp(-pow((r - 1.0 - ripple) / max(0.006, aa * 0.8), 2.0));
    let inner = exp(-pow((r - 0.967) / 0.014, 2.0)) * 0.3;
    let corona = exp(-pow((r - 1.0) / 0.075, 2.0)) * (1.0 - smoothstep(1.08, PORTAL_SURFACE_SCALE, r));
    let arcs = pow(0.5 + 0.5 * sin(angle * 5.0 + time * 0.9 + cloud * 2.0), 12.0)
        * exp(-pow((r - 1.035) / 0.024, 2.0));
    let rim_color = mix(violet, teal, 0.5 + 0.5 * sin(angle * 2.0 - time * 0.6));
    let coverage = 1.0 - smoothstep(1.0 - aa, 1.0 + aa, r);
    let color = mix(indigo * 0.12, core, coverage)
        + rim_color * (rim * 0.85 + inner + corona * 0.24 + arcs * 0.6) * energy;
    let alpha = clamp(coverage + corona * 0.32 + rim * 0.7 + arcs * 0.25, 0.0, 1.0);
    return PortalSurface(color, alpha, clamp(rim * 0.22 + inner * 0.12 + corona * 0.04 + arcs * 0.08, 0.0, 1.0) * energy);
}

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
    @builtin(instance_index) instance: u32,
    @builtin(vertex_index) vertex: u32,
};

// Live morphs of the system avatar body (lip sync, body physics, facial
// expressions, hand poses) share one static, sparse delta bank in the
// palette buffer, read as vectors: per vertex of a part, (first entry,
// entry count); per entry, (position delta, weight column) then (normal
// delta, 0). anim.xyz holds the geometry's first vertex, the vector of its
// first vertex in the bank and the vertex count; anim.w the column that is
// not zero while one of the part's weights is not: a part at rest (most of
// the time) costs one read. The weights are the skeleton's
// (LIVE_MORPH_WEIGHTS).
fn palette_vector(i: u32) -> vec4<f32> { return palettes[i >> 2u][i & 3u]; }

fn live_weight(rec: DrawRecord, k: u32) -> f32 {
    return palettes[rec.flags.y + LIVE_MORPH_WEIGHTS + (k >> 4u)][(k >> 2u) & 3u][k & 3u];
}

fn lip_delta(rec: DrawRecord, in: VsIn) -> mat2x3<f32> {
    var result = mat2x3<f32>(vec3<f32>(0.0), vec3<f32>(0.0));
    if ((rec.flags.x & FLAG_LIP_SYNC) != 0u && in.vertex >= rec.anim.x && in.vertex - rec.anim.x < rec.anim.z
        && live_weight(rec, rec.anim.w) != 0.0) {
        let head = palette_vector(rec.anim.y + in.vertex - rec.anim.x);
        let first = u32(head.x);
        // (a bound on the bank's own data: no part has more live morphs)
        let count = min(u32(head.y), 64u);
        for (var i = 0u; i < count; i++) {
            let delta = palette_vector(first + 2u * i);
            let w = live_weight(rec, u32(delta.w));
            if (w != 0.0) {
                result[0] += delta.xyz * w;
                result[1] += palette_vector(first + 2u * i + 1u).xyz * w;
            }
        }
    }
    return result;
}

fn vertex_position(rec: DrawRecord, in: VsIn) -> vec3<f32> { return in.pos + lip_delta(rec, in)[0]; }

// Terrain reuses the unskinned parallel stream for two octahedral snorm16
// directions. See pack_terrain_frame: no extra buffer or vertex stride.
fn terrain_direction(bytes: vec4<u32>) -> vec3<f32> {
    let bits = bytes.x | (bytes.y << 8u) | (bytes.z << 16u) | (bytes.w << 24u);
    let p = unpack2x16snorm(bits);
    var n = vec3<f32>(p, 1.0 - abs(p.x) - abs(p.y));
    if (n.z < 0.0) {
        n = vec3<f32>((vec2<f32>(1.0) - abs(p.yx)) * select(vec2<f32>(1.0), vec2<f32>(-1.0), p < vec2<f32>(0.0)), n.z);
    }
    return normalize(n);
}

fn vertex_normal(rec: DrawRecord, in: VsIn) -> vec3<f32> {
    if ((rec.flags.x & FLAG_TERRAIN) != 0u) { return terrain_direction(in.joints); }
    return in.normal.xyz + lip_delta(rec, in)[1];
}

// The tangent packed in the normal's w (aurora_render::pack_tangent): an
// angle in 127 steps around the normal, from the axis pair of
// tangent_basis (Duff et al. 2017), the handedness as its sign. Zero
// without one.
fn vertex_tangent(in: VsIn) -> vec4<f32> {
    let code = round(in.normal.w * 127.0);
    if (code == 0.0) {
        return vec4<f32>(0.0);
    }
    let n = normalize(in.normal.xyz);
    let s = select(-1.0, 1.0, n.z >= 0.0);
    let a = -1.0 / (s + n.z);
    let b = n.x * n.y * a;
    let b1 = vec3<f32>(1.0 + s * n.x * n.x * a, s * b, -s * n.x);
    let b2 = vec3<f32>(b, s + n.y * n.y * a, -n.y);
    let angle = (abs(code) - 1.0) * (6.2831853 / 127.0);
    return vec4<f32>(b1 * cos(angle) + b2 * sin(angle), sign(code));
}

// textureUtilV.glsl tangent_space_transform, without the texture
// animation (an animated PBR face keeps the per-pixel frame, derived from
// its animated coordinates): the tangent turned back by the normal map's
// KHR rotation and flipped with the signs of its scale.
fn tangent_space_transform(t: vec3<f32>, w: f32, n: vec3<f32>, scale: vec2<f32>, rotation: f32) -> vec4<f32> {
    let scale_sign = sign(scale);
    var weights = vec2<f32>(cos(rotation), sin(rotation)) * scale_sign;
    weights.x += 1.0 - abs(sign(sign(weights.x) + 0.5 * sign(weights.y)));
    weights = normalize(weights);
    weights.y = -weights.y;
    let b = w * cross(n, t);
    return vec4<f32>(weights.x * t + weights.y * b, w * scale_sign.x * scale_sign.y);
}

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

// `m` with its translation counted from `eye`.
fn from_eye(m: mat4x4<f32>, eye: vec3<f32>) -> mat4x4<f32> {
    var r = m;
    r[3] = vec4<f32>(r[3].xyz - eye, r[3].w);
    return r;
}

// Matrix of a vertex, placed relative to `eye`. What an avatar wears
// follows it without its records being written again: the scene keeps,
// beside each joint palette, the matrix that places the skeleton and the
// posed attachment points (rigid world matrices). A rigged mesh reads the
// first, a worn prim holds its placement relative to its point. The eye is
// taken away from the translation of the matrix that holds the place in
// the world (the record's, or its anchor's) before anything is multiplied.
fn placed_model(rec: DrawRecord, in: VsIn, eye: vec3<f32>) -> mat4x4<f32> {
    if ((rec.flags.x & FLAG_SKINNED) != 0u) {
        let skin = skin_matrix(rec, in.joints, in.weights);
        if ((rec.flags.x & FLAG_ANCHORED) != 0u) {
            return from_eye(palettes[rec.flags.y + SKELETON_ANCHOR], eye) * skin;
        }
        return from_eye(rec.model, eye) * skin;
    }
    if ((rec.flags.x & FLAG_ANCHORED) != 0u) {
        return from_eye(palettes[rec.flags.y], eye) * rec.model;
    }
    return from_eye(rec.model, eye);
}

// World matrix of a vertex.
fn skinned_model(rec: DrawRecord, in: VsIn) -> mat4x4<f32> {
    return placed_model(rec, in, vec3<f32>(0.0));
}

// The same, placed relative to the frame's eye, so the skinning, the vertex
// and the view work on distances from the camera. In region coordinates,
// single precision is too coarse high above the ground (0.24 mm steps above
// 2048 m: up to a pixel on close objects, different whenever the view
// changes).
fn eye_model(rec: DrawRecord, in: VsIn) -> mat4x4<f32> {
    return placed_model(rec, in, frame.eye.xyz);
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
    // coordinates the tangent frame of the normal map is derived from
    @location(8) uv_t: vec2<f32>,
    // the vertices' tangent frame (materialV.glsl vary_tangent, vary_sign):
    // zero for the per-pixel one
    @location(9) tangent: vec3<f32>,
    @location(10) @interpolate(flat) tangent_sign: f32,
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
        let st = planar_st(in.pos, in.normal.xyz, rec.model);
        // Local HUD zoom changes the screen size, never the texture repeats.
        let packed_zoom = rec.flags.x >> 16u;
        if (packed_zoom == 0u) { return st; }
        let zoom = f32(packed_zoom) / 4096.0;
        return (st - vec2<f32>(0.5)) / zoom + vec2<f32>(0.5);
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
// ms, sub-ms fraction bits; Frame / ShadowParams anim_clock.zw). A line-by-line
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

// LLViewerParcelOverlay: ownership stays on the terrain; property lines
// also sit at the water surface. Reuse the terrain vertices and depth.
@vertex
fn vs_land_overlay(in: VsIn) -> VsOut {
    let rec = records[in.instance];
    var out: VsOut;
    out.world_pos = (rec.model * vec4<f32>(in.pos, 1.0)).xyz;
    out.record = in.instance;
    if (rec.params.x > 0.5) {
        out.world_pos.z = max(out.world_pos.z, rec.params.y);
    }
    out.world_pos.z += 0.025;
    out.clip = frame.eye_view_proj * vec4<f32>(out.world_pos - frame.eye.xyz, 1.0);
    return out;
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let rec = records[in.instance];
    let model = eye_model(rec, in);
    let ep = model * vec4<f32>(vertex_position(rec, in), 1.0);
    // (lighting, shadows and fog keep region coordinates)
    let wp = vec4<f32>(ep.xyz + frame.eye.xyz, ep.w);
    var out: VsOut;
    out.clip = frame.eye_view_proj * ep;
    out.world_pos = wp.xyz;
    out.normal = cofactor(model) * vertex_normal(rec, in);
    var st = face_st(in, rec);
    let animated = has_tex_anim(rec);
    if (animated) {
        // texture animation first (PBR: before each map's KHR transform,
        // textureUtilV.glsl texture_transform)
        st = tex_anim_st(st, rec, frame.anim_clock.zw);
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
    // A glTF normal map turns its tangent frame with its own transform
    // (textureUtilV.glsl tangent_space_transform). A legacy one does not:
    // LLFace::getGeometryVolume moves the normal map coordinates (xform) but
    // keeps the tangents of the volume face, built on its own texture
    // coordinates (LLVolumeFace::createTangents), so a map turned or flipped
    // by its repeats, by a planar mapping or by a texture animation has its
    // relief turned with it
    out.uv_t = out.uv_n;
    if ((rec.flags.x & FLAG_PBR) == 0u) {
        out.uv_t = vec2<f32>(in.uv.x, 1.0 - in.uv.y);
    }
    // The tangents built with the face (scene/tangents.rs), moved as the
    // Legacy Viewer moves them: by the normal's matrix, a rigged vertex's by
    // its skinning matrix (materialV.glsl, pbropaqueV.glsl)
    out.tangent = vec3<f32>(0.0);
    out.tangent_sign = 1.0;
    let pbr = (rec.flags.x & FLAG_PBR) != 0u;
    var vt = vec4<f32>(0.0);
    if ((frame.light_mode.z & TANGENT_VERTEX) != 0u && !(pbr && animated)) {
        vt = vertex_tangent(in);
    }
    if (vt.w != 0.0) {
        var t = cofactor(model) * vt.xyz;
        if ((rec.flags.x & FLAG_SKINNED) != 0u) {
            t = mat3x3<f32>(model[0].xyz, model[1].xyz, model[2].xyz) * vt.xyz;
        }
        var tw = vec4<f32>(t, vt.w);
        if (pbr) {
            tw = tangent_space_transform(t, vt.w, normalize(out.normal), rec.mat_uv.xy, rec.legacy.x);
        }
        if (dot(tw.xyz, tw.xyz) > 1e-20) {
            out.tangent = normalize(tw.xyz);
            out.tangent_sign = tw.w;
        }
    }
    out.record = in.instance;
    out.view_depth = distance(ep.xyz, frame.camera_pos.xyz - frame.eye.xyz);
    return out;
}

struct DepthOut {
    @invariant @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) @interpolate(flat) record: u32,
};

// Firestorm fullbrightF / pbropaqueF IS_HUD: base color plus emissive,
// without atmosphere or scene lighting. Keep the shared UV/animation path.
override HUD_SRGB_TARGET: bool = false;

// A HUD face: drawn straight to the screen, never into the G-buffer, it
// keeps its emissive without HDR (eval_material, deferred_emissive_off).
var<private> hud_pass: bool = false;

@fragment
fn fs_hud(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let rec = records[in.record];
    // LLGLSPipeline / LLRenderPass::pushGLTFBatch (indra/llrender/llglstates.h,
    // indra/newview/lldrawpool.cpp, originally LGPL 2.1): back faces are
    // hidden unless double-sided. Per-face rejection preserves the
    // depth-sorted stream shared by translucent HUD materials.
    if (!front && (rec.flags.x & FLAG_DOUBLE_SIDED) == 0u) { discard; }
    hud_pass = true;
    let mat = eval_material(in, front);
    if ((rec.flags.x & FLAG_ALPHA_MASK) != 0u && mat.color.a < rec.params.w) { discard; }
    let blended = (rec.flags.x & FLAG_ALPHA_BLEND) != 0u;
    if (blended && mat.color.a < 0.004) { discard; }
    var rgb = mat.color.rgb + select(vec3<f32>(0.0), mat.emissive, (rec.flags.x & FLAG_PBR) != 0u);
    if (!HUD_SRGB_TARGET) { rgb = ll_linear_to_srgb(rgb); }
    return vec4<f32>(rgb, select(1.0, mat.color.a, blended));
}

// Depth prepass (opaque geometry and terrain): position only.
@vertex
fn vs_depth(in: VsIn) -> DepthOut {
    let rec = records[in.instance];
    let model = eye_model(rec, in);
    let ep = model * vec4<f32>(vertex_position(rec, in), 1.0);
    var out: DepthOut;
    out.clip = frame.eye_view_proj * ep;
    out.world_pos = ep.xyz + frame.eye.xyz;
    out.record = in.instance;
    return out;
}

@fragment
fn fs_depth_portal(in: DepthOut) {
    portal_clip(records[in.record], in.world_pos);
}

// Depth prepass for alpha-masked faces: alpha test only.
@fragment
fn fs_depth_mask(in: VsOut) {
    let rec = records[in.record];
    portal_clip(rec, in.world_pos);
    let a = sample_tex(rec.tex.x, in.uv).a * rec.base_color.a;
    if (a < rec.params.w) {
        discard;
    }
}

// The depth prepass with GTAO's mesh normals (gtao.rs, AURORA_GTAO_NORMALS):
// the position of vs_depth and the vertices' smooth normal, the shading's
// without its normal map. A normal taken from the depth is the flat
// triangle's, and showed the facets of low-poly meshes in the occlusion
// (#993).
struct DepthNormalOut {
    @invariant @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
};

@vertex
fn vs_depth_normal(in: VsIn) -> DepthNormalOut {
    let rec = records[in.instance];
    let model = eye_model(rec, in);
    let ep = model * vec4<f32>(vertex_position(rec, in), 1.0);
    var out: DepthNormalOut;
    out.clip = frame.eye_view_proj * ep;
    out.normal = cofactor(model) * vertex_normal(rec, in);
    return out;
}

// The world normal facing the side drawn (eval_material), octahedral as in
// the G-buffer.
fn depth_normal(n: vec3<f32>, front: bool) -> vec4<f32> {
    let m = normalize(select(-n, n, front));
    return vec4<f32>(oct_encode(m), 0.0, 0.0);
}

@fragment
fn fs_depth_normal(in: DepthNormalOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return depth_normal(in.normal, front);
}

// Only selected while an avatar intersects a portal. The ordinary normal
// prepass keeps its compact vertex output without a record/position fetch.
@fragment
fn fs_depth_portal_normal(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    portal_clip(records[in.record], in.world_pos);
    return depth_normal(in.normal, front);
}

// fs_depth_mask with the normal.
@fragment
fn fs_depth_mask_normal(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let rec = records[in.record];
    portal_clip(rec, in.world_pos);
    let a = sample_tex(rec.tex.x, in.uv).a * rec.base_color.a;
    if (a < rec.params.w) {
        discard;
    }
    return depth_normal(in.normal, front);
}

// Alpha from which fs_depth_contact writes depth: half covered; 1/255 for
// the rigged fringes of `RiggedBlendDepth::Layers` (#1300), the Legacy
// Viewer's MINIMUM_ALPHA.
override DEPTH_CONTACT_ALPHA: f32 = 0.5;

// Depth of the blended faces opaque enough to cast contact shadows (half
// covered, as the impostor pictures keep them), drawn over a copy of the
// prepass (contact_shadows.rs).
fn depth_contact(in: VsOut) {
    let rec = records[in.record];
    if ((rec.flags.x & FLAG_PORTAL_EFFECT) != 0u) {
        // Its solid core occludes through the same opening; the decorative
        // quad corners/corona must never add a rectangular occluder.
        if (length((in.uv * 2.0 - vec2<f32>(1.0)) * PORTAL_SURFACE_SCALE) > 1.0) { discard; }
        return;
    }
    portal_clip(rec, in.world_pos);
    if (sample_tex(rec.tex.x, in.uv).a * rec.base_color.a < DEPTH_CONTACT_ALPHA) {
        discard;
    }
}

@fragment
fn fs_depth_contact(in: VsOut) {
    depth_contact(in);
}

// fs_depth_contact with the normal, into the occlusion's depth.
@fragment
fn fs_depth_contact_normal(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    depth_contact(in);
    return depth_normal(in.normal, front);
}

// Depth of the rigged blended faces nearly opaque, before any of their
// colour (RiggedBlendDepth::Runs): what such a texel hides behind it
// shows through it by 5 % at most, while a fainter one, a soft strand edge,
// must not hide the strands drawn after it (#935).
@fragment
fn fs_depth_opaque(in: VsOut) {
    let rec = records[in.record];
    portal_clip(rec, in.world_pos);
    if (sample_tex(rec.tex.x, in.uv).a * rec.base_color.a < 0.95) {
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
    // legacy "emissive mask": share of the unlit color (the texture alpha)
    unlit: f32,
    // ambient occlusion of the normal map's relief (normal_map_occlusion)
    occlusion: f32,
    // PBR: the material's own occlusion, the red of its packed
    // occlusion-roughness-metal map (softenLightF.glsl: ao = orm.r), 1 for
    // any other face
    ao: f32,
    // the face's normal without its normal map, which offsets the shadow
    // lookup (shadow_normal)
    geo_n: vec3<f32>,
};

// The normal that moves a fragment's shadow lookup off its surface. The
// Legacy Viewer moves it towards the light (sampleDirectionalShadow,
// deferred/shadowUtil.glsl), which leaves the place read in the map
// unchanged whatever the normal map; moved along the normal-mapped normal,
// the lookup followed the relief of the map and drew its pattern along the
// shadow edges. AURORA_SHADOW_FILTER=mapped brings that back, to compare.
fn shadow_normal(m: Material) -> vec3<f32> {
    if ((frame.light_mode.z & SHADOW_OFFSET_MAPPED) != 0u) {
        return m.n;
    }
    return m.geo_n;
}

fn eval_material(in: VsOut, front: bool) -> Material {
    portal_clip(records[in.record], in.world_pos);
    rez_fw = fwidth(in.world_pos);
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
    m.geo_n = n;
    m.n = perturb_normal(n, in.world_pos, in.uv_t, in.tangent, in.tangent_sign, front, nrm_s.xyz);
    if (rec.tex.y == 1u) {
        m.n = n;
    }
    m.occlusion = normal_map_occlusion(n, m.n, in.world_pos, 1.0 / in.clip.w);
    // glTF: G = roughness, B = metallic
    m.metallic = clamp(rec.params.y * mr_s.b, 0.0, 1.0);
    m.roughness = clamp(rec.params.z * mr_s.g, 0.03, 1.0);
    // a PBR face without a metallic-roughness map reads the white texture:
    // no occlusion. AURORA_PBR_OCCLUSION=0 (sky_ll.z = 0) leaves it out.
    m.ao = select(1.0, mr_s.r, (rec.flags.x & FLAG_PBR) != 0u && frame.sky_ll.z > 0.5);
    // TE glow is not emissive: it only feeds the glow pass (fs_glow)
    m.emissive = rec.emissive.rgb * srgb_to_linear(em_s.rgb);
    if (deferred_emissive_off() && !hud_pass && (rec.flags.x & (FLAG_PBR | FLAG_ALPHA_BLEND)) == FLAG_PBR) {
        m.emissive = vec3<f32>(0.0);
    }
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
    // geometric specular anti-aliasing (specular_aa_roughness): the normal's
    // variance read here, where the derivatives are still allowed
    let n_var = normal_variance(m.n) + toksvig_variance(select(length(nrm_s.xyz * 2.0 - 1.0), 1.0, rec.tex.y == 1u));
    if ((frame.light_mode.z & SPECULAR_AA) != 0u) {
        if ((rec.flags.x & FLAG_LEGACY_MAT) != 0u) {
            if (m.legacy_spec.a > 0.0) {
                m.legacy_spec.a = specular_aa_gloss(m.legacy_spec.a, n_var);
            }
        } else {
            m.roughness = specular_aa_roughness(m.roughness, n_var);
        }
    }
    m.unlit = 0.0;
    if ((rec.flags.x & FLAG_EMISSIVE_MASK) != 0u) {
        // "emissive mask": the texture alpha says how much of the face
        // shows its own color instead of the lit one (materialF.glsl
        // getEmissive, mixed in softenLightF.glsl), not an added light
        m.unlit = base_s.a;
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
fn legacy_light(col_in: vec3<f32>, base: vec3<f32>, unlit: f32, spec: vec4<f32>, env: f32, n: vec3<f32>, v: vec3<f32>,
                world_pos: vec3<f32>, frag: vec2<f32>, shadow: f32, sun_lin: vec3<f32>, sky_scale: f32) -> vec3<f32> {
    var col = col_in;
    if (spec.a <= 0.0 && env <= 0.0) {
        return mix(col, base, unlit);
    }
    let r = reflect(-v, n);
    let sky = sky_color(vec3<f32>(r.x, r.y, max(r.z, -0.2))) * sky_scale * (0.35 + 0.65 * shadow);
    // sampleReflectionProbesLegacy: glossy environment at the glossiness mip,
    // legacy environment at the sharpest one
    var gloss_sky = sky;
    var env_sky = sky;
    if (probes_on()) {
        // (both in one walk of the probes)
        let both = sample_probes_pair(
            world_pos,
            frag,
            ProbeLookup(r, (1.0 - spec.a) * probe_max_lod(), false),
            ProbeLookup(r, 0.0, false),
            env > 0.0,
            true,
            vec3<f32>(0.0),
        );
        let g = both.a;
        if (g.w > 0.0) {
            gloss_sky = g.rgb;
        }
        if (env > 0.0) {
            let e = both.b;
            if (e.w > 0.0) {
                env_sky = e.rgb;
            }
        }
    }
    // and both take the mirror, the environment as fully glossy
    let gm = mirror_tap(world_pos, r, spec.a);
    gloss_sky = mix(gloss_sky, gm.rgb, gm.a);
    if (env > 0.0) {
        let em = mirror_tap(world_pos, r, 1.0);
        env_sky = mix(env_sky, em.rgb, em.a);
    }
    // sampleReflectionProbesLegacy clamps both: a very bright sky does not
    // wash a shiny face out
    gloss_sky = clamp(gloss_sky, vec3<f32>(0.0), vec3<f32>(10.0));
    env_sky = clamp(env_sky, vec3<f32>(0.0), vec3<f32>(10.0));
    // with GTAO, the glossy reflection fades in creases as a PBR one
    // (specular_occlusion; its roughness from the glossiness, as the probe
    // level above); the sharp environment is a mirror's lobe, which that
    // occlusion leaves alone
    gloss_sky *= specular_occlusion(frag, r, 1.0 - spec.a, 1.0);
    if (spec.a > 0.0) {
        let l = normalize(frame.sun_dir.xyz);
        col += ll_spec(n, v, l, spec.a) * shadow * sun_lin * spec.rgb;
        // applyGlossEnv: its "fake energy conservation" is not clamped, a
        // face lit above 1 loses light there
        var fresnel = clamp(1.0 - dot(v, n), 0.3, 1.0);
        fresnel = fresnel * fresnel * spec.a;
        let glossenv = gloss_sky * 0.5 * spec.rgb * fresnel * (vec3<f32>(1.0) - col);
        col = max(col + glossenv * 0.5, vec3<f32>(0.0));
    }
    // emissive mask, between the highlights and the environment
    col = mix(col, base, unlit);
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
// A projector's highlight takes the color of its texture, and a face with
// an environment intensity `env` shows the projector's image (spotLightF).
fn legacy_local_spec(spec: vec4<f32>, env: f32, n: vec3<f32>, v: vec3<f32>, world_pos: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    var col = vec3<f32>(0.0);
    if (spec.a <= 0.0 && env <= 0.0) {
        return col;
    }
    var lights = light_walk(world_pos, frag);
    var spots = projector_walk(lights);
    if (spec.a > 0.0) {
        for (var i = next_light(&lights); i >= 0; i = next_light(&lights)) {
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
    }
    for (var i = next_light(&spots); i >= 0; i = next_light(&spots)) {
        let at = projector_at(i, world_pos, n);
        if (at.atten <= 0.0) {
            continue;
        }
        if (spec.a > 0.0) {
            let s = ll_spec(n, v, at.l, spec.a) * at.atten;
            col += clamp(s * at.color / 3.25 * spec.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
        }
        if (env > 0.0) {
            // the view reflected to the plane the projector emits from
            let pj = frame.projectors[i];
            let ref_dir = reflect(-v, n);
            let ds = dot(ref_dir, pj.dir.xyz);
            if (ds < 0.0) {
                let plane = pj.origin.xyz + pj.dir.xyz * (pj.params.x + 0.01);
                let hit = world_pos + ref_dir * dot(plane - world_pos, pj.dir.xyz) / ds;
                let stc = pj.matrix * vec4<f32>(hit, 1.0);
                let uv = stc.xy / stc.w;
                if (stc.z > 0.0 && uv.x > 0.0 && uv.x < 1.0 && uv.y > 0.0 && uv.y < 1.0) {
                    let image = projector_specular(pj.tex.x, uv, (1.0 - spec.a) * 0.6);
                    col += frame.lights[i].color_falloff.rgb / 3.25 * image.rgb * at.shadow * env;
                }
            }
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

// The rez effect (the viewer's scene/rez.rs, `DrawRecord::rez`): an
// object still loading is a glowing grid, then a line rising from its
// bottom reveals it; leaving, the line comes down (a derez) and leaves the
// grid behind. Its colors run along the brand gradient of the theme
// (`frame.rez_gradient`: the leading edge, the middle, the trail); x of
// the first entry's w picks the look (`REZ_STYLE_*`). World-space lines a
// pixel wide, anti-aliased with the derivatives `rez_fw` keeps.
const REZ_HOLOGRAM: u32 = 1u;
const REZ_SWEEP: u32 = 2u;
const REZ_DEREZ: u32 = 3u;
const REZ_FADE: u32 = 4u;
const REZ_GLINT: u32 = 5u;
// the grid's lines trace in over this long (s); a derez's line comes down
// in this part of its time, its grid fades out in the rest
const REZ_INTRO: f32 = 0.2;
const REZ_DEREZ_LINE: f32 = 0.5;
// grid and scanlines; scanlines only; a fine wireframe
const REZ_STYLE_GRID: u32 = 1u;
const REZ_STYLE_SCAN: u32 = 2u;
const REZ_STYLE_WIRE: u32 = 3u;

// fwidth(world position) of the fragment, taken where the control flow is
// still uniform (`eval_material`, `fs_glow`): `shade_material` may run
// after a discard.
var<private> rez_fw: vec3<f32> = vec3<f32>(0.0);

// The brand gradient at `t` (0 the leading edge, 1 the trail), linear.
fn rez_gradient(t: f32) -> vec3<f32> {
    let u = clamp(t, 0.0, 1.0) * 2.0;
    let a = frame.rez_gradient[0].rgb;
    let b = frame.rez_gradient[1].rgb;
    let c = frame.rez_gradient[2].rgb;
    return select(mix(b, c, u - 1.0), mix(a, b, u), u < 1.0);
}

fn rez_hash(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453);
}

// 1 on the lines of a grid `step` metres apart, a pixel wide, none along
// the axis a face is square to (it would light the whole face); faded out
// where the lines get closer than a few pixels.
fn rez_lines(p: vec3<f32>, n: vec3<f32>, step: f32) -> f32 {
    let fw = max(rez_fw / step, vec3<f32>(1e-5));
    let q = abs(fract(p / step - 0.5) - 0.5);
    let l = vec3<f32>(1.0) - smoothstep(fw * 0.5, fw * 1.5, q);
    let along = vec3<f32>(1.0) - smoothstep(vec3<f32>(0.75), vec3<f32>(0.95), abs(n));
    let dense = vec3<f32>(1.0) - smoothstep(vec3<f32>(0.12), vec3<f32>(0.35), fw);
    let w = l * along * dense;
    return max(w.x, max(w.y, w.z));
}

// Horizontal scanlines `step` apart, a pixel wide.
fn rez_scan(z: f32, step: f32) -> f32 {
    let fw = max(rez_fw.z / step, 1e-5);
    let q = abs(fract(z / step - 0.5) - 0.5);
    return (1.0 - smoothstep(fw * 0.5, fw * 1.5, q)) * (1.0 - smoothstep(0.12, 0.35, fw));
}

// The look of what is still to come: a dark tint of the surface, lines in
// the gradient (`g`: where on it), a glowing rim, a soft band of light
// climbing it and a faint shimmer of its cells.
fn rez_hologram(col: vec3<f32>, in: VsOut, n: vec3<f32>, g: f32, h: f32, t: f32, style: u32) -> vec3<f32> {
    let tint = rez_gradient(g);
    let v = normalize(frame.camera_pos.xyz - in.world_pos);
    let rim = pow(1.0 - abs(dot(n, v)), 3.0);
    var lines: f32;
    if (style == REZ_STYLE_SCAN) {
        lines = rez_scan(in.world_pos.z, 0.05);
    } else if (style == REZ_STYLE_WIRE) {
        lines = max(rez_lines(in.world_pos, n, 0.125), 0.35 * rez_lines(in.world_pos, n, 0.03125));
    } else {
        lines = max(rez_lines(in.world_pos, n, 0.25), 0.4 * rez_scan(in.world_pos.z, 0.0625));
    }
    let cell = floor(in.world_pos / 0.25);
    let shimmer = 0.8 + 0.2 * sin(t * 5.0 + rez_hash(cell) * 6.2831853);
    let climb = exp(-pow((fract(h - t * 0.45) - 0.5) * 7.0, 2.0));
    let body = col * 0.08 + tint * (0.035 + 0.05 * climb);
    return body + tint * (lines * (1.0 + 1.6 * climb) * shimmer + rim * 0.6);
}

// The effect's progress, 0 to 1, over the time held above the mode's byte
// (ms).
fn rez_progress(rec: DrawRecord, elapsed: f32) -> f32 {
    return clamp(elapsed * 1000.0 / max(f32((rec.rez.y >> 8u) & 0xfffu), 1.0), 0.0, 1.0);
}

// Where the line is (world height) and its half width. A derez brings it
// down in the first part of its time (`REZ_DEREZ_LINE`), then the grid it
// left fades out from the top (`rez_alpha`).
fn rez_line(rec: DrawRecord, elapsed: f32) -> vec2<f32> {
    let bottom = bitcast<f32>(rec.rez.z);
    let height = max(bitcast<f32>(rec.rez.w), 0.05);
    var t = rez_progress(rec, elapsed);
    let derez = (rec.rez.y & 0xffu) == REZ_DEREZ;
    if (derez) {
        t = clamp(t / REZ_DEREZ_LINE, 0.0, 1.0);
    }
    // eased: it starts and lands softly; a derez runs it backwards
    var e = t * t * (3.0 - 2.0 * t);
    if (derez) {
        e = 1.0 - e;
    }
    let band = max(height * 0.02, 0.015);
    return vec2<f32>(bottom - 3.0 * band + e * (height + 6.0 * band), band);
}

// A derez's fading front, in heights from the bottom (above 1: none yet).
fn rez_fade_front(rec: DrawRecord, elapsed: f32) -> f32 {
    let t = clamp((rez_progress(rec, elapsed) - REZ_DEREZ_LINE) / (1.0 - REZ_DEREZ_LINE), 0.0, 1.0);
    return 1.08 - 1.16 * t;
}

// Opacity of a derezzing face (drawn blended while it lasts: rez.rs): the
// grid goes from the top down. 1 for every other face.
fn rez_alpha(rec: DrawRecord, in: VsOut) -> f32 {
    if ((rec.rez.y & 0xffu) != REZ_DEREZ) {
        return 1.0;
    }
    let elapsed = f32(bitcast<i32>(frame.anim_clock.x - rec.rez.x)) * 0.001;
    let bottom = bitcast<f32>(rec.rez.z);
    let height = max(bitcast<f32>(rec.rez.w), 0.05);
    let h = (in.world_pos.z - bottom) / height;
    let front = rez_fade_front(rec, elapsed);
    return 1.0 - smoothstep(front - 0.08, front, h);
}

// Whether a face still shows the grey texel of a texture loading: its
// base texture's slot points at the shared placeholder layer
// (`textures::PLACEHOLDER`, slot 4) until its first texels arrive.
fn rez_still_grey(rec: DrawRecord) -> bool {
    return tex_loc[rec.tex.x] == tex_loc[4u];
}

// Height of the fragment within its object (0 bottom, 1 top), for the
// gradient: the record's own bounds, or (none given) from its placement,
// the prim's height in its model matrix (a worn or rigged face, placed by
// the GPU, takes the middle).
fn rez_height(rec: DrawRecord, in: VsOut) -> f32 {
    let height = bitcast<f32>(rec.rez.w);
    if (height > 0.0) {
        return clamp((in.world_pos.z - bitcast<f32>(rec.rez.z)) / height, 0.0, 1.0);
    }
    if ((rec.flags.x & (FLAG_ANCHORED | FLAG_SKINNED)) != 0u) {
        return 0.5;
    }
    let half = max(0.5 * length(rec.model[2].xyz), 0.025);
    return clamp((in.world_pos.z - rec.model[3].z + half) / (2.0 * half), 0.0, 1.0);
}

fn rez_apply(rec: DrawRecord, in: VsOut, n: vec3<f32>, col: vec3<f32>) -> vec3<f32> {
    var mode = rec.rez.y & 0xffu;
    if (mode == 0u) {
        // with the effect on (frame.rez_gradient[1].w), a face still
        // showing the grey of a texture loading is the grid instead, until
        // its own texture comes (no grey, and no face waits for another)
        if (frame.rez_gradient[1].w < 0.5 || !rez_still_grey(rec)) {
            return col;
        }
        mode = REZ_HOLOGRAM;
    }
    // (a look of its own above the line's length, else the frame's)
    let own_style = rec.rez.y >> 20u;
    let style = select(u32(frame.rez_gradient[0].w), own_style, own_style != 0u);
    // (the grid of a face with no effect of its own runs on the clock, its
    // lines already traced)
    let elapsed = select(f32(frame.anim_clock.x) * 0.001, f32(bitcast<i32>(frame.anim_clock.x - rec.rez.x)) * 0.001, rec.rez.y != 0u);
    let bottom = bitcast<f32>(rec.rez.z);
    let height = max(bitcast<f32>(rec.rez.w), 0.05);
    let h = rez_height(rec, in);
    let level = rez_level(n);
    if (mode == REZ_HOLOGRAM) {
        // the gradient from the bottom up; its lines trace in from the
        // base first, behind a bright edge, over the grid's dark body (the
        // surface itself is still the untextured grey: never shown)
        let holo = rez_hologram(col, in, n, h, h, elapsed, style);
        let body = col * 0.08 + rez_gradient(h) * 0.035;
        let front = elapsed / REZ_INTRO;
        let shown = 1.0 - smoothstep(front - 0.05, front, h);
        let edge = exp(-pow((h - front) / 0.03, 2.0)) * (1.0 - smoothstep(1.0, 1.15, front)) * level;
        return mix(body, holo, shown) + mix(rez_gradient(0.0), vec3<f32>(1.0), 0.5) * edge * 3.0;
    }
    if (mode == REZ_FADE) {
        // loaded with no line for it: the grid gives way to the textures
        let t = rez_progress(rec, elapsed);
        let holo = rez_hologram(col, in, n, h, h, elapsed, style);
        return mix(holo, col, t * t * (3.0 - 2.0 * t));
    }
    let lb = rez_line(rec, elapsed);
    let above = in.world_pos.z - lb.x;
    let band = lb.y;
    if (mode == REZ_GLINT) {
        // an object arriving: the line passes over what is there and
        // hides nothing; a face still loading stays the grid (on the clock,
        // as without the glint), the others show their surface, with a
        // short trail under the line
        var base = col;
        if (frame.rez_gradient[1].w >= 0.5 && rez_still_grey(rec)) {
            base = rez_hologram(col, in, n, h, h, f32(frame.anim_clock.x) * 0.001, style);
        }
        let below = 1.0 - smoothstep(-band * 0.5, band * 0.5, above);
        let trail = rez_gradient(1.0) * exp(min(above, 0.0) / (band * 8.0)) * 0.5 * below;
        let core = exp(-pow(above / (band * 0.45), 2.0)) * level;
        let halo = exp(-pow(above / (band * 1.6), 2.0)) * level;
        return base + trail + mix(rez_gradient(0.0), vec3<f32>(1.0), 0.65) * core * 4.0 + rez_gradient(0.15) * halo * 1.4;
    }
    // above: still the grid, its color running from the edge's to the
    // trail's with the distance to the line, brighter near it
    let ahead = clamp(above / (height * 0.7), 0.0, 1.0);
    let holo = rez_hologram(col, in, n, ahead, h, elapsed, style) + rez_gradient(0.0) * exp(-max(above, 0.0) / (band * 5.0)) * 0.9;
    // below: the surface, with a short trail in the gradient's end
    let trail = rez_gradient(1.0) * exp(min(above, 0.0) / (band * 8.0)) * 0.7;
    let revealed = col + trail;
    var c = mix(revealed, holo, smoothstep(-band * 0.5, band * 0.5, above));
    // the line itself: white at its core, the leading color around it
    // (softer on a level surface, which it crosses all at once)
    let core = exp(-pow(above / (band * 0.45), 2.0)) * level;
    let halo = exp(-pow(above / (band * 1.6), 2.0)) * level;
    c = c + mix(rez_gradient(0.0), vec3<f32>(1.0), 0.65) * core * 5.0 + rez_gradient(0.15) * halo * 1.6;
    if (mode == REZ_DEREZ) {
        // the fading front, in the gradient's deep end
        let hf = (in.world_pos.z - bottom) / height - rez_fade_front(rec, elapsed);
        c = c + rez_gradient(1.0) * exp(-pow(hf / 0.03, 2.0)) * 2.0 * level;
    }
    return c;
}

// How much of the line a surface of normal `n` shows: a level one (a
// floor, a table top) is crossed in one frame and would flash.
fn rez_level(n: vec3<f32>) -> f32 {
    return mix(1.0, 0.2, smoothstep(0.55, 0.95, abs(n.z)));
}

// What the effect adds to the glow pass (fs_glow): the line and, softly,
// the lines of the grid.
fn rez_glow(rec: DrawRecord, in: VsOut) -> f32 {
    let mode = rec.rez.y & 0xffu;
    if (mode == 0u) {
        return 0.0;
    }
    let elapsed = f32(bitcast<i32>(frame.anim_clock.x - rec.rez.x)) * 0.001;
    let n = normalize(in.normal);
    let grid = rez_lines(in.world_pos, n, 0.25) * 0.25;
    if (mode == REZ_HOLOGRAM) {
        return grid * clamp(elapsed / REZ_INTRO, 0.0, 1.0);
    }
    if (mode == REZ_FADE) {
        return 0.0;
    }
    let lb = rez_line(rec, elapsed);
    let above = in.world_pos.z - lb.x;
    let line = exp(-pow(above / (lb.y * 1.2), 2.0)) * rez_level(n);
    if (mode == REZ_GLINT) {
        // (the line only: no grid hidden above it)
        return line * 0.8;
    }
    return max(line * 0.9, select(0.0, grid, above > 0.0));
}

fn shade(in: VsOut, front: bool) -> Shaded {
    return shade_material(in, eval_material(in, front));
}

// Lighting, haze and fog of an evaluated material. Nothing here takes a
// screen-space derivative (explicit levels and comparisons only): a fragment
// may be discarded between `eval_material` and this (fs_blend).
fn shade_material(in: VsOut, m: Material) -> Shaded {
    let rec = records[in.record];
    surface_occlusion = m.occlusion;
    // a blended face opaque enough (fs_depth_contact) is in the occlusion's
    // depth when BLEND_SCREEN_DEPTH; a more transparent one would get what
    // lies behind it
    screen_surface = (rec.flags.x & FLAG_ALPHA_BLEND) == 0u || ((frame.light_mode.z & BLEND_SCREEN_DEPTH) != 0u && m.color.a >= 0.5);
    // a blended face is in the depth the contact shadows read when it is
    // opaque enough (fs_depth_contact); a more transparent one would get
    // what lies behind it
    contact_surface = (rec.flags.x & FLAG_ALPHA_BLEND) == 0u || (frame.contact.y > 0.5 && m.color.a >= 0.5);
    // Share this calculation across both lighting branches: duplicating its
    // wave/shadow code crashes Apple's Metal pipeline compiler.
    var caustic_light = vec3<f32>(0.0);
    if ((rec.flags.x & FLAG_FULLBRIGHT) == 0u) {
        caustic_light = m.color.rgb * (1.0-m.unlit) * water_caustics(in.world_pos,m.n);
    }
    var col: vec3<f32>;
    let classic = is_classic_sky();
    var hazed = false;
    if ((rec.flags.x & FLAG_FULLBRIGHT) != 0u) {
        col = m.color.rgb + m.emissive;
        // opaque fullbright faces are not hazed on classic skies
        hazed = classic;
    } else if (classic) {
        let v = normalize(frame.camera_pos.xyz - in.world_pos);
        // softenLightF: scol = max(shadow, emissive)
        let sh = max(sun_shadow(in.world_pos, shadow_normal(m), in.view_depth, in.clip.xy), m.unlit);
        let a = ll_atmos(in.world_pos - frame.camera_pos.xyz);
        // softenLightF: (sun + sky + specular + environment + emissive) x
        // final_scale, then the local lights pass, then hazeF
        if ((rec.flags.x & FLAG_PBR) != 0u) {
            col = ll_classic_pbr(m.color.rgb, m.metallic, m.roughness, m.ao, m.n, v, in.world_pos, sh, in.clip.xy, a);
            col = (col + m.emissive) * ll_final_scale()
                + ll_classic_pbr_local(m.color.rgb, m.metallic, m.roughness, m.n, v, in.world_pos, in.clip.xy);
        } else {
            col = ll_classic_sun_ambient(m.color.rgb, m.n, sh, in.clip.xy, a);
            col = legacy_light(col, m.color.rgb, m.unlit, m.legacy_spec, m.legacy_env, m.n, v, in.world_pos, in.clip.xy, sh, ll_sun_linear(a), 1.0);
            col = (col + m.emissive) * ll_final_scale() + ll_local_diffuse(m.color.rgb, m.n, in.world_pos, in.clip.xy)
                + legacy_local_spec(m.legacy_spec, m.legacy_env, m.n, v, in.world_pos, in.clip.xy) * ll_local_scale();
        }
        col = ll_haze(col + caustic_light, a);
        hazed = true;
    } else {
        let v = normalize(frame.camera_pos.xyz - in.world_pos);
        let sh = max(sun_shadow(in.world_pos, shadow_normal(m), in.view_depth, in.clip.xy), m.unlit);
        // a sky with a probe ambiance: softenLightF's non-classic branch, a
        // face without a PBR material lit as a legacy one
        let legacy = (rec.flags.x & FLAG_PBR) == 0u;
        col = shade_pbr(m.color.rgb, m.metallic, m.roughness, m.ao, m.n, v, in.world_pos, sh, in.clip.xy, legacy) + m.emissive;
        if (m.legacy_spec.a > 0.0 || m.legacy_env > 0.0 || m.unlit > 0.0) {
            // its highlights in the sunlight (sunlit_linear) and its
            // environment at the probes' radiance
            col = legacy_light(col - m.emissive, m.color.rgb, m.unlit, m.legacy_spec, m.legacy_env, m.n, v, in.world_pos, in.clip.xy, sh,
                               frame.sun_color.rgb / 3.0, 1.0)
                + legacy_local_spec(m.legacy_spec, m.legacy_env, m.n, v, in.world_pos, in.clip.xy) + m.emissive;
        }
        col += caustic_light;
    }
    if ((rec.flags.x & FLAG_HIGHLIGHT) != 0u) {
        col = mix(col, vec3<f32>(0.55, 0.36, 0.96), 0.35);
    }
    if(water_fx_on(256u) && in.world_pos.z<frame.params.y && (rec.flags.x & FLAG_FULLBRIGHT) == 0u) {
        col = max(col-m.emissive,vec3<f32>(0.0))*water_scene_light(in.world_pos)+m.emissive;
    }
    col = rez_apply(rec, in, m.n, col);
    if (!hazed || camera_underwater() || (water_fx_on(256u) && in.world_pos.z<camera_water_level())) {
        col = apply_fog(col, in.world_pos);
    }
    var s: Shaded;
    col = debug_view(col, in.world_pos, shadow_normal(m), in.view_depth, in.clip.xy);
    s.color = vec4<f32>(finite_hdr(light_tiles_checked(col, in.world_pos, in.clip.xy)), m.color.a);
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

// The alpha tests of masked faces keep every texel at or above the cutoff,
// as LL's materialF.glsl and pbropaqueF.glsl (`alpha < minimum_alpha`): a
// cutoff of 0 draws the whole face, which creators use to show opaque a
// texture whose alpha channel holds something else.
fn mask_color(rec: DrawRecord, c: vec4<f32>) -> vec4<f32> {
    let cutoff = rec.params.w;
    if (cutoff <= 0.0) {
        return vec4<f32>(c.rgb, 1.0);
    }
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
    // A mirrored view (water, mirror) flips the winding: the side facing
    // its camera is the one rasterized as a back face. Single-sided faces
    // show that side only, as in the Legacy Viewer's culled reflection and
    // hero probe passes: a mirror's own surface, a few centimetres in front
    // of its plane, would otherwise hide the whole image behind its back.
    let facing = front != mirrored_pass();
    if (!facing && (rec.flags.x & FLAG_DOUBLE_SIDED) == 0u) {
        discard;
    }
    let s = shade(in, facing);
    let masked = (rec.flags.x & FLAG_ALPHA_MASK) != 0u && s.color.a < rec.params.w;
    if (reflection_clip(in.world_pos) || masked) {
        discard;
    }
    return vec4<f32>(s.color.rgb, 1.0);
}

// Blended faces: invisible fragments leave before the lighting, as in
// Firestorm's alphaF.glsl / pbralphaF.glsl (`final_alpha < minimum_alpha`,
// LLDrawPoolAlpha MINIMUM_ALPHA = 0.004, tested before the atmospherics, the
// probes and the lights). The empty texels of hair, lashes and lace cost
// their texture samples only. `false` lights them first, as before, for
// comparison (AURORA_BLEND_EARLY_DISCARD=0): same image.
override BLEND_EARLY_DISCARD: bool = true;

// RiggedBlendDepth::Split (#935): 1 draws only the texels at least half
// covered (writing their depth as they are drawn), 2 only the fainter ones
// (depth tested, nothing written); 0, every other use, draws them all.
override BLEND_SPLIT: u32 = 0u;

@fragment
fn fs_blend(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let portal_aa = max(length(fwidth(in.uv)) * 2.0 * PORTAL_SURFACE_SCALE, 0.001);
    if ((records[in.record].flags.x & FLAG_PORTAL_EFFECT) != 0u) {
        let surface = portal_surface(records[in.record], in.uv, portal_aa);
        if (surface.alpha < 0.004) { discard; }
        let color = apply_fog(surface.color * frame.sky_cloud_pd2.w, in.world_pos);
        return vec4<f32>(color * surface.alpha, surface.alpha);
    }
    // RiggedBlendDepth::Split: the texels at least half covered, then the
    // fainter ones, by the alpha of fs_depth_contact (the occlusion's depth
    // holds the same ones); tested before the material, which the other
    // pass's texels then never evaluate
    if (BLEND_SPLIT != 0u) {
        let rec = records[in.record];
        let half = sample_tex(rec.tex.x, in.uv).a * rec.base_color.a >= 0.5;
        if ((BLEND_SPLIT == 1u) != half) {
            discard;
        }
    }
    let m = eval_material(in, front);
    if (BLEND_EARLY_DISCARD && m.color.a < 0.004) {
        discard;
    }
    // Blended and alpha-tested: the hair, eyelashes and skirt of a system
    // avatar (LLDrawPoolAvatar::beginPostDeferredPass, minimum alpha).
    let rec = records[in.record];
    if ((rec.flags.x & FLAG_ALPHA_MASK) != 0u && m.color.a < rec.params.w) {
        discard;
    }
    var c = shade_material(in, m).color;
    c.a = c.a * rez_alpha(rec, in);
    if (c.a < 0.004) {
        discard;
    }
    // premultiplied alpha
    return vec4<f32>(c.rgb * c.a, c.a);
}

// Glow amount into the scene alpha (additive, alpha channel only), drawn
// for faces with glow after the scene: emissiveF.glsl (texture alpha x TE
// glow, the face alpha is ignored: invisible faces still glow) and
// pbrglowF.glsl (max emissive component x glow). LL draws them with its
// usual culling: a face seen from behind adds nothing, or the far side of a
// translucent shell (hair, fur, an aura) would glow through its near side
// without being dimmed by it.
@fragment
fn fs_glow(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let portal_aa = max(length(fwidth(in.uv)) * 2.0 * PORTAL_SURFACE_SCALE, 0.001);
    if ((records[in.record].flags.x & FLAG_PORTAL_EFFECT) != 0u) {
        let surface = portal_surface(records[in.record], in.uv, portal_aa);
        if (surface.glow <= 0.0) { discard; }
        return vec4<f32>(0.0, 0.0, 0.0, surface.glow);
    }
    rez_fw = fwidth(in.world_pos);
    let rec = records[in.record];
    portal_clip(rec, in.world_pos);
    if (!front && (rec.flags.x & FLAG_DOUBLE_SIDED) == 0u) {
        discard;
    }
    let base = sample_tex(rec.tex.x, in.uv);
    var a: f32;
    if ((rec.flags.x & FLAG_PBR) != 0u) {
        if ((rec.flags.x & FLAG_ALPHA_MASK) != 0u && base.a * rec.base_color.a < rec.params.w) {
            discard;
        }
        let e = rec.emissive.rgb * srgb_to_linear(sample_tex(rec.tex.w, in.uv_e).rgb);
        a = max(e.r, max(e.g, e.b)) * rec.emissive.w;
    } else {
        a = base.a * rec.emissive.w;
    }
    a = max(a, rez_glow(rec, in));
    if (a <= 0.0) {
        discard;
    }
    return vec4<f32>(0.0, 0.0, 0.0, a);
}

// Debug overlays ("Show transparency", wireframe): one flat
// translucent color per pipeline.
override debug_r: f32 = 1.0;
override debug_g: f32 = 0.0;
override debug_b: f32 = 0.0;
override debug_a: f32 = 0.35;

@fragment
fn fs_debug_flat(in: VsOut) -> @location(0) vec4<f32> {
    portal_clip(records[in.record], in.world_pos);
    return vec4<f32>(debug_r, debug_g, debug_b, debug_a);
}

@fragment
fn fs_hud_debug(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    portal_clip(records[in.record], in.world_pos);
    if (!front && (records[in.record].flags.x & FLAG_DOUBLE_SIDED) == 0u) { discard; }
    return vec4<f32>(debug_r, debug_g, debug_b, debug_a);
}

// One texel per 4 m ParcelOverlay cell. Explicit LOD and texel centres
// preserve ownership and the west/south boundary flags at any distance.
fn parcel_cell(rec: DrawRecord, cell: vec2<f32>) -> vec4<f32> {
    let clamped = clamp(cell, vec2<f32>(0.0), rec.uv_st.xy - vec2<f32>(1.0));
    return sample_tex_level(rec.tex.x, (clamped + vec2<f32>(0.5)) / rec.uv_st.xy, 0.0);
}

@fragment
fn fs_land_overlay(in: VsOut) -> @location(0) vec4<f32> {
    let rec = records[in.record];
    let local = (in.world_pos.xy - rec.model[3].xy) / 4.0;
    let width = max(vec2<f32>(0.0625 / 4.0), fwidth(local) * 0.6);
    let cell = min(floor(local), rec.uv_st.xy - vec2<f32>(1.0));
    let sample = parcel_cell(rec, cell);
    let byte = u32(round(sample.a * 255.0));
    if (rec.params.x < 0.5) {
        return vec4<f32>(sample.rgb, 0.3);
    }
    let owner = byte & 7u;
    if (owner == 0u || owner > 5u) { discard; }
    let within = local - cell;
    let east = u32(round(parcel_cell(rec, cell + vec2<f32>(1.0, 0.0)).a * 255.0));
    let north = u32(round(parcel_cell(rec, cell + vec2<f32>(0.0, 1.0)).a * 255.0));
    // LINE_WIDTH = 0.0625 m, on the inside of each parcel's boundary.
    let west_line = (byte & 64u) != 0u && within.x < width.x;
    let south_line = (byte & 128u) != 0u && within.y < width.y;
    let east_line = ((east & 64u) != 0u || cell.x >= rec.uv_st.x - 1.0) && within.x > 1.0 - width.x;
    let north_line = ((north & 128u) != 0u || cell.y >= rec.uv_st.y - 1.0) && within.y > 1.0 - width.y;
    if (!(west_line || south_line || east_line || north_line)) { discard; }
    return vec4<f32>(sample.rgb, 0.9);
}

// Selection is a separate coverage mask, never an interior grid. Keep
// masked texture holes but retain invisible faces for editing their TE.
@fragment
fn fs_selection_mask(in: VsOut) -> @location(0) vec4<f32> {
    portal_clip(records[in.record], in.world_pos);
    let rec = records[in.record];
    if ((rec.flags.x & FLAG_ALPHA_MASK) != 0u) {
        let alpha = sample_tex(rec.tex.x, in.uv).a * rec.base_color.a;
        if (rec.base_color.a >= 0.004 && alpha < rec.params.w) { discard; }
    }
    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
}

// Glow suppression of a face LL draws in its alpha pool: the glow under it
// is scaled by (1 - its alpha) before it adds its own (fs_glow).
@fragment
fn fs_glow_suppress(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    portal_clip(records[in.record], in.world_pos);
    let rec = records[in.record];
    if (!front && (rec.flags.x & FLAG_DOUBLE_SIDED) == 0u) {
        discard;
    }
    let a = sample_tex(rec.tex.x, in.uv).a * rec.base_color.a;
    if (a <= 0.0) {
        discard;
    }
    return vec4<f32>(0.0, 0.0, 0.0, a);
}

// ---------------------------------------------- what is drawn under a pixel

// The faces of the view drawn once more through a camera narrowed to a
// clicked pixel (drawn_pick.rs): placed and posed by the same code as the
// scene, so a rigged mesh is found where it is shown.
struct PickOut {
    @invariant @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // surface coordinates before the texture transform (a touch's ST)
    @location(1) st: vec2<f32>,
    @location(2) @interpolate(flat) record: u32,
    @location(3) world_pos: vec3<f32>,
};

@vertex
fn vs_pick(in: VsIn) -> PickOut {
    let rec = records[in.instance];
    let model = eye_model(rec, in);
    let ep = model * vec4<f32>(vertex_position(rec, in), 1.0);
    var out: PickOut;
    out.clip = frame.eye_view_proj * ep;
    let st = face_st(in, rec);
    out.st = st;
    out.uv = diffuse_uv(st, rec, frame.anim_clock.zw);
    out.record = in.instance;
    out.world_pos = ep.xyz + frame.eye.xyz;
    return out;
}

// The transparency overlay shows full triangles, independent of alpha.
// LLViewerWindow::pickAsync / pickImmediate include them while debug alpha
// is shown, so camera focus must stop at that displayed surface too.
@fragment
fn fs_pick_overlay(in: PickOut) -> @location(0) vec4<u32> {
    portal_clip(records[in.record], in.world_pos);
    return vec4<u32>(in.record + 1u, bitcast<u32>(in.clip.z), bitcast<u32>(in.st.x), bitcast<u32>(in.st.y));
}

// Record + 1, depth and surface coordinates: Mask keeps its displayed
// cutout; Blend always intercepts the click, including zero-alpha texels.
@fragment
fn fs_pick(in: PickOut) -> @location(0) vec4<u32> {
    let rec = records[in.record];
    portal_clip(rec, in.world_pos);
    if ((rec.flags.x & FLAG_ALPHA_MASK) != 0u) {
        // (the level the scene samples: each texel of the target is a
        // pixel of the screen)
        let a = sample_tex(rec.tex.x, in.uv).a * rec.base_color.a;
        if (a < rec.params.w) {
            discard;
        }
    }
    return vec4<u32>(in.record + 1u, bitcast<u32>(in.clip.z), bitcast<u32>(in.st.x), bitcast<u32>(in.st.y));
}

// LLToolCamera::handleMouseDown / LLImageGL::updatePickMask (LGPL 2.1):
// Alt-focus skips transparent texture holes, even on a scripted attachment.
// Test the texture's alpha, not its tint: a faint but solid glass pane is
// still a surface. Invisible TE faces are already absent from the draw lists.
@fragment
fn fs_pick_visible(in: PickOut) -> @location(0) vec4<u32> {
    let rec = records[in.record];
    portal_clip(rec, in.world_pos);
    // An emissive mask uses texture alpha for lighting, not opacity
    // (eval_material): its visible surface must remain a camera target.
    let terrain_pbr = (rec.flags.x & (FLAG_TERRAIN | FLAG_PBR)) == (FLAG_TERRAIN | FLAG_PBR);
    let hidden_texel = (rec.flags.x & FLAG_EMISSIVE_MASK) == 0u && sample_tex(select(rec.tex.x, 0u, terrain_pbr), in.uv).a <= 32.0 / 255.0;
    // An opaque glTF material ignores its base color alpha (FaceDraw::
    // fully_transparent): drawn, it stays a target.
    let gltf_opaque = (rec.flags.x & FLAG_PBR) != 0u && (rec.flags.x & (FLAG_ALPHA_MASK | FLAG_ALPHA_BLEND)) == 0u;
    if (hidden_texel || (rec.base_color.a < 0.004 && !gltf_opaque)) {
        discard;
    }
    return vec4<u32>(in.record + 1u, bitcast<u32>(in.clip.z), bitcast<u32>(in.st.x), bitcast<u32>(in.st.y));
}

// ------------------------------------------------------------- shadow pass

struct ShadowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) record: u32,
    @location(2) world_pos: vec3<f32>,
};

struct ShadowParams {
    vp: mat4x4<f32>,
    anim_clock: vec4<u32>,  // the animation clocks, as Frame's
};

@group(3) @binding(0) var<uniform> shadow_params: ShadowParams;
@group(3) @binding(1) var shadow_clamp: sampler;

@vertex
fn vs_shadow(in: VsIn) -> ShadowOut {
    let rec = records[in.instance];
    let wp = skinned_model(rec, in) * vec4<f32>(vertex_position(rec, in), 1.0);
    var out: ShadowOut;
    out.clip = shadow_params.vp * wp;
    out.uv = diffuse_uv(face_st(in, rec), rec, shadow_params.anim_clock.zw);
    out.record = in.instance;
    out.world_pos = wp.xyz;
    return out;
}

// Transparent texels cast no shadow; semi-transparent texels use alternating
// shadow-map columns. Opaque material modes ignore the texture alpha.
// Blended faces (a blended face that is also alpha-tested casts like a
// blended one), as LLPipeline::renderAlphaObjects (pipeline.cpp, LGPL 2.1)
// draws them:
// - static: shadowAlphaMaskF.glsl with ALPHA_BLEND_CUTOFF, the texture alpha
//   first (a pane under 0.598 casts nothing, its opaque frame does), then
//   times the face alpha: none under 0.05, dithered under 0.88;
// - glTF: pbrShadowAlphaBlendF.glsl, the same without the texture test;
// - rigged and avatars: dithered under 0.598, as avatarAlphaShadowF.glsl
//   (LLDrawPoolAvatar::renderShadow).
@fragment
fn fs_shadow(in: ShadowOut) {
    let rec = records[in.record];
    portal_clip(rec, in.world_pos);
    let tex_alpha = sample_tex(rec.tex.x, in.uv).a;
    let alpha = tex_alpha * rec.base_color.a;
    if ((rec.flags.x & (FLAG_ALPHA_MASK | FLAG_ALPHA_BLEND)) != 0u) {
        if ((rec.flags.x & FLAG_ALPHA_BLEND) == 0u) {
            if (alpha < rec.params.w) { discard; }
        } else {
            let pbr = (rec.flags.x & FLAG_PBR) != 0u;
            let skinned = (rec.flags.x & FLAG_SKINNED) != 0u;
            if (!pbr && !skinned && tex_alpha < 0.598) { discard; }
            if (alpha < 0.05) { discard; }
            let cutoff = select(0.88, 0.598, skinned && !pbr);
            if (alpha < cutoff && (u32(in.clip.x) % 2u) == 0u) { discard; }
        }
    }
}

// --------------------------------------------------------------- particles

const PART_EMISSIVE: u32 = 1u;
const PART_ADDITIVE: u32 = 2u;
const PART_AXIS: u32 = 4u;
const PART_FOLLOW_VELOCITY: u32 = 8u;

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
    if ((in.flags & PART_FOLLOW_VELOCITY) != 0u) {
        // LLVOPartGroup::getGeometry projects velocity into the billboard
        // plane, then rotates it without shortening the requested scale.
        // A world-space velocity axis can otherwise turn a long, thin
        // particle edge-on to the camera and make it disappear.
        let to_cam = normalize(frame.camera_pos.xyz - in.pos);
        let view_right = cross(vec3<f32>(0.0, 0.0, 1.0), to_cam);
        if (dot(view_right, view_right) > 1e-8) {
            right = normalize(view_right);
            up = normalize(cross(to_cam, right));
        }
        let projected = vec2<f32>(dot(in.axis, right), dot(in.axis, up));
        if (dot(projected, projected) > 1e-8) {
            let direction = normalize(projected);
            let old_right = right;
            right = right * direction.y - up * direction.x;
            up = old_right * direction.x + up * direction.y;
        }
        offset = right * c.x * in.size.x * 0.5 + up * c.y * in.size.y * 0.5;
    } else if ((in.flags & PART_AXIS) != 0u && dot(in.axis, in.axis) > 1e-8) {
        // Ribbon segments retain their world-space endpoints.
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

// High bit of a card's tile: its picture is all fullbright (the silhouette
// of an avatar over the complexity limit).
const IMPOSTOR_FULLBRIGHT: u32 = 0x80000000u;

struct SpriteOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world_pos: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) @interpolate(flat) fullbright: u32,
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
    let tile = in.tile & ~IMPOSTOR_FULLBRIGHT;
    let origin = vec2<f32>(f32(tile % per), f32(tile / per));
    var out: SpriteOut;
    out.clip = frame.view_proj * vec4<f32>(p, 1.0);
    out.uv = (origin + vec2<f32>(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5)) / impostor_tiles_per_row;
    out.world_pos = p;
    out.normal = normalize(cross(in.right, in.up));
    out.fullbright = in.tile & IMPOSTOR_FULLBRIGHT;
    return out;
}

fn impostor_color(in: SpriteOut) -> vec4<f32> {
    let t = textureSample(impostor_atlas, impostor_sampler, in.uv);
    if (t.a < 0.5) {
        discard;
    }
    // the picture was taken lit, without haze or fog: the card takes those
    // of its real distance, as `shade` gives them to the avatar drawn in
    // full (the haze of a classic sky, which a fullbright face does not
    // take; the fog of the others)
    var col = t.rgb / max(t.a, 1e-3);
    if (is_classic_sky()) {
        if (in.fullbright == 0u) {
            col = ll_haze(col, ll_atmos(in.world_pos - frame.camera_pos.xyz));
        }
    }
    col*=select(vec3<f32>(1.0),water_scene_light(in.world_pos),in.fullbright==0u);
    if(!is_classic_sky() || camera_underwater() || (water_fx_on(256u) && in.world_pos.z<camera_water_level())){col=apply_fog(col,in.world_pos);}
    return vec4<f32>(col, 1.0);
}

// The card's depth alone (contact shadows): where its picture is covered.
@fragment
fn fs_impostor_depth(in: SpriteOut) {
    if (textureSample(impostor_atlas, impostor_sampler, in.uv).a < 0.5) {
        discard;
    }
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
    if ((rec.flags.x & FLAG_ALPHA_MASK) != 0u && s.color.a < rec.params.w) {
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
