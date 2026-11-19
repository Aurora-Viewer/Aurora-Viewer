// Ray-traced indirect light of the main view (rt_shadows.rs `Gi`): one
// bounce off the surfaces around, in place of GTAO's screen-space indirect
// light (frame binding 14, common.wgsl `screen_indirect`), in its units:
// irradiance over pi, which the shading multiplies by the diffuse colour.
// The sky's own light stays the hemisphere's and the probes'.
//
// `cs_gi`, at half resolution: one cosine-weighted ray per pixel and frame
// from the depth prepass (the first pixel of each 2×2, always: see `cs_gi`),
// nearest hit. The light is gathered about the mesh's smooth normal where
// the prepass drew it (rt_history.wgsl `smooth_normal`), as the shading
// takes it: about the triangle's, a mesh drawn smooth over few triangles
// showed each one as a block of its own light (#993, as GTAO's). The rays
// start off the triangle's. A hit the view sees takes last frame's lit colour from the
// scene copy (so the local lights and the light already bounced come back
// too); one it does not see is shaded here (rt_light.wgsl `hit_radiance`:
// the sun, the sky and the point lights, a ray to the sun and one to a
// lamp drawn among them where the view traces their shadows; measured in a
// room its lamps light, such hits gave half the light of the same seen on
// screen before the lamps were counted, and 85 % with them, the rest being
// the light bounced once already);
// a miss brings nothing. The ray is not the pixel's light as it is: a face
// lit by a few bright places (a patch of sun on the ground beside it) finds
// them with a ray in many, and its pixels disagree for as long. ReSTIR GI
// (Ouyang et al. 2021, "ReSTIR GI: Path Resampling for Real-Time Path
// Tracing") keeps for each pixel the hit worth the most to it, its `Pick`:
// the ray's hit is weighed against the pixel's pick of last frame by the
// light each brings (resampled importance sampling, as rt_lights.wgsl
// chooses its lamps), and one of the two stays.
// `cs_gi_reuse`: the picks of `NEIGHBOURS` pixels around on the same face
// are weighed in too, each as this pixel would receive its hit (the paper's
// Jacobian), so the light found by one pixel serves those around; shadow
// rays keep it from crossing what stands between, and count only the
// neighbours that could have found the hit themselves (the paper's
// unbiased form), so no light is lost or gained by it. The pick left
// lights the pixel this frame and goes no further: a pixel's pick of last
// frame is of its own rays alone. (Kept for the next frame, as ReSTIR
// does for lamps, a pick taken from a neighbour is weighed again by the
// pixels around it at every frame: measured, the plaza's indirect light
// ended a third too bright.) That light
// is blended with last frame's, reprojected (`kept`), until
// `params.half.w` frames; a pixel that finds its surface in none of the
// history's pixels around starts again. Last frame's is taken as the
// denoiser's first pass left it (`prev_fed`), as SVGF feeds its history.
// Where the light changed lately (`light_changed`: a lamp switched, moved
// or dimmed, the sun's or the sky's light jumped; rt_light_changes.rs),
// the history holds two frames and a pick whose hit is there is left: a
// lamp gone out would go on lighting the room for the history's length.
// Near a lamp that moves both hold fewer frames (`lamps_fresh`).
// Its guide keeps the luminance's second moment beside, for the denoiser
// (rt_denoise.wgsl), which then filters the half resolution.
// `cs_gi_filter`, at full resolution: the denoised half resolution brought
// up, through the nearest half resolution pixels at the same depth.

// After rt_common.wgsl, rt_view.wgsl, rt_light.wgsl, rt_history.wgsl and
// wgpu_ray_query's enable (rt_shadows.rs `source`).

struct Params {
    // This frame's projection (with the prepass's jitter), its inverse, and
    // last frame's.
    clip_from_world: mat4x4<f32>,
    world_from_clip: mat4x4<f32>,
    prev_clip_from_world: mat4x4<f32>,
    // xyz the eye, this frame's and last frame's.
    camera: vec4<f32>,
    prev_camera: vec4<f32>,
    lit: HitLight,
    // full resolution: width, height, 1 / width, 1 / height
    screen: vec4<f32>,
    // half resolution: width, height; z: the strength (Indirect light
    // intensity, as GTAO's); w: the frames the history averages at most
    // (rt_shadows.rs `gi_history_frames`: half a second of them)
    half: vec4<f32>,
    // x: the rays' reach (m), y: 1 when the avatars' top level is traced,
    // z: the frame's number, w: 1 when last frame's history is valid.
    mode: vec4<f32>,
    // Where the light changed lately (rt_light_changes.rs): xyz a ball's
    // centre, w its radius.
    changed: array<vec4<f32>, 8>,
    // x: how many of them, y: 1 when it changed everywhere, z: the frames
    // the history averages at most where it did.
    changes: vec4<f32>,
    // x: the point lights (`lights`), y: 1 when their shadows are traced,
    // z: 1 when one of them moved since the last frame.
    lamps: vec4<f32>,
    // The view `scene_tex` was drawn with, and xy: its size, z: 1 when
    // there is one, w: 1 when `scene_depth_tex` is its depth. Last frame's
    // and the screen's, but for a frame with no history: the view's last
    // frame kept for a photo, with its depth, or none.
    scene_clip_from_world: mat4x4<f32>,
    scene_world_from_clip: mat4x4<f32>,
    scene: vec4<f32>,
};

