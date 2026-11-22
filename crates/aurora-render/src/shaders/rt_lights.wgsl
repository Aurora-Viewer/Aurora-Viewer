// Ray-traced shadows of the local point lights (rt_shadows.rs `Lights`):
//
// `cs_lights`, a pixel of the depth prepass at a time: the light it traces,
// by resampled importance sampling (Talbot 2005) over the point lights that
// reach it, each weighed by the diffuse light it would give unshadowed
// (every light once when no more than `CANDIDATES` reach it), then with
// ReSTIR (Bitterli et al. 2020) merged with the pixel's choice last frame;
// then one shadow ray toward it. A light has a size (`light_size`): the ray
// goes to a point of the disc it shows the pixel, another each frame, so
// its shadow has a penumbra, narrow at the foot of what casts it and wider
// away from it. The choice and what the ray found are written to `mid`.
// `cs_lights_reuse`: the share of the local light that arrives, from the
// pixel's own sample and `NEIGHBOURS` around it on the same surface, as
// Tokuyoshi's visibility reuse (AMD, "Efficient Visibility Reuse for
// Real-time ReSTIR", 2024): each sample's light was drawn in proportion to
// its unshadowed light, so the visibilities found, weighed by how much
// their light gives this pixel against how likely it was drawn there,
// average to the share (the ratio estimator, Heitz et al. 2018). Where a
// neighbour's light is the one the pixel traced, the neighbour's
// visibility counts only within the penumbra the rays found there (another
// point of the same soft edge); further than that the pixel's own replaces
// it, so a shadow stays sharp where it is sharp. The same samples give
// the share of each colour (the tint): each visibility weighed by its
// light's red, green and blue in place of its luminance, so where a warm
// lamp is hidden and a blue one is not, the blue arrives whole and the
// warm does not, whatever their number. The tint,
// blended with last frame's reprojected as rt_gi.wgsl's history, is what
// the shading multiplies its point lights by (common.wgsl `local_share`),
// whose light stays the Legacy Viewer's where nothing is in the way. The
// share is what the denoiser measures the noise by (rt_denoise.wgsl): its
// guide keeps the second moment beside.
// The samples merged become the choice the next frame starts from.
// The history follows its lights: a light that moves takes the frames over
// which it goes a quarter of its diameter (`light_fresh`), so the shadows
// it carries across a floor do not trail behind it, and where a lamp came,
// went or changed its brightness
// (rt_history.wgsl `light_changed`) the history holds a few frames, so a
// lamp switched on has its shadows at once.
// The sun's soft shadow shares this history: `cs_lights_reuse` averages
// the sun's ray of each frame (rt_shadows.wgsl) beside the share, and where
// a shadow moved off a still surface (a walking avatar's, off the floor)
// keeps the history within what this frame's rays found around the pixel,
// as AMD's FidelityFX shadow denoiser does, so the floor clears at once
// instead of holding a trail. The colour the glass gives the sun
// (rt_shadows.wgsl `out_glass`, its ray as noisy as the shadow's) is
// averaged the same way beside it (`glass_history`), then over the rays
// that arrived (`glass_of`).
// `cs_lights_filter`: the denoised tint and sun, written with the contact
// shadows into the mask the frame binds: `.r` the sun's shadow (1 in
// shadow, the darker of the traced and the contact shadow), `.gba` the
// tint (the share, where the lights are not traced: 1).
//
// The lights are weighed by the mesh's smooth normal where the prepass drew
// it (rt_history.wgsl `smooth_normal`), as the shading lights them; the
// rays of a triangle turned from its light take rt_common.wgsl `shadows`.
//
// A light's own prim does not shadow it (its bulb, its lamp's body, a strip
// light's tube): a ray goes the opaque way up to the sphere about the
// light that holds its prim (`source.x`), then on to the light past every
// face but those of its prim (`source.w`, rt_common.wgsl `shadows_but`),
// so what stands inside that sphere beside the prim still casts. With
// `EMITTER_SPHERE` (`AURORA_RT_EMITTER=sphere`, rt_shadows.rs `source`)
// the rays stop at the sphere, for a comparison.
//
// A projector is one of the lights drawn and shared like the others, over
// its frustum, weighed by its colour without its texture, its rays aimed
// at the apex of its frustum (`light_target`, `light_aim`): the tint then
// shadows its beam (common.wgsl `projector_at`) in its shadow map's place:
// the map is no longer drawn (renderer.rs `traced_projectors`), so the
// faces this pass does not see (the blended ones, the reflections) take
// the beam unshadowed, as the Legacy Viewer's blended faces (alphaF.glsl).
// The Legacy Viewer shadows two projectors, by
// maps (spotLightF.glsl, pipeline.cpp RenderShadowDetail); traced, every
// projector of the list casts, at any distance.
//
// Glass: a ray that nothing stops goes on through the blended faces on its
// way (`glass_along`), and what they leave of the light's red, green and
// blue is the sample's visibility in place of its 1 (`mid`'s z, packed),
// which the share takes by its luminance and the tint by colour: a lamp
// behind a red pane lights red.

// After rt_common.wgsl, rt_view.wgsl, rt_light.wgsl, rt_history.wgsl and
// wgpu_ray_query's enable (rt_shadows.rs `source`).

struct Params {
    // This frame's projection inverted (with the prepass's jitter), and
    // last frame's projection.
    world_from_clip: mat4x4<f32>,
    prev_clip_from_world: mat4x4<f32>,
    // xyz the eye, this frame's and last frame's; the first's w: 1 when
    // `motion_tex` is the prepass's, the second's: 1 when the tint is kept
    // (`prev_tint`, `out_tint`, `tint_tex`: one pixel each otherwise).
    camera: vec4<f32>,
    prev_camera: vec4<f32>,
    // width, height, 1 / width, 1 / height
    screen: vec4<f32>,
    // x: the lights, y: 1 when the avatars' top level is traced, z: the
    // frame's number, w: 1 when last frame's history is valid, 3 when its
    // choices are of the same lights too.
    mode: vec4<f32>,
    // Where a lamp changed lately (rt_light_changes.rs): xyz a ball's
    // centre, w its radius.
    changed: array<vec4<f32>, 8>,
    // x: how many of them, y: 0 (the sun's and the sky's light are not
    // this history's), z: the frames the history averages at most where
    // one did.
    changes: vec4<f32>,
    // The lights by screen tile and depth (light_tiles.rs `LightSets`, the
    // main view's): xyz the camera's forward axis, w: 1 when the sets
    // below narrow the list; then the lights of each column, each row and
    // each slice of depth, two sets per element.
    sets: vec4<f32>,
    columns: array<vec4<u32>, 16>,
    rows: array<vec4<u32>, 16>,
    slices: array<vec4<u32>, 16>,
    // x: the lamps' softness, Preferences' (`light_size`: 0 sharp).
    tuning: vec4<f32>,
};

