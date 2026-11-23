// Ray-traced reflections of the main view (rt_shadows.rs `Reflect`): one
// mirror ray per pixel close to a mirror (`TRACED_ROUGH_MAX`), through
// the scene's and the posed faces' acceleration structures, nearest hit;
// and for the rougher ones, to a roughness of `GLOSS_ROUGH_MAX`, a ray
// spread as their roughness spreads the light, kept from frame to frame and
// filtered (`cs_gloss`, below).
// A hit the view sees takes its lit colour from the scene copy; one it does
// not see is shaded here (`hit_view`), and the two are mixed where the view
// half sees it: at the screen's edges and where the depth there leaves the
// hit, so that no line runs between them. ssr.wgsl's `fs_ssr_rt` blends the
// result with SSR's weights; a miss (alpha 0) leaves the probes' reflection.
// The hit shaded here takes the point lights too (rt_light.wgsl `hit_light`:
// a shadow ray to each that counts, where the view traces their shadows),
// and where it is itself a mirror the ray goes on from it once (`hit_sent`,
// `BOUNCES`).
// Not yet: the projectors, and the materials' maps past the base colour
// and a PBR face's packed one (`pbr_orm`).

// After rt_common.wgsl, rt_view.wgsl, rt_light.wgsl, rt_probes.wgsl,
// rt_history.wgsl and wgpu_ray_query's enable (rt_shadows.rs `source`).

struct Params {
    // This frame's projection, with the jitter the depth prepass was drawn
    // with, and its inverse.
    clip_from_world: mat4x4<f32>,
    world_from_clip: mat4x4<f32>,
    // xyz the eye, w 1 when `motion_tex` is the prepass's and not a black
    // stand-in, 2 when its motion is not read (a photo).
    camera: vec4<f32>,
    lit: HitLight,
    // width, height, 1 / width, 1 / height
    screen: vec4<f32>,
    // x: the rays' reach (m), y: 1 when the avatars' top level is traced,
    // z: the point lights (`lights`), w: 1 when their shadows are traced.
    mode: vec4<f32>,
    // `cs_gloss`: last frame's projection and eye.
    prev_clip_from_world: mat4x4<f32>,
    prev_camera: vec4<f32>,
    // half resolution: width, height; z: the frame's number, w: the frames
    // the glossy history averages at most.
    half: vec4<f32>,
    // x: 1 when last frame's glossy history is valid, y: the rays of a
    // glossy pixel this frame, z: 1 when a PBR face's occlusion counts
    // (AURORA_PBR_OCCLUSION, as the frame's `sky_ll.z`).
    gloss: vec4<f32>,
    // (rt_history.wgsl `light_changed`, which the glossy history does not
    // ask: a reflection shows the light of what it meets, not the pixel's)
    changed: array<vec4<f32>, 8>,
    changes: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
// oct normal xy, roughness, metallic (the G-buffer ssr.wgsl reads)
@group(0) @binding(2) var gbuf_tex: texture_2d<f32>;
// the lit scene before its blended faces (the scene copy)
@group(0) @binding(3) var scene_tex: texture_2d<f32>;
@group(0) @binding(4) var scene_tlas: acceleration_structure;
@group(0) @binding(5) var posed_tlas: acceleration_structure;
@group(0) @binding(6) var out_tex: texture_storage_2d<rgba16float, write>;
@group(0) @binding(7) var<storage, read> lights: array<Light>;
// the depth prepass's normals and motion (rt_history.wgsl): xy the mesh's
// smooth normal (rt_view.wgsl `mesh_normal`)
@group(0) @binding(8) var motion_tex: texture_2d<f32>;
// `cs_reflect`: the base colour of a metal pixel (`base_under`)
@group(0) @binding(9) var out_base: texture_storage_2d<rgba8unorm, write>;
// `cs_gloss`, at half resolution: last frame's glossy history (a its
// frames; rgb its mean, which `prev_fed` has once filtered), its guide
// (rt_denoise.wgsl) and the share of its rays that met something, and this
// frame's
@group(0) @binding(10) var prev_color: texture_2d<f32>;
@group(0) @binding(11) var prev_guide: texture_2d<f32>;
@group(0) @binding(12) var out_color: texture_storage_2d<rgba16float, write>;
@group(0) @binding(13) var out_guide: texture_storage_2d<rgba16float, write>;
@group(0) @binding(14) var prev_fed: texture_2d<f32>;
@group(0) @binding(15) var prev_cover: texture_2d<f32>;
@group(0) @binding(16) var out_cover: texture_storage_2d<r32float, write>;
// `cs_gloss_filter`: the history filtered (rt_denoise.wgsl), its guide and
// its cover, and the full resolution result (rgb, and the cover in a)
@group(0) @binding(17) var gloss_color: texture_2d<f32>;
@group(0) @binding(18) var gloss_guide: texture_2d<f32>;
@group(0) @binding(19) var gloss_cover: texture_2d<f32>;
@group(0) @binding(20) var out_gloss: texture_storage_2d<rgba16float, write>;
// the reflection probes, for the hits the view does not show (rt_probes.wgsl)
@group(0) @binding(21) var<storage, read> probe_set: ProbeSet;
@group(0) @binding(22) var probe_tex: texture_cube_array<f32>;
@group(0) @binding(23) var probe_sampler: sampler;
@group(0) @binding(24) var probe_irr_tex: texture_cube_array<f32>;

fn oct_decode(e_in: vec2<f32>) -> vec3<f32> {
    let e = e_in * 2.0 - vec2<f32>(1.0);
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>((vec2<f32>(1.0) - abs(n.yx)) * s, n.z);
    }
    return normalize(n);
}