// Frames the history averages at most on a surface that moved on its own,
// whose light changes as it goes.
const HISTORY_MOVING: f32 = 8.0;
// Rays a pixel traces on a frame with no history.
const RESET_RAYS: u32 = 16u;
// The brightest a sample counts (its luminance, in the lit colour's units):
// a rare ray that meets a lamp would leave a bright speck for the history's
// length.
const SAMPLE_MAX: f32 = 8.0;
// The rays a pick of last frame stands for at most (ReSTIR's M): past it a
// new ray would no longer weigh against it, and a light that changed would
// stay. Fewer on a surface that moved on its own.
const PICK_RAYS: f32 = 20.0;
// How far a lamp may go while the history holds its light at a place, over
// its distance to the place (`lamps_fresh`). Measured with a lamp carried
// at 3 m/s across a room at night, 60 frames a second: the room's indirect
// light 2.2 levels of 255 off on average with the whole history, 1.9 with a
// tenth, 0.9 with this (0.3 between two runs of the same), for a twentieth
// more grain.
const LAMP_LAG: f32 = 0.03;
const PICK_RAYS_MOVING: f32 = 8.0;
// The pixels around whose picks a pixel weighs, and how far they are taken
// (half resolution pixels). Two: four took a tenth more of the grain out
// for half as much time again in the reuse.
const NEIGHBOURS: u32 = 2u;
const NEIGHBOUR_REACH: f32 = 30.0;
// The most a neighbour's pick gains or loses by this pixel being nearer its
// hit or more in front of it than the neighbour is (past it the pick is
// left), and the most a pick's weight is kept at: past them one pick would
// outshine the rest for its whole stay.
const JACOBIAN_MAX: f32 = 4.0;
const WEIGHT_MAX: f32 = 1.0e4;

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var depth_tex: texture_depth_2d;

// cs_gi and cs_gi_reuse
// the guide (rt_history.wgsl `pack_normal`): z the distance, w the
// luminance's second moment
@group(0) @binding(3) var prev_guide: texture_2d<f32>;
@group(0) @binding(4) var scene_tlas: acceleration_structure;
@group(0) @binding(5) var posed_tlas: acceleration_structure;
// the depth prepass's motion (rt_history.wgsl), or black without it
@group(0) @binding(12) var motion_tex: texture_2d<f32>;

// cs_gi
// last frame's lit scene (the scene copy, not yet made this frame)
@group(0) @binding(8) var scene_tex: texture_2d<f32>;
// the depth `scene_tex` was drawn with, when it is kept beside it
// (`params.scene.w`); this frame's otherwise, unread
@group(0) @binding(21) var scene_depth_tex: texture_depth_2d;
// last frame's picks (`Pick`): xyz the hit, w its rays and normal
// (`pack_tail`); rgb the hit's light, a the pick's weight
@group(0) @binding(14) var prev_hit: texture_2d<f32>;
@group(0) @binding(15) var prev_pick: texture_2d<f32>;
// this frame's
@group(0) @binding(16) var out_hit: texture_storage_2d<rgba32float, write>;
@group(0) @binding(17) var out_pick: texture_storage_2d<rgba16float, write>;
// The frame's point lights: for the hits the view does not see (cs_gi), and
// for those that move (`lamps_fresh`).
@group(0) @binding(20) var<storage, read> lights: array<Light>;

// cs_gi_reuse
// last frame's history: a the frames it averages (its rgb is `prev_fed`'s
// before the denoiser)
@group(0) @binding(2) var prev_color: texture_2d<f32>;
@group(0) @binding(6) var out_color: texture_storage_2d<rgba16float, write>;
@group(0) @binding(7) var out_guide: texture_storage_2d<rgba16float, write>;
// last frame's history after the denoiser's first pass (rt_denoise.wgsl
// `cs_atrous` at a step of one pixel): rgb
@group(0) @binding(13) var prev_fed: texture_2d<f32>;
// this frame's picks, as `cs_gi` left them
@group(0) @binding(18) var hit_tex: texture_2d<f32>;
@group(0) @binding(19) var pick_tex: texture_2d<f32>;