// Candidates a pixel draws among the lights reaching it.
const CANDIDATES: u32 = 8u;
// The most a choice kept from last frame counts for, against this frame's.
const CONFIDENCE_MAX: f32 = 20.0;
// Frames the share's history averages at most; on a surface that moved on
// its own, whose shadows sweep over it, fewer.
const HISTORY: f32 = 12.0;
const HISTORY_MOVING: f32 = 4.0;
// The least cosine between a pixel's normal and its history's
// (`history_pixel`): a curve's turn over a frame of motion stays above it,
// the other wall of a corner does not.
const HISTORY_FACING: f32 = 0.7;
const MAX_LIGHTS: u32 = 64u;
// Rays a pixel traces on a frame with no history (cs_lights).
const RESET_RAYS: u32 = 8u;
// The neighbours whose samples a pixel reuses, and how far they are at most
// (pixels).
const NEIGHBOURS: u32 = 4u;
const REUSE_RADIUS: f32 = 8.0;
// How far around a pixel (pixels each way) this frame's sun rays bound its
// history.
const SUN_RANGE: i32 = 2;
// A light's radius (m): its prim's, within these. The Legacy Viewer's point
// lights cast no shadow: the size is this pass's own, a lamp's bulb at
// least, and no wider than a large lamp however large the prim.
const LIGHT_SIZE_MIN: f32 = 0.04;
const LIGHT_SIZE_MAX: f32 = 0.3;
// `mid`'s first word: the light's index, these, then the confidence in its
// high half (in 256ths).
const MID_SEEN: u32 = 0x100u;
const MID_DRAWN: u32 = 0x200u;

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
// the frame's point lights (rt_light.wgsl `Light`)
@group(0) @binding(2) var<storage, read> lights: array<Light>;

// A choice: the light's index, its weight, its confidence (ReSTIR's M).
@group(0) @binding(3) var prev_choice: texture_2d<f32>;
@group(0) @binding(4) var out_choice: texture_storage_2d<rgba16float, write>;
// (cs_lights alone traces: a pass pays for the ray queries its shader
// holds whether it runs them or not, and cs_lights_reuse would only on a
// frame with no history)
@group(0) @binding(5) var scene_tlas: acceleration_structure;
@group(0) @binding(6) var posed_tlas: acceleration_structure;
// The history: r the share and a the frames it averages, g the sun's light
// (1 lit) and b its frames.
@group(0) @binding(7) var prev_share: texture_2d<f32>;
@group(0) @binding(8) var out_share: texture_storage_2d<rgba16float, write>;
// The guide (rt_history.wgsl `pack_normal`): z the distance, w the share's
// second moment.
@group(0) @binding(13) var prev_guide: texture_2d<f32>;
@group(0) @binding(14) var out_guide: texture_storage_2d<rgba16float, write>;
// the depth prepass's motion (rt_history.wgsl), or black without it
@group(0) @binding(12) var motion_tex: texture_2d<f32>;
// A pixel's sample, from cs_lights to cs_lights_reuse: x the light's index
// with `MID_SEEN` when its ray found it and `MID_DRAWN` when there is one,
// and its confidence, y its weight, z the width of the penumbra where its
// ray was stopped (m; y and it f32 bits), where it was not what the glass
// left of its light (pack4x8unorm, `glass_along`), or on a frame with no
// history what its rays found (`pack_reset`), w the normal
// (rt_history.wgsl `pack_normal`, pack2x16snorm).
@group(0) @binding(15) var out_mid: texture_storage_2d<rgba32uint, write>;
@group(0) @binding(16) var mid: texture_2d<u32>;

// This frame's sun ray (rt_shadows.wgsl): r the contact shadows, g 1 where
// the ray was stopped. cs_lights_reuse and cs_lights_filter.
@group(0) @binding(10) var sun_tex: texture_2d<f32>;
// cs_lights_filter (its own bind group: it reads what the denoiser wrote):
// r the share, g the sun's light
@group(0) @binding(9) var share_tex: texture_2d<f32>;
@group(0) @binding(11) var out_mask: texture_storage_2d<rgba8unorm, write>;
// The tint: rgb what arrives of the local lights' red, green and blue. Its
// history beside the share's (cs_lights_reuse), and denoised
// (cs_lights_filter).
@group(0) @binding(17) var prev_tint: texture_2d<f32>;
@group(0) @binding(18) var out_tint: texture_storage_2d<rgba16float, write>;
@group(0) @binding(19) var tint_tex: texture_2d<f32>;
// What the glass leaves of the sun: this frame's ray (rt_shadows.wgsl
// `out_glass`) and its history, rgb the colour times the light and a the
// light (cs_lights_reuse, `glass_history`), then what the frame binds
// (cs_lights_filter, the frame's binding 31: common.wgsl `sun_glass`).
@group(0) @binding(20) var sun_glass_now: texture_2d<f32>;
@group(0) @binding(21) var prev_glass: texture_2d<f32>;
@group(0) @binding(22) var out_glass_history: texture_storage_2d<rgba16float, write>;
@group(0) @binding(23) var glass_kept: texture_2d<f32>;
@group(0) @binding(24) var out_glass: texture_storage_2d<rgba8unorm, write>;
// cs_lights_filter: this frame's guide as cs_lights_reuse wrote it
// (`out_guide`): the surfaces (`glass_of`).
@group(0) @binding(26) var guide_now: texture_2d<f32>;

