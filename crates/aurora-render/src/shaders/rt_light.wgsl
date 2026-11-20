// The light at a traced hit, which the reflections and the GI share
// (rt_shadows.rs `source`). Each shader that takes it declares `scene_tlas`
// and `posed_tlas` (the scene's and the posed faces' top levels), and
// `lights`, the frame's point lights (an array of `Light`).

const PI: f32 = 3.14159265;

// The frame's light (common.wgsl `FrameU`).
struct HitLight {
    // xyz towards the sun or moon (unit), w 1 when it is up.
    light: vec4<f32>,
    // the sun's radiance and the hemisphere's ambient
    sun_color: vec4<f32>,
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    ground_color: vec4<f32>,
};

// A light of the frame's list (the frame uniform's `lights`, in order).
struct Light {
    pos_radius: vec4<f32>,
    color_falloff: vec4<f32>,
    // x: the radius of the prim it shines from, y: 1 for a projector, z:
    // how far it moved since last frame (m).
    source: vec4<f32>,
};

// The point lights at a hit.
struct Lamps {
    // how many of `lights`
    count: u32,
    // their shadows are traced in the view (rt_lights.wgsl): here too
    shadowed: bool,
    // With shadows, a number in 0..1 that draws the one lamp whose ray is
    // traced, in proportion to its light (it then stands for them all: for
    // a light averaged over frames), or -1 for a ray to each lamp that
    // counts, `LAMP_RAYS` at most (a light shown as it is).
    chance: f32,
};

const HIT_LIGHTS: u32 = 64u;
const LAMP_RAYS: u32 = 4u;
const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// The sky's ambient along `n` (common.wgsl `shade_pbr`'s hemisphere).
fn hemisphere(lit: HitLight, n: vec3<f32>) -> vec3<f32> {
    let up = n.z * 0.5 + 0.5;
    return mix(mix(lit.ground_color.rgb, lit.sky_horizon.rgb, clamp(up * 2.0, 0.0, 1.0)),
               lit.sky_zenith.rgb, clamp(up * 2.0 - 1.0, 0.0, 1.0));
}

// Whether nothing stops a ray from `origin` along `dir` within `reach`:
// the scene's faces, then the posed ones when `posed` (the alpha-masked
// faces tested at each candidate, rt_common.wgsl `stops`).
fn unblocked(origin: vec3<f32>, dir: vec3<f32>, reach: f32, posed: bool) -> bool {
    let ray = RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT | SHADOW_CULL, 0xFFu, 0.0, reach, origin, dir);
    var rq: ray_query;
    rayQueryInitialize(&rq, scene_tlas, ray);
    while (rayQueryProceed(&rq)) {
        if (stops(rayQueryGetCandidateIntersection(&rq), false, dir)) {
            rayQueryConfirmIntersection(&rq);
        }
    }
    if (rayQueryGetCommittedIntersection(&rq).kind != RAY_QUERY_INTERSECTION_NONE) {
        return false;
    }
    if (!posed) {
        return true;
    }
    var rp: ray_query;
    rayQueryInitialize(&rp, posed_tlas, ray);
    while (rayQueryProceed(&rp)) {
        if (stops(rayQueryGetCandidateIntersection(&rp), true, dir)) {
            rayQueryConfirmIntersection(&rp);
        }
    }
    return rayQueryGetCommittedIntersection(&rp).kind == RAY_QUERY_INTERSECTION_NONE;
}

// How far along `dir` from `origin`, a pixel of the view, something stops
// a shadow ray within `reach` (any of what does, not the nearest), or -1
// when nothing does: for the shadows that soften with that distance
// (rt_lights.wgsl). `curved`: rt_common.wgsl `shadows`. One query serves
// the scene's level then the posed one, and its flags are constant where
// each starts: a pass pays for each query its shader holds, run or not.
fn blocked_at(origin: vec3<f32>, dir: vec3<f32>, reach: f32, posed: bool, curved: bool) -> f32 {
    var rq: ray_query;
    if (curved) {
        let ray = RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT | RAY_FLAG_FORCE_NO_OPAQUE, 0xFFu, 0.0, reach, origin, dir);
        rayQueryInitialize(&rq, scene_tlas, ray);
        while (rayQueryProceed(&rq)) {
            if (shadows(rayQueryGetCandidateIntersection(&rq), false, dir, true)) {
                rayQueryConfirmIntersection(&rq);
            }
        }
        var hit = rayQueryGetCommittedIntersection(&rq);
        if (hit.kind == RAY_QUERY_INTERSECTION_NONE && posed) {
            rayQueryInitialize(&rq, posed_tlas, ray);
            while (rayQueryProceed(&rq)) {
                if (shadows(rayQueryGetCandidateIntersection(&rq), true, dir, true)) {
                    rayQueryConfirmIntersection(&rq);
                }
            }
            hit = rayQueryGetCommittedIntersection(&rq);
        }
        if (hit.kind != RAY_QUERY_INTERSECTION_NONE) {
            return hit.t;
        }
        return -1.0;
    }
    let ray = RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT | SHADOW_CULL, 0xFFu, 0.0, reach, origin, dir);
    rayQueryInitialize(&rq, scene_tlas, ray);
    while (rayQueryProceed(&rq)) {
        if (stops(rayQueryGetCandidateIntersection(&rq), false, dir)) {
            rayQueryConfirmIntersection(&rq);
        }
    }
    var hit = rayQueryGetCommittedIntersection(&rq);
    if (hit.kind == RAY_QUERY_INTERSECTION_NONE && posed) {
        rayQueryInitialize(&rq, posed_tlas, ray);
        while (rayQueryProceed(&rq)) {
            if (stops(rayQueryGetCandidateIntersection(&rq), true, dir)) {
                rayQueryConfirmIntersection(&rq);
            }
        }
        hit = rayQueryGetCommittedIntersection(&rq);
    }
    if (hit.kind != RAY_QUERY_INTERSECTION_NONE) {
        return hit.t;
    }
    return -1.0;
}