// cs_gi_filter (its own bind group: it reads what the denoiser wrote)
@group(0) @binding(9) var gi_color: texture_2d<f32>;
@group(0) @binding(10) var gi_guide: texture_2d<f32>;
@group(0) @binding(11) var out_gi: texture_storage_2d<rgba16float, write>;

// `c` no brighter than `SAMPLE_MAX`, its colour kept: held channel by
// channel, a strong coloured light turned towards white.
fn capped(c: vec3<f32>) -> vec3<f32> {
    // (a light past any a lamp gives first: scaling it would leave no number)
    let bounded = min(c, vec3<f32>(1.0e4));
    return bounded * min(1.0, SAMPLE_MAX / max(luma(bounded), 1e-6));
}

// `c` as a 16-bit float holds it, each channel rounded up or down by the
// chance `u` so that its mean is `c`, for a value kept from frame to frame.
// A store rounds as the card does, and one that rounds down takes a little
// of the light at every frame: measured over the 128 frames the history
// holds at 240 frames a second, a room's wall was 3 levels of 255 darker
// than at 60, and 4 darker than with no history. (rt_denoise.wgsl has the
// same.)
fn dithered(c: vec3<f32>, u: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(65504.0));
    // a 16-bit float's steps: 1024 to each power of two, down to 2^-24
    let e = max(floor(log2(max(x, vec3<f32>(1.0e-30)))), vec3<f32>(-14.0));
    let step = exp2(e - vec3<f32>(10.0));
    let q = x / step;
    let lo = floor(q);
    return (lo + vec3<f32>(u < (q - lo))) * step;
}

// The share of a frame in the history of the light at `p`, `slowest` where
// no lamp moves: the light a lamp that moves carries with it goes as far,
// and a history of half a second trailed it across a room. Each lamp's share
// is its step over what `LAMP_LAG` of its distance allows, and they are
// weighed by their light at `p` among the lamps', the sky's and the sun's:
// under the day's light a lamp that moves shortens nothing.
fn lamps_fresh(p: vec3<f32>, slowest: f32) -> f32 {
    // the frames no lamp moves, which is most of them, read no light
    if (params.lamps.z < 0.5) {
        return slowest;
    }
    // what the sky and the sun give a face, as `lamp_light` counts a lamp's
    var all = luma(params.lit.sky_zenith.rgb);
    if (params.lit.light.w > 0.5) {
        all += luma(params.lit.sun_color.rgb) / PI;
    }
    var fresh = all * slowest;
    let count = min(u32(params.lamps.x), HIT_LIGHTS);
    for (var i = 0u; i < count; i++) {
        let lt = lights[i];
        let d = distance(lt.pos_radius.xyz, p);
        if (lt.source.y > 0.5 || d >= lt.pos_radius.w) {
            continue;
        }
        let f = max(lt.color_falloff.w, 0.0);
        let x = 1.0 - clamp((d / lt.pos_radius.w + f) / (1.0 + f), 0.0, 1.0);
        let w = luma(lt.color_falloff.rgb) * 2.0 * x * x / PI;
        all += w;
        fresh += w * clamp(lt.source.z / (LAMP_LAG * max(d, 0.5)), slowest, 1.0);
    }
    return fresh / max(all, 1e-6);
}

// A direction about `n`, more of them where the cosine is larger.
fn cosine_dir(n: vec3<f32>, u: vec2<f32>) -> vec3<f32> {
    let r = sqrt(u.x);
    let phi = 2.0 * PI * u.y;
    let s = select(-1.0, 1.0, n.z >= 0.0);
    let a = -1.0 / (s + n.z);
    let b = n.x * n.y * a;
    let t = vec3<f32>(1.0 + s * n.x * n.x * a, s * b, -s * n.x);
    let bt = vec3<f32>(b, s + n.y * n.y * a, -n.y);
    return normalize(t * (r * cos(phi)) + bt * (r * sin(phi)) + n * sqrt(max(1.0 - u.x, 0.0)));
}

// Where `p` is in `scene_tex`, the lit scene of a frame before this one
// (its pixels), or negative where that frame did not show it.
fn scene_pixel(p: vec3<f32>) -> vec2<f32> {
    if (params.scene.z < 0.5) {
        return vec2<f32>(-1.0);
    }
    let clip = params.scene_clip_from_world * vec4<f32>(p, 1.0);
    if (clip.w <= 0.0) {
        return vec2<f32>(-1.0);
    }
    let ndc = clip.xy / clip.w;
    if (any(abs(ndc) >= vec2<f32>(1.0))) {
        return vec2<f32>(-1.0);
    }
    return (ndc * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5)) * params.scene.xy;
}

// What the view shows of a hit: last frame's lit colour there, and the
// normal of what it shows.
struct Seen {
    light: vec3<f32>,
    normal: vec3<f32>,
    seen: bool,
};