// common.wgsl `ll_light_atten`.
fn light_atten(d: f32, falloff: f32) -> f32 {
    let f = max(falloff, 0.0);
    let x = 1.0 - clamp((d + f) / (1.0 + f), 0.0, 1.0);
    return x * x * 2.0;
}

// Where light `i`'s rays aim: a point light's centre, a projector's apex.
fn light_aim(i: u32) -> vec3<f32> {
    let lt = lights[i];
    return select(lt.pos_radius.xyz, lt.apex.xyz, lt.source.y > 0.5);
}

// The diffuse light light `i` gives `world` facing `n`, unshadowed (its
// luminance): what a choice is weighed by. A projector's within its
// frustum only, from its apex, without its texture (common.wgsl
// `projector_at`: the reach from the light's centre, the direction from
// the apex).
fn light_target(i: u32, world: vec3<f32>, n: vec3<f32>) -> f32 {
    let lt = lights[i];
    // a projector whose shadows are off (rt_shadows.rs `traced_lights`):
    // never drawn, its beam the maps' (common.wgsl `projector_at`)
    if (lt.source.y > 1.5) {
        return 0.0;
    }
    let to_l = lt.pos_radius.xyz - world;
    let dist = length(to_l);
    let radius = lt.pos_radius.w;
    if (dist >= radius) {
        return 0.0;
    }
    var l = to_l / max(dist, 1e-4);
    if (lt.source.y > 0.5) {
        let tc = lt.beam * vec4<f32>(world, 1.0);
        if (tc.z < 0.0 || tc.w <= 0.0) {
            return 0.0;
        }
        let uv = tc.xy / tc.w;
        if (any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0))) {
            return 0.0;
        }
        l = normalize(lt.apex.xyz - world);
    }
    // A light behind the surface still counts, by `BEHIND`: the rays' normal
    // is the mesh's, or the depth's on an edge (rt_history.wgsl
    // `smooth_normal`), and the shading's own normal map or an edge's real
    // normal may still face a light it turns from. A pixel no light reached
    // was not shadowed at all (`cs_lights_reuse`: nothing to shadow), so a
    // lamp behind a wall lit the rims of an avatar and the edges of the
    // room in its colour (2026-11-22, grid report). Its ray is traced now,
    // and the wall stops it.
    let nl = max(dot(n, l), BEHIND);
    let lum = dot(lt.color_falloff.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    return lum * light_atten(dist / radius, lt.color_falloff.w) * nl;
}

// The weight of a light behind the surface (`light_target`), as if it
// came at 87°: small, so that its visibility weighs little in the share of
// the lights the surface faces.
const BEHIND: f32 = 0.05;

// Point light `i`'s colour over its luminance: `light_target` times it is
// the red, green and blue the light gives.
fn light_tint(i: u32) -> vec3<f32> {
    let c = lights[i].color_falloff.rgb;
    return c / max(dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-6);
}

// What a frame with no history found (`mid`'s z): how many of its
// `RESET_RAYS` arrived, each by the luminance the glass left of it,
// rounded (4 bits), then the tint (9 bits a colour: the mask keeps 8).
fn pack_reset(arrived: f32, tint: vec3<f32>) -> u32 {
    let t = vec3<u32>(round(clamp(tint, vec3<f32>(0.0), vec3<f32>(1.0)) * 511.0));
    return u32(arrived) | (t.r << 4u) | (t.g << 13u) | (t.b << 22u);
}

// xyz the tint, w the share.
fn unpack_reset(z: u32) -> vec4<f32> {
    let t = vec3<u32>(z >> 4u, z >> 13u, z >> 22u) & vec3<u32>(0x1FFu);
    return vec4<f32>(vec3<f32>(t) / 511.0, f32(z & 0xFu) / f32(RESET_RAYS));
}

// Sets of lights, a bit each in the list's order, as common.wgsl walks
// them (`light_set`, `next_light`). A pixel walked the whole list, three
// times and more where many lights reach it, for the few whose sphere holds
// it: it walks the lights of its screen tile and its depth, which the frame
// has sorted for the shading. The same lights in the same order, so the
// same picture.

// The first `n` lights.
fn first_lights(n: u32) -> vec2<u32> {
    let part = (1u << (n & 31u)) - 1u;
    let lo = select(part, 0xffffffffu, n >= 32u);
    let hi = select(select(part, 0xffffffffu, n >= 64u), 0u, n <= 32u);
    return vec2<u32>(lo, hi);
}

fn set_of(pair: vec4<u32>, i: u32) -> vec2<u32> {
    return select(pair.xy, pair.zw, (i & 1u) == 1u);
}

// The lights that can reach `world`, seen at `pixel`, of the `listed`.
fn lights_near(pixel: vec2<i32>, world: vec3<f32>, listed: u32) -> vec2<u32> {
    var held = first_lights(listed);
    if (params.sets.w > 0.5) {
        let at = (vec2<f32>(pixel) + vec2<f32>(0.5)) * params.screen.zw * 32.0;
        let column = min(u32(at.x), 31u);
        let row = min(u32(at.y), 31u);
        // common.wgsl `tile_cell`: LIGHT_SLICE_NEAR, LIGHT_SLICES_PER_OCTAVE
        let depth = dot(world - params.camera.xyz, params.sets.xyz);
        let slice = min(u32(log2(max(depth, 0.25) / 0.25) * 3.0), 31u);
        held &= set_of(params.columns[column >> 1u], column)
            & set_of(params.rows[row >> 1u], row)
            & set_of(params.slices[slice >> 1u], slice);
    }
    return held;
}

// The first light of `among`, taken out of it; `MAX_LIGHTS` when it is empty.
fn next_light(among: ptr<function, vec2<u32>>) -> u32 {
    let lo = (*among).x;
    if (lo != 0u) {
        (*among).x = lo & (lo - 1u);
        return firstTrailingBit(lo);
    }
    let hi = (*among).y;
    if (hi != 0u) {
        (*among).y = hi & (hi - 1u);
        return 32u + firstTrailingBit(hi);
    }
    return MAX_LIGHTS;
}

fn with_light(among: vec2<u32>, i: u32) -> vec2<u32> {
    let bit = 1u << (i & 31u);
    return among | select(vec2<u32>(bit, 0u), vec2<u32>(0u, bit), i >= 32u);
}

