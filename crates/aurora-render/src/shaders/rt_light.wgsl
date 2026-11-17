// The light at a traced hit, which the reflections and the GI share
// (rt_shadows.rs `source`). Each shader that takes it declares `scene_tlas`
// and `posed_tlas` (the scene's and the posed faces' top levels).

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
    let ray = RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT, 0xFFu, 0.0, reach, origin, dir);
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

// The light a hit's surface sends back, as diffuse: its base colour (from
// mip `lod`) lit by the sun, with a shadow ray (`offset` off the surface),
// and by the sky's hemisphere, plus its emission. shade_pbr's diffuse: the
// sunlight over pi on a PBR face, a third of it on a legacy one.
fn hit_radiance(lit: HitLight, s: Surface, hit: vec3<f32>, lod: f32, offset: f32, reach: f32, posed: bool) -> vec3<f32> {
    let base = base_color(s.record, s.uv, lod).rgb;
    var light = hemisphere(lit, s.normal);
    let l = lit.light.xyz;
    let nl = dot(s.normal, l);
    if (lit.light.w > 0.5 && nl > 0.0 && unblocked(hit + s.normal * offset, l, reach, posed)) {
        let pbr = (record_vec(s.record, RECORD_FLAGS).x & FLAG_PBR) != 0u;
        light += lit.sun_color.rgb * nl / select(3.0, PI, pbr);
    }
    return base * light + emissive_color(s.record);
}