// The scene copy's colour where the view sees `p`, a point of a surface
// facing `n` (rgb), and how surely it sees it (a): 0 off screen or behind
// something else, 1 on it, between the two over the screen's last twentieth
// and where the depth leaves `p`. The four pixels around `p` are weighed as
// a texture filter would, each by whether its depth is `p`'s: the eye's
// distance to within 5 cm and 1 %, more on a face seen at a slant, where
// a pixel's step along the face is as much deeper. The nearest of them
// alone makes the view sure of `p`: beside an outline, the pixels of the
// face in front leave the colour and take nothing from the sureness (the
// rim of a glossy face mirrors what is right behind it, and came in specks
// of the hit's own shading).
fn seen(p: vec3<f32>, n: vec3<f32>) -> vec4<f32> {
    let clip = params.clip_from_world * vec4<f32>(p, 1.0);
    if (clip.w <= 0.0) {
        return vec4<f32>(0.0);
    }
    let ndc = clip.xyz / clip.w;
    let edge = 1.0 - max(abs(ndc.x), abs(ndc.y));
    if (edge <= 0.0) {
        return vec4<f32>(0.0);
    }
    let size = vec2<i32>(params.screen.xy);
    let at = (ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5)) * params.screen.xy - vec2<f32>(0.5);
    let corner = vec2<i32>(floor(at));
    let f = at - floor(at);
    let to_p = p - params.camera.xyz;
    let dist = length(to_p);
    let facing = abs(dot(n, to_p / dist));
    // a pixel's width at `p`, and how much farther the face is a pixel on
    let across = pixel_width(corner, ndc.z);
    let slope = across * sqrt(max(1.0 - facing * facing, 0.0)) / max(facing, 0.05);
    let near = 0.05 + 0.01 * dist + slope;
    var sum = vec4<f32>(0.0);
    var sure = 0.0;
    for (var i = 0u; i < 4u; i++) {
        let o = vec2<i32>(i32(i & 1u), i32(i >> 1u));
        let px = corner + o;
        if (any(px < vec2<i32>(0)) || any(px >= size)) {
            continue;
        }
        let d = textureLoad(depth_tex, px, 0);
        if (d <= 0.0) {
            continue;
        }
        let off = abs(distance(unproject(px, d), params.camera.xyz) - dist);
        let share = mix(1.0 - f.x, f.x, f32(o.x)) * mix(1.0 - f.y, f.y, f32(o.y));
        let same = 1.0 - smoothstep(near, 1.5 * near, off);
        sum += vec4<f32>(textureLoad(scene_tex, px, 0).rgb, 1.0) * (share * same);
        // the nearest pixel has a quarter of the filter at least
        sure = max(sure, same * min(4.0 * share, 1.0));
    }
    if (sum.a <= 1e-4) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(sum.rgb / sum.a, sure * smoothstep(0.0, 0.1, edge));
}

// What a hit the view does not show sends back along the ray that met it
// (`dir`): its own light, and the share of what it mirrors, which the ray
// goes on to find along `reflect(dir, normal)`.
struct Sent {
    body: vec3<f32>,
    mirror: vec3<f32>,
};

