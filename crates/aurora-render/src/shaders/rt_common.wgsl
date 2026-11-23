// What the ray-traced passes share (rt_shadows.rs): the scene's tables as
// a ray's hit reads them, the placement of a vertex as the vertex shaders
// place it (object.wgsl `placed_model`: palette times inverse bind, blended
// over four joints; a worn prim placed by its anchor; without the live
// morphs), and the surface under a hit (its record, normal, texture
// coordinates and base colour). Prepended to each ray-traced shader.
//
// Group 1 is the scene, group 2 the renderer's bindless textures.
//
// `SHADOW_CULL`, put before this file (rt_shadows.rs `source`): the ray
// flag of the shadow rays' culling (rt_shadows.rs `Cull`), by default the
// faces met from their front, so that a one-sided face casts only from its
// side turned to the light (#3895). The instances' flags (rt_shadows.rs
// `facing`) keep a face drawn on both sides and a mirrored one right.

const FLAG_FULLBRIGHT: u32 = 1u;
const FLAG_ALPHA_MASK: u32 = 2u;
const FLAG_ALPHA_BLEND: u32 = 4u;
const FLAG_PBR: u32 = 8u;
const FLAG_PLANAR: u32 = 32u;
const FLAG_TERRAIN: u32 = 64u;
const FLAG_TEX_ANIM: u32 = 1024u;
const FLAG_SKINNED: u32 = 128u;
const FLAG_ANCHORED: u32 = 4096u;
const SKELETON_ANCHOR: u32 = 208u;
// vec4s of a draw record (types::DrawRecord) and where its parts start
const RECORD_VEC4S: u32 = 19u;
const RECORD_BASE_COLOR: u32 = 4u;
const RECORD_EMISSIVE: u32 = 5u;
const RECORD_UV_ST: u32 = 6u;
const RECORD_PARAMS: u32 = 7u;
const RECORD_TEX: u32 = 8u;
const RECORD_FLAGS: u32 = 9u;
const RECORD_ANIM: u32 = 14u;
const RECORD_ANIM_XF: u32 = 15u;
// u32 words of an arena vertex and of a skin vertex (types.rs)
const VERTEX_WORDS: u32 = 6u;
const SKIN_WORDS: u32 = 3u;

struct SkinBind {
    inverse_bind: mat4x4<f32>,
    joint: vec4<u32>,
};

// gpu_cull::CullFace: a record's geometry in the arena, its first fields
// (`cull_face`)
struct CullFace {
    index_count: u32,
    first_index: u32,
    base_vertex: i32,
    object: u32,
    bits: u32,
};

// The faces are read as words, `CULL_FACE_WORDS` a face (rt_shadows.rs
// `source`: the Rust struct's size), so that fields added after these
// (the culling's own) leave the rays' reading right. A WGSL copy of the
// whole struct read the faces at the wrong stride once the culling's grew:
// each hit took another face's triangle, its normal and its texture
// coordinates, and the alpha-masked leaves cast shadows in shards.
fn cull_face(record: u32) -> CullFace {
    let at = record * CULL_FACE_WORDS;
    return CullFace(
        cull_faces[at],
        cull_faces[at + 1u],
        bitcast<i32>(cull_faces[at + 2u]),
        cull_faces[at + 3u],
        cull_faces[at + 4u],
    );
}