// Last frame's lit colour where the view sees `p` (the view's depth this
// frame finds it in front), met along `dir`; `seen` false where it does
// not. Its normal is the depth's there, turned to face the ray: the hit's
// own (rt_common.wgsl `surface_at`) reads its face's vertices, a quarter
// of the pass's time when every hit asked for it.
fn seen_before(p: vec3<f32>, dir: vec3<f32>) -> Seen {
    var out = Seen(vec3<f32>(0.0), vec3<f32>(0.0, 0.0, 1.0), false);
    if (params.scene.w > 0.5) {
        // A photo: what the view kept for it shows of `p`, by that view's
        // own depth and not this frame's. A tile of a photo sees its part
        // alone, and a hit lit one way in a tile and another in the next
        // drew the tiles' edges on the walls. (The caller reads the hit's
        // own normal: this depth is not `normal_at`'s.)
        let at = scene_pixel(p);
        if (at.x < 0.0) {
            return out;
        }
        let px = vec2<i32>(at);
        let d = textureLoad(scene_depth_tex, px, 0);
        if (d <= 0.0) {
            return out;
        }
        let ndc = (vec2<f32>(px) + vec2<f32>(0.5)) / params.scene.xy * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
        let shown = params.scene_world_from_clip * vec4<f32>(ndc, d, 1.0);
        if (distance(shown.xyz / shown.w, p) > 0.05 + 0.01 * distance(p, params.camera.xyz)) {
            return out;
        }
        out.light = textureLoad(scene_tex, px, 0).rgb;
        out.seen = true;
        return out;
    }
    let clip = params.clip_from_world * vec4<f32>(p, 1.0);
    if (clip.w <= 0.0) {
        return out;
    }
    let ndc = clip.xy / clip.w;
    if (any(abs(ndc) >= vec2<f32>(1.0))) {
        return out;
    }
    let px = vec2<i32>((ndc * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5)) * params.screen.xy);
    let d = textureLoad(depth_tex, px, 0);
    if (d <= 0.0) {
        return out;
    }
    let there = unproject(px, d);
    if (distance(there, p) > 0.05 + 0.01 * distance(p, params.camera.xyz)) {
        return out;
    }
    let before = scene_pixel(p);
    if (before.x < 0.0) {
        return out;
    }
    let n = normal_at(px, there, d);
    out.light = textureLoad(scene_tex, vec2<i32>(before), 0).rgb;
    out.normal = select(n, -n, dot(n, dir) > 0.0);
    out.seen = true;
    return out;
}

// The guide's normal (rt_history.wgsl `pack_normal`).
fn unpack_normal(e: vec2<f32>) -> vec3<f32> {
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>((vec2<f32>(1.0) - abs(n.yx)) * s, n.z);
    }
    return normalize(n);
}

// A surface's light of last frame: rgb, the frames it averages (0 without
// any) and the second moment of its luminance.
struct Kept {
    color: vec3<f32>,
    count: f32,
    moment: f32,
};

// Whether last frame's half resolution pixel `q` held the surface `back`
// describes, facing `n`: its guide in `g`.
fn held(q: vec2<i32>, back: Back, n: vec3<f32>, g: ptr<function, vec4<f32>>) -> bool {
    if (any(q < vec2<i32>(0)) || any(q >= vec2<i32>(params.half.xy))) {
        return false;
    }
    let was = textureLoad(prev_guide, q, 0);
    *g = was;
    return was.z > 0.0 && abs(was.z - back.expected) < back.tolerance && dot(unpack_normal(was.xy), n) > 0.9;
}

// Last frame's light of the surface at `back`, facing `n`: the history
// filtered between the four half resolution pixels around where the
// surface was, those that held it. Beside an outline none may have, once
// the view moves: the neighbours' of the same face serve then, where
// starting again leaves one ray's noise.
fn kept(back: Back, n: vec3<f32>) -> Kept {
    var k = Kept(vec3<f32>(0.0), 0.0, 0.0);
    if (params.mode.w < 0.5 || back.at.x < 0.0) {
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
        if (w < 1e-3 || !held(corner + o, back, n, &g)) {
            continue;
        }
        k.color += textureLoad(prev_fed, corner + o, 0).rgb * w;
        k.count += textureLoad(prev_color, corner + o, 0).a * w;
        k.moment += g.w * w;
        weight += w;
    }
    if (weight < 1e-3) {
        weight = 0.0;
        let near = vec2<i32>(round(c));
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let q = near + vec2<i32>(x, y);
                if (!held(q, back, n, &g)) {
                    continue;
                }
                k.color += textureLoad(prev_fed, q, 0).rgb;
                k.count += textureLoad(prev_color, q, 0).a;
                k.moment += g.w;
                weight += 1.0;
            }
        }
    }
    if (weight > 0.0) {
        k.color /= weight;
        k.count /= weight;
        k.moment /= weight;
    }
    return k;
}