// types.rs `DrawRecord`: `spec_uv`, `legacy` and `spec_color`;
// common.wgsl's flag
const RECORD_SPEC_UV: u32 = 11u;
const RECORD_LEGACY: u32 = 12u;
const RECORD_SPEC_COLOR: u32 = 13u;
const FLAG_LEGACY_MAT: u32 = 256u;
// The hits of a ray: a mirror met off the screen sends it on once.
const BOUNCES: u32 = 2u;
// The share of a pixel's reflection under which what a hit mirrors is the
// sky's hemisphere, no ray.
const MIRROR_MIN: f32 = 0.02;
// The roughness from which a surface takes no traced reflection, and the one
// from which its share fades (ssr.wgsl `traced_share`): a rougher surface
// keeps the probes' blurred image, which one mirror ray cannot give.
const TRACED_ROUGH_MAX: f32 = 0.15;
const TRACED_ROUGH_FADE: f32 = 0.1;
// The roughness to which a surface takes a traced reflection at all: the
// mirror's, then the glossy one, which ssr.wgsl `gloss_share` fades into
// the probes' towards it.
const GLOSS_ROUGH_MAX: f32 = 0.6;
// The brightest a glossy ray's light counts (`gloss_capped`): one ray in
// many that meets a lamp would stay a bright spot for as long as its
// history.
const GLOSS_SAMPLE_MAX: f32 = 8.0;
// Frames the glossy history averages at most on a surface that moved on its
// own, and at least on any.
const GLOSS_HISTORY_MOVING: f32 = 6.0;
const GLOSS_HISTORY_MIN: f32 = 2.0;
// The frames it averages at most for the narrowest spread, and the GGX
// roughness (the roughness squared) from which it is as long as it may: a
// narrow spread has less noise to average, and shows what moves in it
// sharply enough for a long history to trail it.
const GLOSS_HISTORY_NARROW: f32 = 8.0;
const GLOSS_HISTORY_ALPHA: f32 = 0.25;
// How many spreads' worth the reflection may turn over a history: what a
// glossy face shows moves across it as the eye does, and a history longer
// than that trails it.
const GLOSS_DRIFT: f32 = 2.0;

// A PBR face's occlusion, roughness and metallic at `uv` (x, y, z), as
// object.wgsl `eval_material_base` reads them: the red of its packed map,
// and its green and blue times the material's factors. A material that
// keeps its roughness and its metal in the map has factors of 1: by them
// alone it reflected as a rough metal whatever the map says. A face
// without the map reads the white texture. The terrain's record keeps its
// layers where a face keeps its maps: its factors alone.
fn pbr_orm(record: u32, uv: vec2<f32>, lod: f32) -> vec3<f32> {
    let factors = record_vecf(record, RECORD_PARAMS);
    var map = vec3<f32>(1.0);
    if ((record_vec(record, RECORD_FLAGS).x & FLAG_TERRAIN) == 0u) {
        let at = pbr_uv(uv, record_vecf(record, RECORD_SPEC_UV), record_vecf(record, RECORD_LEGACY).y);
        map = sample_tex_level(record_vec(record, RECORD_TEX).z, at, lod).rgb;
    }
    let occlusion = select(1.0, map.r, params.gloss.z > 0.5);
    return vec3<f32>(occlusion, clamp(factors.z * map.g, 0.03, 1.0), clamp(factors.y * map.b, 0.0, 1.0));
}

// A fullbright face sends its colour, unlit (object.wgsl `fs_main`); the
// others rt_light.wgsl's diffuse and what they mirror. A PBR face, by its
// packed map (`pbr_orm`): less a metal's diffuse, which has none, and the
// Fresnel share of common.wgsl `shade_pbr`, a mirror's as far as ssr.wgsl
// `reflection_weight` and `traced_share` make it one, the rest of it the
// probes' reflection there; the material's occlusion dims its ambient and
// what it mirrors, as `shade_pbr`'s `mat_ao`. The ambient is the probes'
// too (rt_probes.wgsl), as the shading's: inside a room neither is the open
// sky's. A legacy material, by its factors without its maps: object.wgsl
// `legacy_light`'s glossy environment and its environment (applyGlossEnv,
// applyLegacyEnv), which dims the face as it covers it; a face with neither
// mirrors nothing, as it is drawn.
// The sky's ambient a hit takes along its normal (rt_probes.wgsl
// `ambient_at`), darkened as the screen's where the traced global
// illumination's probes do not see the sky (gi_sample.wgsl `gi_sky`;
// common.wgsl `screen_sky`): a reflected room is as dark as the room.
// `ray`: from where the reflected ray left to the hit.
fn hit_ambient(hit: vec3<f32>, n: vec3<f32>, ray: vec3<f32>) -> vec3<f32> {
    return ambient_at(params.lit, hit, n) * gi_sky(hit, n, -ray);
}

