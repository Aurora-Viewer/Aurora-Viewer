// Ray-traced sun shadows of the main view (rt_shadows.rs): one ray per
// pixel of the single-sample depth prepass, towards the sun or the moon,
// through the scene's acceleration structures. The result takes the place of
// the contact shadows' mask (binding 13 of the frame group): `.r` is 0 lit,
// 1 in shadow, the darker of the traced shadow and the contact shadow, so
// `sun_shadow` (common.wgsl) needs nothing more than that mask.
// The ray's origin and its offset: pushed off the surface along the normal
// bent towards the eye, by more far away, where depth is coarser.

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
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var contact_tex: texture_2d<f32>;
@group(0) @binding(3) var scene_tlas: acceleration_structure;
@group(0) @binding(4) var out_tex: texture_storage_2d<rgba8unorm, write>;
// The avatars and what they wear, posed (rt_pose.wgsl).
@group(0) @binding(5) var posed_tlas: acceleration_structure;

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
        return;
    }
    let world = unproject(pixel, depth);
    let n = normal_at(pixel, world, depth);
    let dist = distance(world, params.camera.xyz);
    let off = normalize(n + normalize(params.camera.xyz - world));
    let origin = world + off * (0.01 + 0.002 * dist);
    let dir = params.light.xyz;
    let ray = RayDesc(u32(params.mode.x) | RAY_FLAG_TERMINATE_ON_FIRST_HIT, 0xFFu, 0.0, params.mode.z, origin, dir);
    var shadow = 0.0;
    var rq: ray_query;
    rayQueryInitialize(&rq, scene_tlas, ray);
    while (rayQueryProceed(&rq)) {
        if (stops(rayQueryGetCandidateIntersection(&rq), false, dir)) {
            rayQueryConfirmIntersection(&rq);
        }
    }
    if (rayQueryGetCommittedIntersection(&rq).kind != RAY_QUERY_INTERSECTION_NONE) {
        shadow = 1.0;
    } else if (params.mode.w > 0.5) {
        var rp: ray_query;
        rayQueryInitialize(&rp, posed_tlas, ray);
        while (rayQueryProceed(&rp)) {
            if (stops(rayQueryGetCandidateIntersection(&rp), true, dir)) {
                rayQueryConfirmIntersection(&rp);
            }
        }
        if (rayQueryGetCommittedIntersection(&rp).kind != RAY_QUERY_INTERSECTION_NONE) {
            shadow = 1.0;
        }
    }
    textureStore(out_tex, pixel, vec4<f32>(max(shadow, contact), shadow, 0.0, 0.0));
}