@group(1) @binding(0) var<storage, read> records: array<vec4<u32>>;
@group(1) @binding(1) var<storage, read> palettes: array<mat4x4<f32>>;
@group(1) @binding(2) var<storage, read> skin_binds: array<SkinBind>;
@group(1) @binding(3) var<storage, read> vertices: array<u32>;
@group(1) @binding(4) var<storage, read> skin: array<u32>;
@group(1) @binding(5) var<storage, read> indices: array<u32>;
@group(1) @binding(6) var<storage, read> cull_faces: array<u32>;
// The records of the posed top level's faces: an instance's custom index
// is where its skeleton's start, its geometries are its faces in order.
@group(1) @binding(7) var<storage, read> posed_records: array<u32>;
// The tables of the scene's merged cells (rt_shadows.rs `Cell`,
// `cell_table`; `hit_of` below): the small still faces of a cell share one
// level, in world space.
@group(1) @binding(8) var<storage, read> merged_records: array<u32>;
// The texture animations' clock, the shading's (renderer.rs `anim_now`
// zw, object.wgsl `tex_anim_st`'s `clock`): milliseconds, then the bits of
// their fraction (rt_shadows.rs `write_texture_clock`).
@group(1) @binding(9) var<storage, read> tex_clock: vec4<u32>;
// The bit of the scene's custom index that says a merged cell.
const MERGED: u32 = 0x800000u;
// The instances' cull masks (rt_shadows.rs `SOLID`, `GLASS`): the opaque
// and alpha-masked faces, which stop a ray, and the blended ones, which
// only the shadow rays meet, to colour what passes (`glass_tint`).
const SOLID: u32 = 1u;
const GLASS: u32 = 2u;

@group(2) @binding(0) var tex_pages: binding_array<texture_2d_array<f32>>;
@group(2) @binding(1) var tex_sampler: sampler;
struct TextureLocations {
    page_count: u32,
    values: array<u32>,
}
@group(2) @binding(2) var<storage, read> tex_loc: TextureLocations;

fn record_vec(record: u32, part: u32) -> vec4<u32> {
    return records[record * RECORD_VEC4S + part];
}

fn record_vecf(record: u32, part: u32) -> vec4<f32> {
    return bitcast<vec4<f32>>(record_vec(record, part));
}

fn record_model(record: u32) -> mat4x4<f32> {
    let base = record * RECORD_VEC4S;
    return mat4x4<f32>(
        bitcast<vec4<f32>>(records[base]),
        bitcast<vec4<f32>>(records[base + 1u]),
        bitcast<vec4<f32>>(records[base + 2u]),
        bitcast<vec4<f32>>(records[base + 3u]),
    );
}

fn index_at(i: u32) -> u32 {
    let word = indices[i >> 1u];
    return select(word & 0xffffu, word >> 16u, (i & 1u) == 1u);
}

fn vertex_pos(v: u32) -> vec3<f32> {
    let w = v * VERTEX_WORDS;
    return vec3<f32>(bitcast<f32>(vertices[w]), bitcast<f32>(vertices[w + 1u]), bitcast<f32>(vertices[w + 2u]));
}

fn vertex_normal(v: u32) -> vec3<f32> {
    return unpack4x8snorm(vertices[v * VERTEX_WORDS + 3u]).xyz;
}

fn vertex_uv(v: u32) -> vec2<f32> {
    let w = v * VERTEX_WORDS;
    return vec2<f32>(bitcast<f32>(vertices[w + 4u]), bitcast<f32>(vertices[w + 5u]));
}

fn joint_matrix(flags: vec4<u32>, j: u32) -> mat4x4<f32> {
    let b = skin_binds[flags.z + j];
    return palettes[flags.y + b.joint.x] * b.inverse_bind;
}

// Where the vertex shaders put vertex `v` of `record` (object.wgsl
// `placed_model`).
fn placed_matrix(record: u32, v: u32) -> mat4x4<f32> {
    let flags = record_vec(record, RECORD_FLAGS);
    if ((flags.x & FLAG_SKINNED) != 0u) {
        let s = v * SKIN_WORDS;
        let jw = skin[s];
        let joints = vec4<u32>(jw & 0xffu, (jw >> 8u) & 0xffu, (jw >> 16u) & 0xffu, jw >> 24u);
        let weights = vec4<f32>(
            f32(skin[s + 1u] & 0xffffu),
            f32(skin[s + 1u] >> 16u),
            f32(skin[s + 2u] & 0xffffu),
            f32(skin[s + 2u] >> 16u),
        ) / 65535.0;
        let ws = max(weights.x + weights.y + weights.z + weights.w, 1e-4);
        var sm = joint_matrix(flags, joints.x) * weights.x;
        if (weights.y > 0.0) { sm += joint_matrix(flags, joints.y) * weights.y; }
        if (weights.z > 0.0) { sm += joint_matrix(flags, joints.z) * weights.z; }
        if (weights.w > 0.0) { sm += joint_matrix(flags, joints.w) * weights.w; }
        sm = sm * (1.0 / ws);
        if ((flags.x & FLAG_ANCHORED) != 0u) {
            return palettes[flags.y + SKELETON_ANCHOR] * sm;
        }
        return record_model(record) * sm;
    }
    if ((flags.x & FLAG_ANCHORED) != 0u) {
        return palettes[flags.y] * record_model(record);
    }
    return record_model(record);
}