// `t`: how far the ray went from where it left to the hit.
fn hit_sent(s: Surface, hit: vec3<f32>, dir: vec3<f32>, t: f32, lod: f32, offset: f32) -> Sent {
    let flags = record_vec(s.record, RECORD_FLAGS).x;
    let base = base_color(s.record, s.uv, lod).rgb;
    let glow = emissive_color(s.record);
    if ((flags & FLAG_FULLBRIGHT) != 0u) {
        return Sent(base + glow, vec3<f32>(0.0));
    }
    let lamps = Lamps(u32(params.mode.z), params.mode.w > 0.5, -1.0);
    let grazing = 1.0 - max(dot(s.normal, -dir), 0.0);
    if ((flags & FLAG_PBR) != 0u) {
        let orm = pbr_orm(s.record, s.uv, lod);
        let occlusion = orm.x;
        let roughness = orm.y;
        let metallic = orm.z;
        // (a metal has no diffuse: no light to find for it)
        var light = vec3<f32>(0.0);
        if (metallic < 0.999) {
            let ambient = hit_ambient(hit, s.normal, dir * t) * occlusion;
            light = hit_light(params.lit, s, hit, offset, params.mode.x, params.mode.y > 0.5, lamps, ambient);
        }
        let f0 = mix(vec3<f32>(0.04), base, metallic);
        let fresnel = (f0 + (vec3<f32>(1.0) - f0) * pow(grazing, 5.0)) * occlusion;
        let gloss = clamp(1.0 - 2.0 * roughness, 0.0, 1.0);
        let sharp = gloss * gloss * (1.0 - smoothstep(TRACED_ROUGH_FADE, TRACED_ROUGH_MAX, roughness));
        let blurred = fresnel * (1.0 - sharp) * mirrored_at(params.lit, hit, reflect(dir, s.normal), roughness);
        return Sent(base * (1.0 - metallic) * light + blurred + glow, fresnel * sharp);
    }
    let light = hit_light(params.lit, s, hit, offset, params.mode.x, params.mode.y > 0.5, lamps, hit_ambient(hit, s.normal, dir * t));
    var lit = base * light;
    var mirror = vec3<f32>(0.0);
    if ((flags & FLAG_LEGACY_MAT) != 0u) {
        let legacy = record_vecf(s.record, RECORD_LEGACY);
        let env = clamp(legacy.w, 0.0, 1.0);
        if (legacy.z > 0.0) {
            let f = clamp(grazing, 0.3, 1.0);
            mirror = record_vecf(s.record, RECORD_SPEC_COLOR).rgb * (0.25 * f * f * legacy.z) * (vec3<f32>(1.0) - lit);
        }
        if (env > 0.0) {
            let f = min(grazing * grazing + env, 1.0);
            mirror = mirror * (1.0 - env) + vec3<f32>(env * env * f * 0.5);
            lit *= 1.0 - env;
        }
    }
    return Sent(lit + glow, mirror);
}

// The mirror ray `r` of the pixel, kept on its own side of its surface.
// The G-buffer's normal has the normal map in it: seen at a slant, the far
// slope of a bump sends the ray into the face it leaves. It went through
// the object and showed what stands behind it, in patches the shape of the
// relief (AURORA_DEMO_GLOSSY_BUMPS: the holed panel behind the polished one).
// Such a ray is sent on as the face around the bump would send it: mirrored
// about the surface without its map, the mesh's smooth normal where the
// prepass drew it, read first since a ray above it is left as it is (every
// ray of a face with no normal map), and the triangle's from depth where
// the prepass's is not this face's (rt_history.wgsl `smooth_normal`) or was
// not drawn.
fn kept_above(pixel: vec2<i32>, depth: f32, r: vec3<f32>) -> vec3<f32> {
    var face: vec3<f32>;
    if (params.camera.w > 0.5) {
        face = mesh_normal(textureLoad(motion_tex, pixel, 0).xy);
        if (dot(r, face) >= 0.0) {
            return r;
        }
        let flat = normal_at(pixel, depth);
        if (dot(face, flat) <= 0.7) {
            face = flat;
        }
    } else {
        face = normal_at(pixel, depth);
    }
    return mirrored_above(r, face);
}

// `r` on the side of the surface `face` faces: mirrored about it when it
// points into it.
fn mirrored_above(r: vec3<f32>, face: vec3<f32>) -> vec3<f32> {
    let into = dot(r, face);
    if (into >= 0.0) {
        return r;
    }
    return r - 2.0 * into * face;
}

// The nearest hit of the ray from `origin` along `dir`, between `least` and
// `reach` along it, of the two top levels: the scene's, then the posed
// faces' up to it. `t` is negative without one.
struct Near {
    t: f32,
    record: u32,
    primitive: u32,
    bary: vec2<f32>,
};