// The light of `among` after `nth` others, which has more than `nth`.
fn nth_light(among: vec2<u32>, nth: u32) -> u32 {
    let low = countOneBits(among.x);
    let high = nth >= low;
    var word = select(among.x, among.y, high);
    let skip = select(nth, nth - low, high);
    for (var k = 0u; k < skip; k++) {
        word &= word - 1u;
    }
    return select(0u, 32u, high) + firstTrailingBit(word);
}

// The light ray `r` of a frame with no history goes to: the lights reaching
// the pixel (`reaching`, `total` their light in all) drawn in proportion to
// their light at even steps from `start` (systematic sampling).
fn reset_light(r: u32, start: f32, total: f32, reaching: vec2<u32>, world: vec3<f32>, n: vec3<f32>) -> u32 {
    let goal = (f32(r) + start) / f32(RESET_RAYS) * total;
    var reached = 0.0;
    var to = 0u;
    var walk = reaching;
    for (var i = next_light(&walk); i < MAX_LIGHTS; i = next_light(&walk)) {
        reached += light_target(i, world, n);
        to = i;
        if (goal < reached) {
            break;
        }
    }
    return to;
}

// (times Preferences' softness: 0 a point, whose shadows are sharp)
fn light_size(i: u32) -> f32 {
    return clamp(lights[i].source.x, LIGHT_SIZE_MIN, LIGHT_SIZE_MAX) * params.tuning.x;
}

// The share of a frame in light `i`'s history: one of `HISTORY` for a
// light that stands, more for one that moves. Its shadows move with it, by
// as much as its penumbras are wide for each diameter it goes, and the
// history is late by its length times the light's step: kept within a
// quarter of the light's diameter, an eighth of a penumbra. (Measured on
// the shadow a lamp carried at 3 m/s swept across a floor, 60 frames a
// second: 9 levels of 255 off at worst with the 12 frames, 6 within a
// diameter, 4 within a quarter; the shadow's grain the same.)
fn light_fresh(i: u32) -> f32 {
    return clamp(2.0 * lights[i].source.z / max(light_size(i), LIGHT_SIZE_MIN), 1.0 / HISTORY, 1.0);
}

// What the glass on the ray from `origin` up to `far` leaves of a light's
// red, green and blue (rt_common.wgsl `glass_passes`, the rule of PR #3821),
// the panes of the light's own prim aside (`own`, as rt_common.wgsl
// `shadows_but`): a lamp's bulb is often a blended face. The still faces'
// level and, with `posed`, the moving faces' (a pane being carried); worn
// glass is not traced yet (rt_shadows.rs `look`). A pane met within
// `lying` of the ray's way (m) lies on the surface (`lying_on`).
fn glass_along(origin: vec3<f32>, dir: vec3<f32>, far: f32, own: f32, posed: bool, lying: f32) -> vec3<f32> {
    var through = vec3<f32>(1.0);
    if (far <= 0.0) {
        return through;
    }
    let ray = RayDesc(SHADOW_CULL, GLASS, 0.0, far, origin, dir);
    var rq: ray_query;
    rayQueryInitialize(&rq, scene_tlas, ray);
    while (rayQueryProceed(&rq)) {
        through *= pane_passes(rayQueryGetCandidateIntersection(&rq), false, dir, own, lying);
    }
    if (posed) {
        rayQueryInitialize(&rq, posed_tlas, ray);
        while (rayQueryProceed(&rq)) {
            through *= pane_passes(rayQueryGetCandidateIntersection(&rq), true, dir, own, lying);
        }
    }
    return through;
}

// `glass_passes`, 1 for a pane of the light's own prim (`own`) and for one
// nearer than `lying` (`lying_on`).
fn pane_passes(c: RayIntersection, posed: bool, dir: vec3<f32>, own: f32, lying: f32) -> vec3<f32> {
    if (c.kind == RAY_QUERY_INTERSECTION_TRIANGLE && c.t < lying) {
        return vec3<f32>(1.0);
    }
    if (own > 0.5 && c.kind == RAY_QUERY_INTERSECTION_TRIANGLE) {
        let h = hit_of(posed, c.instance_custom_data, c.geometry_index, c.primitive_index);
        if (f32(cull_face(h.record).object + 1u) == own) {
            return vec3<f32>(1.0);
        }
    }
    return glass_passes(c, posed, dir);
}

// The ray from `origin` to the point `u` (two numbers in 0..1) of the disc
// light `i` shows it: w -1 when nothing stands in its way, xyz then what
// the glass on its way leaves of the light (`glass_along`); else w the
// width of the penumbra there (m): the light's width seen through what
// stopped the ray, from as far behind it as the pixel is. `flat` is the
// normal of the pixel's triangle.
fn light_ray(i: u32, origin: vec3<f32>, flat: vec3<f32>, u: vec2<f32>) -> vec4<f32> {
    let lt = lights[i];
    let size = light_size(i);
    // the sphere about the light's prim, and the disc the rays aim at (a
    // projector's about its apex)
    let centre = lt.pos_radius.xyz - origin;
    let aim = light_aim(i) - origin;
    let axis = aim / max(length(aim), 1e-6);
    let side = normalize(cross(axis, select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(axis.z) > 0.9)));
    let up = cross(axis, side);
    let r = size * sqrt(u.x);
    let phi = 2.0 * PI * u.y;
    let toward = aim + (side * cos(phi) + up * sin(phi)) * r;
    let span = length(toward);
    let dir = toward / max(span, 1e-6);
    // up to where the ray enters the sphere that holds the light's own
    // prim, then (`EMITTER_SPHERE` aside) on to the light past that prim
    // alone, short of the light's point by a centimetre
    let body = max(lt.source.x, max(size, 0.05));
    let along = dot(centre, dir);
    let beside = dot(centre, centre) - along * along;
    let inside = body * body - beside;
    // (a projector's ray may pass beside its prim's sphere)
    let reach = select(span, min(along - sqrt(max(inside, 0.0)), span), inside > 0.0);
    let posed = params.mode.y > 0.5;
    let curved = dot(flat, dir) <= 0.0;
    var t = -1.0;
    if (reach > 0.0) {
        t = blocked_at(origin, dir, 0.0, reach, posed, curved, 0.0);
    }
    let near = max(reach, 0.0);
    let last = span - 0.01;
    let own = lt.source.w;
    let past = !EMITTER_SPHERE && own > 0.5 && last > near;
    if (t < 0.0 && past) {
        t = blocked_at(origin, dir, near, last, posed, curved, own);
    }
    if (t < 0.0) {
        // the glass over the same way as the opaque test's, but the layers
        // lying on the surface (`lying_on`)
        let lying = lying_on(flat, dir, distance(origin, params.camera.xyz));
        return vec4<f32>(glass_along(origin, dir, select(reach, last, past), select(0.0, own, past), posed, lying), -1.0);
    }
    return vec4<f32>(vec3<f32>(0.0), min(2.0 * size * t / max(span - t, 0.05), 4.0));
}

