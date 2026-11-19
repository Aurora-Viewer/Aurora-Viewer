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

// A pixel's place from the eye (`params.camera`). `unproject`'s is told to
// a float's precision at the eye's own place in the region, 0.015 mm at
// 200 m, which is the width of a pixel seen from very close (a zoom on a
// face narrows the view to a few degrees): the difference of two
// neighbours was then noise, and so were the normal and the pixel's size
// made from it, which showed as specks in the lamps' shadows and in the
// indirect light. The eye's place is taken out of each column before they
// are summed, so only what is left of it, the same for every pixel,
// carries that error.
fn from_eye(pixel: vec2<i32>, depth: f32) -> vec3<f32> {
    let ndc = (vec2<f32>(pixel) + vec2<f32>(0.5)) * params.screen.zw * vec2<f32>(2.0, -2.0)
        + vec2<f32>(-1.0, 1.0);
    let m = params.world_from_clip;
    let eye = params.camera.xyz;
    let place = (m[0].xyz - eye * m[0].w) * ndc.x + (m[1].xyz - eye * m[1].w) * ndc.y
        + (m[2].xyz - eye * m[2].w) * depth + (m[3].xyz - eye * m[3].w);
    return place / (m[0].w * ndc.x + m[1].w * ndc.y + m[2].w * depth + m[3].w);
}

// The width of a pixel on the surface at `pixel` (m).
fn pixel_width(pixel: vec2<i32>, depth: f32) -> f32 {
    return distance(from_eye(pixel + vec2<i32>(1, 0), depth), from_eye(pixel, depth));
}

fn depth_at(pixel: vec2<i32>) -> f32 {
    let size = vec2<i32>(params.screen.xy);
    return textureLoad(depth_tex, clamp(pixel, vec2<i32>(0), size - vec2<i32>(1)), 0);
}

// The surface's normal from depth: of the two neighbours on each axis, the
// one nearer in depth, so an edge does not bend the normal of the surface
// in front of it. Their places are taken from the eye (`from_eye`).
fn normal_at(pixel: vec2<i32>, depth: f32) -> vec3<f32> {
    let here = from_eye(pixel, depth);
    let dl = depth_at(pixel - vec2<i32>(1, 0));
    let dr = depth_at(pixel + vec2<i32>(1, 0));
    let du = depth_at(pixel - vec2<i32>(0, 1));
    let dd = depth_at(pixel + vec2<i32>(0, 1));
    var dx: vec3<f32>;
    if (abs(dr - depth) < abs(dl - depth)) {
        dx = from_eye(pixel + vec2<i32>(1, 0), dr) - here;
    } else {
        dx = here - from_eye(pixel - vec2<i32>(1, 0), dl);
    }
    var dy: vec3<f32>;
    if (abs(dd - depth) < abs(du - depth)) {
        dy = from_eye(pixel + vec2<i32>(0, 1), dd) - here;
    } else {
        dy = here - from_eye(pixel - vec2<i32>(0, 1), du);
    }
    var n = normalize(cross(dy, dx));
    if (dot(n, here) > 0.0) {
        n = -n;
    }
    return n;
}

// The mesh's smooth normal, the shading's without its normal map, from
// the xy of the depth prepass's normals target (object.wgsl `depth_normal`,
// as gtao.wgsl `oct_decode` reads it).
fn mesh_normal(packed: vec2<f32>) -> vec3<f32> {
    let e = vec2<f32>(packed.x - select(0.0, 2.0, packed.x > 1.5), packed.y) * 2.0 - vec2<f32>(1.0);
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>((vec2<f32>(1.0) - abs(n.yx)) * s, n.z);
    }
    return normalize(n);
}

// Where a ray from the pixel at `world` starts: off the surface along its
// normal bent towards the eye (the depth's normal is a face's, so on curved
// meshes, avatars most of all, the eye's side is the one surely outside),
// by more far away, where depth is coarser.
fn ray_origin(world: vec3<f32>, n: vec3<f32>, dist: f32) -> vec3<f32> {
    let off = normalize(n + normalize(params.camera.xyz - world));
    return world + off * (0.01 + 0.002 * dist);
}

// Two numbers in [0, 1) for a pixel's point on a light's disc this frame
// (rt_shadows.wgsl's sun, rt_lights.wgsl's lamps): a sequence that fills
// the disc evenly from frame to frame (two of R3's three, Roberts 2018:
// rt_lights.wgsl `turn` is the third), from a start that differs evenly
// from a pixel to its neighbours (interleaved gradient noise, Jimenez
// 2014).
fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

fn disc_point(id: vec2<u32>, frame: u32) -> vec2<f32> {
    let p = vec2<f32>(id);
    let start = vec2<f32>(ign(p), ign(p.yx + vec2<f32>(31.0, 17.0)));
    let step = vec2<f32>(vec2<u32>(frame * 2882110345u, frame * 2360945575u) >> vec2<u32>(8u)) / 16777216.0;
    return fract(start + step);
}