fn nearest(origin: vec3<f32>, dir: vec3<f32>, least: f32, reach: f32) -> Near {
    var near = Near(-1.0, 0u, 0u, vec2<f32>(0.0));
    var rq: ray_query;
    rayQueryInitialize(&rq, scene_tlas, RayDesc(0u, SOLID, least, reach, origin, dir));
    while (rayQueryProceed(&rq)) {
        if (stops(rayQueryGetCandidateIntersection(&rq), false, dir)) {
            rayQueryConfirmIntersection(&rq);
        }
    }
    let hs = rayQueryGetCommittedIntersection(&rq);
    if (hs.kind != RAY_QUERY_INTERSECTION_NONE) {
        let h = hit_of(false, hs.instance_custom_data, hs.geometry_index, hs.primitive_index);
        near = Near(hs.t, h.record, h.primitive, hs.barycentrics);
    }
    if (params.mode.y > 0.5) {
        var rp: ray_query;
        rayQueryInitialize(&rp, posed_tlas, RayDesc(0u, SOLID, least, select(reach, near.t, near.t >= 0.0), origin, dir));
        while (rayQueryProceed(&rp)) {
            if (stops(rayQueryGetCandidateIntersection(&rp), true, dir)) {
                rayQueryConfirmIntersection(&rp);
            }
        }
        let hp = rayQueryGetCommittedIntersection(&rp);
        if (hp.kind != RAY_QUERY_INTERSECTION_NONE) {
            let h = hit_of(true, hp.instance_custom_data, hp.geometry_index, hp.primitive_index);
            near = Near(hp.t, h.record, h.primitive, hp.barycentrics);
        }
    }
    return near;
}