// The number that draws a pixel's light this frame: with rt_view.wgsl
// `disc_point`'s two, a sequence that fills its range evenly from frame to
// frame (R3, Roberts 2018, so the frames a light is drawn spread over its
// disc too), from a start that differs evenly from a pixel to its
// neighbours (R2).
fn turn(id: vec2<u32>, frame: u32) -> f32 {
    return f32((id.x * 3242174889u + id.y * 2447445414u + frame * 3518319154u) >> 8u) / 16777216.0;
}

fn rand(seed: ptr<function, u32>) -> f32 {
    *seed = pcg(*seed);
    return f32(*seed >> 8u) / 16777216.0;
}

fn unpack_normal(e: vec2<f32>) -> vec3<f32> {
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>((vec2<f32>(1.0) - abs(n.yx)) * s, n.z);
    }
    return normalize(n);
}

// Last frame's pixel of the same surface, or -1: as far from last frame's
// eye as this point was, and turned the same way as `n`. On distance
// alone, the pixel by a room's corner took the other wall's history, which
// is as far: its light slid along the corner as the view moved (2026-11-22,
// grid report), and the anti-aliasing's history spread it further.
fn history_pixel(pixel: vec2<i32>, world: vec3<f32>, dist: f32, n: vec3<f32>) -> vec3<i32> {
    let came = back_of(pixel, world, dist);
    if (params.mode.w > 0.5 && came.at.x >= 0.0) {
        let b = clamp(vec2<i32>(came.at), vec2<i32>(0), vec2<i32>(params.screen.xy) - vec2<i32>(1));
        let guide = textureLoad(prev_guide, b, 0);
        let was = guide.z;
        if (was > 0.0 && abs(was - came.expected) < came.tolerance
            && dot(unpack_normal(guide.xy), n) > HISTORY_FACING) {
            return vec3<i32>(b, select(0, 1, came.moving));
        }
    }
    return vec3<i32>(-1);
}

@compute @workgroup_size(8, 8)
fn cs_lights(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    if (any(pixel >= vec2<i32>(params.screen.xy))) {
        return;
    }
    let depth = textureLoad(depth_tex, pixel, 0);
    // reversed depth: 0 is the sky
    if (depth <= 0.0) {
        textureStore(out_mid, pixel, vec4<u32>(0u));
        return;
    }
    let world = unproject(pixel, depth);
    let flat = normal_at(pixel, depth);
    let n = smooth_normal(pixel, flat);
    let dist = distance(world, params.camera.xyz);
    var seed = pcg(id.x + pcg(id.y + pcg(u32(params.mode.z))));

    // the lights reaching the pixel, and their unshadowed light in all
    var count = 0u;
    var total = 0.0;
    let listed = min(u32(params.mode.x), MAX_LIGHTS);
    var reaching = vec2<u32>(0u);
    var near = lights_near(pixel, world, listed);
    for (var i = next_light(&near); i < MAX_LIGHTS; i = next_light(&near)) {
        let p = light_target(i, world, n);
        if (p > 0.0) {
            count += 1u;
            total += p;
            reaching = with_light(reaching, i);
        }
    }

    // this frame's light. A few lights: drawn in exact proportion to their
    // light, by a sequence that visits them evenly from frame to frame and
    // from a pixel to its neighbours (`turn`), so the share converges as a
    // stratified estimate and the neighbours a pixel reuses bring other
    // lights than its own. Many:
    // candidates drawn evenly among them (Solari's `initial_and_temporal`).
    var chosen = 0u;
    var chosen_target = 0.0;
    var sum = 0.0;
    var confidence = 0.0;
    let few = count <= CANDIDATES;
    if (count > 0u) {
        if (few) {
            let at = turn(id.xy, u32(params.mode.z)) * total;
            var reached = 0.0;
            var walk = reaching;
            for (var i = next_light(&walk); i < MAX_LIGHTS; i = next_light(&walk)) {
                let p = light_target(i, world, n);
                reached += p;
                chosen = i;
                chosen_target = p;
                if (at < reached) {
                    break;
                }
            }
            sum = total;
        } else {
            for (var k = 0u; k < CANDIDATES; k++) {
                let entry = min(u32(rand(&seed) * f32(count)), count - 1u);
                let i = nth_light(reaching, entry);
                let p = light_target(i, world, n);
                let weight = p * f32(count) / f32(CANDIDATES);
                sum += weight;
                if (rand(&seed) * sum < weight) {
                    chosen = i;
                    chosen_target = p;
                }
            }
        }
        confidence = 1.0;
    }
    // last frame's choice, of the same lights (ReSTIR's temporal reuse),
    // where the lights are too many to draw among them all
    let back = history_pixel(pixel, world, dist, n);
    if (!few && params.mode.w > 2.5 && back.x >= 0) {
        let prev = textureLoad(prev_choice, back.xy, 0);
        let row = u32(prev.x);
        if (prev.z > 0.0 && row < listed) {
            let conf = min(prev.z, CONFIDENCE_MAX);
            let p = light_target(row, world, n);
            let weight = p * prev.y * conf;
            sum += weight;
            confidence += conf;
            if (weight > 0.0 && rand(&seed) * sum < weight) {
                chosen = row;
                chosen_target = p;
            }
        }
    }
    var word = 0u;
    var w = 0.0;
    // the penumbra's width, or on a frame with no history `pack_reset`
    var found = 0u;
    if (chosen_target > 0.0 && confidence > 0.0) {
        w = sum / (confidence * chosen_target);
        word = chosen | MID_DRAWN | (u32(min(confidence, 255.0) * 256.0) << 16u);
        let origin = ray_origin(world, flat, dist);
        let at = disc_point(id.xy, u32(params.mode.z));
        // with no history (a new size, a photo's first round), `RESET_RAYS` at once, to
        // lights drawn in proportion to their light at even steps
        // (systematic sampling): what arrives of them is the share itself
        let reset = params.mode.w < 0.5;
        let rays = select(1u, RESET_RAYS, reset);
        let start = turn(id.xy, u32(params.mode.z));
        // which of the rays arrived (a bit each), and what the glass left
        // of their light (0 for one stopped)
        var arrived = 0u;
        var through: array<vec3<f32>, RESET_RAYS>;
        var penumbra = 0.0;
        for (var r = 0u; r < rays; r++) {
            var to = chosen;
            if (reset) {
                to = reset_light(r, start, total, reaching, world, n);
            }
            let ray = light_ray(to, origin, flat, fract(at + f32(r) * vec2<f32>(0.7548777, 0.5698403)));
            if (ray.w < 0.0) {
                arrived |= 1u << r;
                through[r] = ray.xyz;
            } else {
                penumbra = ray.w;
            }
        }
        found = bitcast<u32>(penumbra);
        if (reset) {
            // the colours the rays set out for and those that arrived, from
            // their lights drawn again: sums carried through the rays would
            // be more for every traced frame to keep across them
            var sent = vec3<f32>(0.0);
            var got = vec3<f32>(0.0);
            var lit = 0.0;
            for (var r = 0u; r < RESET_RAYS; r++) {
                let tint = light_tint(reset_light(r, start, total, reaching, world, n));
                sent += tint;
                got += tint * through[r];
                lit += dot(through[r], LUMA);
            }
            // (a colour none of the lights drawn has takes the share)
            let share = lit / f32(RESET_RAYS);
            found = pack_reset(round(lit), select(vec3<f32>(share), got / max(sent, vec3<f32>(1e-6)), sent > vec3<f32>(0.0)));
        } else if (arrived != 0u) {
            word |= MID_SEEN;
            found = pack4x8unorm(vec4<f32>(through[0], 0.0));
        }
    }
    textureStore(out_mid, pixel, vec4<u32>(word, bitcast<u32>(w), found, pack2x16snorm(pack_normal(n))));
}