// The diffuse light point light `i` gives a face at `p` turned to `n`,
// unshadowed, before its base colour: common.wgsl's, over pi on a PBR face
// (`shade_pbr`) and over 3.25 on a legacy one (`ll_local_diffuse`). None
// from a projector, which lights its frustum through its texture.
fn lamp_light(i: u32, p: vec3<f32>, n: vec3<f32>, pbr: bool) -> vec3<f32> {
    let lt = lights[i];
    if (lt.source.y > 0.5) {
        return vec3<f32>(0.0);
    }
    let to_l = lt.pos_radius.xyz - p;
    let dist = length(to_l);
    let radius = lt.pos_radius.w;
    if (dist >= radius) {
        return vec3<f32>(0.0);
    }
    let nl = dot(n, to_l / max(dist, 1e-4));
    if (nl <= 0.0) {
        return vec3<f32>(0.0);
    }
    // common.wgsl `ll_light_atten`
    let f = max(lt.color_falloff.w, 0.0);
    let x = 1.0 - clamp((dist / radius + f) / (1.0 + f), 0.0, 1.0);
    return lt.color_falloff.rgb * (2.0 * x * x * nl / select(3.25, PI, pbr));
}

// The ray from `start` to point light `i`: xyz its direction, w how far it
// goes, up to the light's own prim (rt_lights.wgsl `light_blocked`); w 0
// from inside it.
fn lamp_ray(i: u32, start: vec3<f32>) -> vec4<f32> {
    let lt = lights[i];
    let to_l = lt.pos_radius.xyz - start;
    let dist = length(to_l);
    return vec4<f32>(to_l / max(dist, 1e-6), max(dist - max(lt.source.x, 0.05), 0.0));
}

// The light that falls on a hit's surface, for its diffuse: its `ambient`
// (the sky's hemisphere, or the probes': rt_probes.wgsl `ambient_at`), the
// sun with a shadow ray (`offset` off the surface), and the point lights
// (`lamps`), with shadow rays where the view traces theirs.
// shade_pbr's diffuse: the sunlight over pi on a PBR face, a third of it on
// a legacy one.
fn hit_light(lit: HitLight, s: Surface, hit: vec3<f32>, offset: f32, reach: f32, posed: bool, lamps: Lamps, ambient: vec3<f32>) -> vec3<f32> {
    var light = ambient;
    let pbr = (record_vec(s.record, RECORD_FLAGS).x & FLAG_PBR) != 0u;
    // the shadow rays (xyz the direction, w how far), and what each brings
    // if nothing stops it: one loop traces them all, for a pass pays for
    // each ray query its shader holds
    var to: array<vec4<f32>, 5>;
    var brings: array<vec3<f32>, 5>;
    var rays = 0u;
    let nl = dot(s.normal, lit.light.xyz);
    if (lit.light.w > 0.5 && nl > 0.0) {
        to[0] = vec4<f32>(lit.light.xyz, reach);
        brings[0] = lit.sun_color.rgb * nl / select(3.0, PI, pbr);
        rays = 1u;
    }
    let start = hit + s.normal * offset;
    let count = min(lamps.count, HIT_LIGHTS);
    var all = vec3<f32>(0.0);
    for (var i = 0u; i < count; i++) {
        all += lamp_light(i, hit, s.normal, pbr);
    }
    let total = dot(all, LUMA);
    if (!lamps.shadowed || total <= 0.0) {
        light += all;
    } else if (lamps.chance >= 0.0) {
        let goal = lamps.chance * total;
        var reached = 0.0;
        for (var i = 0u; i < count; i++) {
            let c = lamp_light(i, hit, s.normal, pbr);
            let l = dot(c, LUMA);
            reached += l;
            if (l > 0.0 && goal < reached) {
                to[rays] = lamp_ray(i, start);
                brings[rays] = c * (total / l);
                rays += 1u;
                break;
            }
        }
    } else {
        // a lamp counts from a fiftieth of the lamps' light here, wholly
        // from a tenth: its shadow comes in by degrees, so no line shows
        // where it starts to count
        let first = rays;
        for (var i = 0u; i < count; i++) {
            let c = lamp_light(i, hit, s.normal, pbr);
            let counts = smoothstep(0.02, 0.1, dot(c, LUMA) / total);
            if (counts > 0.0 && rays < first + LAMP_RAYS) {
                light += c * (1.0 - counts);
                to[rays] = lamp_ray(i, start);
                brings[rays] = c * counts;
                rays += 1u;
            } else {
                light += c;
            }
        }
    }
    for (var k = 0u; k < rays; k++) {
        if (to[k].w <= 0.0 || unblocked(start, to[k].xyz, to[k].w, posed)) {
            light += brings[k];
        }
    }
    return light;
}

// The light a hit's surface sends back, as diffuse: its base colour (from
// mip `lod`) under `hit_light`, plus its emission.
fn hit_radiance(lit: HitLight, s: Surface, hit: vec3<f32>, lod: f32, offset: f32, reach: f32, posed: bool, lamps: Lamps) -> vec3<f32> {
    let base = base_color(s.record, s.uv, lod).rgb;
    return base * hit_light(lit, s, hit, offset, reach, posed, lamps, hemisphere(lit, s.normal)) + emissive_color(s.record);
}