// The base colour of the metal under a pixel (rgb, its square root: 8 bits
// hold the dark ones) and 1, or 0 in a where none is found. A metal mirrors
// in its own colour (common.wgsl `shade_pbr`: its reflection starts from
// it), which the G-buffer does not hold: ssr.wgsl `fs_ssr_rt` weighs the
// reflection by it, where a dark metal mirrored as chrome does. The face
// is found again along the eye's ray, at the pixel's distance. What is no
// metal needs none.
fn base_under(world: vec3<f32>, v: vec3<f32>, dist: f32, metallic: f32) -> vec4<f32> {
    if (metallic < 0.01) {
        return vec4<f32>(0.0);
    }
    let slack = 0.02 + 0.01 * dist;
    let near = nearest(params.camera.xyz, -v, max(dist - slack, 0.0), dist + slack);
    if (near.t < 0.0) {
        return vec4<f32>(0.0);
    }
    let s = surface_at(near.record, near.primitive, near.bary, -v);
    let base = base_color(s.record, s.uv, log2(1.0 + dist)).rgb;
    return vec4<f32>(sqrt(clamp(base, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
}

// What the ray from `start` along `toward` brings back (rgb), from a pixel
// `dist` from the eye, and 1 in a, or nothing where it meets nothing. Its
// light is worth all of it at first, then what each hit mirrors of it. A
// hit the view sees ends it with the scene copy's colour; one it does not
// is shaded here and, where it is a mirror, sends the ray on, `bounces`
// hits in all: a chrome ball behind the camera shows the room in a mirror,
// where it showed the sky's hemisphere.
fn traced(start: vec3<f32>, toward: vec3<f32>, dist: f32, bounces: u32) -> vec4<f32> {
    var origin = start;
    var dir = toward;
    var along = 0.0;
    var worth = vec3<f32>(1.0);
    var color = vec3<f32>(0.0);
    for (var bounce = 0u; bounce < bounces; bounce++) {
        let near = nearest(origin, dir, 0.0, params.mode.x);
        let t = near.t;
        if (t < 0.0) {
            // the first ray's miss is the caller's to answer; a later one's
            // is the sky
            if (bounce == 0u) {
                return vec4<f32>(0.0);
            }
            color += worth * hemisphere(params.lit, dir);
            break;
        }
        let hit = origin + dir * t;
        let s = surface_at(near.record, near.primitive, near.bary, dir);
        let on_screen = seen(hit, s.normal);
        color += worth * on_screen.rgb * on_screen.a;
        if (on_screen.a >= 0.999) {
            break;
        }
        along += t;
        // a coarser mip further along the ray, where a texel covers more
        let offset = 0.01 + 0.002 * (dist + along);
        let sent = hit_sent(s, hit, dir, t, log2(1.0 + along), offset);
        worth *= 1.0 - on_screen.a;
        color += worth * sent.body;
        worth *= sent.mirror;
        let onward = reflect(dir, s.normal);
        if (bounce + 1u == bounces || max(max(abs(worth.r), abs(worth.g)), abs(worth.b)) < MIRROR_MIN) {
            color += worth * mirrored_at(params.lit, hit, onward, 0.0);
            break;
        }
        origin = hit + s.normal * offset;
        dir = onward;
    }
    return vec4<f32>(color, 1.0);
}

@compute @workgroup_size(8, 8)
fn cs_reflect(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    if (any(pixel >= vec2<i32>(params.screen.xy))) {
        return;
    }
    let depth = textureLoad(depth_tex, pixel, 0);
    let g = textureLoad(gbuf_tex, pixel, 0);
    // reversed depth: 0 is the sky; rough surfaces keep the probes'
    if (depth <= 0.0 || g.z > GLOSS_ROUGH_MAX) {
        textureStore(out_tex, pixel, vec4<f32>(0.0));
        textureStore(out_base, pixel, vec4<f32>(0.0));
        return;
    }
    let world = unproject(pixel, depth);
    let n = oct_decode(g.xy);
    let v = normalize(params.camera.xyz - world);
    let dist = distance(world, params.camera.xyz);
    textureStore(out_base, pixel, base_under(world, v, dist, g.w));
    // (`cs_gloss` has the others)
    if (g.z >= TRACED_ROUGH_MAX) {
        textureStore(out_tex, pixel, vec4<f32>(0.0));
        return;
    }
    let r = kept_above(pixel, depth, reflect(-v, n));
    textureStore(out_tex, pixel, traced(world + n * (0.01 + 0.002 * dist), r, dist, BOUNCES));
}

// ---- The glossy reflections: the surfaces between a mirror and a matte
// one (a roughness of `TRACED_ROUGH_FADE` to `GLOSS_ROUGH_MAX`). A mirror
// ray gives them a sharp picture, cut up by their normal map, where their
// roughness spreads the light. Each half resolution pixel sends one ray a
// frame along a direction drawn as its roughness spreads them (`ggx_facet`),
// keeps the mean of the frames before (`gloss_kept`: the surface is found
// again as rt_history.wgsl `back_of` says) and its luminance's second
// moment, and rt_denoise.wgsl filters what noise is left; `cs_gloss_filter`
// brings the result to the full resolution. A frame with no history sends
// `params.gloss.y` rays instead of one. A ray that meets nothing brings
// nothing, and the share of those that met something is kept beside (the
// cover): the probes' reflection keeps the rest, the sky's as they have it.

fn unpack_normal(e: vec2<f32>) -> vec3<f32> {
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>((vec2<f32>(1.0) - abs(n.yx)) * s, n.z);
    }
    return normalize(n);
}

// `c` no brighter than `GLOSS_SAMPLE_MAX`, its colour kept (rt_gi.wgsl
// `capped`).
fn gloss_capped(c: vec3<f32>) -> vec3<f32> {
    let bounded = min(c, vec3<f32>(1.0e4));
    return bounded * min(1.0, GLOSS_SAMPLE_MAX / max(luma(bounded), 1e-6));
}

// A facet's normal of a surface of GGX roughness `alpha` (the roughness
// squared) facing `n`, drawn by the numbers `u` among those the eye sees
// along `v`, each as likely as it shows (Heitz 2018, "Sampling the GGX
// Distribution of Visible Normals"): the mirror rays about such facets are
// spread as the surface spreads the light.
fn ggx_facet(n: vec3<f32>, v: vec3<f32>, alpha: f32, u: vec2<f32>) -> vec3<f32> {
    let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n.z) < 0.999);
    let t = normalize(cross(up, n));
    let b = cross(n, t);
    // the eye in the surface's frame, the surface stretched to a roughness of 1
    let vh = normalize(vec3<f32>(alpha * dot(v, t), alpha * dot(v, b), max(dot(v, n), 1e-4)));
    let lensq = vh.x * vh.x + vh.y * vh.y;
    let t1 = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-vh.y, vh.x, 0.0) / sqrt(max(lensq, 1e-12)), lensq > 1e-8);
    let t2 = cross(vh, t1);
    // a point of the disc the eye sees the half sphere as
    let r = sqrt(u.x);
    let phi = 2.0 * PI * u.y;
    let p1 = r * cos(phi);
    let s = 0.5 * (1.0 + vh.z);
    let p2 = (1.0 - s) * sqrt(max(1.0 - p1 * p1, 0.0)) + s * r * sin(phi);
    let nh = p1 * t1 + p2 * t2 + sqrt(max(1.0 - p1 * p1 - p2 * p2, 0.0)) * vh;
    let h = normalize(vec3<f32>(alpha * nh.x, alpha * nh.y, max(nh.z, 0.0)));
    return normalize(t * h.x + b * h.y + n * h.z);
}