// A pixel's pick (ReSTIR GI's reservoir): the hit kept, the light it
// sends, its normal (facing where it was traced from), the weight the
// light takes at the pixel (the paper's W) and the rays it stands for (M).
struct Pick {
    hit: vec3<f32>,
    normal: vec3<f32>,
    light: vec3<f32>,
    weight: f32,
    rays: f32,
};

fn no_pick() -> Pick {
    return Pick(vec3<f32>(0.0), vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0), 0.0, 0.0);
}

// A pick's rays (31 at most) and its normal (9 bits a side) as one whole
// number, which a 32-bit float holds as it is.
fn pack_tail(rays: f32, n: vec3<f32>) -> f32 {
    let e = clamp(pack_normal(n) * 0.5 + vec2<f32>(0.5), vec2<f32>(0.0), vec2<f32>(1.0));
    let q = vec2<u32>(e * 511.0 + vec2<f32>(0.5));
    return f32(u32(clamp(rays, 0.0, 31.0)) + 32u * (q.x + 512u * q.y));
}

fn unpack_pick(hit: vec4<f32>, light: vec4<f32>) -> Pick {
    let bits = u32(hit.w);
    let e = vec2<f32>(f32((bits >> 5u) & 511u), f32(bits >> 14u)) / 511.0;
    return Pick(hit.xyz, unpack_normal(e * 2.0 - vec2<f32>(1.0)), light.rgb, light.a, f32(bits & 31u));
}

// What the light `light` from `hit` is worth to the surface at `seat`
// facing `n` (ReSTIR's target): its luminance as the surface receives it,
// over pi.
fn worth(light: vec3<f32>, hit: vec3<f32>, seat: vec3<f32>, n: vec3<f32>) -> f32 {
    return luma(light) * max(dot(n, normalize(hit - seat)), 0.0) / PI;
}

// Numbers in [0, 1) for pixel `p` on frame `frame`, apart from `random2`'s
// of the ray and from one `stream` to the next.
fn chance(p: vec2<u32>, frame: u32, stream: u32) -> vec2<f32> {
    return random2(vec2<u32>(p.x, p.y + stream * 65536u), frame);
}

// Last frame's half resolution pixel that held the surface at `back`
// facing `n`, the nearest of the four around where it was; negative
// without one. A pick is one hit: it is taken whole, not filtered.
fn held_at(back: Back, n: vec3<f32>) -> vec2<i32> {
    var best = vec2<i32>(-1);
    if (back.at.x < 0.0) {
        return best;
    }
    let c = (back.at - vec2<f32>(0.5)) * 0.5;
    let corner = vec2<i32>(floor(c));
    let f = c - floor(c);
    var most = 0.0;
    var g = vec4<f32>(0.0);
    for (var i = 0u; i < 4u; i++) {
        let o = vec2<i32>(i32(i & 1u), i32(i >> 1u));
        let w = mix(1.0 - f.x, f.x, f32(o.x)) * mix(1.0 - f.y, f.y, f32(o.y));
        if (w > most && held(corner + o, back, n, &g)) {
            most = w;
            best = corner + o;
        }
    }
    return best;
}