// The surface under a hit.
struct Surface {
    record: u32,
    // world normal, facing the ray's origin
    normal: vec3<f32>,
    uv: vec2<f32>,
    // the ray met it from behind: its normal was turned to face the origin
    back: bool,
};

// The surface of triangle `primitive` of `record` at `bary` (the hit's
// barycentrics, of its second and third vertices), seen along `dir`.
fn surface_at(record: u32, primitive: u32, bary: vec2<f32>, dir: vec3<f32>) -> Surface {
    let face = cull_face(record);
    let first = face.first_index + primitive * 3u;
    let base = u32(max(face.base_vertex, 0));
    let v0 = base + index_at(first);
    let v1 = base + index_at(first + 1u);
    let v2 = base + index_at(first + 2u);
    let w = vec3<f32>(1.0 - bary.x - bary.y, bary.x, bary.y);
    let m = placed_matrix(record, v0);
    let local_n = vertex_normal(v0) * w.x + vertex_normal(v1) * w.y + vertex_normal(v2) * w.z;
    var n = normalize((m * vec4<f32>(local_n, 0.0)).xyz + vec3<f32>(0.0, 0.0, 1e-6));
    var s: Surface;
    s.back = dot(n, dir) > 0.0;
    if (s.back) {
        n = -n;
    }
    s.record = record;
    s.normal = n;
    s.uv = uv_of(record, v0, v1, v2, w);
    return s;
}

// The texture coordinates at barycentric weights `w` of a triangle of
// `record`: `surface_at`'s, and `uv_at`'s for the alpha test, the same
// expression for both so that the alpha test decides as the shading reads.
// A planar-mapped face takes them from its position (object.wgsl
// `face_st`; a HUD's zoom aside, no HUD is traced).
fn uv_of(record: u32, v0: u32, v1: u32, v2: u32, w: vec3<f32>) -> vec2<f32> {
    if ((record_vec(record, RECORD_FLAGS).x & FLAG_PLANAR) != 0u) {
        let local_p = vertex_pos(v0) * w.x + vertex_pos(v1) * w.y + vertex_pos(v2) * w.z;
        let local_n = vertex_normal(v0) * w.x + vertex_normal(v1) * w.y + vertex_normal(v2) * w.z;
        return planar_st(local_p, local_n, record_model(record));
    }
    return vertex_uv(v0) * w.x + vertex_uv(v1) * w.y + vertex_uv(v2) * w.z;
}

// `surface_at`'s texture coordinates alone: the alpha test of a candidate
// hit (`stops`) needs no normal, and every traced ray near the avatars
// tests many (their hair, their clothes' cut-outs).
fn uv_at(record: u32, primitive: u32, bary: vec2<f32>) -> vec2<f32> {
    let face = cull_face(record);
    let first = face.first_index + primitive * 3u;
    let base = u32(max(face.base_vertex, 0));
    let w = vec3<f32>(1.0 - bary.x - bary.y, bary.x, bary.y);
    return uv_of(record, base + index_at(first), base + index_at(first + 1u), base + index_at(first + 2u), w);
}

// The face of a hit, and the hit's triangle in that face.
struct Hit {
    record: u32,
    primitive: u32,
};

