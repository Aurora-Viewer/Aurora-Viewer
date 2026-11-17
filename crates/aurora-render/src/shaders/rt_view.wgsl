// The view the ray-traced passes start from (rt_shadows.rs `source`): a
// pixel's world position and normal from the single-sample depth prepass.
// Each shader that takes it declares `depth_tex` and a `params` uniform with
// `world_from_clip` (this frame's projection inverted, with the prepass's
// jitter), `camera` (xyz the eye) and `screen` (width, height and their
// inverses).

fn unproject(pixel: vec2<i32>, depth: f32) -> vec3<f32> {
    let ndc = (vec2<f32>(pixel) + vec2<f32>(0.5)) * params.screen.zw * vec2<f32>(2.0, -2.0)
        + vec2<f32>(-1.0, 1.0);
    let world = params.world_from_clip * vec4<f32>(ndc, depth, 1.0);
    return world.xyz / world.w;
}

fn depth_at(pixel: vec2<i32>) -> f32 {
    let size = vec2<i32>(params.screen.xy);
    return textureLoad(depth_tex, clamp(pixel, vec2<i32>(0), size - vec2<i32>(1)), 0);
}

// The surface's normal from depth: of the two neighbours on each axis, the
// one nearer in depth, so an edge does not bend the normal of the surface
// in front of it.
fn normal_at(pixel: vec2<i32>, world: vec3<f32>, depth: f32) -> vec3<f32> {
    let dl = depth_at(pixel - vec2<i32>(1, 0));
    let dr = depth_at(pixel + vec2<i32>(1, 0));
    let du = depth_at(pixel - vec2<i32>(0, 1));
    let dd = depth_at(pixel + vec2<i32>(0, 1));
    var dx: vec3<f32>;
    if (abs(dr - depth) < abs(dl - depth)) {
        dx = unproject(pixel + vec2<i32>(1, 0), dr) - world;
    } else {
        dx = world - unproject(pixel - vec2<i32>(1, 0), dl);
    }
    var dy: vec3<f32>;
    if (abs(dd - depth) < abs(du - depth)) {
        dy = unproject(pixel + vec2<i32>(0, 1), dd) - world;
    } else {
        dy = world - unproject(pixel - vec2<i32>(0, 1), du);
    }
    var n = normalize(cross(dy, dx));
    if (dot(n, params.camera.xyz - world) < 0.0) {
        n = -n;
    }
    return n;
}

// Where a ray from the pixel at `world` starts: off the surface along its
// normal bent towards the eye (the depth's normal is a face's, so on curved
// meshes, avatars most of all, the eye's side is the one surely outside),
// by more far away, where depth is coarser.
fn ray_origin(world: vec3<f32>, n: vec3<f32>, dist: f32) -> vec3<f32> {
    let off = normalize(n + normalize(params.camera.xyz - world));
    return world + off * (0.01 + 0.002 * dist);
}
