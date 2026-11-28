// Ray-traced indirect light of the main view (rt_shadows.rs `Gi`): one
// bounce off the surfaces around, in place of GTAO's screen-space indirect
// light (frame binding 14, common.wgsl `screen_indirect`), in its units:
// irradiance over pi, which the shading multiplies by the diffuse colour.
// The sky's own light stays the hemisphere's and the probes'.
//
// `cs_gi`, at half resolution (the High ray tracing quality; Medium and Low
// trace one pixel of each 3×3 or 4×4, `params.grid`, and every pass below
// runs at that size; "half resolution" below says the traced pixels): one
// cosine-weighted ray per pixel and frame
// from the depth prepass (the first pixel of each 2×2, always: see `cs_gi`),
// nearest hit. The light is gathered about the mesh's smooth normal where
// the prepass drew it (rt_history.wgsl `smooth_normal`), as the shading
// takes it: about the triangle's, a mesh drawn smooth over few triangles
// showed each one as a block of its own light (#993, as GTAO's). The rays
// start off the triangle's. A hit the view sees takes last frame's lit colour from the
// scene copy (so the local lights and the light already bounced come back
// too); one it does not see is shaded here (gi_hit.wgsl `gi_hit_radiance`:
// the sun, the sky, or the traced probes' light where they cover the hit
// (gi.rs; the light bounced more than once), and the point lights, a ray to the sun and one to a
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
// unbiased form), each by how likely it was to find it (the balance
// heuristic), so no light is lost or gained by it and a hit lights no
// pixel more than the one that found it. The pick left
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
    // The places the reflection passes are drawn from (`gi_points`): xyz,
    // w 1 when there is one. 0: the reflection probe captured this frame,
    // 1: in front of the mirror.
    points: array<vec4<f32>, 2>,
    // The traced pixels' grid (RtTuning, the ray tracing quality): x the
    // screen pixels a side of one (`params.half` is the screen over it,
    // each the first pixel of its block), y one over x, z the neighbours
    // whose picks `cs_gi_reuse` weighs (`NEIGHBOURS` at most); w the
    // diagnosis of the passes' cost (`AURORA_RT_GI_DIAG`, rt_shadows.rs
    // `GiDiag`), 0 without one: `DIAG_*`.
    grid: vec4<f32>,
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
// The pixels around whose picks a pixel weighs at most (`params.grid.z`
// says how many), and how far they are taken (screen pixels). Two: four
// took a tenth more of the grain out for half as much time again in the
// reuse.
const NEIGHBOURS: u32 = 2u;
const NEIGHBOUR_REACH: f32 = 60.0;
// A pixel whose history averages fewer frames than this (one that just lost
// it: the view moved, or a jitter of the projection put another leaf there)
// shows its few rays nearly as they are; their light is held to
// `FRESH_GAIN` times what last frame's pixels around it had (`around`).
const FRESH_FRAMES: f32 = 4.0;
const FRESH_GAIN: f32 = 3.0;
// How many frames history borrowed from the leaves around may count for
// (`kept`): with only `FRESH_FRAMES` the edges of leaves and twigs kept too
// little and still sparkled. Clamped all the while (`cs_gi_reuse`).
const LOOSE_FRAMES: f32 = 16.0;

// The screen pixels a side of a traced pixel (`params.grid`).
fn grid_step() -> i32 {
    return i32(params.grid.x + 0.5);
}

// `AURORA_RT_GI_DIAG`'s variants (`params.grid.w`), each leaving out one
// part of the work to measure it, the image with it: the view's rays take
// the posed faces as opaque (no alpha test), or leave them out (and so do
// the reuse's shadow rays), the hits the view does not see are lit with
// no shadow ray, or the probes are read with no visibility ray.
const DIAG_POSED_OPAQUE: u32 = 1u;
const DIAG_NO_POSED: u32 = 2u;
const DIAG_NO_HIT_SHADOWS: u32 = 3u;
const DIAG_NO_PROBE_RAYS: u32 = 4u;

fn diag() -> u32 {
    return u32(params.grid.w + 0.5);
}