// A surface's glossy reflection of last frame: rgb, the frames it averages
// (0 without any), the second moment of its luminance and its cover.
struct GlossKept {
    color: vec3<f32>,
    count: f32,
    moment: f32,
    cover: f32,
};

// Whether last frame's half resolution pixel `q` held the surface `back`
// describes, facing `n`, with a glossy reflection: its guide in `g`.
fn gloss_held(q: vec2<i32>, back: Back, n: vec3<f32>, g: ptr<function, vec4<f32>>) -> bool {
    if (any(q < vec2<i32>(0)) || any(q >= vec2<i32>(params.half.xy))) {
        return false;
    }
    let was = textureLoad(prev_guide, q, 0);
    *g = was;
    return was.z > 0.0 && abs(was.z - back.expected) < back.tolerance && dot(unpack_normal(was.xy), n) > 0.9;
}

// Last frame's glossy reflection of the surface at `back`, facing `n`:
// filtered between the four half resolution pixels around where it was,
// those that held it, or the nine nearest where none of them did (beside an
// outline once the view moves), as rt_gi.wgsl `kept`.
fn gloss_kept(back: Back, n: vec3<f32>) -> GlossKept {
    var k = GlossKept(vec3<f32>(0.0), 0.0, 0.0, 0.0);
    if (params.gloss.x < 0.5 || back.at.x < 0.0) {
        return k;
    }
    // a half resolution pixel is the first of its 2×2
    let c = (back.at - vec2<f32>(0.5)) * 0.5;
    let corner = vec2<i32>(floor(c));
    let f = c - floor(c);
    var weight = 0.0;
    var g = vec4<f32>(0.0);
    for (var i = 0u; i < 4u; i++) {
        let o = vec2<i32>(i32(i & 1u), i32(i >> 1u));
        let w = mix(1.0 - f.x, f.x, f32(o.x)) * mix(1.0 - f.y, f.y, f32(o.y));
        if (w < 1e-3 || !gloss_held(corner + o, back, n, &g)) {
            continue;
        }
        k.color += textureLoad(prev_fed, corner + o, 0).rgb * w;
        k.count += textureLoad(prev_color, corner + o, 0).a * w;
        k.moment += g.w * w;
        k.cover += textureLoad(prev_cover, corner + o, 0).r * w;
        weight += w;
    }
    if (weight < 1e-3) {
        weight = 0.0;
        let near = vec2<i32>(round(c));
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let q = near + vec2<i32>(x, y);
                if (!gloss_held(q, back, n, &g)) {
                    continue;
                }
                k.color += textureLoad(prev_fed, q, 0).rgb;
                k.count += textureLoad(prev_color, q, 0).a;
                k.moment += g.w;
                k.cover += textureLoad(prev_cover, q, 0).r;
                weight += 1.0;
            }
        }
    }
    if (weight > 0.0) {
        k.color /= weight;
        k.count /= weight;
        k.moment /= weight;
        k.cover /= weight;
    }
    return k;
}

// Whether a pixel of this roughness takes a glossy reflection.
fn glossy(roughness: f32) -> bool {
    return roughness >= TRACED_ROUGH_FADE && roughness <= GLOSS_ROUGH_MAX;
}

