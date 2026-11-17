// Where a pixel's surface was last frame, for the traced passes that keep a
// history (rt_gi.wgsl, rt_lights.wgsl). A still surface goes back through
// the camera alone; one that moved on its own (an avatar, a moving object)
// through the depth prepass's motion, as taa.wgsl reprojects them.
//
// Expects `params.prev_clip_from_world`, `params.camera`,
// `params.prev_camera`, `params.screen` and `motion_tex` (the prepass's
// normals and motion target: zw the pixel's screen motion since last frame,
// in uv; 0 where nothing moved or no motion was drawn).

struct Back {
    // last frame's pixel (full resolution), negative off its screen
    at: vec2<f32>,
    // the distance from last frame's eye its history should have, and how
    // far from it the history may be and still be the same surface
    expected: f32,
    tolerance: f32,
    // the surface moved on its own
    moving: bool,
};

// Where `p` was on last frame's screen (full resolution pixels), or
// negative off it.
fn prev_pixel(p: vec3<f32>) -> vec2<f32> {
    let clip = params.prev_clip_from_world * vec4<f32>(p, 1.0);
    if (clip.w <= 0.0) {
        return vec2<f32>(-1.0);
    }
    let ndc = clip.xy / clip.w;
    if (any(abs(ndc) >= vec2<f32>(1.0))) {
        return vec2<f32>(-1.0);
    }
    return (ndc * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5)) * params.screen.xy;
}

fn back_of(pixel: vec2<i32>, world: vec3<f32>, dist: f32) -> Back {
    let still = prev_pixel(world);
    let expected = distance(world, params.prev_camera.xyz);
    var b = Back(still, expected, 0.02 + 0.03 * expected, false);
    let m = textureLoad(motion_tex, pixel, 0).zw;
    if (any(m != vec2<f32>(0.0))) {
        let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) * params.screen.zw - m;
        let at = uv * params.screen.xy;
        // the motion has no jitter and this pixel does: half a pixel each
        // way still counts as the camera's own move
        if (still.x < 0.0 || distance(at, still) > 1.5) {
            b.moving = true;
            b.at = select(vec2<f32>(-1.0), at, all(uv >= vec2<f32>(0.0)) && all(uv < vec2<f32>(1.0)));
            // where it stood then is unknown: about as far as now, give or
            // take the eye's own move
            b.expected = dist;
            b.tolerance = 0.05 + 0.03 * dist + distance(params.camera.xyz, params.prev_camera.xyz);
        }
    }
    return b;
}

// The denoiser's guide of a pixel (rt_denoise.wgsl): its normal in two
// numbers (octahedral, `unpack_normal` there), its distance from the eye,
// and the second moment of its luminance.
fn pack_normal(n: vec3<f32>) -> vec2<f32> {
    let p = n.xy / (abs(n.x) + abs(n.y) + abs(n.z));
    if (n.z >= 0.0) {
        return p;
    }
    return (vec2<f32>(1.0) - abs(p.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0));
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}