// Whether the avatars' level is traced: there are posed faces, and no
// diagnosis leaves them out.
fn posed_traced() -> bool {
    return params.mode.y > 0.5 && diag() != DIAG_NO_POSED;
}
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
// the probes' sight of the sky at half resolution (`cs_gi_sky`), read by
// `cs_gi_filter`
@group(0) @binding(21) var out_sky: texture_storage_2d<rgba16float, write>;
@group(0) @binding(22) var sky_tex: texture_2d<f32>;
// how open the places of `params.points` are, in r (`gi_points`)
@group(0) @binding(23) var out_points: texture_storage_2d<rgba16float, write>;

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
    let n = normal_at(px, d);
    // Last frame showed `p` there only if its surface there was `p`, as its
    // guide says (half resolution: the distance from last frame's eye and
    // the normal, which faced that eye). A view that went through a thin
    // wall saw its other side there, and its outer light lit the room for
    // a second. (Looser than the photo's: the guide is of the 2×2's first
    // pixel, a little off `p` on a face seen aslant.)
    let q = min(vec2<i32>(before * params.grid.y), vec2<i32>(params.half.xy) - vec2<i32>(1));
    let was = textureLoad(prev_guide, q, 0);
    let expected = distance(p, params.prev_camera.xyz);
    if (params.mode.w < 0.5 || was.z <= 0.0 || abs(was.z - expected) > 0.05 + 0.03 * expected
        || dot(unpack_normal(was.xy), n) < 0.5) {
        return out;
    }
    // The colour of the pixel the guide describes, not of `before`: on a
    // leaf or a twig against the sky the two differ under a jittered
    // projection, and `before` showed the sky behind (the clouds' light
    // crawled over the leaves).
    let shown_at = min(q * grid_step(), vec2<i32>(params.scene.xy) - vec2<i32>(1));
    out.light = textureLoad(scene_tex, shown_at, 0).rgb;
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
    // the history came from the leaves around (`kept`'s last resort)
    loose: bool,
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
    var k = Kept(vec3<f32>(0.0), 0.0, 0.0, false);
    if (params.mode.w < 0.5 || back.at.x < 0.0) {
        return k;
    }
    // a half resolution pixel is the first of its 2×2
    let c = (back.at - vec2<f32>(0.5)) * params.grid.y;
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
    // None of the same face either: in foliage, where a jitter of the
    // projection or a turn of the view puts each pixel on another leaf, the
    // leaves around at about the same distance, whatever they face, for
    // `LOOSE_FRAMES` at most. Starting again there left one ray's light on
    // each, and the leaves sparkled.
    var loose = false;
    if (weight < 1e-3) {
        weight = 0.0;
        let near = vec2<i32>(round(c));
        // a 3 x 3 first, then a 5 x 5 (a twig's tip, the sky all around it)
        for (var reach = 1; reach <= 2 && weight < 0.5; reach++) {
            for (var y = -reach; y <= reach; y++) {
                for (var x = -reach; x <= reach; x++) {
                    let q = near + vec2<i32>(x, y);
                    if (any(q < vec2<i32>(0)) || any(q >= vec2<i32>(params.half.xy))) {
                        continue;
                    }
                    let was = textureLoad(prev_guide, q, 0);
                    if (was.z <= 0.0 || abs(was.z - back.expected) >= 2.0 * back.tolerance) {
                        continue;
                    }
                    k.color += textureLoad(prev_fed, q, 0).rgb;
                    k.count += textureLoad(prev_color, q, 0).a;
                    k.moment += was.w;
                    weight += 1.0;
                }
            }
        }
        loose = true;
    }
    if (weight > 0.0) {
        k.color /= weight;
        k.count /= weight;
        k.moment /= weight;
    }
    if (loose) {
        k.count = min(k.count, LOOSE_FRAMES);
    }
    k.loose = loose && weight > 0.0;
    return k;
}