@compute @workgroup_size(8, 8)
fn cs_gi(@builtin(global_invocation_id) id: vec3<u32>) {
    let hp = vec2<i32>(id.xy);
    if (any(hp >= vec2<i32>(params.half.xy))) {
        return;
    }
    let frame = u32(params.mode.z);
    // The same pixel of each 2×2 at every frame. Taking each in turn, a
    // 2×2 across an edge was one face then the other: its history and what
    // `cs_gi_filter` spread from it changed with the frame, and the edges
    // glittered with the view still.
    let pixel = hp * 2;
    let depth = textureLoad(depth_tex, pixel, 0);
    // reversed depth: 0 is the sky
    if (depth <= 0.0) {
        textureStore(out_hit, hp, vec4<f32>(0.0));
        textureStore(out_pick, hp, vec4<f32>(0.0));
        return;
    }
    let world = unproject(pixel, depth);
    let flat = normal_at(pixel, world, depth);
    let n = smooth_normal(pixel, flat);
    let dist = distance(world, params.camera.xyz);
    let origin = ray_origin(world, flat, dist);
    let posed = params.mode.y > 0.5;
    // with no history (a photo, a new size), more rays at once
    let rays = select(1u, RESET_RAYS, params.mode.w < 0.5);
    var pick = no_pick();
    // the worths weighed so far, each over how likely its hit was drawn
    var sum = 0.0;
    for (var ray = 0u; ray < rays; ray++) {
        let dir = cosine_dir(n, random2(vec2<u32>(id.xy), frame * RESET_RAYS + ray));

        var t = -1.0;
        var record = 0u;
        var primitive = 0u;
        var bary = vec2<f32>(0.0);
        var rq: ray_query;
        rayQueryInitialize(&rq, scene_tlas, RayDesc(0u, 0xFFu, 0.0, params.mode.x, origin, dir));
        while (rayQueryProceed(&rq)) {
            if (stops(rayQueryGetCandidateIntersection(&rq), false, dir)) {
                rayQueryConfirmIntersection(&rq);
            }
        }
        let hs = rayQueryGetCommittedIntersection(&rq);
        if (hs.kind != RAY_QUERY_INTERSECTION_NONE) {
            t = hs.t;
            let h = hit_of(false, hs.instance_custom_data, hs.geometry_index, hs.primitive_index);
            record = h.record;
            primitive = h.primitive;
            bary = hs.barycentrics;
        }
        if (posed) {
            let reach = select(params.mode.x, t, t >= 0.0);
            var rp: ray_query;
            rayQueryInitialize(&rp, posed_tlas, RayDesc(0u, 0xFFu, 0.0, reach, origin, dir));
            while (rayQueryProceed(&rp)) {
                if (stops(rayQueryGetCandidateIntersection(&rp), true, dir)) {
                    rayQueryConfirmIntersection(&rp);
                }
            }
            let hp2 = rayQueryGetCommittedIntersection(&rp);
            if (hp2.kind != RAY_QUERY_INTERSECTION_NONE) {
                t = hp2.t;
                let h = hit_of(true, hp2.instance_custom_data, hp2.geometry_index, hp2.primitive_index);
                record = h.record;
                primitive = h.primitive;
                bary = hp2.barycentrics;
            }
        }
        // a miss brings nothing: it counts as a ray and is never the pick
        if (t < 0.0) {
            continue;
        }
        let hit = origin + dir * t;
        var one = vec3<f32>(0.0);
        var facing = vec3<f32>(0.0);
        let before = seen_before(hit, dir);
        if (before.seen && params.scene.w < 0.5) {
            one = before.light;
            facing = before.normal;
        } else {
            let s = surface_at(record, primitive, bary, dir);
            if (before.seen) {
                one = before.light;
            } else {
                let lamps = Lamps(u32(params.lamps.x), params.lamps.y > 0.5, chance(id.xy, frame, 40u + ray).x);
                one = hit_radiance(params.lit, s, hit, 2.0 + log2(1.0 + t), 0.01 + 0.002 * (dist + t), params.mode.x, posed, lamps);
            }
            facing = s.normal;
        }
        let light = capped(one);
        // the ray was drawn with a density of its cosine over pi
        let w = worth(light, hit, origin, n) * PI / max(dot(n, dir), 1e-4);
        sum += w;
        if (w > 0.0 && chance(id.xy, frame, 1u + ray).x * sum < w) {
            pick.hit = hit;
            pick.normal = facing;
            pick.light = light;
        }
    }
    pick.rays = f32(rays);

    // last frame's pick at the same surface, as this pixel receives its hit
    if (params.mode.w > 0.5) {
        let back = back_of(pixel, world, dist);
        let q = held_at(back, n);
        var was: Pick;
        if (q.x >= 0) {
            was = unpack_pick(textureLoad(prev_hit, q, 0), textureLoad(prev_pick, q, 0));
        }
        // (a hit where the light changed kept the light it had: left)
        if (q.x >= 0 && !light_changed(was.hit)) {
            let longest = select(PICK_RAYS, PICK_RAYS_MOVING, back.moving);
            was.rays = min(was.rays, 1.0 / lamps_fresh(was.hit, 1.0 / longest));
            let w = worth(was.light, was.hit, origin, n) * was.weight * was.rays;
            sum += w;
            pick.rays += was.rays;
            if (w > 0.0 && chance(id.xy, frame, 20u).x * sum < w) {
                pick.hit = was.hit;
                pick.normal = was.normal;
                pick.light = was.light;
            }
        }
    }
    let p = worth(pick.light, pick.hit, origin, n);
    pick.weight = select(0.0, min(sum / (pick.rays * p), WEIGHT_MAX), p > 0.0);
    textureStore(out_hit, hp, vec4<f32>(pick.hit, pack_tail(pick.rays, pick.normal)));
    textureStore(out_pick, hp, vec4<f32>(pick.light, pick.weight));
}