@compute @workgroup_size(8, 8)
fn cs_lights_reuse(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    let size = vec2<i32>(params.screen.xy);
    if (any(pixel >= size)) {
        return;
    }
    let depth = textureLoad(depth_tex, pixel, 0);
    let tinted = params.prev_camera.w > 0.5;
    if (depth <= 0.0) {
        textureStore(out_choice, pixel, vec4<f32>(0.0));
        textureStore(out_share, pixel, vec4<f32>(1.0, 1.0, 0.0, 0.0));
        textureStore(out_guide, pixel, vec4<f32>(0.0));
        textureStore(out_glass_history, pixel, vec4<f32>(1.0, 1.0, 1.0, 0.0));
        if (tinted) {
            textureStore(out_tint, pixel, vec4<f32>(1.0, 1.0, 1.0, 0.0));
        }
        return;
    }
    let world = unproject(pixel, depth);
    let listed = min(u32(params.mode.x), MAX_LIGHTS);
    // (with no light listed, cs_lights did not run: `mid` is not read)
    var own = vec4<u32>(0u);
    var n: vec3<f32>;
    if (listed > 0u) {
        own = textureLoad(mid, pixel, 0);
        n = unpack_normal(unpack2x16snorm(own.w));
    } else {
        n = smooth_normal(pixel, normal_at(pixel, depth));
    }
    let dist = distance(world, params.camera.xyz);
    var seed = pcg(id.y + pcg(id.x + pcg(u32(params.mode.z) + 0x9e3779b9u)));

    var count = 0u;
    var total = 0.0;
    // a frame's share in the history: each light's, by its light here
    var fresh = 0.0;
    var near = lights_near(pixel, world, listed);
    for (var i = next_light(&near); i < MAX_LIGHTS; i = next_light(&near)) {
        let p = light_target(i, world, n);
        if (p > 0.0) {
            count += 1u;
            total += p;
            fresh += p * light_fresh(i);
        }
    }

    // with no light reaching it, nothing to shadow
    var sample = 1.0;
    var tint = vec3<f32>(1.0);
    var choice = vec4<f32>(0.0);
    if (total > 0.0 && params.mode.w < 0.5) {
        // a frame with no history: cs_lights traced its share at once
        if ((own.x & MID_DRAWN) != 0u) {
            let found = unpack_reset(own.z);
            sample = found.w;
            tint = found.xyz;
            choice = vec4<f32>(f32(own.x & 0xFFu), bitcast<f32>(own.y), f32(own.x >> 16u) / 256.0, 0.0);
        }
    } else if (total > 0.0) {
        // the pixel's sample and its neighbours' on the same surface: their
        // light, its weight here (ReSTIR's p̂ · W · M) and its visibility
        var light: array<u32, 5>;
        var weight: array<f32, 5>;
        // what arrived of its light's red, green and blue (0 stopped, 1
        // through no glass)
        var seen: array<vec3<f32>, 5>;
        var confidence: array<f32, 5>;
        // how far the sample's pixel is (pixels), and the widest penumbra
        // their rays found (m)
        var away: array<f32, 5>;
        var penumbra = 0.0;
        var samples = 0u;
        let angle = rand(&seed) * 2.0 * PI;
        // one light: its visibility from here is the share
        let around = select(NEIGHBOURS, 0u, count == 1u);
        for (var k = 0u; k <= around; k++) {
            var q = pixel;
            var m = own;
            var far = 0.0;
            if (k > 0u) {
                // a golden-angle spiral, turned anew for each pixel and frame
                let r = REUSE_RADIUS * sqrt((f32(k) - 0.5) / f32(NEIGHBOURS));
                let a = angle + f32(k) * 2.39996;
                q = pixel + vec2<i32>(round(vec2<f32>(cos(a), sin(a)) * r));
                far = length(vec2<f32>(q - pixel));
                if (any(q < vec2<i32>(0)) || any(q >= size)) {
                    continue;
                }
                let dq = textureLoad(depth_tex, q, 0);
                if (dq <= 0.0) {
                    continue;
                }
                m = textureLoad(mid, q, 0);
                if (dot(n, unpack_normal(unpack2x16snorm(m.w))) < 0.9
                    || abs(dot(n, unproject(q, dq) - world)) > 0.01 + 0.005 * dist) {
                    continue;
                }
            }
            if ((m.x & MID_DRAWN) == 0u) {
                continue;
            }
            let row = m.x & 0xFFu;
            let conf = f32(m.x >> 16u) / 256.0;
            light[samples] = row;
            // no sample stands for more than all the light here (one drawn
            // where its light hardly reaches would, where it reaches well)
            weight[samples] = min(light_target(row, world, n) * bitcast<f32>(m.y), total) * conf;
            let arrived = (m.x & MID_SEEN) != 0u;
            seen[samples] = select(vec3<f32>(0.0), unpack4x8unorm(m.z).xyz, arrived);
            away[samples] = far;
            penumbra = max(penumbra, select(bitcast<f32>(m.z), 0.0, arrived));
            confidence[samples] = conf;
            samples += 1u;
        }

        // the choice the next frame starts from (ReSTIR's spatial reuse)
        var sum = 0.0;
        var m_sum = 0.0;
        var kept = 0u;
        var kept_target = 0.0;
        for (var k = 0u; k < samples; k++) {
            let wk = weight[k];
            sum += wk;
            m_sum += confidence[k];
            if (wk > 0.0 && rand(&seed) * sum < wk) {
                kept = light[k];
                kept_target = light_target(kept, world, n);
            }
        }
        if (kept_target > 0.0 && m_sum > 0.0) {
            choice = vec4<f32>(f32(kept), min(sum / (m_sum * kept_target), 6e4), m_sum, 0.0);
        }
        // the share: the visibilities weighed by their light here. A
        // neighbour's of the light the pixel traced is another point of
        // the same penumbra within half its width; further, across a
        // sharper edge than that, the pixel's own stands for it. (A second
        // ray from here to another of the lights showed no difference, for
        // a fifth of the pass.)
        let own_light = select(0xFFFFFFFFu, own.x & 0xFFu, (own.x & MID_DRAWN) != 0u);
        let own_seen = select(vec3<f32>(0.0), unpack4x8unorm(own.z).xyz, (own.x & MID_SEEN) != 0u);
        let pixel_size = pixel_width(pixel, depth);
        let soft = 0.5 * penumbra / max(pixel_size, 1e-6);
        var arrived = 0.0;
        var reached = 0.0;
        // the same of each colour: a sample's weight is its light's
        // luminance here, which its light's colour over its luminance
        // turns into its red, green and blue
        var arrived_by = vec3<f32>(0.0);
        var reached_by = vec3<f32>(0.0);
        for (var k = 0u; k < samples; k++) {
            let v = select(seen[k], own_seen, light[k] == own_light && away[k] > soft);
            arrived += weight[k] * dot(v, LUMA);
            reached += weight[k];
            let by = weight[k] * light_tint(light[k]);
            arrived_by += by * v;
            reached_by += by;
        }
        if (reached > 0.0) {
            sample = arrived / reached;
            // (a colour none of the samples' lights has takes the share)
            let has = reached_by > vec3<f32>(1e-6 * reached);
            tint = select(vec3<f32>(sample), arrived_by / max(reached_by, vec3<f32>(1e-30)), has);
        }
    }
    textureStore(out_choice, pixel, choice);

    var share = sample;
    var moment = sample * sample;
    // (a frame with no history counts as one, though it traced more: its
    // variance then comes from its neighbours, rt_denoise.wgsl)
    var frames = 1.0;
    // the sun's light: this frame's ray, then its history
    let sun_now = 1.0 - textureLoad(sun_tex, pixel, 0).g;
    var sun = sun_now;
    var sun_frames = 1.0;
    // and the colour the glass gives it, the same way (`glass_history`)
    var glass = vec4<f32>(textureLoad(sun_glass_now, pixel, 0).rgb, sun_now);
    let back = history_pixel(pixel, world, dist, n);
    if (back.x >= 0) {
        let h = textureLoad(prev_share, back.xy, 0);
        let longest = select(HISTORY, HISTORY_MOVING, back.z > 0);
        var kept = longest;
        if (total > 0.0) {
            kept = min(kept, total / fresh);
        }
        if (light_changed(world)) {
            kept = min(kept, params.changes.z);
        }
        frames = min(h.a + 1.0, kept);
        share = mix(h.r, sample, 1.0 / frames);
        moment = mix(textureLoad(prev_guide, back.xy, 0).w, moment, 1.0 / frames);
        if (tinted) {
            tint = mix(textureLoad(prev_tint, back.xy, 0).rgb, tint, 1.0 / frames);
        }

        var was = h.g;
        sun_frames = min(h.b + 1.0, longest);
        if (abs(was - sun_now) > 0.002) {
            // a history this frame's ray disagrees with: a penumbra's
            // mixed rays leave it be, rays that all agree around the pixel
            // (a shadow that came or left) bring it to them at once, and
            // its length down by as much
            var lo = sun_now;
            var hi = sun_now;
            for (var y = -SUN_RANGE; y <= SUN_RANGE; y++) {
                for (var x = -SUN_RANGE; x <= SUN_RANGE; x++) {
                    // every other pixel of the square: 13 rays tell a
                    // penumbra from a shadow gone as well as its 25
                    if (((x + y) & 1) != 0) {
                        continue;
                    }
                    let q = clamp(pixel + vec2<i32>(x, y), vec2<i32>(0), size - vec2<i32>(1));
                    let v = 1.0 - textureLoad(sun_tex, q, 0).g;
                    lo = min(lo, v);
                    hi = max(hi, v);
                }
            }
            let within = clamp(was, lo, hi);
            sun_frames = max(sun_frames * (1.0 - abs(within - was)), 1.0);
            was = within;
        }
        sun = mix(was, sun_now, 1.0 / sun_frames);
        // (the mask has 8 bits: nearer than that to lit or dark is it)
        sun = select(sun, round(sun), abs(sun - round(sun)) < 0.002);
        glass = glass_history(pixel, size, glass_back(pixel, world, dist), glass, sun_frames);
    }
    textureStore(out_share, pixel, vec4<f32>(share, sun, sun_frames, frames));
    textureStore(out_glass_history, pixel, glass);
    textureStore(out_guide, pixel, vec4<f32>(pack_normal(n), dist, moment));
    if (tinted) {
        textureStore(out_tint, pixel, vec4<f32>(tint, 0.0));
    }
}