// The face of triangle `primitive` of an instance's geometry `geometry`. A
// placed face's instance carries its record. A posed skeleton's carries
// where its faces' records start, its geometries being its faces in order.
// A merged cell's carries where its table starts in `merged_records`: the
// number of its opaque faces and of its alpha-masked ones, then for each
// face, in that order, its first triangle in its geometry and its record
// (rt_shadows.rs `cell_table`). The cell's geometries are its opaque faces
// together, then its masked ones (one alone when it has one kind): the
// face is the last that starts at or before the triangle. A cell part has
// 4,096 faces at most (`PART_FACES`): twelve steps.
fn hit_of(posed: bool, custom: u32, geometry: u32, primitive: u32) -> Hit {
    if (posed) {
        return Hit(posed_records[custom + geometry], primitive);
    }
    if ((custom & MERGED) == 0u) {
        return Hit(custom, primitive);
    }
    let head = custom & (MERGED - 1u);
    let opaque = merged_records[head];
    let masked = geometry == 1u || opaque == 0u;
    let base = head + 2u + select(0u, 2u * opaque, masked);
    var lo = 0u;
    var hi = select(opaque, merged_records[head + 1u], masked);
    for (var step = 0u; step < 16u && hi > lo + 1u; step++) {
        let mid = (lo + hi) / 2u;
        if (merged_records[base + 2u * mid] <= primitive) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let at = base + 2u * lo;
    return Hit(merged_records[at + 1u], primitive - merged_records[at]);
}

fn texture_page_index(loc: u32) -> u32 {
    let page = loc >> 16u;
    let last_bound_page = max(tex_loc.page_count, 1u) - 1u;
    return select(page, 0u, page > last_bound_page);
}

fn sample_tex_level(idx: u32, uv: vec2<f32>, lod: f32) -> vec4<f32> {
    let loc = tex_loc.values[idx];
    return textureSampleLevel(tex_pages[texture_page_index(loc)], tex_sampler, uv, loc & 0xffffu, lod);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3<f32>(0.0)) + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

// A PBR map's coordinates (object.wgsl `pbr_uv`): `so` its scale and
// offset, `rot` its rotation.
fn pbr_uv(uv: vec2<f32>, so: vec4<f32>, rot: f32) -> vec2<f32> {
    let c = cos(rot);
    let sn = sin(rot);
    let v = vec2<f32>(uv.x, 1.0 - uv.y) * so.xy;
    return vec2<f32>(c * v.x + sn * v.y, -sn * v.x + c * v.y) + so.zw;
}

// The diffuse map's coordinates (object.wgsl `diffuse_uv`: the texture
// animation at the shading's time, then `te_uv`, `ll_uv` / `pbr_uv`), from
// the face's own (`surface_at`: planar mapping is in them).
fn diffuse_uv(record: u32, uv: vec2<f32>) -> vec2<f32> {
    let so = record_vecf(record, RECORD_UV_ST);
    let rot = record_vecf(record, RECORD_PARAMS).x;
    let flags = record_vec(record, RECORD_FLAGS).x;
    var st = uv;
    if ((flags & FLAG_TEX_ANIM) != 0u) {
        // (an animated legacy face's animation holds its texture entry's
        // transform)
        let a = tex_anim_st(uv, anim_record(record), tex_clock.xy);
        if ((flags & FLAG_PBR) == 0u) {
            return vec2<f32>(a.x, 1.0 - a.y);
        }
        st = a;
    }
    let c = cos(rot);
    let sn = sin(rot);
    if ((flags & FLAG_PBR) != 0u) {
        return pbr_uv(st, so, rot);
    }
    var s = st.x - 0.5;
    var t = st.y - 0.5;
    let tmp = s;
    s = s * c + t * sn;
    t = -tmp * sn + t * c;
    return vec2<f32>(s * so.x + so.z + 0.5, 1.0 - (t * so.y + so.w + 0.5));
}

// The texture animation and the planar mapping of a face, as the shading
// draws them: object.wgsl's functions, taken here as they are
// (rt_shadows.rs `rt_texture_coordinates_are_the_shadings`), on the parts
// of a draw record they read, under the shading's name. The rays read a
// face's texture where the eye sees it: the sea's animated foam cast the
// shadow of its unmoved texture, where no foam was shown (2026-11-22, grid
// report of beta7).
struct DrawRecord {
    flags: vec4<u32>,
    anim: vec4<u32>,
    anim_xf: vec4<f32>,
};

fn anim_record(record: u32) -> DrawRecord {
    return DrawRecord(record_vec(record, RECORD_FLAGS), record_vec(record, RECORD_ANIM), record_vecf(record, RECORD_ANIM_XF));
}

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

// A mip past any texture's last: its 1×1, the texture's mean colour.
const MEAN_LOD: f32 = 16.0;

// The four layers' shares of a terrain at the composition `c` (0 to 3),
// as env.wgsl `terrain_weights` mixes them: mix(mix(d3, d2, R(c - 2)),
// mix(d1, d0, R(c)), R(c - 1)), its ramp R (the Legacy Viewer's
// alpha_gradient.tga, eased at both ends) taken as a straight line, which
// stays within 0.05 of it. The PBR terrain's ramp shifts by a noise along
// the ground, a ray's hit takes its mean.
fn terrain_shares(c: f32) -> vec4<f32> {
    let x = clamp(c, 0.0, 3.0);
    let a1 = clamp(1.0 - x, 0.0, 1.0);
    let a2 = clamp(3.0 - x, 0.0, 1.0);
    let af = clamp(2.0 - x, 0.0, 1.0);
    return vec4<f32>(af * a1, af * (1.0 - a1), (1.0 - af) * a2, (1.0 - af) * (1.0 - a2));
}

// The linear base colour of a terrain `record` where its composition is
// `c` (its vertices' first texture coordinate, world/terrain.rs
// `build_chunk`), for a ray's hit: the four layers' textures (the PBR
// terrain's base colour maps) taken at their mean colour, mixed by their
// shares, as env.wgsl `terrain_color` and `terrain_material` mix them on
// screen. The detail of the textures is left: the indirect light and the
// reflections it serves take a ground's colour over metres, and the hit
// has no place on the region for them. A paint map's shares are its mean
// too.
fn terrain_hit_color(record: u32, c: f32, pbr: bool) -> vec3<f32> {
    let layers = record_vec(record, RECORD_TEX);
    var w = terrain_shares(c);
    if (!pbr) {
        if (layers.x == 0u && layers.y == 0u) {
            // no detail textures: env.wgsl `terrain_color`'s flat palette
            let flat = vec3<f32>(0.62, 0.55, 0.40) * w.x + vec3<f32>(0.20, 0.32, 0.14) * w.y
                + vec3<f32>(0.30, 0.25, 0.18) * w.z + vec3<f32>(0.42, 0.42, 0.44) * w.w;
            return srgb_to_linear(flat) * 1.6;
        }
        let mixed = sample_tex_level(layers.x, vec2<f32>(0.5), MEAN_LOD).rgb * w.x
            + sample_tex_level(layers.y, vec2<f32>(0.5), MEAN_LOD).rgb * w.y
            + sample_tex_level(layers.z, vec2<f32>(0.5), MEAN_LOD).rgb * w.z
            + sample_tex_level(layers.w, vec2<f32>(0.5), MEAN_LOD).rgb * w.w;
        return srgb_to_linear(mixed);
    }
    let paint = record_vec(record, RECORD_FLAGS).z;
    if (paint != 0u) {
        // env.wgsl `terrain_mix_weights`: slots 2 to 4 in linear RGB, the
        // first the remainder
        var other = max(sample_tex_level(paint, vec2<f32>(0.5), MEAN_LOD).rgb, vec3<f32>(0.0));
        other /= max(1.0, dot(other, vec3<f32>(1.0)));
        w = vec4<f32>(max(0.0, 1.0 - dot(other, vec3<f32>(1.0))), other);
    } else {
        // (the material cutoff of `terrain_mix_weights`)
        w = max(w - vec4<f32>(0.1), vec4<f32>(0.0));
        w /= max(dot(w, vec4<f32>(1.0)), 1e-6);
    }
    var color = vec3<f32>(0.0);
    for (var i = 0u; i < 4u; i++) {
        if (w[i] <= 0.0) {
            continue;
        }
        // a layer is a material's record (terrain_surface.rs `layer_record`)
        let layer = layers[i];
        let tex = sample_tex_level(record_vec(layer, RECORD_TEX).x, vec2<f32>(0.5), MEAN_LOD).rgb;
        color += srgb_to_linear(tex) * record_vecf(layer, RECORD_BASE_COLOR).rgb * w[i];
    }
    return color;
}

// The linear base colour and alpha of `record` at `uv`, from its texture's
// mip `lod` (object.wgsl `eval_material_base`); a terrain's, by
// `terrain_hit_color`, is opaque (its masked layers are not traced yet).
fn base_color(record: u32, uv: vec2<f32>, lod: f32) -> vec4<f32> {
    let flags = record_vec(record, RECORD_FLAGS).x;
    if ((flags & FLAG_TERRAIN) != 0u) {
        return vec4<f32>(terrain_hit_color(record, uv.x, (flags & FLAG_PBR) != 0u), 1.0);
    }
    let tint = record_vecf(record, RECORD_BASE_COLOR);
    let tex = sample_tex_level(record_vec(record, RECORD_TEX).x, diffuse_uv(record, uv), lod);
    if ((flags & FLAG_PBR) != 0u) {
        return vec4<f32>(srgb_to_linear(tex.rgb), tex.a) * tint;
    }
    return vec4<f32>(srgb_to_linear(tex.rgb * tint.rgb), tex.a * tint.a);
}

// Whether an alpha-masked face lets a ray through at `uv`: under its
// cutoff (object.wgsl `mask_color`).
fn masked_out(record: u32, uv: vec2<f32>) -> bool {
    let flags = record_vec(record, RECORD_FLAGS).x;
    if ((flags & FLAG_ALPHA_MASK) == 0u) {
        return false;
    }
    return base_color(record, uv, 0.0).a < record_vecf(record, RECORD_PARAMS).w;
}

// The light an emissive face gives off (rgb, linear; without its emissive
// map).
fn emissive_color(record: u32) -> vec3<f32> {
    return record_vecf(record, RECORD_EMISSIVE).rgb;
}

// Whether a candidate hit stops the ray: an opaque face always (the
// traversal commits those itself), an alpha-masked one where its texture is
// over its cutoff. (Naga takes no ray query as a function's argument, so
// each trace runs its own loop around this.)
// `dir` is kept for the callers: the test reads no normal. (A face that is
// not alpha-masked stops it before its vertices are read; the texture
// coordinates are `surface_at`'s, so the same candidates stop.)
fn stops(c: RayIntersection, posed: bool, dir: vec3<f32>) -> bool {
    if (c.kind != RAY_QUERY_INTERSECTION_TRIANGLE) {
        return false;
    }
    let h = hit_of(posed, c.instance_custom_data, c.geometry_index, c.primitive_index);
    if ((record_vec(h.record, RECORD_FLAGS).x & FLAG_ALPHA_MASK) == 0u) {
        return true;
    }
    return !masked_out(h.record, uv_at(h.record, h.primitive, c.barycentrics));
}

// How far from a pixel of the view its own curved surface may still be met
// from behind (m): `shadows`.
const OWN_REACH: f32 = 4.0;

// Whether a candidate hit stops a shadow ray from a pixel of the view:
// `stops`, but for the pixel's own curved surface. `curved` says the
// pixel's flat triangle is turned from the light while its smooth normal
// still catches it, the band along a low-poly mesh's terminator: the ray
// then goes through the mesh as its triangles are, and each triangle of
// the band would be dark whole beside lit ones (the shadow terminator
// problem). What such a ray meets from behind by the smooth normals,
// within `OWN_REACH`, is that rounded surface itself, which the shading's
// own N·L already darkens: only what it meets from the front shadows it.
// (Such a ray is traced with RAY_FLAG_FORCE_NO_OPAQUE, so the opaque faces
// come here too.)
fn shadows(c: RayIntersection, posed: bool, dir: vec3<f32>, curved: bool) -> bool {
    return shadows_but(c, posed, dir, curved, 0.0);
}

// `shadows`, but a face of object `own` never does: the prim a lamp shines
// from, on the last stretch of its rays (rt_lights.wgsl `lamp_trace`).
// `own` is the object's index plus one (`CullFace::object`, as a float:
// rt_shadows.rs `traced_lights`), 0 for none.
fn shadows_but(c: RayIntersection, posed: bool, dir: vec3<f32>, curved: bool, own: f32) -> bool {
    if (c.kind != RAY_QUERY_INTERSECTION_TRIANGLE) {
        return false;
    }
    let h = hit_of(posed, c.instance_custom_data, c.geometry_index, c.primitive_index);
    if (own > 0.5 && f32(cull_face(h.record).object + 1u) == own) {
        return false;
    }
    let s = surface_at(h.record, h.primitive, c.barycentrics, dir);
    if (curved && s.back && c.t < OWN_REACH) {
        return false;
    }
    return !masked_out(h.record, s.uv);
}

// What a pane of glass (a blended face, the `GLASS` instances) that a
// shadow ray meets lets through of the light: its colour as far as its
// alpha covers it, times what its alpha leaves, mix(1, rgb, a) * (1 - a),
// panes in a row multiplying. The rule PR #3821 (circuskid) gives the
// point lights' shadow maps, here for the rays. Only where the texel casts
// at all (`glass_casts`), as the shadow maps' blended faces. 1 for a
// candidate that is no triangle.
fn glass_passes(c: RayIntersection, posed: bool, dir: vec3<f32>) -> vec3<f32> {
    if (c.kind != RAY_QUERY_INTERSECTION_TRIANGLE) {
        return vec3<f32>(1.0);
    }
    let h = hit_of(posed, c.instance_custom_data, c.geometry_index, c.primitive_index);
    let s = surface_at(h.record, h.primitive, c.barycentrics, dir);
    let b = base_color(h.record, s.uv, 0.0);
    if (!glass_casts(h.record, b.a)) {
        return vec3<f32>(1.0);
    }
    let a = clamp(b.a, 0.0, 1.0);
    return mix(vec3<f32>(1.0), clamp(b.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), a) * (1.0 - a);
}

// Whether a blended texel of `record`, of opacity `alpha` (its texture's
// times its colour's), casts a shadow: the test of the shadow maps'
// blended faces (object.wgsl `fs_shadow`, as LLPipeline::renderAlphaObjects
// draws them with shadowAlphaMaskF.glsl, indra/newview/pipeline.cpp,
// LGPL 2.1): a legacy face whose texture is under ALPHA_BLEND_CUTOFF
// (0.598) casts nothing, and no face under 0.05. A mostly transparent
// face, the sea's waves and foam, a veil, cast a shadow of their whole
// surface before (2026-11-22, grid report of beta6: a grid of dark squares
// on the sand under the waves). Over the cutoff the pane keeps its
// coloured light (#3821) where the maps dither it to half: what it lets
// through, (1 - a) times its tint, is a little less than their half from
// 0.6 to 0.88, and a little more than their none above.
fn glass_casts(record: u32, alpha: f32) -> bool {
    if (alpha < 0.05) {
        return false;
    }
    let flags = record_vec(record, RECORD_FLAGS).x;
    if ((flags & (FLAG_PBR | FLAG_SKINNED)) != 0u) {
        return true;
    }
    // the texture's own opacity, the colour's taken out of the texel's
    let tint = record_vecf(record, RECORD_BASE_COLOR).a;
    return alpha >= ALPHA_BLEND_CUTOFF * tint;
}

// LLPipeline's ALPHA_BLEND_CUTOFF (pipeline.cpp), object.wgsl `fs_shadow`'s.
const ALPHA_BLEND_CUTOFF: f32 = 0.598;

// A hash of `v` (PCG), for the passes' random numbers.
fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

// Two numbers in [0, 1) for pixel `p` on frame `frame`.
fn random2(p: vec2<u32>, frame: u32) -> vec2<f32> {
    let a = pcg(p.x + pcg(p.y + pcg(frame)));
    let b = pcg(a);
    return vec2<f32>(f32(a >> 8u), f32(b >> 8u)) / 16777216.0;
}