// What the weight of a hit at `hit` facing `hn` gains when it passes from
// the pixel whose rays start at `a` on a surface facing `an`, which drew
// it, to the one whose rays start at `b`, facing `bn`: the hit lights `b`
// as it faces it and by how near it is, not as it lit `a` (the paper's
// Jacobian, its eq. 11). Negative where it does not pass: behind one of
// the two or past the rays' reach from it, edge-on, or a gain or a loss
// past `JACOBIAN_MAX`. Both pixels are taken where their rays start
// (rt_view.wgsl `ray_origin`), which is where a hit's direction was drawn
// from: taken at the surface, the hits nearest a pixel weighed more than
// they brought, and what stands beside a wall lit it too much.
fn carried(hit: vec3<f32>, hn: vec3<f32>, a: vec3<f32>, an: vec3<f32>, b: vec3<f32>, bn: vec3<f32>) -> f32 {
    let to_a = a - hit;
    let to_b = b - hit;
    let da = dot(to_a, to_a);
    let db = dot(to_b, to_b);
    let reach = params.mode.x * params.mode.x;
    if (dot(an, to_a) >= 0.0 || dot(bn, to_b) >= 0.0 || da > reach || db > reach) {
        return -1.0;
    }
    let facing_a = dot(hn, to_a) * inverseSqrt(max(da, 1e-8));
    let facing_b = dot(hn, to_b) * inverseSqrt(max(db, 1e-8));
    if (facing_a <= 1e-3 || facing_b <= 1e-3) {
        return -1.0;
    }
    let gain = facing_b / facing_a * da / max(db, 1e-8);
    if (gain > JACOBIAN_MAX || gain * JACOBIAN_MAX < 1.0) {
        return -1.0;
    }
    return gain;
}

// Whether the pixel whose rays start at `origin`, `dist` from the eye, sees
// `hit`: one shadow ray.
fn sees(origin: vec3<f32>, dist: f32, hit: vec3<f32>) -> bool {
    let to = hit - origin;
    let far = length(to);
    let reach = far - (0.02 + 0.004 * (dist + far));
    return reach <= 0.0 || unblocked(origin, to / far, reach, params.mode.y > 0.5);
}