@compute @workgroup_size(8, 8)
fn cs_gloss(@builtin(global_invocation_id) id: vec3<u32>) {
    let hp = vec2<i32>(id.xy);
    if (any(hp >= vec2<i32>(params.half.xy))) {
        return;
    }
    // the same pixel of each 2×2 at every frame (rt_gi.wgsl `cs_gi`)
    let pixel = hp * 2;
    let depth = textureLoad(depth_tex, pixel, 0);
    let g = textureLoad(gbuf_tex, pixel, 0);
    if (depth <= 0.0 || !glossy(g.z)) {
        textureStore(out_color, hp, vec4<f32>(0.0));
        textureStore(out_guide, hp, vec4<f32>(0.0));
        textureStore(out_cover, hp, vec4<f32>(0.0));
        return;
    }
    let world = unproject(pixel, depth);
    let n = oct_decode(g.xy);
    let v = normalize(params.camera.xyz - world);
    let dist = distance(world, params.camera.xyz);
    // The surface without its normal map: the rays stay on its side
    // (`kept_above`), start off it, and the history and the filter tell it
    // from the next by it, so that a normal map's grain does not part the
    // pixels of one face.
    let face = smooth_normal(pixel, normal_at(pixel, depth));
    let origin = ray_origin(world, face, dist);
    let alpha = g.z * g.z;
    let frame = u32(params.half.z);
    let rays = max(u32(params.gloss.y), 1u);
    var sum = vec3<f32>(0.0);
    var moment = 0.0;
    var met = 0.0;
    for (var i = 0u; i < rays; i++) {
        let facet = ggx_facet(n, v, alpha, disc_point(id.xy, frame + i));
        let dir = mirrored_above(reflect(-v, facet), face);
        // one hit: what it mirrors is the sky's hemisphere, blurred as this is
        let back = traced(origin, dir, dist, 1u);
        let light = gloss_capped(back.rgb);
        sum += light;
        met += back.a;
        let l = luma(light);
        moment += l * l;
    }
    let back = back_of(pixel, world, dist);
    let k = gloss_kept(back, face);
    // The frames the history averages: as many as it may for a wide spread,
    // fewer for a narrow one, and no more than the eye's move lets the
    // reflection stand still within its own spread (the angle the eye
    // turned about the surface is the one its reflection turned by).
    let turned = length(v - normalize(params.prev_camera.xyz - world));
    var most = mix(GLOSS_HISTORY_NARROW, params.half.w, smoothstep(0.0, GLOSS_HISTORY_ALPHA, alpha));
    most = min(most, GLOSS_DRIFT * alpha / max(turned, 1e-6));
    if (back.moving) {
        most = min(most, GLOSS_HISTORY_MOVING);
    }
    most = max(most, GLOSS_HISTORY_MIN);
    let count = min(k.count + f32(rays), max(most, f32(rays)));
    let share = f32(rays) / count;
    let mean = mix(k.color, sum / f32(rays), share);
    textureStore(out_color, hp, vec4<f32>(mean, count));
    textureStore(out_guide, hp, vec4<f32>(pack_normal(face), dist, mix(k.moment, moment / f32(rays), share)));
    textureStore(out_cover, hp, vec4<f32>(mix(k.cover, met / f32(rays), share), 0.0, 0.0, 0.0));
}

// The glossy reflection at the full resolution: each pixel's from the half
// resolution ones around it that hold its surface (the same depth, the
// same way round), nearer ones first, as rt_gi.wgsl `cs_gi_filter`. The
// cover takes them from further around: the filter evened the light out,
// and the cover's own noise would show against it.
@compute @workgroup_size(8, 8)
fn cs_gloss_filter(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    if (any(pixel >= vec2<i32>(params.screen.xy))) {
        return;
    }
    let depth = textureLoad(depth_tex, pixel, 0);
    if (depth <= 0.0 || !glossy(textureLoad(gbuf_tex, pixel, 0).z)) {
        textureStore(out_gloss, pixel, vec4<f32>(0.0));
        return;
    }
    let dist = distance(unproject(pixel, depth), params.camera.xyz);
    let face = smooth_normal(pixel, normal_at(pixel, depth));
    // a half resolution pixel is the first of its 2×2
    let centre = vec2<f32>(pixel) * 0.5;
    let base = vec2<i32>(floor(centre));
    let last = vec2<i32>(params.half.xy) - vec2<i32>(1);
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    var cover = 0.0;
    var cover_weight = 0.0;
    for (var y = -2; y <= 3; y++) {
        for (var x = -2; x <= 3; x++) {
            let q = base + vec2<i32>(x, y);
            if (any(q < vec2<i32>(0)) || any(q > last)) {
                continue;
            }
            let guide = textureLoad(gloss_guide, q, 0);
            if (guide.z <= 0.0) {
                continue;
            }
            let off = vec2<f32>(q) - centre;
            let far = dot(off, off);
            let same = exp(-abs(guide.z - dist) / (0.01 + 0.02 * dist))
                * pow(max(dot(unpack_normal(guide.xy), face), 0.0), 8.0);
            let w = exp(-far) * same;
            sum += textureLoad(gloss_color, q, 0).rgb * w;
            weight += w;
            let wide = exp(-far / 4.0) * same;
            cover += textureLoad(gloss_cover, q, 0).r * wide;
            cover_weight += wide;
        }
    }
    if (weight <= 1e-4) {
        textureStore(out_gloss, pixel, vec4<f32>(0.0));
        return;
    }
    textureStore(out_gloss, pixel, vec4<f32>(sum / weight, cover / cover_weight));
}
