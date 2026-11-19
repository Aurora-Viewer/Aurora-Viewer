// Where a pixel's surface was last frame, for the traced passes that keep a
// history (rt_gi.wgsl, rt_lights.wgsl). A still surface goes back through
// the camera alone; one that moved on its own (an avatar, a moving object)
// through the depth prepass's motion, as taa.wgsl reprojects them.
//
// Expects `params.prev_clip_from_world`, `params.camera` (w: 1 when
// `motion_tex` is the prepass's and not a black stand-in),
// `params.prev_camera`, `params.screen`, `params.changed` and
// `params.changes` (`light_changed`), and `motion_tex` (the prepass's
// normals and motion target: xy the mesh's smooth normal, object.wgsl
// `depth_normal`; zw the pixel's screen motion since last frame, in uv, 0
// where nothing moved or no motion was drawn).

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
    // (camera.w 2: a photo's round, nothing moved since the one before)
    if (params.camera.w < 1.5 && any(m != vec2<f32>(0.0))) {
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

// The mesh's smooth normal at `pixel`, the shading's without its normal
// map, where the prepass drew one; else `flat`, the triangle's from depth
// (rt_view.wgsl `normal_at`). A low-poly mesh is lit by the first: what the
// lights are weighed by, and what tells a surface from the next, follow it,
// or each triangle shows (gtao.wgsl `oct_decode`, #993). One far from the
// triangle's is not this surface's (nothing drew it there).
fn smooth_normal(pixel: vec2<i32>, flat: vec3<f32>) -> vec3<f32> {
    if (params.camera.w < 0.5) {
        return flat;
    }
    let packed = textureLoad(motion_tex, pixel, 0).xy;
    let e = vec2<f32>(packed.x - select(0.0, 2.0, packed.x > 1.5), packed.y) * 2.0 - vec2<f32>(1.0);
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>((vec2<f32>(1.0) - abs(n.yx)) * s, n.z);
    }
    n = normalize(n);
    if (dot(n, flat) > 0.7) {
        return n;
    }
    return flat;
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

// Whether the light that falls at `p` changed lately: a lamp there came,
// went, moved or changed its brightness, or the sun's or the sky's light
// jumped (rt_light_changes.rs). The pass's `params` keep the places
// (`changed`: xyz and a radius each) and `changes`: x how many, y 1 when
// the light changed everywhere, z the frames a history averages at most
// where it did.
fn light_changed(p: vec3<f32>) -> bool {
    if (params.changes.y > 0.5) {
        return true;
    }
    let count = u32(params.changes.x);
    for (var i = 0u; i < count; i++) {
        let c = params.changed[i];
        let d = p - c.xyz;
        if (dot(d, d) < c.w * c.w) {
            return true;
        }
    }
    return false;
}