// Last frame's glass history where `pixel`'s surface stood, read between
// the four pixels about that point (bilinear) of those that were of the
// same surface (`history_pixel`'s test): the nearest pixel's alone, as the
// share's, would draw a pane's coloured edge in steps where the view
// moves. -1 where none was.
fn glass_back(pixel: vec2<i32>, world: vec3<f32>, dist: f32) -> vec4<f32> {
    let came = back_of(pixel, world, dist);
    if (params.mode.w < 0.5 || came.at.x < 0.0) {
        return vec4<f32>(-1.0);
    }
    let p = came.at - vec2<f32>(0.5);
    let base = vec2<i32>(floor(p));
    let f = p - floor(p);
    var sum = vec4<f32>(0.0);
    var weight = 0.0;
    for (var k = 0; k < 4; k++) {
        let o = vec2<i32>(k & 1, k >> 1u);
        let q = base + o;
        if (any(q < vec2<i32>(0)) || any(q >= vec2<i32>(params.screen.xy))) {
            continue;
        }
        let was = textureLoad(prev_guide, q, 0).z;
        if (was <= 0.0 || abs(was - came.expected) >= came.tolerance) {
            continue;
        }
        let w = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
        sum += textureLoad(prev_glass, q, 0) * w;
        weight += w;
    }
    if (weight < 1e-3) {
        return vec4<f32>(-1.0);
    }
    return sum / weight;
}

