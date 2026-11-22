// Ray-traced sun shadows of the main view (rt_shadows.rs): one ray per
// pixel of the single-sample depth prepass, towards the sun or the moon,
// through the scene's acceleration structures. The ray starts where
// rt_view.wgsl `ray_origin` puts it.
//
// Soft (`soft.x`, the tangent of the disc's radius): the ray goes to a
// point of the sun's disc, another each frame, so a shadow has a penumbra
// that widens with the distance to what casts it. This frame's ray is then
// written as it is, `.g` (1 in shadow) beside the contact shadows' `.r`,
// for rt_lights.wgsl to average over the frames and rt_denoise.wgsl to
// filter, which write the mask.
// Sharp (`soft.x` 0): the ray goes to the disc's centre and the result is
// the mask the frame binds in the contact shadows' place (binding 13 of
// the frame group): `.r` is 0 lit, 1 in shadow, the darker of the traced
// shadow and the contact shadow, so `sun_shadow` (common.wgsl) needs
// nothing more than that mask.
//
// Glass: a ray that nothing stops goes on through the blended faces (the
// `GLASS` instances, rt_common.wgsl `glass_passes`), and what they leave of
// the sun's red, green and blue is `out_glass`: 1 where it meets none, 0
// where the ray was stopped. Its ray goes to another point of the sun's
// disc each frame, as the shadow's: the lights' passes average it as they
// do the shadow (rt_lights.wgsl `glass_history`), over the rays that
// arrived, into the frame's binding 31 (common.wgsl `sun_glass`).

// After rt_common.wgsl, rt_view.wgsl and wgpu_ray_query's enable
// (rt_shadows.rs `source`).

struct Params {
    // This frame's clip-from-world inverted, with the jitter the depth
    // prepass was drawn with.
    world_from_clip: mat4x4<f32>,
    // xyz the eye.
    camera: vec4<f32>,
    // xyz towards the sun or moon (unit).
    light: vec4<f32>,
    // width, height, 1 / width, 1 / height
    screen: vec4<f32>,
    // x: the ray flags, y: 1 when the contact mask is read, z: the rays'
    // reach (m), w: 1 when the avatars' top level is traced.
    mode: vec4<f32>,
    // x: the tangent of the disc's radius (0: sharp), y: the frame's number.
    soft: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var contact_tex: texture_2d<f32>;
@group(0) @binding(3) var scene_tlas: acceleration_structure;
@group(0) @binding(4) var out_tex: texture_storage_2d<rgba8unorm, write>;
// The avatars and what they wear, posed (rt_pose.wgsl).
@group(0) @binding(5) var posed_tlas: acceleration_structure;
@group(0) @binding(6) var out_glass: texture_storage_2d<rgba8unorm, write>;

@compute @workgroup_size(8, 8)
fn cs_trace(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    if (any(pixel >= vec2<i32>(params.screen.xy))) {
        return;
    }
    var contact = 0.0;
    if (params.mode.y > 0.5) {
        contact = textureLoad(contact_tex, pixel, 0).r;
    }
    let depth = textureLoad(depth_tex, pixel, 0);
    // reversed depth: 0 is the sky
    if (depth <= 0.0 || params.light.w < 0.5) {
        textureStore(out_tex, pixel, vec4<f32>(contact, 0.0, 0.0, 0.0));
        textureStore(out_glass, pixel, vec4<f32>(1.0));
        return;
    }
    let world = unproject(pixel, depth);
    let n = normal_at(pixel, depth);
    let dist = distance(world, params.camera.xyz);
    let origin = ray_origin(world, n, dist);
    var dir = params.light.xyz;
    let soft = params.soft.x > 0.0;
    if (soft) {
        let u = disc_point(id.xy, u32(params.soft.y));
        let side = normalize(cross(dir, select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(dir.z) > 0.9)));
        let up = cross(dir, side);
        let r = params.soft.x * sqrt(u.x);
        let phi = 6.2831853 * u.y;
        dir = normalize(dir + (side * cos(phi) + up * sin(phi)) * r);
    }
    // a triangle turned from the sun on a rounded mesh: rt_common.wgsl `shadows`
    let curved = dot(n, dir) <= 0.0;
    // the faces turned from the sun let it through (#3895, rt_common.wgsl
    // `SHADOW_CULL`), but on a curved pixel, whose own mesh `shadows` sorts
    let flags = select(SHADOW_CULL, RAY_FLAG_NONE, curved) | RAY_FLAG_TERMINATE_ON_FIRST_HIT
        | select(RAY_FLAG_NONE, RAY_FLAG_FORCE_NO_OPAQUE, curved);
    let ray = RayDesc(flags, SOLID, 0.0, params.mode.z, origin, dir);
    // the masked and blended layers lying on the pixel's surface cast
    // nothing on it, as in the shadow maps (rt_view.wgsl `lies_on`); none
    // for a curved pixel, turned from the sun
    let lying = lying_on(n, dir, dist);
    var shadow = 0.0;
    // (one query for both levels: a pass pays for each query it holds)
    var rq: ray_query;
    rayQueryInitialize(&rq, scene_tlas, ray);
    while (rayQueryProceed(&rq)) {
        let c = rayQueryGetCandidateIntersection(&rq);
        if (!lies_on(c, false, dir, n, lying) && shadows(c, false, dir, curved)) {
            rayQueryConfirmIntersection(&rq);
        }
    }
    if (rayQueryGetCommittedIntersection(&rq).kind != RAY_QUERY_INTERSECTION_NONE) {
        shadow = 1.0;
    } else if (params.mode.w > 0.5) {
        rayQueryInitialize(&rq, posed_tlas, ray);
        while (rayQueryProceed(&rq)) {
            let c = rayQueryGetCandidateIntersection(&rq);
            if (!lies_on(c, true, dir, n, lying) && shadows(c, true, dir, curved)) {
                rayQueryConfirmIntersection(&rq);
            }
        }
        if (rayQueryGetCommittedIntersection(&rq).kind != RAY_QUERY_INTERSECTION_NONE) {
            shadow = 1.0;
        }
    }
    // through the glass, where nothing stopped it: every pane is a
    // candidate, none is committed
    var glass = vec3<f32>(1.0);
    if (shadow < 0.5) {
        let pane = RayDesc(SHADOW_CULL, GLASS, 0.0, params.mode.z, origin, dir);
        rayQueryInitialize(&rq, scene_tlas, pane);
        while (rayQueryProceed(&rq)) {
            let c = rayQueryGetCandidateIntersection(&rq);
            if (!lies_on(c, false, dir, n, lying)) {
                glass *= glass_passes(c, false, dir);
            }
        }
        // the moving faces are the posed level's (a pane being carried);
        // worn glass is not traced yet
        if (params.mode.w > 0.5) {
            rayQueryInitialize(&rq, posed_tlas, pane);
            while (rayQueryProceed(&rq)) {
                let c = rayQueryGetCandidateIntersection(&rq);
                if (!lies_on(c, true, dir, n, lying)) {
                    glass *= glass_passes(c, true, dir);
                }
            }
        }
    }
    // (a stopped ray's 0: the average is over those that arrived)
    textureStore(out_glass, pixel, vec4<f32>(select(glass, vec3<f32>(0.0), shadow > 0.5), 1.0));
    textureStore(out_tex, pixel, vec4<f32>(select(max(shadow, contact), contact, soft), shadow, 0.0, 0.0));
}