@compute @workgroup_size(8, 8)
fn cs_gi_reuse(@builtin(global_invocation_id) id: vec3<u32>) {
    let hp = vec2<i32>(id.xy);
    if (any(hp >= vec2<i32>(params.half.xy))) {
        return;
    }
    let frame = u32(params.mode.z);
    let pixel = hp * 2;
    let depth = textureLoad(depth_tex, pixel, 0);
    if (depth <= 0.0) {
        textureStore(out_color, hp, vec4<f32>(0.0));
        textureStore(out_guide, hp, vec4<f32>(0.0));
        return;
    }
    let world = unproject(pixel, depth);
    let flat = normal_at(pixel, world, depth);
    let n = smooth_normal(pixel, flat);
    let dist = distance(world, params.camera.xyz);
    let origin = ray_origin(world, flat, dist);

    let own = unpack_pick(textureLoad(hit_tex, hp, 0), textureLoad(pick_tex, hp, 0));
    var pick = own;
    var sum = worth(own.light, own.hit, origin, n) * own.weight * own.rays;
    // the neighbours taken: where their rays start, facing what, how far
    // from the eye, the rays of their pick
    // (0: not taken); and the one the pick came from, if not the pixel's own
    var near_at: array<vec3<f32>, NEIGHBOURS>;
    var near_n: array<vec3<f32>, NEIGHBOURS>;
    var near_dist: array<f32, NEIGHBOURS>;
    var near_rays: array<f32, NEIGHBOURS>;
    var source = NEIGHBOURS;
    // one in each ring of equal area about the pixel, the rings turned by
    // the golden angle and all of them anew each frame
    let u = chance(id.xy, frame, 21u);
    let last = vec2<i32>(params.half.xy) - vec2<i32>(1);
    for (var k = 0u; k < NEIGHBOURS; k++) {
        near_rays[k] = 0.0;
        let radius = NEIGHBOUR_REACH * sqrt((f32(k) + u.x) / f32(NEIGHBOURS));
        let angle = 2.0 * PI * (u.y + f32(k) * 0.61803399);
        let q = hp + vec2<i32>(round(vec2<f32>(cos(angle), sin(angle)) * radius));
        if (any(q < vec2<i32>(0)) || any(q > last) || all(q == hp)) {
            continue;
        }
        let depth_q = textureLoad(depth_tex, q * 2, 0);
        if (depth_q <= 0.0) {
            continue;
        }
        // on this pixel's face: in its plane and facing as it does, where
        // the two receive most of the same light
        let at_q = unproject(q * 2, depth_q);
        if (abs(dot(flat, at_q - world)) > 0.01 + 0.01 * dist) {
            continue;
        }
        let flat_q = normal_at(q * 2, at_q, depth_q);
        let n_q = smooth_normal(q * 2, flat_q);
        if (dot(n, n_q) < 0.9) {
            continue;
        }
        let theirs = unpack_pick(textureLoad(hit_tex, q, 0), textureLoad(pick_tex, q, 0));
        let dist_q = distance(at_q, params.camera.xyz);
        let origin_q = ray_origin(at_q, flat_q, dist_q);
        near_at[k] = origin_q;
        near_n[k] = n_q;
        near_dist[k] = dist_q;
        near_rays[k] = theirs.rays;
        if (theirs.weight <= 0.0) {
            continue;
        }
        let gain = carried(theirs.hit, theirs.normal, origin_q, n_q, origin, n);
        if (gain < 0.0) {
            continue;
        }
        let w = worth(theirs.light, theirs.hit, origin, n) * theirs.weight * theirs.rays * gain;
        sum += w;
        if (w > 0.0 && chance(id.xy, frame, 22u + k).x * sum < w) {
            pick.hit = theirs.hit;
            pick.normal = theirs.normal;
            pick.light = theirs.light;
            source = k;
        }
    }
    // The pixel's light this frame: its pick's, at the weight it took. A
    // neighbour's hit that is behind something from here brings none (its
    // share of the weights was for a light that does not arrive): left
    // unchecked, the light went round the corners and under what stands
    // on the ground.
    let p = worth(pick.light, pick.hit, origin, n);
    var sample = vec3<f32>(0.0);
    if (p > 0.0 && sum > 0.0 && (source == NEIGHBOURS || sees(origin, dist, pick.hit))) {
        // The rays the weights are shared between: those of the pixels
        // that could have drawn this hit, the pixel's own and each
        // neighbour that sees it (a shadow ray each; the one it came from
        // drew it). Counting every neighbour, as the paper's cheaper form
        // does, a pixel beside a wall or under an arm lost the light its
        // neighbours could not see from where they are: a third of the
        // indirect light over the demo's plaza, most of it on the avatars
        // and in the corners.
        var among = own.rays;
        for (var k = 0u; k < NEIGHBOURS; k++) {
            if (near_rays[k] <= 0.0) {
                continue;
            }
            if (k != source) {
                if (carried(pick.hit, pick.normal, near_at[k], near_n[k], origin, n) < 0.0) {
                    continue;
                }
                if (!sees(near_at[k], near_dist[k], pick.hit)) {
                    continue;
                }
            }
            among += near_rays[k];
        }
        let weight = min(sum / (among * p), WEIGHT_MAX);
        sample = capped(pick.light * (max(dot(n, normalize(pick.hit - origin)), 0.0) / PI * weight));
    }
    var moment = luma(sample) * luma(sample);

    // last frame's at the same surface
    var color = sample;
    let rays = select(1.0, f32(RESET_RAYS), params.mode.w < 0.5);
    var count = min(rays, params.half.w);
    let back = back_of(pixel, world, dist);
    let past = kept(back, n);
    if (past.count > 0.0) {
        var most = select(params.half.w, HISTORY_MOVING, back.moving);
        most = 1.0 / lamps_fresh(world, 1.0 / most);
        if (light_changed(world)) {
            most = min(most, params.changes.z);
        }
        count = min(past.count + 1.0, most);
        color = mix(past.color, sample, 1.0 / count);
        moment = mix(past.moment, moment, 1.0 / count);
    }
    let toss = vec3<f32>(chance(id.xy, u32(params.mode.z), 30u), chance(id.xy, u32(params.mode.z), 31u).x);
    textureStore(out_color, hp, vec4<f32>(dithered(color, toss), count));
    textureStore(out_guide, hp, vec4<f32>(pack_normal(n), dist, moment));
}

@compute @workgroup_size(8, 8)
fn cs_gi_filter(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    if (any(pixel >= vec2<i32>(params.screen.xy))) {
        return;
    }
    let depth = textureLoad(depth_tex, pixel, 0);
    if (depth <= 0.0) {
        textureStore(out_gi, pixel, vec4<f32>(0.0));
        return;
    }
    let dist = distance(unproject(pixel, depth), params.camera.xyz);
    // a half resolution pixel is the first of its 2×2
    let centre = vec2<f32>(pixel) * 0.5;
    let base = vec2<i32>(floor(centre));
    let last = vec2<i32>(params.half.xy) - vec2<i32>(1);
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var y = -1; y <= 2; y++) {
        for (var x = -1; x <= 2; x++) {
            let q = clamp(base + vec2<i32>(x, y), vec2<i32>(0), last);
            let qd = textureLoad(gi_guide, q, 0).z;
            if (qd <= 0.0) {
                continue;
            }
            let c = textureLoad(gi_color, q, 0);
            let off = vec2<f32>(q) - centre;
            // nearer and of the same depth weigh more
            let w_space = exp(-dot(off, off));
            let w_depth = exp(-abs(qd - dist) / (0.01 + 0.02 * dist));
            let w = w_space * w_depth;
            sum += c.rgb * w;
            weight += w;
        }
    }
    let gi = select(vec3<f32>(0.0), sum / max(weight, 1e-6), weight > 1e-4);
    textureStore(out_gi, pixel, vec4<f32>(gi * params.half.z, 1.0));
}