// The colour the glass gives the sun at `pixel` times its light (a stopped
// ray's is 0, rt_shadows.wgsl), and that light (a), averaged over the
// frames as the sun's shadow is (cs_lights_reuse, `frames` its length):
// its ray goes to another point of the sun's disc each frame, so a pane's
// edge is a penumbra of colour. Both are averaged alike, so that their
// ratio is the colour of the rays that arrived (`glass_of`). A history
// this frame's rays around the pixel disagree with (a pane carried off)
// comes to them at once, its length down by as much. `was` is
// `glass_back`'s, `now` this frame's ray.
fn glass_history(pixel: vec2<i32>, size: vec2<i32>, back: vec4<f32>, now: vec4<f32>, frames_in: f32) -> vec4<f32> {
    if (back.a < 0.0) {
        return now;
    }
    var was = back;
    var frames = frames_in;
    let d = abs(was - now);
    if (max(max(d.r, d.g), max(d.b, d.a)) > 0.002) {
        var lo = now;
        var hi = now;
        for (var y = -SUN_RANGE; y <= SUN_RANGE; y++) {
            for (var x = -SUN_RANGE; x <= SUN_RANGE; x++) {
                // (every other pixel of the square, as the sun's)
                if (((x + y) & 1) != 0) {
                    continue;
                }
                let q = clamp(pixel + vec2<i32>(x, y), vec2<i32>(0), size - vec2<i32>(1));
                let v = vec4<f32>(textureLoad(sun_glass_now, q, 0).rgb, 1.0 - textureLoad(sun_tex, q, 0).g);
                lo = min(lo, v);
                hi = max(hi, v);
            }
        }
        let within = clamp(was, lo, hi);
        let m = abs(within - was);
        frames = max(frames * (1.0 - max(max(m.r, m.g), max(m.b, m.a))), 1.0);
        was = within;
    }
    return mix(was, now, 1.0 / frames);
}

// The colour the glass gives the sunlight that arrives at `pixel`: the
// glass history's colour times the light over its light, over the pixels of its 3 × 3 on the same surface, which smooths what the
// history's reprojection leaves of the rays' grain where the view moves.
// A stopped ray counting white would pale a pane's colour within the
// penumbra of another shadow; over the light, it does not count. Where
// hardly any light arrives the colour fades to 1: it lights nothing there.
fn glass_of(pixel: vec2<i32>) -> vec3<f32> {
    let size = vec2<i32>(params.screen.xy);
    let centre = textureLoad(guide_now, pixel, 0);
    if (centre.z <= 0.0) {
        return vec3<f32>(1.0);
    }
    let n = unpack_normal(centre.xy);
    var colour = vec3<f32>(0.0);
    var light = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let q = clamp(pixel + vec2<i32>(x, y), vec2<i32>(0), size - vec2<i32>(1));
            let g = textureLoad(guide_now, q, 0);
            if (g.z <= 0.0 || abs(g.z - centre.z) > 0.01 + 0.02 * centre.z || dot(n, unpack_normal(g.xy)) < 0.9) {
                continue;
            }
            let kept = textureLoad(glass_kept, q, 0);
            colour += kept.rgb;
            light += kept.a;
        }
    }
    let through = clamp(colour / max(light, 1e-4), vec3<f32>(0.0), vec3<f32>(1.0));
    return mix(vec3<f32>(1.0), through, clamp(light * 20.0, 0.0, 1.0));
}

@compute @workgroup_size(8, 8)
fn cs_lights_filter(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    if (any(pixel >= vec2<i32>(params.screen.xy))) {
        return;
    }
    let ray = textureLoad(sun_tex, pixel, 0);
    let light = clamp(textureLoad(share_tex, pixel, 0).rg, vec2<f32>(0.0), vec2<f32>(1.0));
    let shadow = 1.0 - light.y;
    var tint = vec3<f32>(light.x);
    if (params.prev_camera.w > 0.5) {
        tint = clamp(textureLoad(tint_tex, pixel, 0).rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    textureStore(out_mask, pixel, vec4<f32>(max(shadow, ray.r), tint));
    textureStore(out_glass, pixel, vec4<f32>(glass_of(pixel), 1.0));
}