// The light of last frame's half resolution pixels around where the
// surface at `back` was whose history averaged `FRESH_FRAMES` or more, any
// face but the sky's, at about its distance (as `kept`'s leaves around: a
// twig before a shadowed wall was held to the wall's light, and a lit face
// a turn of the view uncovered behind a dark one to the dark one's), their
// luminance's mean without the brightest, so
// that one speck among them does not raise it; negative with fewer than
// three of them. Fresh ones are left out: held to their own level, a
// foliage that stays fresh would darken frame after frame. The 3 x 3 first,
// then the 5 x 5: a twig or a leaf's edge against the sky has too few of its
// own in the first, and went unclamped.
fn around(back: Back) -> f32 {
    let level = around_within(back, 1);
    if (level >= 0.0) {
        return level;
    }
    return around_within(back, 2);
}

fn around_within(back: Back, reach: i32) -> f32 {
    if (params.mode.w < 0.5 || back.at.x < 0.0) {
        return -1.0;
    }
    let near = vec2<i32>(round((back.at - vec2<f32>(0.5)) * params.grid.y));
    var sum = 0.0;
    var most = 0.0;
    var seen = 0.0;
    for (var y = -reach; y <= reach; y++) {
        for (var x = -reach; x <= reach; x++) {
            let q = near + vec2<i32>(x, y);
            if (any(q < vec2<i32>(0)) || any(q >= vec2<i32>(params.half.xy))) {
                continue;
            }
            let z = textureLoad(prev_guide, q, 0).z;
            if (z <= 0.0 || abs(z - back.expected) >= 2.0 * back.tolerance
                || textureLoad(prev_color, q, 0).a < FRESH_FRAMES) {
                continue;
            }
            let l = luma(textureLoad(prev_fed, q, 0).rgb);
            sum += l;
            most = max(most, l);
            seen += 1.0;
        }
    }
    if (seen < 3.0) {
        return -1.0;
    }
    return (sum - most) / (seen - 1.0);
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
    let c = (back.at - vec2<f32>(0.5)) * params.grid.y;
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

// The trace's pass: `gi_trace_pixel`, its reads of the probes counted for
// `AURORA_RT_GI_DIAG=count` (gi.rs `PLAIN_HOOKS` count nothing,
// shaders/gi_count.wgsl), whatever `return` it ends on.
@compute @workgroup_size(8, 8)
fn cs_gi(@builtin(global_invocation_id) id: vec3<u32>, @builtin(workgroup_id) group: vec3<u32>) {
    gi_count_start(0u, group);
    gi_trace_pixel(id);
    gi_count_flush();
}

fn gi_trace_pixel(id: vec3<u32>) {
    let hp = vec2<i32>(id.xy);
    if (any(hp >= vec2<i32>(params.half.xy))) {
        return;
    }
    let frame = u32(params.mode.z);
    // The same pixel of each 2×2 at every frame. Taking each in turn, a
    // 2×2 across an edge was one face then the other: its history and what
    // `cs_gi_filter` spread from it changed with the frame, and the edges
    // glittered with the view still.
    let pixel = hp * grid_step();
    let depth = textureLoad(depth_tex, pixel, 0);
    // reversed depth: 0 is the sky
    if (depth <= 0.0) {
        textureStore(out_hit, hp, vec4<f32>(0.0));
        textureStore(out_pick, hp, vec4<f32>(0.0));
        return;
    }
    let world = unproject(pixel, depth);
    let flat = normal_at(pixel, depth);
    let n = smooth_normal(pixel, flat);
    let dist = distance(world, params.camera.xyz);
    let origin = ray_origin(world, flat, dist);
    let posed = posed_traced();
    gi_probe_rays = diag() != DIAG_NO_PROBE_RAYS;
    let posed_flags = select(0u, RAY_FLAG_FORCE_OPAQUE, diag() == DIAG_POSED_OPAQUE);
    // with no history (a photo, a new size), more rays at once
    let rays = select(1u, RESET_RAYS, params.mode.w < 0.5);
    var pick = no_pick();
    // the worths weighed so far, each over how likely its hit was drawn
    var sum = 0.0;
    // The ray queries are declared out of the loop and initialised again at
    // each ray. naga 30 gives a variable declared in a loop's body a store of
    // its zero value at each turn, and a ray query's is an `OpConstantNull`
    // of `OpTypeRayQueryKHR`, which SPIR-V forbids (an opaque type has no
    // null): AMD's compiler (LLPC, an RX 9070 XT) crashed on it in
    // vkCreateComputePipelines (#4596), and it is an assignment Metal's
    // compiler refuses for a ray query. `rt_shaders_have_no_null_ray_query`
    // and `no_ray_query_is_declared_in_a_loop` check every traced pass.
    var rq: ray_query;
    var rp: ray_query;
    for (var ray = 0u; ray < rays; ray++) {
        let dir = cosine_dir(n, random2(vec2<u32>(id.xy), frame * RESET_RAYS + ray));

        var t = -1.0;
        var record = 0u;
        var primitive = 0u;
        var bary = vec2<f32>(0.0);
        rayQueryInitialize(&rq, scene_tlas, RayDesc(0u, SOLID, 0.0, params.mode.x, origin, dir));
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
            rayQueryInitialize(&rp, posed_tlas, RayDesc(posed_flags, SOLID, 0.0, reach, origin, dir));
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
                // (the diagnosis with no shadow ray: no lamp's, and a reach
                // of 0 for the sun's)
                let shadows = diag() != DIAG_NO_HIT_SHADOWS;
                let lamps = Lamps(u32(params.lamps.x), params.lamps.y > 0.5 && shadows, chance(id.xy, frame, 40u + ray).x);
                let reach = select(0.0, params.mode.x, shadows);
                one = gi_hit_radiance(params.lit, s, hit, dir * t, 2.0 + log2(1.0 + t), 0.01 + 0.002 * (dist + t), reach, posed, lamps);
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
    return reach <= 0.0 || unblocked(origin, to / far, reach, posed_traced());
}

@compute @workgroup_size(8, 8)
fn cs_gi_reuse(@builtin(global_invocation_id) id: vec3<u32>) {
    let hp = vec2<i32>(id.xy);
    if (any(hp >= vec2<i32>(params.half.xy))) {
        return;
    }
    let frame = u32(params.mode.z);
    let pixel = hp * grid_step();
    let depth = textureLoad(depth_tex, pixel, 0);
    if (depth <= 0.0) {
        textureStore(out_color, hp, vec4<f32>(0.0));
        textureStore(out_guide, hp, vec4<f32>(0.0));
        return;
    }
    let world = unproject(pixel, depth);
    let flat = normal_at(pixel, depth);
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
    let taken = min(u32(params.grid.z + 0.5), NEIGHBOURS);
    for (var k = 0u; k < NEIGHBOURS; k++) {
        near_rays[k] = 0.0;
    }
    for (var k = 0u; k < taken; k++) {
        let radius = NEIGHBOUR_REACH * params.grid.y * sqrt((f32(k) + u.x) / f32(taken));
        let angle = 2.0 * PI * (u.y + f32(k) * 0.61803399);
        let q = hp + vec2<i32>(round(vec2<f32>(cos(angle), sin(angle)) * radius));
        if (any(q < vec2<i32>(0)) || any(q > last) || all(q == hp)) {
            continue;
        }
        let depth_q = textureLoad(depth_tex, q * grid_step(), 0);
        if (depth_q <= 0.0) {
            continue;
        }
        // on this pixel's face: in its plane and facing as it does, where
        // the two receive most of the same light
        let at_q = unproject(q * grid_step(), depth_q);
        if (abs(dot(flat, at_q - world)) > 0.01 + 0.01 * dist) {
            continue;
        }
        let flat_q = normal_at(q * grid_step(), depth_q);
        let n_q = smooth_normal(q * grid_step(), flat_q);
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
        //
        // Each of them counts by how likely it was to draw the hit: its
        // rays times how the hit stands before it (its own target; the
        // hit's light is the same for all and goes out), in this pixel's
        // measure (`carried`). That is the balance heuristic between the
        // pixels (Bitterli et al. 2020, "Spatiotemporal reservoir
        // resampling", section 4.4's weights). Shared by the rays alone, a
        // hit that grazed a neighbour, drawn there once in many rays and
        // heavy for it, came here at that weight where the surface is
        // turned towards it: on a curved surface (an avatar, a pillar) a
        // frame had hundreds of pixels lit many times their mean. Weighed
        // so, a neighbour's hit lights this pixel no more than it lights
        // the neighbour, by the rays of each.
        let facing = max(dot(n, normalize(pick.hit - origin)), 0.0);
        var among = own.rays * facing;
        var drawn = facing;
        for (var k = 0u; k < NEIGHBOURS; k++) {
            if (near_rays[k] <= 0.0) {
                continue;
            }
            let gain = carried(pick.hit, pick.normal, near_at[k], near_n[k], origin, n);
            if (gain < 0.0) {
                continue;
            }
            if (k != source && !sees(near_at[k], near_dist[k], pick.hit)) {
                continue;
            }
            let there = max(dot(near_n[k], normalize(pick.hit - near_at[k])), 0.0) / gain;
            among += near_rays[k] * there;
            if (k == source) {
                drawn = there;
            }
        }
        let weight = min(sum * drawn / (among * p), WEIGHT_MAX);
        sample = capped(pick.light * (facing / PI * weight));
    }
    let back = back_of(pixel, world, dist);
    let past = kept(back, n);
    // A pixel with little or no history: its few rays, held to the light
    // around it, so that one which met a bright face does not light the
    // whole leaf for a frame. Dense foliage loses its history at every frame
    // under a jittered projection (TAA, FSR, DLSS), each pixel on another
    // leaf or the sky between them, and sparkled; a turn of the view did it
    // everywhere. Not where the light changed: that light is new.
    if ((past.count < FRESH_FRAMES || past.loose)
        && params.mode.w > 0.5
        && params.half.w >= FRESH_FRAMES
        && !light_changed(world)) {
        let level = around(back);
        let l = luma(sample);
        if (level >= 0.0 && l > level * FRESH_GAIN) {
            sample *= level * FRESH_GAIN / l;
        }
    }
    var moment = luma(sample) * luma(sample);

    // last frame's at the same surface
    var color = sample;
    let rays = select(1.0, f32(RESET_RAYS), params.mode.w < 0.5);
    var count = min(rays, params.half.w);
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

// The share of the sky's ambient the traced probes let through at each
// half resolution pixel (gi_sample.wgsl `gi_sky`; common.wgsl
// `screen_sky`), at the place and with the normal its guide holds: the
// whole sky where it holds no face. At half resolution, as the indirect
// light: it changes slowly over the screen, and at each full resolution
// pixel it cost 0.2 ms on an RTX 4090 at 2560 x 1370 (2026-11-21).
// How open the places the reflection passes are drawn from are (texel i
// for `params.points[i]`), 1 where there is none: the passes see no screen
// of their own, so their faces take the openness of the place they are
// seen from (common.wgsl `reflection_open`). A probe's capture and the
// mirror otherwise lit a closed room with the whole sky's ambient and
// mirrored the outside on its shiny faces, which the view's shiny faces
// reflect in turn.
fn gi_points() {
    for (var i = 0u; i < 2u; i++) {
        var open = 1.0;
        if (params.points[i].w > 0.0) {
            open = gi_open_at(params.points[i].xyz);
        }
        textureStore(out_points, vec2<i32>(i32(i), 0), vec4<f32>(open, open, 0.0, 0.0));
    }
}

// The sky's pass: `gi_sky_pixel`, counted as `cs_gi`.
@compute @workgroup_size(8, 8)
fn cs_gi_sky(@builtin(global_invocation_id) id: vec3<u32>, @builtin(workgroup_id) group: vec3<u32>) {
    gi_count_start(1u, group);
    gi_sky_pixel(id);
    gi_count_flush();
}

fn gi_sky_pixel(id: vec3<u32>) {
    gi_probe_rays = diag() != DIAG_NO_PROBE_RAYS;
    if (all(id.xy == vec2<u32>(0u))) {
        gi_points();
    }
    let hp = vec2<i32>(id.xy);
    if (any(hp >= vec2<i32>(params.half.xy))) {
        return;
    }
    let guide = textureLoad(gi_guide, hp, 0);
    // (a half resolution pixel is the first of its 2×2)
    let pixel = hp * grid_step();
    let depth = textureLoad(depth_tex, pixel, 0);
    // (y: how open the place is, read by the shading straight from this
    // texture, common.wgsl `screen_open`)
    var sky = vec2<f32>(1.0);
    if (guide.z > 0.0 && depth > 0.0) {
        let pos = unproject(pixel, depth);
        sky = gi_sky_view(pos, unpack_normal(guide.xy), params.camera.xyz - pos);
    }
    textureStore(out_sky, hp, vec4<f32>(sky, 0.0, 0.0));
}

// `cs_gi_filter`'s traced pixels, read once by its workgroup: the 8 x 8
// screen pixels of a workgroup take their 4 x 4 taps among 7 x 7 traced
// pixels at most (5 x 5 at a step of 4), each read here by one invocation
// in the place of up to sixteen. Each holds what the traced pixel at
// `GiTile`'s place, clamped to the image, gives: the guide's distance, the
// light and the sky's share. The same values in the same order: the image
// is the same as with each tap read on its own.
const GI_TILE: u32 = 8u;
var<workgroup> gi_tile_dist: array<f32, 64>;
var<workgroup> gi_tile_light: array<vec3<f32>, 64>;
var<workgroup> gi_tile_sky: array<f32, 64>;

@compute @workgroup_size(8, 8)
fn cs_gi_filter(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
) {
    // The tile's first traced pixel: the one before the first the
    // workgroup's first pixel takes (its taps go from -1 to 2).
    let last = vec2<i32>(params.half.xy) - vec2<i32>(1);
    let first = vec2<i32>(floor(vec2<f32>(group.xy * GI_TILE) * params.grid.y)) - vec2<i32>(1);
    {
        let q = clamp(first + vec2<i32>(i32(local % GI_TILE), i32(local / GI_TILE)), vec2<i32>(0), last);
        gi_tile_dist[local] = textureLoad(gi_guide, q, 0).z;
        gi_tile_light[local] = textureLoad(gi_color, q, 0).rgb;
        gi_tile_sky[local] = textureLoad(sky_tex, q, 0).r;
    }
    workgroupBarrier();
    let pixel = vec2<i32>(id.xy);
    if (any(pixel >= vec2<i32>(params.screen.xy))) {
        return;
    }
    let depth = textureLoad(depth_tex, pixel, 0);
    if (depth <= 0.0) {
        // (a: the whole sky, so that a face's edge sampled towards the sky
        // keeps its ambient)
        textureStore(out_gi, pixel, vec4<f32>(0.0, 0.0, 0.0, 1.0));
        return;
    }
    let dist = distance(unproject(pixel, depth), params.camera.xyz);
    // a half resolution pixel is the first of its 2×2
    let centre = vec2<f32>(pixel) * params.grid.y;
    let base = vec2<i32>(floor(centre));
    var sum = vec3<f32>(0.0);
    var sky_sum = 0.0;
    var weight = 0.0;
    for (var y = -1; y <= 2; y++) {
        for (var x = -1; x <= 2; x++) {
            let at = base + vec2<i32>(x, y);
            let q = clamp(at, vec2<i32>(0), last);
            let k = u32(at.x - first.x) + GI_TILE * u32(at.y - first.y);
            let qd = gi_tile_dist[k];
            if (qd <= 0.0) {
                continue;
            }
            let off = vec2<f32>(q) - centre;
            // nearer and of the same depth weigh more
            let w_space = exp(-dot(off, off));
            let w_depth = exp(-abs(qd - dist) / (0.01 + 0.02 * dist));
            let w = w_space * w_depth;
            sum += gi_tile_light[k] * w;
            sky_sum += gi_tile_sky[k] * w;
            weight += w;
        }
    }
    let gi = select(vec3<f32>(0.0), sum / max(weight, 1e-6), weight > 1e-4);
    // a: the share of the sky's ambient the traced probes let through
    // (`cs_gi_sky`, upsampled as the light; common.wgsl `screen_sky`), the
    // whole sky where no half resolution pixel holds the face
    let sky = select(1.0, sky_sum / max(weight, 1e-6), weight > 1e-4);
    textureStore(out_gi, pixel, vec4<f32>(gi * params.half.z, sky));
}
