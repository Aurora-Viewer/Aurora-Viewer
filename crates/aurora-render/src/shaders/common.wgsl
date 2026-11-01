enable wgpu_binding_array;

// Shared declarations for Aurora Viewer shaders.
//
// The atmospheric sky and water fog functions are ports of the Second Life
// viewer shaders (skyV.glsl, cloudsV/F.glsl, waterFogF.glsl), Copyright (C)
// Linden Research, Inc., LGPL 2.1.

const MAX_LIGHTS: u32 = 64u;
const CASCADES: u32 = 3u;
const PI: f32 = 3.14159265359;

struct Light {
    pos_radius: vec4<f32>,   // xyz position, w radius
    color_falloff: vec4<f32>, // rgb linear color * intensity, w falloff
};

// Projector of a light with a texture (types.rs `Projector`).
struct ProjectorLight {
    matrix: mat4x4<f32>,     // world to the projected texture
    origin: vec4<f32>,       // apex of the frustum, w focus
    dir: vec4<f32>,          // direction, w depth of the frustum
    params: vec4<f32>,       // x near distance, y ambiance
    tex: vec4<u32>,          // x texture slot
};

struct Frame {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    // view-projection for positions relative to `eye` (object vertices)
    eye_view_proj: mat4x4<f32>,
    // inverse of the projection by the view's rotation alone: directions
    // from the camera, with no region position to round (sky)
    inv_view_proj_rot: mat4x4<f32>,
    eye: vec4<f32>,             // xyz: at or near the camera
    camera_pos: vec4<f32>,      // xyz, w = time (s)
    sun_dir: vec4<f32>,         // xyz towards the light, w = sun visible (0..1)
    sun_color: vec4<f32>,       // rgb radiance
    sky_zenith: vec4<f32>,      // ambient from above
    sky_horizon: vec4<f32>,     // ambient from the horizon
    ground_color: vec4<f32>,    // ambient from below
    fog: vec4<f32>,             // rgb color, w density
    params: vec4<f32>,          // x exposure, y water height, z shadows enabled, w light count
    cascade_splits: vec4<f32>,  // view-space distances
    screen: vec4<f32>,          // width, height, 1/w, 1/h
    misc: vec4<f32>,            // x ssao, y ssr, z water reflection texture, w reflection pass (2: mirrored)
    clip_plane: vec4<f32>,      // reflection passes: keep dot(n, p) + w >= 0
    cam_right: vec4<f32>,       // camera right axis, w = particle soft fade distance
    cam_up: vec4<f32>,          // camera up axis, camera-local macro waterline
    water_fog: vec4<f32>,       // linear fog color, w density (kd)
    water_params: vec4<f32>,    // x ks, y fresnel scale, z fresnel offset, w blur multiplier
    water_waves: vec4<f32>,     // wave1 dir xy, wave2 dir xy
    water_normal: vec4<f32>,    // normal scale xyz, w underwater fog density
    water_air_vp: mat4x4<f32>,
    water_air_inv_vp: mat4x4<f32>,
    water_air: vec4<f32>,       // x: exterior capture available
    mirror_plane: vec4<f32>,    // n.xyz, d (0 normal = no mirror)
    mirror_box: mat4x4<f32>,    // world -> mirror probe box ([-0.5, 0.5]^3)
    mirror_view_proj: mat4x4<f32>, // the mirror pass's camera (mirror_tex)
    // EEP sky (legacy haze model)
    sky_light: vec4<f32>,       // light norm xyz (z >= -0.1), sun/moon glow factor
    sky_sunlight: vec4<f32>,    // sunlight rgb, cloud shadow
    sky_ambient: vec4<f32>,     // ambient rgb, density multiplier
    sky_blue_horizon: vec4<f32>,// rgb, haze horizon
    sky_blue_density: vec4<f32>,// rgb, haze density
    sky_glow: vec4<f32>,        // glow xyz, max_y
    sky_cloud_color: vec4<f32>, // rgb, cloud scale
    sky_cloud_pd1: vec4<f32>,   // cloud pos/density 1, w cloud variance
    sky_cloud_pd2: vec4<f32>,   // cloud pos/density 2, w hdr scale
    sky_sun: vec4<f32>,         // true sun dir, sun scale
    sky_moon: vec4<f32>,        // moon dir, moon scale
    sky_misc: vec4<f32>,        // dome offset, dome radius, moon brightness, star brightness
    tex_slots: vec4<u32>,       // cloud, sun, moon, water normal map
    sky_ll: vec4<f32>,          // x classic sky (1 = no reflection probe ambiance), y distance multiplier, z 1 = PBR materials' occlusion applied, w 1 = specular occlusion from GTAO's bent normal (in ao_tex.gba)
    sky_obj_light: vec4<f32>,   // rgb sun / moon color lighting objects (mSunDiffuse, max component <= 1)
    sky_optics: vec4<f32>,      // x moisture level (rainbow), y droplet radius, z ice level (22° halo)
    sky_optics_tex: vec4<u32>,  // x rainbow texture slot, y halo texture slot (0: none)
    // night sky (stars.rs StarFrame): region directions of the catalogue's
    // axes; w: catalogue (-1 none), Milky Way strength, 0
    sky_stars: array<vec4<f32>, 3>,
    sky_star_pass: vec4<f32>,   // this pass's target: width, height, 1/w, 1/h (0: frame.screen)
    anim_clock: vec4<u32>,      // effects clock (rez, portals): x ms (wrapping), y sub-ms fraction (f32 bits); zw the same for texture animations (held while the world is frozen)
    selection_color: vec4<f32>, // selected-face overlay, from the UI palette
    rez_gradient: array<vec4<f32>, 3>, // rez effect: the brand gradient (linear rgb), edge to trail; [0].w its look
    cascade_vp: array<mat4x4<f32>, 3>,
    lights: array<Light, 64>,
    // which of `lights` a fragment walks (light_tiles.rs, LightSets)
    light_grid: vec4<f32>,      // x, y: columns and rows per pixel
    light_forward: vec4<f32>,   // camera forward axis (fragment depth)
    light_mode: vec4<u32>,      // x: 1 = the list narrowed by the sets below, y: 1 = checked, z: the view shown instead of the image (debug_view: 1 shadows, 2 ambient occlusion), w: how the probes are walked (PROBE_WALK_*)
    // lights that cover each screen column, each screen row and each slice
    // of depth: two sets per element, each its low then its high 32 lights
    light_columns: array<vec4<u32>, 16>,
    light_rows: array<vec4<u32>, 16>,
    light_slices: array<vec4<u32>, 16>,
    // projector of light i, for the lights whose bit is set in
    // `light_projectors` (x: lights 0..31, y: lights 32..63)
    projectors: array<ProjectorLight, 64>,
    light_projectors: vec4<u32>,
    // shadow maps of the two projectors that cast shadows: world to the
    // clip space of each, its light (-1: none), and what it leaves unlit
    // of its shadows (1 - fade)
    spot_vp: array<mat4x4<f32>, 2>,
    spot_lights: vec4<i32>,
    spot_fade: vec4<f32>,
    contact: vec4<f32>,         // x: 1 = the contact shadows' mask is read (main view), y: 1 = blended faces opaque enough cast it
    features: vec4<u32>,        // x: FEATURE_* bits, in every pass (main view, reflections, probes)
    // Motion vectors (main view; zero in the other passes), without the TAA
    // jitter: this frame's view-projection for positions relative to `eye`,
    // the last frame's for the same positions, and this frame's clip space
    // to the last frame's (a point the depth places, where nothing drew).
    motion_vp: mat4x4<f32>,
    motion_prev_vp: mat4x4<f32>,
    motion_reproj: mat4x4<f32>,
};

// frame.features.x: "HDR and Emissive" off, the Legacy Viewer's G-buffer has
// no emissive target (RenderEnableEmissiveBuffer, HAS_EMISSIVE): the PBR
// faces it draws deferred (opaque, alpha-masked, terrain) lose their
// emissive; blended and HUD faces keep it (pbralphaF, pbropaqueF IS_HUD)
const FEATURE_NO_EMISSIVE: u32 = 1u;

fn deferred_emissive_off() -> bool {
    return (frame.features.x & FEATURE_NO_EMISSIVE) != 0u;
}

struct DrawRecord {
    model: mat4x4<f32>,
    base_color: vec4<f32>,
    emissive: vec4<f32>,      // rgb emissive, a glow
    uv_st: vec4<f32>,         // scale s, scale t, offset s, offset t
    params: vec4<f32>,        // x rotation, y metallic, z roughness, w alpha cutoff
    tex: vec4<u32>,           // base, normal, metallic-roughness, emissive
    flags: vec4<u32>,         // x low 16 flags / high 16 HUD zoom ×4096 (0 = 1), y palette base (anchored: anchor matrix), z skin binds, w texture animation mode | size x << 8 | size y << 16
    mat_uv: vec4<f32>,        // normal map: scale s, t, offset s, t
    spec_uv: vec4<f32>,       // legacy specular / PBR metallic-roughness map: scale s, t, offset s, t
    legacy: vec4<f32>,        // x normal rot, y spec / MR rot, z glossiness (PBR: emissive rot), w env intensity
    spec_color: vec4<f32>,    // legacy specular light color (PBR: emissive map scale s, t, offset s, t)
    anim: vec4<u32>,          // texture animation: time origin (ms), start, length, rate (f32 bits)
    anim_xf: vec4<f32>,       // texture animation: texture entry parts it leaves alone (tex_anim.rs)
    rez: vec4<u32>,           // rez effect: x start (anim clock ms), y mode | line's length (ms) << 8 | own look << 20, z bottom, w height (f32 bits)
    portal_clip: vec4<f32>,   // finite opening: center.xyz, diamond-coded entry normal
};

const FLAG_FULLBRIGHT: u32 = 1u;
const FLAG_PORTAL_CLIP: u32 = 16384u;
const FLAG_PORTAL_EFFECT: u32 = 32768u;
const PORTAL_HALF_WIDTH: f32 = 1.0;
const PORTAL_HALF_HEIGHT: f32 = 1.5;
const PORTAL_SURFACE_SCALE: f32 = 1.16;
const FLAG_ALPHA_MASK: u32 = 2u;
const FLAG_ALPHA_BLEND: u32 = 4u;
const FLAG_PBR: u32 = 8u;
const FLAG_HIGHLIGHT: u32 = 16u;
const FLAG_PLANAR: u32 = 32u;
const FLAG_TERRAIN: u32 = 64u;
const FLAG_SKINNED: u32 = 128u;
const FLAG_LEGACY_MAT: u32 = 256u;
const FLAG_EMISSIVE_MASK: u32 = 512u;
const FLAG_TEX_ANIM: u32 = 1024u;
const FLAG_DOUBLE_SIDED: u32 = 2048u;
// placed by the GPU: model relative to the anchor matrix palettes[flags.y]
// (worn objects); skinned: skeleton placed by its own anchor
const FLAG_ANCHORED: u32 = 4096u;
const FLAG_LIP_SYNC: u32 = 8192u;
// a skeleton's own anchor (skeleton space to world), from its first joint
const SKELETON_ANCHOR: u32 = 208u;
// the weights of a skeleton's live morphs, after its anchors (object.wgsl)
const LIVE_MORPH_WEIGHTS: u32 = 256u;

@group(0) @binding(0) var<uniform> frame: Frame;
// atlas of 2x2 tiles, cascade i in tile (i % 2, i / 2)
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(0) @binding(3) var ao_tex: texture_2d<f32>;
@group(0) @binding(4) var lin_clamp: sampler;
@group(0) @binding(5) var water_refl_tex: texture_2d<f32>;
@group(0) @binding(6) var mirror_tex: texture_2d<f32>;
@group(0) @binding(7) var scene_tex: texture_2d<f32>;
@group(0) @binding(8) var depth_tex: texture_depth_2d;
@group(0) @binding(20) var water_interface: texture_depth_2d;
@group(0) @binding(21) var water_air_color: texture_2d<f32>;
@group(0) @binding(22) var water_air_depth: texture_depth_2d;
@group(0) @binding(23) var water_exclusion_mask: texture_2d<f32>;
// GTAO's screen-space indirect light (gtao.rs), when SCREEN_INDIRECT
@group(0) @binding(14) var il_tex: texture_2d<f32>;

// Reflection probes (probes.rs, reflectionProbeF.glsl).
struct Probe {
    center_radius: vec4<f32>,
    // x ambiance scale, y radiance scale, z fade in, w kind (0 auto, 1 sphere, -1 box)
    params: vec4<f32>,
    slot: vec4<u32>,
    box_inv: mat4x4<f32>,
};
struct ProbeSet {
    count: u32,
    max_lod: f32,
    enabled: u32,
    _pad: u32,
    // the box probes (x: probes 0..31, y: 32..63), which sample_probes_pair
    // looks through first
    boxes: vec4<u32>,
    // the probes that cover each screen column, each screen row and each
    // slice of depth of the main view (probes::tiles), laid out as the
    // lights' sets of the frame: two sets per element, each its low then
    // its high 32 probes
    tile_grid: vec4<f32>,       // x, y: columns and rows per pixel
    tile_forward: vec4<f32>,    // camera forward axis (fragment depth)
    tile_mode: vec4<u32>,
    tile_columns: array<vec4<u32>, 16>,
    tile_rows: array<vec4<u32>, 16>,
    tile_slices: array<vec4<u32>, 16>,
    probes: array<Probe, 64>,
};
@group(0) @binding(9) var<storage, read> probe_set: ProbeSet;
@group(0) @binding(10) var probe_tex: texture_cube_array<f32>;
@group(0) @binding(11) var probe_sampler: sampler;
@group(0) @binding(12) var probe_irr_tex: texture_cube_array<f32>;
// the sun's contact shadows (contact_shadows.wgsl): 0 lit, 1 in shadow
@group(0) @binding(13) var contact_tex: texture_2d<f32>;

@group(1) @binding(0) var<storage, read> records: array<DrawRecord>;
@group(1) @binding(1) var<storage, read> palettes: array<mat4x4<f32>>;
struct SkinBind {
    inverse_bind: mat4x4<f32>,
    joint: vec4<u32>,
};
@group(1) @binding(2) var<storage, read> skin_binds: array<SkinBind>;
// What the last frame drew with (motion_history.rs): the joint palettes and
// each record's model matrix.
@group(1) @binding(3) var<storage, read> prev_palettes: array<mat4x4<f32>>;
@group(1) @binding(4) var<storage, read> prev_models: array<mat4x4<f32>>;

// Bindless textures pooled in pages (textures.rs): a slot's location packs
// the page (high 16 bits) and its layer (low 16 bits).
@group(2) @binding(0) var tex_pages: binding_array<texture_2d_array<f32>>;
@group(2) @binding(1) var tex_sampler: sampler;
@group(2) @binding(2) var<storage, read> tex_loc: array<u32>;

// Local lights of a fragment (light_tiles.rs). In the main view, the lights
// that cover its screen column, its screen row and its slice of depth at
// once (bit i for `frame.lights[i]`); in the others (reflections, probe
// faces, impostor pictures), the whole list.
const LIGHT_TILES: u32 = 32u;
const LIGHT_SLICE_NEAR: f32 = 0.25;
const LIGHT_SLICES_PER_OCTAVE: f32 = 3.0;

// Set `i` of a table that holds two sets per element.
fn light_table_set(pair: vec4<u32>, i: u32) -> vec2<u32> {
    return select(pair.xy, pair.zw, (i & 1u) == 1u);
}

// Column, row and slice of depth of a fragment of the main view, in tables
// of `grid.xy` columns and rows per pixel, for a camera looking along
// `forward`.
fn tile_cell(world_pos: vec3<f32>, frag: vec2<f32>, grid: vec2<f32>, forward: vec3<f32>) -> vec3<u32> {
    let column = min(u32(frag.x * grid.x), LIGHT_TILES - 1u);
    let row = min(u32(frag.y * grid.y), LIGHT_TILES - 1u);
    let depth = dot(world_pos - frame.camera_pos.xyz, forward);
    let octaves = log2(max(depth, LIGHT_SLICE_NEAR) / LIGHT_SLICE_NEAR);
    let slice = min(u32(octaves * LIGHT_SLICES_PER_OCTAVE), LIGHT_TILES - 1u);
    return vec3<u32>(column, row, slice);
}

// The lights of the main view's sets for a fragment.
fn light_set(world_pos: vec3<f32>, frag: vec2<f32>) -> vec2<u32> {
    let cell = tile_cell(world_pos, frag, frame.light_grid.xy, frame.light_forward.xyz);
    return light_table_set(frame.light_columns[cell.x >> 1u], cell.x)
        & light_table_set(frame.light_rows[cell.y >> 1u], cell.y)
        & light_table_set(frame.light_slices[cell.z >> 1u], cell.z);
}

// Next member of a set, lowest first; -1 once it is empty.
fn next_member(members: ptr<function, vec2<u32>>) -> i32 {
    let lo = (*members).x;
    if (lo != 0u) {
        (*members).x = lo & (lo - 1u);
        return i32(firstTrailingBit(lo));
    }
    let hi = (*members).y;
    if (hi != 0u) {
        (*members).y = hi & (hi - 1u);
        return i32(32u + firstTrailingBit(hi));
    }
    return -1;
}

// The lights a fragment walks: the point lights left in `held` (bit i for
// `frame.lights[i]`), and its projectors. The two are walked apart, each
// with its own loop, and the point lights' is written out in each lighting
// function: sharing one loop and one `LightAt` with the projectors cost
// 0.09 ms of GPU time a frame with 64 point lights and no projector.
struct LightWalk {
    held: vec2<u32>,
    projectors: vec2<u32>,
};

// Bits of the first `n` lights of the frame's list.
fn light_first(n: u32) -> vec2<u32> {
    let part = (1u << (n & 31u)) - 1u;
    let lo = select(part, 0xffffffffu, n >= 32u);
    let hi = select(select(part, 0xffffffffu, n >= 64u), 0u, n <= 32u);
    return vec2<u32>(lo, hi);
}

fn light_walk(world_pos: vec3<f32>, frag: vec2<f32>) -> LightWalk {
    // the lights of the fragment's sets or, without the sets, the list
    var held = light_first(u32(frame.params.w));
    if (frame.light_mode.x != 0u) {
        held = light_set(world_pos, frag);
    }
    let projectors = frame.light_projectors.xy;
    var walk: LightWalk;
    walk.held = held & ~projectors;
    walk.projectors = held & projectors;
    return walk;
}

// The walk of the fragment's projectors.
fn projector_walk(walk: LightWalk) -> LightWalk {
    var w: LightWalk;
    w.held = walk.projectors;
    w.projectors = vec2<u32>(0u);
    return w;
}

// Next light of the walk, in the order of the frame's list (the lights
// are summed in that order); -1 at its end.
fn next_light(walk: ptr<function, LightWalk>) -> i32 {
    let lo = (*walk).held.x;
    if (lo != 0u) {
        (*walk).held.x = lo & (lo - 1u);
        return i32(firstTrailingBit(lo));
    }
    let hi = (*walk).held.y;
    if (hi != 0u) {
        (*walk).held.y = hi & (hi - 1u);
        return i32(32u + firstTrailingBit(hi));
    }
    return -1;
}

// AURORA_LIGHT_TILES=check: `col`, or magenta where a light of the frame's
// list reaches the fragment without being in its set. That never happens:
// the image is the one the whole list gives.
fn light_tiles_checked(col: vec3<f32>, world_pos: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    if (frame.light_mode.y == 0u) {
        return col;
    }
    let held = light_set(world_pos, frag);
    let count = u32(frame.params.w);
    for (var i = 0u; i < count; i++) {
        let lt = frame.lights[i];
        var bit = held.x >> (i & 31u);
        if (i >= 32u) {
            bit = held.y >> (i & 31u);
        }
        if (distance(lt.pos_radius.xyz, world_pos) < lt.pos_radius.w && (bit & 1u) == 0u) {
            return vec3<f32>(1.0, 0.0, 1.0);
        }
    }
    return col;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3<f32>(0.0)) + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn sample_tex(idx: u32, uv: vec2<f32>) -> vec4<f32> {
    let loc = tex_loc[idx];
    return textureSample(tex_pages[loc >> 16u], tex_sampler, uv, loc & 0xffffu);
}

fn sample_tex_level(idx: u32, uv: vec2<f32>, lod: f32) -> vec4<f32> {
    let loc = tex_loc[idx];
    return textureSampleLevel(tex_pages[loc >> 16u], tex_sampler, uv, loc & 0xffffu, lod);
}

fn in_reflection_pass() -> bool {
    return frame.misc.w > 0.5;
}

/// Planar reflection passes (water, mirror): a view mirrored by a plane.
fn mirrored_pass() -> bool {
    return frame.misc.w > 1.5;
}

// Probes are sampled in the main view only (captures see the sky instead,
// a single bounce).
fn probes_on() -> bool {
    return probe_set.enabled != 0u && !in_reflection_pass();
}

fn probe_max_lod() -> f32 {
    return probe_set.max_lod;
}

// reflectionProbeF.glsl sampleProbes (+ shouldSampleProbe, tapRefMap,
// sphereWeight, boxIntersect), in world space: radiance seen along `dir`
// from `pos` at mip `lod`. Probe 0 is the default probe, sampled whenever
// automatic probes are. w = 0 when no probe covers `pos`.
// `frag`: the fragment's pixel, `pos` being where it is in the world (the
// main view finds its probes by screen tile and depth).
fn sample_probes(pos: vec3<f32>, frag: vec2<f32>, dir: vec3<f32>, lod: f32) -> vec4<f32> {
    let radiance = ProbeLookup(dir, lod, false);
    return sample_probes_pair(pos, frag, radiance, radiance, false, true, vec3<f32>(0.0)).a;
}

// sampleReflectionProbesWater: the default probe and manual probes only
// (automatic probes are not sampled for water).
fn sample_probes_water(pos: vec3<f32>, frag: vec2<f32>, dir: vec3<f32>, lod: f32) -> vec4<f32> {
    let radiance = ProbeLookup(dir, lod, false);
    return sample_probes_pair(pos, frag, radiance, radiance, false, false, vec3<f32>(0.0)).a;
}

// How a fragment walks the probes (`frame.light_mode.w`, probes::ProbeWalk):
// the whole list (the views other than the main one), the whole list twice
// per lookup as at first, the probes of its screen tile and depth, or those
// and a check of them.
const PROBE_WALK_LIST: u32 = 0u;
const PROBE_WALK_PER_LOOKUP: u32 = 1u;
const PROBE_WALK_TILES: u32 = 2u;
const PROBE_WALK_CHECK: u32 = 3u;

// Depth under which a position is not one the sets were filled for (they
// hold what is in front of the camera, light_tiles.rs).
const PROBE_TILES_NEAREST: f32 = 0.01;

// The probes of the main view's sets for a fragment (bit i for
// `probe_set.probes[i]`): those that cover its screen column, its screen
// row and its slice of depth at once.
//
// The whole list for a position that is not in front of the camera. There
// are such fragments: under multisampling, a sample at the edge of a
// triangle seen edge-on is shaded where the triangle's plane meets the
// centre of its pixel, which can be behind the camera. The probes that
// cover that point are those the whole list gives.
fn probe_tiles(world_pos: vec3<f32>, frag: vec2<f32>) -> vec2<u32> {
    let forward = probe_set.tile_forward.xyz;
    let cell = tile_cell(world_pos, frag, probe_set.tile_grid.xy, forward);
    let held = light_table_set(probe_set.tile_columns[cell.x >> 1u], cell.x)
        & light_table_set(probe_set.tile_rows[cell.y >> 1u], cell.y)
        & light_table_set(probe_set.tile_slices[cell.z >> 1u], cell.z);
    let in_front = dot(world_pos - frame.camera_pos.xyz, forward) >= PROBE_TILES_NEAREST;
    return select(vec2<u32>(0xffffffffu), held, in_front);
}

// AURORA_PROBE_WALK=check: whether a probe of the list covers `pos` without
// being in the fragment's sets. That never happens: the image is the one
// the whole list gives.
fn probe_tiles_miss(pos: vec3<f32>, frag: vec2<f32>) -> bool {
    let held = probe_tiles(pos, frag);
    let count = min(probe_set.count, 64u);
    for (var i = 1u; i < count; i++) {
        var bit = held.x >> (i & 31u);
        if (i >= 32u) {
            bit = held.y >> (i & 31u);
        }
        if ((bit & 1u) != 0u) {
            continue;
        }
        if (probe_set.probes[i].params.w < 0.0) {
            let lp = (probe_set.probes[i].box_inv * vec4<f32>(pos, 1.0)).xyz;
            if (all(abs(lp) <= vec3<f32>(1.0))) {
                return true;
            }
        } else {
            let sphere = probe_set.probes[i].center_radius;
            let delta = pos - sphere.xyz;
            if (dot(delta, delta) <= sphere.w * sphere.w) {
                return true;
            }
        }
    }
    // (the default probe is in every set)
    return count > 0u && (held.x & 1u) == 0u;
}

fn sample_probes_ex(pos: vec3<f32>, dir: vec3<f32>, lod: f32, irradiance: bool, amblit: vec3<f32>, water: bool) -> vec4<f32> {
    let count = min(probe_set.count, 64u);
    // never let automatic probes encroach on box probes
    var sample_automatic = true;
    for (var i = 1u; i < count; i++) {
        let p = probe_set.probes[i];
        if (p.params.w < 0.0) {
            let lp = (p.box_inv * vec4<f32>(pos, 1.0)).xyz;
            if (all(abs(lp) <= vec3<f32>(1.0))) {
                sample_automatic = false;
            }
        }
    }
    var col0 = vec3<f32>(0.0);
    var col1 = vec3<f32>(0.0);
    var w0 = 0.0;
    var w1 = 0.0;
    var dw1 = 0.0;
    for (var i = 0u; i < count; i++) {
        let p = probe_set.probes[i];
        let kind = p.params.w;
        let manual = kind != 0.0;
        if (!manual && (!sample_automatic || (water && i != 0u))) {
            continue;
        }
        let c = p.center_radius.xyz;
        var v: vec3<f32>;
        var w: f32;
        var dw = 0.0;
        if (kind < 0.0) {
            // box influence volume with parallax correction
            let lp = (p.box_inv * vec4<f32>(pos, 1.0)).xyz;
            if (any(abs(lp) > vec3<f32>(1.0))) {
                continue;
            }
            let ray = (p.box_inv * vec4<f32>(dir, 0.0)).xyz;
            // the irradiance lookup pads the box (boxIntersect scale 3)
            let scale = select(1.0, 3.0, irradiance);
            let first = (vec3<f32>(scale) - lp) / ray;
            let second = (vec3<f32>(-scale) - lp) / ray;
            let far = max(first, second);
            let dist = min(far.x, min(far.y, far.z));
            v = pos + dir * dist - c;
            w = max(1.0 - max(max(abs(lp.x), abs(lp.y)), abs(lp.z)), 0.001);
        } else {
            let r = p.center_radius.w;
            let delta = pos - c;
            if (i != 0u && dot(delta, delta) > r * r) {
                continue;
            }
            if (kind >= 1.0) {
                // sphereIntersect: parallax for manual sphere probes only
                let l = c - pos;
                let tca = dot(l, dir);
                let d2 = dot(l, l) - tca * tca;
                v = pos + dir * (tca + sqrt(max(r * r - d2, 0.0))) - c;
            } else {
                v = dir;
            }
            // sphereWeight
            let r1 = r * 0.5;
            let d = max(length(delta), 0.001);
            let atten = 1.0 - max(d - r1, 0.0) / max(r - r1, 0.001);
            let base = p.params.z / d;
            dw = base * atten * max(r, 1.0) * 4.0;
            w = base * atten;
        }
        var refcol: vec3<f32>;
        if (irradiance) {
            let irr = textureSampleLevel(probe_irr_tex, probe_sampler, v, i32(p.slot.x), 0.0).rgb * p.params.x;
            refcol = mix(amblit, irr, min(p.params.x, 1.0));
        } else {
            refcol = textureSampleLevel(probe_tex, probe_sampler, v, i32(p.slot.x), lod).rgb * p.params.y;
        }
        if (manual) {
            col1 += refcol * w;
            w1 += w;
            dw1 += dw;
        } else {
            col0 += refcol * w;
            w0 += w;
        }
    }
    // mix automatic and manual probes
    if (sample_automatic && w0 > 0.0) {
        col0 /= w0;
        if (w1 > 0.0) {
            col1 = mix(col0, col1 / w1, min(dw1, 1.0));
            col0 = vec3<f32>(0.0);
        }
    } else if (w1 > 0.0) {
        col1 /= w1;
        col0 = vec3<f32>(0.0);
    }
    return vec4<f32>(col0 + col1, select(0.0, 1.0, w0 + w1 > 0.0));
}

// What a fragment asks of the probes: the radiance along `dir` at mip `lod`,
// or the irradiance along it.
struct ProbeLookup {
    dir: vec3<f32>,
    lod: f32,
    irradiance: bool,
}

struct ProbePair {
    a: vec4<f32>,
    b: vec4<f32>,
}

// A probe's contribution to a lookup, along `v` in its cube (tapRefMap,
// tapIrradianceMap).
fn probe_tap(i: u32, v: vec3<f32>, lookup: ProbeLookup, amblit: vec3<f32>) -> vec3<f32> {
    let slot = i32(probe_set.probes[i].slot.x);
    let scale = probe_set.probes[i].params;
    if (lookup.irradiance) {
        let irr = textureSampleLevel(probe_irr_tex, probe_sampler, v, slot, 0.0).rgb * scale.x;
        return mix(amblit, irr, min(scale.x, 1.0));
    }
    return textureSampleLevel(probe_tex, probe_sampler, v, slot, lookup.lod).rgb * scale.y;
}

// Where a lookup leaves a box probe, from `lp` inside it (boxIntersect; the
// irradiance lookup pads the box, scale 3).
fn probe_box_exit(box_inv: mat4x4<f32>, lp: vec3<f32>, pos: vec3<f32>, lookup: ProbeLookup) -> vec3<f32> {
    let ray = (box_inv * vec4<f32>(lookup.dir, 0.0)).xyz;
    let scale = select(1.0, 3.0, lookup.irradiance);
    let first = (vec3<f32>(scale) - lp) / ray;
    let second = (vec3<f32>(-scale) - lp) / ray;
    let far = max(first, second);
    return pos + lookup.dir * min(far.x, min(far.y, far.z));
}

// sphereIntersect: where a lookup leaves a manual sphere probe.
fn probe_sphere_exit(c: vec3<f32>, r: f32, pos: vec3<f32>, dir: vec3<f32>) -> vec3<f32> {
    let l = c - pos;
    let tca = dot(l, dir);
    let d2 = dot(l, l) - tca * tca;
    return pos + dir * (tca + sqrt(max(r * r - d2, 0.0)));
}

// Mix of the automatic (`col0`, weight `w0`) and manual probes of a lookup.
fn probe_mix(col0_in: vec3<f32>, col1_in: vec3<f32>, w0: f32, w1: f32, dw1: f32, sample_automatic: bool) -> vec4<f32> {
    var col0 = col0_in;
    var col1 = col1_in;
    if (sample_automatic && w0 > 0.0) {
        col0 /= w0;
        if (w1 > 0.0) {
            col1 = mix(col0, col1 / w1, min(dw1, 1.0));
            col0 = vec3<f32>(0.0);
        }
    } else if (w1 > 0.0) {
        col1 /= w1;
        col0 = vec3<f32>(0.0);
    }
    return vec4<f32>(col0 + col1, select(0.0, 1.0, w0 + w1 > 0.0));
}

// Two lookups of a fragment (`b` only if `want_b`) in one walk of the
// probes: which probes cover the point, and with what weight, depends on
// the point alone. A PBR fragment asks for its ambient and its reflection,
// a legacy one for its glossy and its environment reflection: each used to
// walk the list for itself, twice (the box probes first, to know whether
// the automatic ones may be sampled). The box probes are found through
// their mask (`probe_set.boxes`).
//
// In the main view the walk is that of the probes which cover the
// fragment's screen column, its screen row and its slice of depth at once
// (`probe_tiles`; `frag` is the fragment's pixel and `pos` where it is in
// the world), in the list's order: a probe left out does not cover `pos`.
// The walk of the list, 32 probes by default, was what the probes cost a
// fragment, far more than the samples of the two or three that cover it.
//
// `automatic`: false for water, which only samples the default probe and
// the manual ones. Same sums, in the same order, as sample_probes_ex, which
// AURORA_PROBE_WALK=0 brings back to compare.
fn sample_probes_pair(pos: vec3<f32>, frag: vec2<f32>, a: ProbeLookup, b: ProbeLookup, want_b: bool, automatic: bool, amblit: vec3<f32>) -> ProbePair {
    var out: ProbePair;
    out.b = vec4<f32>(0.0);
    let walk = frame.light_mode.w;
    if (walk == PROBE_WALK_PER_LOOKUP) {
        out.a = sample_probes_ex(pos, a.dir, a.lod, a.irradiance, amblit, !automatic);
        if (want_b) {
            out.b = sample_probes_ex(pos, b.dir, b.lod, b.irradiance, amblit, !automatic);
        }
        return out;
    }
    // the probes of the fragment's sets or, without the sets, the list
    var held = light_first(min(probe_set.count, 64u));
    if (walk >= PROBE_WALK_TILES) {
        if (walk == PROBE_WALK_CHECK && probe_tiles_miss(pos, frag)) {
            out.a = vec4<f32>(4.0, 0.0, 4.0, 1.0);
            out.b = out.a;
            return out;
        }
        held &= probe_tiles(pos, frag);
    }
    // never let automatic probes encroach on box probes
    var sample_automatic = true;
    var boxes = probe_set.boxes.xy & held;
    for (var n = next_member(&boxes); n >= 0; n = next_member(&boxes)) {
        let lp = (probe_set.probes[n].box_inv * vec4<f32>(pos, 1.0)).xyz;
        if (all(abs(lp) <= vec3<f32>(1.0))) {
            sample_automatic = false;
        }
    }
    var a0 = vec3<f32>(0.0);
    var a1 = vec3<f32>(0.0);
    var b0 = vec3<f32>(0.0);
    var b1 = vec3<f32>(0.0);
    var w0 = 0.0;
    var w1 = 0.0;
    var dw1 = 0.0;
    for (var n = next_member(&held); n >= 0; n = next_member(&held)) {
        let i = u32(n);
        let kind = probe_set.probes[i].params.w;
        let manual = kind != 0.0;
        if (!manual && (!sample_automatic || (!automatic && i != 0u))) {
            continue;
        }
        let sphere = probe_set.probes[i].center_radius;
        let c = sphere.xyz;
        var va: vec3<f32>;
        var vb: vec3<f32>;
        var w: f32;
        var dw = 0.0;
        if (kind < 0.0) {
            // box influence volume with parallax correction
            let box_inv = probe_set.probes[i].box_inv;
            let lp = (box_inv * vec4<f32>(pos, 1.0)).xyz;
            if (any(abs(lp) > vec3<f32>(1.0))) {
                continue;
            }
            va = probe_box_exit(box_inv, lp, pos, a) - c;
            if (want_b) {
                vb = probe_box_exit(box_inv, lp, pos, b) - c;
            }
            w = max(1.0 - max(max(abs(lp.x), abs(lp.y)), abs(lp.z)), 0.001);
        } else {
            let r = sphere.w;
            let delta = pos - c;
            if (i != 0u && dot(delta, delta) > r * r) {
                continue;
            }
            if (kind >= 1.0) {
                // parallax for manual sphere probes only
                va = probe_sphere_exit(c, r, pos, a.dir) - c;
                vb = probe_sphere_exit(c, r, pos, b.dir) - c;
            } else {
                va = a.dir;
                vb = b.dir;
            }
            // sphereWeight
            let r1 = r * 0.5;
            let d = max(length(delta), 0.001);
            let atten = 1.0 - max(d - r1, 0.0) / max(r - r1, 0.001);
            let base = probe_set.probes[i].params.z / d;
            dw = base * atten * max(r, 1.0) * 4.0;
            w = base * atten;
        }
        let ca = probe_tap(i, va, a, amblit) * w;
        var cb = vec3<f32>(0.0);
        if (want_b) {
            cb = probe_tap(i, vb, b, amblit) * w;
        }
        if (manual) {
            a1 += ca;
            b1 += cb;
            w1 += w;
            dw1 += dw;
        } else {
            a0 += ca;
            b0 += cb;
            w0 += w;
        }
    }
    out.a = probe_mix(a0, a1, w0, w1, dw1, sample_automatic);
    if (want_b) {
        out.b = probe_mix(b0, b1, w0, w1, dw1, sample_automatic);
    }
    return out;
}

// Radiance of the environment along `r` for a surface of the given
// perceptual roughness (glossiness = 1 - roughness): the probes when they
// cover `pos`, else `fallback` (sky reflection).
fn probe_radiance(pos: vec3<f32>, frag: vec2<f32>, r: vec3<f32>, roughness: f32, fallback: vec3<f32>) -> vec3<f32> {
    if (!probes_on()) {
        return fallback;
    }
    let s = sample_probes(pos, frag, r, clamp(roughness, 0.0, 1.0) * probe_max_lod());
    return select(fallback, s.rgb, s.w > 0.0);
}

// The largest value of the Rgba16Float scene targets (HDR_FORMAT).
const HDR_MAX: f32 = 65504.0;

// `c` written to a scene target as it would be stored, but finite: a
// highlight brighter than the format holds (a sun specular on a smooth
// metal or mirror) stays as bright as it can instead of turning infinite,
// and a NaN counts as black. One such pixel would otherwise reach the TAA
// history and the bloom chain, which spread it over the whole image (#1380).
fn finite_hdr(c: vec3<f32>) -> vec3<f32> {
    return select(clamp(c, vec3<f32>(-HDR_MAX), vec3<f32>(HDR_MAX)), vec3<f32>(0.0), c != c);
}

/// Reflection passes: drop geometry behind the mirror / below the water.
fn reflection_clip(world_pos: vec3<f32>) -> bool {
    return in_reflection_pass() && dot(frame.clip_plane.xyz, world_pos) + frame.clip_plane.w < 0.0;
}

fn screen_uv(frag: vec2<f32>) -> vec2<f32> {
    return frag * frame.screen.zw;
}

/// World position from a reverse-Z depth value at a screen uv.
fn world_from_depth(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let w = frame.inv_view_proj * vec4<f32>(ndc, max(depth, 1e-7), 1.0);
    return w.xyz / w.w;
}

fn scene_depth_at(uv: vec2<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(depth_tex));
    let px = clamp(vec2<i32>(uv * vec2<f32>(size)), vec2<i32>(0), size - vec2<i32>(1));
    return textureLoad(depth_tex, px, 0);
}

// ------------------------------------------------------- EEP atmospherics

struct Atmos {
    color: vec3<f32>,          // haze color (sky)
    sunlight: vec3<f32>,       // attenuated sunlight
    tmp_ambient: vec3<f32>,
    haze_glow: f32,
    combined_haze: vec3<f32>,
    below: vec3<f32>,          // haze color below the clouds
    light_atten: vec3<f32>,
    rel_z: f32,                // dome point height relative to the camera (+50)
    light_dot: f32,            // cosine to the light (skyV's vary_LightNormPosDot)
};

/// Point of the sky dome seen in direction `d` (camera-relative), as in
/// LLVOWLSky: a sphere cap of `dome_radius` lowered by `dome_offset`.
fn dome_point(d: vec3<f32>) -> vec3<f32> {
    let r = max(frame.sky_misc.y, 100.0);
    let h = frame.sky_misc.x * r;
    let b = h * d.z;
    let t = -b + sqrt(max(b * b - (h * h - r * r), 0.0));
    return d * t;
}

fn atmos_eval(dir: vec3<f32>, for_clouds: bool) -> Atmos {
    let d = normalize(dir);
    var rel = dome_point(d) + vec3<f32>(0.0, 0.0, 50.0);
    var a: Atmos;
    a.rel_z = rel.z;
    let max_y = frame.sky_glow.w;
    if (rel.z > 0.0) {
        rel *= max_y / rel.z;
    }
    if (rel.z < 0.0) {
        rel *= -32000.0 / rel.z;
    }
    let rel_norm = normalize(rel);
    let rel_len = length(rel);
    let lightnorm = frame.sky_light.xyz;
    let blue_density = frame.sky_blue_density.rgb;
    let haze_density = frame.sky_blue_density.w;
    let blue_horizon = frame.sky_blue_horizon.rgb;
    let haze_horizon = frame.sky_blue_horizon.w;
    let density_multiplier = frame.sky_ambient.w;
    let cloud_shadow = frame.sky_sunlight.w;
    let ambient_color = frame.sky_ambient.rgb;
    let glow = frame.sky_glow.xyz;
    let glow_factor = frame.sky_light.w;

    var sunlight = frame.sky_sunlight.rgb;
    let light_atten = (blue_density + vec3<f32>(haze_density * 0.25)) * (density_multiplier * max_y);
    a.light_atten = light_atten;
    var combined_haze = max(abs(blue_density) + vec3<f32>(abs(haze_density)), vec3<f32>(1e-6));
    let blue_weight = blue_density / combined_haze;
    let haze_weight = haze_density / combined_haze;
    let off_axis = 1.0 / max(1e-6, max(0.0, rel_norm.z) + lightnorm.z);
    sunlight *= exp(-light_atten * off_axis);
    let density_dist = rel_len * density_multiplier;
    combined_haze = exp(-combined_haze * density_dist);

    a.light_dot = dot(rel_norm, lightnorm);
    var haze_glow = max(1.0 - a.light_dot, 0.001);
    haze_glow *= glow.x;
    haze_glow = pow(haze_glow, glow.z);
    if (for_clouds) {
        haze_glow *= glow_factor;
        haze_glow = select(haze_glow + 0.25, 0.0, glow_factor < 1.0);
    } else {
        haze_glow = select(glow_factor * (haze_glow + 0.25), 0.0, glow_factor < 1.0);
    }
    a.haze_glow = haze_glow;

    var color = blue_horizon * blue_weight * (sunlight + ambient_color)
              + (haze_horizon * haze_weight) * (sunlight * haze_glow + ambient_color);
    color *= (vec3<f32>(1.0) - combined_haze);

    let ambient = ambient_color + max(vec3<f32>(0.0), vec3<f32>(1.0) - ambient_color) * cloud_shadow * 0.5;
    a.tmp_ambient = ambient;
    sunlight *= max(0.0, 1.0 - cloud_shadow);
    a.sunlight = sunlight;
    let add_below_cloud = blue_horizon * blue_weight * (sunlight + ambient)
                        + (haze_horizon * haze_weight) * (sunlight * haze_glow + ambient);
    a.below = add_below_cloud;
    combined_haze = sqrt(combined_haze);
    a.combined_haze = combined_haze;
    color += (add_below_cloud - color) * (vec3<f32>(1.0) - sqrt(combined_haze));
    a.color = color;
    return a;
}

/// Linear HDR sky radiance for a direction, without clouds (reflections).
fn sky_color(dir: vec3<f32>) -> vec3<f32> {
    let a = atmos_eval(dir, false);
    let c = clamp(a.color * 2.0, vec3<f32>(0.0), vec3<f32>(5.0));
    return srgb_to_linear(c) * frame.sky_cloud_pd2.w;
}

fn cloud_noise(uv: vec2<f32>) -> f32 {
    return sample_tex_level(frame.tex_slots.x, uv, 0.0).x;
}

/// Cloud layer over the sky (cloudsV/F.glsl), in the sky's sRGB-like space.
/// Returns rgb and coverage.
fn sky_clouds(dir: vec3<f32>) -> vec4<f32> {
    let cloud_scale = frame.sky_cloud_color.w;
    if (frame.tex_slots.x == 0u || cloud_scale < 0.001) {
        return vec4<f32>(0.0);
    }
    let d = normalize(dir);
    // planar dome texture coordinates (LLVOWLSky::buildStripsBuffer)
    let r = max(frame.sky_misc.y, 100.0);
    let local = (dome_point(d) + vec3<f32>(0.0, 0.0, frame.sky_misc.x * r)) / r;
    var tc0 = vec2<f32>((1.0 - local.x) * 0.5, (1.0 - local.y) * 0.5);
    tc0 = vec2<f32>(-tc0.x, tc0.y);
    tc0 = (tc0 - vec2<f32>(0.5)) / cloud_scale + vec2<f32>(0.5);
    let ln = frame.sky_light.xyz;
    let tc1 = tc0 + vec2<f32>(ln.y * 0.0125, ln.x * 0.0125);
    let tc2 = tc0 * 16.0;
    let tc3 = tc1 * 16.0;

    let a = atmos_eval(d, true);
    let max_y = frame.sky_glow.w;
    var altitude_blend = clamp((a.rel_z + 512.0) / max_y, 0.0, 1.0);
    if (a.rel_z < 0.0) {
        altitude_blend = 0.0;
    }
    // cloud lighting
    let cloud_color = frame.sky_cloud_color.rgb;
    let off_axis = 1.0 / max(1e-6, ln.z * 2.0);
    let sun_c = frame.sky_sunlight.rgb * exp(-a.light_atten * off_axis);
    let col_sun = (sun_c * a.haze_glow) * cloud_color * a.combined_haze;
    let col_amb = a.tmp_ambient * cloud_color * a.combined_haze + a.below * (vec3<f32>(1.0) - a.combined_haze);
    let cloud_shadow = frame.sky_sunlight.w;
    var density = 2.0 * (cloud_shadow - 0.25);

    let pd1 = frame.sky_cloud_pd1.xyz;
    let pd2 = frame.sky_cloud_pd2.xyz;
    let variance = frame.sky_cloud_pd1.w;
    var uv1 = tc0;
    var uv2 = tc1;
    var uv3 = tc2;
    var uv4 = tc3;
    let dist1 = vec2<f32>(cloud_noise(uv1 / 8.0), cloud_noise((uv3 + uv1) / 16.0)) * variance * (1.0 - cloud_scale * 0.25);
    let dist2 = vec2<f32>(cloud_noise((uv1 + uv3) / 4.0), cloud_noise((uv4 + uv2) / 8.0)) * variance * (1.0 - cloud_scale * 0.25);
    uv1 += pd1.xy + dist1 * 0.2;
    uv2 += pd1.xy;
    uv3 += pd2.xy;
    uv4 += pd2.xy;
    let dv = min(1.0, (dist1.x * 2.0 + dist1.y * 2.0 + dist2.x + dist2.y) * 4.0);
    density *= 1.0 - dv * dv;
    var alpha1 = (cloud_noise(uv1) - 0.5) + (cloud_noise(uv3) - 0.5) * pd2.z;
    alpha1 = min(max(alpha1 + density, 0.0) * 10.0 * pd1.z, 1.0);
    alpha1 = 1.0 - alpha1 * alpha1;
    alpha1 = 1.0 - alpha1 * alpha1;
    alpha1 = clamp(alpha1 * altitude_blend, 0.0, 1.0);
    var alpha2 = cloud_noise(uv2) - 0.5;
    alpha2 = min(max(alpha2 + density, 0.0) * 2.5 * pd1.z, 1.0);
    alpha2 = 1.0 - alpha2;
    alpha2 = 1.0 - alpha2 * alpha2;
    var color = col_sun * (1.0 - alpha2) + col_amb;
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)) * 2.0;
    return vec4<f32>(color, alpha1);
}

// ------------------------------------------------------------- water fog

// The CPU inverts the shared Gerstner swell once per frame. Add the current
// fine wake directly from its GPU field, without a readback or per-pixel swell.
fn camera_water_level() -> f32 {
    var level = frame.cam_up.w;
    let separation = abs(level-frame.camera_pos.z);
    // Only the waterline needs exact wake height. Fade its tiny fog-boundary
    // adjustment outside that band, avoiding field fetches in the deep view.
    if(separation<0.38 && !in_reflection_pass() && water_fx_on(16u)) {
        let native_level = water_fx_bed(frame.camera_pos.xy).y;
        level += clamp(water_wake_sample(frame.camera_pos.xy,native_level).z,-0.16,0.16)
            *(1.0-smoothstep(0.16,0.38,separation));
    }
    return level;
}

// A submerged optical interval. Callers with an actual interface hit pass
// that segment directly; only opaque source shading needs an air-plane clip.
fn legacy_water_fog_segment(delta:vec3<f32>,kd:f32,depth:f32)->vec4<f32> {
    if(kd<=0.0 || length(delta)<0.0001){return vec4<f32>(0.0,0.0,0.0,1.0);}
    let view=normalize(delta);
    let es=max(-view.z,0.0001);
    let e0=max(depth,0.0);
    let l=max(length(delta),0.1);
    let ks=frame.water_params.x;
    let t1=-kd*pow(0.98,ks*e0);
    let t2=kd+ks*es;
    let t3=pow(0.98,t2*l)-1.0;
    let lum=pow(clamp(t1/t2*t3,0.0,1.0),1.0/1.7);
    return vec4<f32>(frame.water_fog.rgb*lum,pow(0.98,l*kd));
}

fn legacy_water_fog_view(pos_rel:vec3<f32>,kd:f32)->vec4<f32> {
    let depth=camera_water_level()-frame.camera_pos.z;
    var end=pos_rel;
    if(depth>0.0 && end.z>depth){end*=depth/max(end.z,0.0001);}
    var entry=vec3<f32>(0.0);
    if(depth<0.0 && length(end)>0.0001){
        let view=normalize(end);entry=view*(-depth)/max(-view.z,0.0001);
    }
    return legacy_water_fog_segment(end-entry,kd,max(depth,0.0));
}

struct WaterFog {
    color: vec3<f32>,
    transmission: vec3<f32>,
};

fn water_fog_segment(delta:vec3<f32>,kd:f32,start_depth:f32,end_depth:f32)->WaterFog {
    if(!water_fx_on(256u)) {
        let legacy=legacy_water_fog_segment(delta,kd,start_depth);
        return WaterFog(legacy.rgb,vec3<f32>(legacy.a));
    }
    let path=length(delta);
    if(kd<=0.0 || path<0.0001){return WaterFog(vec3<f32>(0.0),vec3<f32>(1.0));}
    let tint=frame.water_fog.rgb/max(max(frame.water_fog.r,frame.water_fog.g),max(frame.water_fog.b,0.001));
    let sigma=(vec3<f32>(0.14,0.055,0.032)+(vec3<f32>(1.0)-tint)*0.03)*kd;
    let transmission=exp(-sigma*path);
    let luma=vec3<f32>(0.2126,0.7152,0.0722);
    let illumination=clamp(0.08+dot(frame.sky_zenith.rgb,luma)*0.7
        +dot(frame.sun_color.rgb,luma)*max(frame.sun_dir.z,0.0)*0.14,0.04,1.2);
    let mean_depth=(max(start_depth,0.0)+max(end_depth,0.0))*0.5;
    let radiance=frame.water_fog.rgb*(2.5*illumination*exp(-mean_depth*0.035*kd));
    return WaterFog(radiance*(vec3<f32>(1.0)-transmission),transmission);
}

fn water_fog_view(pos_rel:vec3<f32>,kd:f32)->WaterFog {
    if(!water_fx_on(256u)) {
        let legacy=legacy_water_fog_view(pos_rel,kd);
        return WaterFog(legacy.rgb,vec3<f32>(legacy.a));
    }
    let depth=camera_water_level()-frame.camera_pos.z;
    var start=vec3<f32>(0.0);
    var end=pos_rel;
    if(depth>0.0) {
        if(end.z>depth){end*=depth/max(end.z,0.0001);}
    } else {
        if(end.z>=depth){return WaterFog(vec3<f32>(0.0),vec3<f32>(1.0));}
        start=end*clamp(depth/end.z,0.0,1.0);
    }
    // Attenuation follows metres of water, even with the eye a millimetre
    // above/below the surface. An immersed endpoint still has a water path
    // when a wave/mesh approximation classifies the eye just outside it.
    return water_fog_segment(end-start,kd,max(depth-start.z,0.0),max(depth-end.z,0.0));
}

fn camera_underwater() -> bool {
    if(in_reflection_pass()) { return false; }
    let depth = frame.cam_up.w-frame.camera_pos.z;
    // Outside the bounded wake height the side is already known. Ordinary
    // above-water materials need no additional texture fetch for this test.
    if(abs(depth)>0.16 || !water_fx_on(16u)) { return depth>0.0; }
    return frame.camera_pos.z < camera_water_level();
}

// LLDrawPoolWaterExclusion: the nearest front-facing exclusion surface
// hides water only when it is in front of the nearest water interface.
fn water_excluded(world_pos:vec3<f32>) -> bool {
    let clip=frame.view_proj*vec4<f32>(world_pos,1.0);
    if(clip.w<=0.0){return false;}
    let uv=clip.xy/clip.w*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5);
    let size=vec2<i32>(textureDimensions(water_exclusion_mask));
    let pixel=clamp(vec2<i32>(uv*vec2<f32>(size)),vec2<i32>(0),size-vec2<i32>(1));
    return textureLoad(water_exclusion_mask,pixel,0).r<0.5;
}

fn water_scene_light(world_pos:vec3<f32>) -> vec3<f32> {
    if(!water_fx_on(256u)) { return vec3<f32>(1.0); }
    if(!camera_underwater() && water_excluded(world_pos)){return vec3<f32>(1.0);}
    // Attenuate environmental lighting, leaving emissive/fullbright sources
    // to lose their wavelengths along the view path only.
    let depth = max(frame.params.y-world_pos.z,0.0);
    return exp(-vec3<f32>(0.06,0.025,0.012)*depth*frame.water_normal.w);
}

fn apply_air_fog(col:vec3<f32>,world_pos:vec3<f32>)->vec3<f32> {
    if (frame.sky_ll.x < 0.5) {
        // a sky with a reflection probe ambiance: hazeF.glsl, as its surfaces
        // are lit by softenLightF's non-classic branch (env.rs)
        return ll_haze(col, ll_atmos(world_pos - frame.camera_pos.xyz));
    }
    let dist = distance(world_pos, frame.camera_pos.xyz);
    let height_factor = exp(-max(world_pos.z - frame.params.y, 0.0) * 0.004);
    let f = 1.0 - exp(-dist * frame.fog.w * height_factor);
    return mix(col, frame.fog.rgb, clamp(f, 0.0, 1.0));
}

fn apply_fog(col: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    if(camera_underwater() || (water_fx_on(256u) && world_pos.z<camera_water_level())) {
        if(!camera_underwater() && water_excluded(world_pos)){return apply_air_fog(col,world_pos);}
        let wf=water_fog_view(world_pos-frame.camera_pos.xyz,frame.water_normal.w);
        return col*wf.transmission+wf.color;
    }
    return apply_air_fog(col,world_pos);
}

// ------------------------------------------------------------- shadows

// Half-width of the shadow filter in metres, the same in every cascade:
// with a filter sized in texels, each cascade (texels from ~6 mm to ~7 cm)
// had its own softness and the cascade borders showed as steps. Firestorm
// softens its shadows in screen space (RenderShadowBlurSize).
const SHADOW_PENUMBRA: f32 = 0.03;

// frame.light_mode.z: the shadow filter of before, with its gaps
// (AURORA_SHADOW_FILTER=gaps, to compare).
const SHADOW_FILTER_GAPS: u32 = 0x100u;
// frame.light_mode.z: the shadow lookup of an object offset along its
// normal-mapped normal, as before (AURORA_SHADOW_FILTER=mapped; object.wgsl
// shadow_normal).
const SHADOW_OFFSET_MAPPED: u32 = 0x200u;
// frame.light_mode.z: il_tex holds GTAO's indirect light of this frame
const SCREEN_INDIRECT: u32 = 0x400u;
// frame.light_mode.z: normal maps on a unit, orthogonal tangent frame as the
// Legacy Viewer's per-vertex tangents (AURORA_TANGENTS, perturb_normal)
const TANGENT_UNIT: u32 = 0x800u;
// frame.light_mode.z: normal maps on the tangents of the vertices where
// they have some (object.wgsl vertex_tangent; AURORA_TANGENTS)
const TANGENT_VERTEX: u32 = 0x1000u;
// frame.light_mode.z: the receiver bias of before, a fixed 0.0005 of the
// cascade's depth range (AURORA_SHADOW_BIAS=legacy; cascade_shadow)
const SHADOW_BIAS_LEGACY: u32 = 0x2000u;
// frame.light_mode.z: blended faces take the screen-space occlusion and
// indirect light of what lies behind them, as before (AURORA_BLEND_AO=old;
// screen_ao)
const BLEND_SCREEN_TERMS: u32 = 0x4000u;
// frame.light_mode.z: the occlusion's depth holds the blended faces at least
// half covered (renderer.rs, "occlusion depth"), which then take the
// screen-space terms (screen_surface; AURORA_BLEND_AO)
const BLEND_SCREEN_DEPTH: u32 = 0x10000u;

// frame.light_mode.z: geometric specular anti-aliasing of the objects
// (AURORA_SPECULAR_AA, specular_aa_roughness)
const SPECULAR_AA: u32 = 0x8000u;

// Geometric specular anti-aliasing (Kaplanyan & Hoffman 2016, "Stable
// Geometric Specular Antialiasing with Projected-Space NDF Filtering", in
// the form Filament gives it, normalFiltering): where the shading normal
// turns faster than a pixel, the highlight is widened so it no longer
// shimmers from one frame to the next. The kernel adds to the GGX lobe's
// alpha², capped, so a large smooth surface keeps its highlight. Only the
// change a pixel cannot resolve counts: below SPECULAR_AA_FLOOR (a normal
// turning about 0.06 rad per pixel: an object some 35 pixels across, or a
// normal map's relief seen from afar) nothing changes, so a polished ball
// up close keeps its pinpoint highlights. The
// variance a normal map's coarser mips lose (their averaged normals are
// shorter than 1: Toksvig) joins it; a normal a few percent short at its
// finest level is the map's own quantization and is left out.
const SPECULAR_AA_VARIANCE: f32 = 0.15;
const SPECULAR_AA_THRESHOLD: f32 = 0.2;
const SPECULAR_AA_FLOOR: f32 = 0.001;
const SPECULAR_AA_TOKSVIG_FROM: f32 = 0.985;

// The screen-space variance of the shading normal, read where control flow
// is still uniform (the derivatives need it).
fn normal_variance(n: vec3<f32>) -> f32 {
    let du = dpdx(n);
    let dv = dpdy(n);
    return SPECULAR_AA_VARIANCE * (dot(du, du) + dot(dv, dv));
}

// The variance a normal map sample of length `map_len` stands for.
fn toksvig_variance(map_len: f32) -> f32 {
    let len = clamp(map_len, 1e-3, 1.0);
    return max(SPECULAR_AA_TOKSVIG_FROM - len, 0.0) / len;
}

// Perceptual roughness widened by the normal's variance.
fn specular_aa_roughness(perceptual: f32, variance: f32) -> f32 {
    let kernel = min(2.0 * max(variance - SPECULAR_AA_FLOOR, 0.0), SPECULAR_AA_THRESHOLD);
    let a = perceptual * perceptual;
    return sqrt(sqrt(clamp(a * a + kernel, 0.0, 1.0)));
}

// The same for a legacy material's glossiness (its Blinn-Phong exponent is
// gloss² × 368, ll_light_func): through the GGX alpha of the same lobe
// width, alpha = sqrt(2 / (e + 2)), and back. Without variance the
// glossiness comes back unchanged.
fn specular_aa_gloss(gloss: f32, variance: f32) -> f32 {
    let e = gloss * gloss * 368.0;
    let alpha = sqrt(2.0 / (e + 2.0));
    let p = specular_aa_roughness(sqrt(alpha), variance);
    let a2 = max(p * p, 1e-4);
    let e2 = max(2.0 / (a2 * a2) - 2.0, 0.0);
    return clamp(sqrt(e2 / 368.0), 0.0, 1.0);
}

// Taps per axis for a filter spanning `span` (tile coordinates), so that
// two taps are never more than a texel apart: 4 when they already are not,
// up to 7 (a span of six texels, the most `cascade_shadow` gives).
// Mirrored by `shadow_cull::shadow_taps`, which the tests check.
fn shadow_taps(span: f32, texel: f32) -> u32 {
    return clamp(u32(ceil(span / texel - 1e-3)) + 1u, 4u, 7u);
}

// Shadow of one cascade (1 lit, 0 shadowed); -1 when the point is outside
// its tile, so that a blend never takes light from a cascade that does not
// see the point.
fn cascade_shadow(world_pos: vec3<f32>, n: vec3<f32>, cascade: u32) -> f32 {
    let bias_n = n * (0.02 + 0.04 * f32(cascade));
    let m = frame.cascade_vp[cascade];
    let lp = m * vec4<f32>(world_pos + bias_n, 1.0);
    let ndc = lp.xyz / lp.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0) {
        return -1.0;
    }
    // texel of a tile, in tile coordinates
    let texel = 2.0 / f32(textureDimensions(shadow_map).x);
    // half extent of the cascade in metres: orthographic, so the first row
    // of its matrix is the light's right axis divided by it
    let radius = 1.0 / max(length(vec3<f32>(m[0].x, m[1].x, m[2].x)), 1e-6);
    let half_uv = SHADOW_PENUMBRA * 0.5 / radius;
    // The penumbra spans three spacings of 4 taps, a spacing kept between
    // half a texel and two texels. A bilinear compare tap only blends over
    // one texel: taps more than a texel apart leave flat steps between their
    // ramps, which show as parallel lines along every shadow edge (in the
    // finest cascade, where the penumbra covers four to six texels). There
    // the same span takes as many taps as it covers texels, at most 7 x 7,
    // so that the ramps meet: the same penumbra, without the lines.
    let spacing = clamp(half_uv * (2.0 / 3.0), 0.5 * texel, 2.0 * texel);
    let span = 3.0 * spacing;
    var taps = shadow_taps(span, texel);
    if ((frame.light_mode.z & SHADOW_FILTER_GAPS) != 0u) {
        taps = 4u;
    }
    let step = span / f32(taps - 1u);
    let center = 0.5 * f32(taps - 1u);
    let tile = vec2<f32>(f32(cascade % 2u), f32(cascade / 2u));
    // Receiver bias in metres along the light: two texels of the cascade.
    // The depth range of a cascade is its square plus CASCADE_DEPTH_PAD on
    // each side (about 810 m for the finest), so a fixed 0.0005 of it was
    // about 0.4 m: a point only fell in shadow behind a caster more than
    // 0.4 m nearer the sun, and where that distance crossed 0.4 m (a curved
    // back under the shoulders, a skirt under the waist) the limit followed
    // the stored depth texel by texel, as sharp stairs that no filter
    // softens. The normal offset above and the slope-scaled bias of the
    // shadow pipelines keep the acne away. The Legacy Viewer biases by a
    // fixed 0.004 of a shadow range fitted to the scene (RenderShadowBias,
    // shadowUtil.glsl pcfShadow). (frame.light_mode.z SHADOW_BIAS_LEGACY:
    // the bias of before, AURORA_SHADOW_BIAS.)
    let depth_per_metre = length(vec3<f32>(m[0].z, m[1].z, m[2].z));
    var receiver_bias = 2.0 * texel * radius * 2.0 * depth_per_metre;
    if ((frame.light_mode.z & SHADOW_BIAS_LEGACY) != 0u) {
        receiver_bias = 0.0005;
    }
    var sum = 0.0;
    for (var y = 0u; y < taps; y++) {
        for (var x = 0u; x < taps; x++) {
            let o = (vec2<f32>(f32(x), f32(y)) - center) * step;
            // clamped to the centers of the tile's edge texels: the same
            // result as the clamp-to-edge of a separate texture, never a
            // texel of the neighbouring tile
            let t = clamp(uv + o, vec2<f32>(0.5 * texel), vec2<f32>(1.0 - 0.5 * texel));
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, (t + tile) * 0.5, ndc.z - receiver_bias);
        }
    }
    return sum / f32(taps * taps);
}

// The motion of a pixel on the screen from its clip positions, this frame's
// and the last frame's: in uv units (y down), what to take from its uv to
// find where the last frame drew it. None for a point that was behind the
// camera.
fn screen_motion(cur: vec4<f32>, prev: vec4<f32>) -> vec2<f32> {
    if (prev.w <= 1e-6 || cur.w <= 1e-6) {
        return vec2<f32>(0.0);
    }
    return (cur.xy / cur.w - prev.xy / prev.w) * vec2<f32>(0.5, -0.5);
}

// The view shown instead of the image (frame.light_mode.z, Preferences ›
// Debug), or `col`.
// 1 (also AURORA_DEBUG_SHADOWS): the cascade each pixel reads — red, green,
// blue from the finest, mixed where two are blended, black beyond the
// shadow distance — darkened where the pixel is in shadow. The choice
// mirrors shadow_factor.
// 2: the ambient occlusion of the screen alone, light grey where nothing is
// occluded (and everywhere when the effect is off), whatever the exposure
// of the sky; cubed, the occlusion being too faint to read as it is.
fn debug_view(col: vec3<f32>, world_pos: vec3<f32>, n: vec3<f32>, vertex_depth: f32, frag: vec2<f32>) -> vec3<f32> {
    let view = frame.light_mode.z & 0xffu;
    if (view == 0u) {
        return col;
    }
    if (view == 2u) {
        let ao = screen_ao(frag);
        return vec3<f32>(ao * ao * ao * 0.8 / max(frame.params.x, 1e-3));
    }
    let splits = frame.cascade_splits;
    var own_depth = vertex_depth;
    if (splits.w > 0.5) {
        own_depth = distance(world_pos, frame.camera_pos.xyz);
    }
    var w = vec3<f32>(0.0);
    if (own_depth <= splits.z) {
        var k = 0u;
        if (own_depth > splits.x) { k = 1u; }
        if (own_depth > splits.y) { k = 2u; }
        w[k] = 1.0;
        if (k < 2u) {
            let start = select(0.0, splits.x, k == 1u);
            let end = select(splits.x, splits.y, k == 1u);
            let band = (end - start) * 0.15;
            let t = clamp((own_depth - (end - band)) / max(band, 1e-4), 0.0, 1.0);
            w[k] = 1.0 - t;
            w[k + 1u] = t;
        }
    }
    return w * (0.12 + 0.88 * sun_shadow(world_pos, n, vertex_depth, frag));
}

// Whether the fragment being shaded is the surface of the depth prepass,
// whose contact shadows the mask holds: not a blended face.
var<private> contact_surface: bool = true;

// The sun's contact shadow at a pixel of the main view (contact_shadows.rs):
// what the shadow map is too coarse to show, taken as the darker of the
// two (sun_shadow). 1 elsewhere: other views, blended faces, off.
fn contact_shadow(frag: vec2<f32>) -> f32 {
    if (frame.contact.x < 0.5 || !contact_surface || in_reflection_pass()) {
        return 1.0;
    }
    let p = vec2<i32>(frag);
    let size = vec2<i32>(textureDimensions(contact_tex));
    if (any(p < vec2<i32>(0)) || any(p >= size)) {
        return 1.0;
    }
    return 1.0 - textureLoad(contact_tex, p, 0).r;
}

// The sun's shadow of an opaque surface of the main view: the shadow map,
// and the contact shadows where they are on.
fn sun_shadow(world_pos: vec3<f32>, n: vec3<f32>, vertex_depth: f32, frag: vec2<f32>) -> f32 {
    let s = shadow_factor(world_pos, n, vertex_depth);
    if (frame.params.z < 0.5) {
        return s;
    }
    return min(s, contact_shadow(frag));
}

fn shadow_factor(world_pos: vec3<f32>, n: vec3<f32>, vertex_depth: f32) -> f32 {
    if (frame.params.z < 0.5) {
        return 1.0;
    }
    let splits = frame.cascade_splits;
    // The cascade comes from the pixel's own distance to the camera, as for
    // the terrain and the water (and as Firestorm, which reads each pixel's
    // position: sampleDirectionalShadow, deferred/shadowUtil.glsl). The
    // distance interpolated from the vertices is longer inside a triangle
    // that spans a range of distances: on a large floor, the pixels near
    // the camera read a far cascade, and no caster could be left out of a
    // far cascade for being near (shadow_cull.rs). splits.w == 0 keeps that
    // distance, to compare (AURORA_SHADOW_LEGACY).
    var view_depth = vertex_depth;
    if (splits.w > 0.5) {
        view_depth = distance(world_pos, frame.camera_pos.xyz);
    }
    var cascade = 0u;
    if (view_depth > splits.x) { cascade = 1u; }
    if (view_depth > splits.y) { cascade = 2u; }
    if (view_depth > splits.z) {
        return 1.0;
    }
    var s = cascade_shadow(world_pos, n, cascade);
    // blend into the next cascade over the last 15 % of this one: no visible
    // border between cascades
    if (cascade < 2u) {
        let start = select(0.0, splits.x, cascade == 1u);
        let end = select(splits.x, splits.y, cascade == 1u);
        let band = (end - start) * 0.15;
        let t = clamp((view_depth - (end - band)) / max(band, 1e-4), 0.0, 1.0);
        if (t > 0.0) {
            let next = cascade_shadow(world_pos, n, cascade + 1u);
            if (s < 0.0) {
                s = next;
            } else if (next >= 0.0) {
                s = mix(s, next, t);
            }
        }
    }
    if (s < 0.0) {
        return 1.0;
    }
    // fade out at the far end of the last cascade
    let fade = clamp((splits.z - view_depth) / (splits.z * 0.1), 0.0, 1.0);
    return mix(1.0, s, fade);
}

// ---------------------------------------------------------------- BRDF

fn d_ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d + 1e-6);
}

fn v_smith(nv: f32, nl: f32, a: f32) -> f32 {
    let k = a * 0.5;
    let gv = nv / (nv * (1.0 - k) + k);
    let gl = nl / (nl * (1.0 - k) + k);
    return gv * gl / max(4.0 * nv * nl, 1e-4);
}

fn f_schlick(f0: vec3<f32>, vh: f32) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vh, 5.0);
}

// Karis' analytic approximation of the split-sum environment BRDF.
fn env_brdf(f0: vec3<f32>, roughness: f32, nv: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

// Occlusion the normal map of the fragment being shaded adds to the
// screen-space one (normal_map_occlusion); none outside the object shader.
var<private> surface_occlusion: f32 = 0.0;

// False while a translucent blended face is shaded: the screen-space
// occlusion and indirect light come from the occlusion's depth, so on such
// a face they are those of what lies behind it, around a closer object a
// dark halo (#556). The Legacy Viewer's alpha pass applies neither
// (materialF.glsl, alphaF.glsl: no adjustIrradiance outside softenLightF).
// A blended face at least half covered is in that depth (BLEND_SCREEN_DEPTH)
// and takes them like an opaque one: a head whose halves differ in alpha
// mode no longer shows a line down the middle (#882).
var<private> screen_surface: bool = true;

// Whether the fragment being shaded takes the screen-space terms (see
// screen_surface).
fn screen_terms() -> bool {
    return screen_surface || (frame.light_mode.z & BLEND_SCREEN_TERMS) != 0u;
}

fn screen_ao(frag: vec2<f32>) -> f32 {
    if (frame.misc.x < 0.5 || in_reflection_pass() || !screen_terms()) {
        return 1.0;
    }
    return max(textureSampleLevel(ao_tex, lin_clamp, screen_uv(frag), 0.0).r - surface_occlusion, 0.0);
}

// The light the surfaces around bounce onto the fragment at `frag`
// (irradiance over pi, in the units of the lit colour: a Lambert surface
// of albedo `c` sends back `c` times it), from GTAO's visibility bitmask
// pass; none without it, in reflection passes and on the HUD. It is
// weighted by GTAO's occlusion like the rest of the ambient light: the
// bounce is gathered as if the surfaces it comes from were open, while in
// a crease the light reaching them and coming back is occluded too; left
// as is, it lit the creases and contacts back up and the occlusion seemed
// gone.
fn screen_indirect(frag: vec2<f32>) -> vec3<f32> {
    if ((frame.light_mode.z & SCREEN_INDIRECT) == 0u || in_reflection_pass() || !screen_terms()) {
        return vec3<f32>(0.0);
    }
    return textureSampleLevel(il_tex, lin_clamp, screen_uv(frag), 0.0).rgb * screen_ao(frag);
}

// The solid angle two spherical caps share, divided by 2 pi: caps of
// cosines `cos_a` and `cos_b` whose axes are `cos_d` apart. Oat and Sander,
// "Ambient Aperture Lighting" (2007), with the smooth approximation of the
// partial overlap Filament uses (sphericalCapsIntersection).
fn spherical_caps_intersection(cos_a: f32, cos_b: f32, cos_d: f32) -> f32 {
    let ra = acos(clamp(cos_a, -1.0, 1.0));
    let rb = acos(clamp(cos_b, -1.0, 1.0));
    let d = acos(clamp(cos_d, -1.0, 1.0));
    if (min(ra, rb) <= max(ra, rb) - d) {
        // one cap inside the other
        return 1.0 - max(cos_a, cos_b);
    }
    if (ra + rb <= d) {
        return 0.0;
    }
    let delta = abs(ra - rb);
    let x = 1.0 - clamp((d - delta) / max(ra + rb - delta, 1e-4), 0.0, 1.0);
    return x * x * (3.0 - 2.0 * x) * (1.0 - max(cos_a, cos_b));
}

// How much of a reflection the surface's surroundings let through: the
// visibility cone around the bent normal (its aperture from the visibility,
// cos = sqrt(1 - V)) against the specular cone around the reflected ray
// (from the roughness), as Jimenez et al., "Practical Real-Time Strategies
// for Accurate Indirect Occlusion" (2016), section 6. Below a roughness of
// about 0.3 the occlusion fades out as in Filament (specularAO_Cones): a
// mirror's narrow lobe would otherwise lose every reflection to the
// screen-space visibility's noise.
// Measured against the open hemisphere (visibility 1) around the same bent
// normal: the part of a lobe below the horizon is already the probes' and
// the environment BRDF's business, and counting it here darkened the rims
// of convex shapes, whose grazing reflections nothing hides (Aurora's
// choice; an open surface keeps exactly its reflection). And a reflection
// is never darkened more than the ambient light (the visibility): what
// hides the sky is itself lit, which the cones take for black — a polished
// metal turned black in an inside corner, with the bent normal's noise.
// And the lobe is never narrower than about 26 degrees here: a narrow lobe
// swings from open to hidden with the smallest turn of the bent normal,
// which drew soft patches and waves over an inside corner of half-polished
// metal; the bent normal and the cone are too coarse for a finer test.
fn cone_specular_occlusion(bent: vec3<f32>, visibility: f32, r: vec3<f32>, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let cos_v = sqrt(max(1.0 - visibility, 0.0));
    // the lobe's aperture: log2(10) x alpha^2, at least acos 0.9
    let cos_s = min(exp2(-3.321928 * a * a), 0.9);
    let cos_b = dot(bent, r);
    let open = spherical_caps_intersection(0.0, cos_s, cos_b);
    if (open <= 1e-6) {
        return 1.0;
    }
    let so = spherical_caps_intersection(cos_v, cos_s, cos_b) / open;
    return mix(1.0, max(clamp(so, 0.0, 1.0), visibility), smoothstep(0.01, 0.09, a));
}

// The specular occlusion of the fragment at `frag` for the reflected ray
// `r` (world) and the perceptual `roughness`, from GTAO's bent normal
// (frame.sky_ll.w), or `otherwise` (what the reflection took before).
fn specular_occlusion(frag: vec2<f32>, r: vec3<f32>, roughness: f32, otherwise: f32) -> f32 {
    if (frame.sky_ll.w < 0.5 || frame.misc.x < 1.5 || in_reflection_pass()) {
        return otherwise;
    }
    // GTAO's result: .g the visibility blurred for this, .ba the bent
    // normal's x and y in view space (x right, y up, z in front), facing
    // the camera
    let s = textureSampleLevel(ao_tex, lin_clamp, screen_uv(frag), 0.0);
    var xy = s.ba * 2.0 - vec2<f32>(1.0);
    let l = length(xy);
    if (l > 1.0) {
        xy /= l;
    }
    let b = vec3<f32>(xy, -sqrt(max(1.0 - dot(xy, xy), 0.0)));
    let forward = cross(frame.cam_up.xyz, frame.cam_right.xyz);
    let bent = normalize(frame.cam_right.xyz * b.x + frame.cam_up.xyz * b.y + forward * b.z);
    return cone_specular_occlusion(bent, s.g, r, roughness);
}

/// The planar mirror (SL hero probe) seen along the reflected ray `r` from
/// `world_pos`: .rgb its image, .a how much the surface takes it. Weight as
/// tapHeroProbe (reflectionProbeF.glsl): any surface inside the probe's box,
/// fading over the outer tenth, in front of the mirror plane (from 5 cm
/// behind it) and glossy enough (`gloss` 0.75 to 1), whatever its normal.
/// LL reads a cube map taken from the camera mirrored by the plane along
/// `r`; the planar image is that same view, so `r` is projected through
/// the mirror pass's camera: exact on the plane, and a tilted or normal
/// mapped face looks up the same direction the cube map would.
fn mirror_tap(world_pos: vec3<f32>, r: vec3<f32>, gloss: f32) -> vec4<f32> {
    if (dot(frame.mirror_plane.xyz, frame.mirror_plane.xyz) < 0.5 || in_reflection_pass()) {
        return vec4<f32>(0.0);
    }
    let lp = (frame.mirror_box * vec4<f32>(world_pos, 1.0)).xyz;
    let inside = max(1.0 - 2.0 * max(max(abs(lp.x), abs(lp.y)), abs(lp.z)), 0.0);
    let clip = clamp((dot(frame.mirror_plane.xyz, world_pos) + frame.mirror_plane.w) * 9.5 + 0.5, 0.0, 1.0);
    var w = clamp(inside * 10.0 * clip, 0.0, 1.0) * clamp((gloss - 0.75) * 4.0, 0.0, 1.0);
    let c = frame.mirror_view_proj * vec4<f32>(r, 0.0);
    if (w <= 0.0 || c.w <= 1e-5) {
        return vec4<f32>(0.0);
    }
    let ndc = c.xy / c.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    // the image covers the screen's frustum only, where the cube map is
    // whole: rays leaving it fade back to the probes
    let out = max(max(-uv.x, uv.x - 1.0), max(-uv.y, uv.y - 1.0));
    w *= clamp(1.0 - out * 20.0, 0.0, 1.0);
    return vec4<f32>(textureSampleLevel(mirror_tex, lin_clamp, uv, 0.0).rgb, w);
}

// `mat_ao`: the material's own occlusion (ORM red), which pbrIbl applies to
// the image based diffuse and specular alike (deferredUtil.glsl).
// `legacy`: a face without a PBR material, lit as softenLightF's legacy
// branch of a sky with a probe ambiance: the sunlight (frame.sun_color / 3,
// env.rs) times n.l on its albedo, no specular nor reflection of its own
// (legacy_light adds those of its shininess or material)
fn shade_pbr(base: vec3<f32>, metallic: f32, roughness_in: f32, mat_ao: f32, n: vec3<f32>, v: vec3<f32>,
             world_pos: vec3<f32>, shadow: f32, frag: vec2<f32>, legacy: bool) -> vec3<f32> {
    let roughness = clamp(roughness_in, 0.04, 1.0);
    let a = roughness * roughness;
    let f0 = mix(vec3<f32>(0.04), base, metallic);
    let diffuse_color = base * (1.0 - metallic);
    let nv = max(dot(n, v), 1e-4);
    let ao = screen_ao(frag);

    var col = vec3<f32>(0.0);
    // sun / moon
    let l = normalize(frame.sun_dir.xyz);
    let nl = max(dot(n, l), 0.0);
    if (legacy) {
        col += diffuse_color * frame.sun_color.rgb * (nl * shadow / 3.0);
    } else if (nl > 0.0) {
        let h = normalize(l + v);
        let nh = max(dot(n, h), 0.0);
        let vh = max(dot(v, h), 0.0);
        let f = f_schlick(f0, vh);
        let spec = d_ggx(nh, a) * v_smith(nv, nl, a) * f;
        let kd = (vec3<f32>(1.0) - f) * diffuse_color / PI;
        col += (kd + spec) * frame.sun_color.rgb * nl * shadow;
    }
    // local lights
    var lights = light_walk(world_pos, frag);
    var spots = projector_walk(lights);
    for (var i = next_light(&lights); i >= 0; i = next_light(&lights)) {
        let lt = frame.lights[i];
        let to_l = lt.pos_radius.xyz - world_pos;
        let dist = length(to_l);
        let radius = lt.pos_radius.w;
        if (dist < radius) {
            let ll = to_l / max(dist, 1e-4);
            let lnl = max(dot(n, ll), 0.0);
            if (lnl > 0.0) {
                let atten = ll_light_atten(dist / radius, lt.color_falloff.w);
                let h = normalize(ll + v);
                let nh = max(dot(n, h), 0.0);
                let vh = max(dot(v, h), 0.0);
                let f = f_schlick(f0, vh);
                let spec = d_ggx(nh, a) * v_smith(nv, lnl, a) * f;
                let kd = (vec3<f32>(1.0) - f) * diffuse_color / PI;
                col += (kd + spec) * lt.color_falloff.rgb * lnl * atten;
            }
        }
    }
    for (var i = next_light(&spots); i >= 0; i = next_light(&spots)) {
        col += pbr_local_light(projector_at(i, world_pos, n), n, v, f0, a, nv, diffuse_color);
    }
    // ambient: hemisphere diffuse + sky reflection
    let up = n.z * 0.5 + 0.5;
    let hemi = mix(mix(frame.ground_color.rgb, frame.sky_horizon.rgb, clamp(up * 2.0, 0.0, 1.0)),
                   frame.sky_zenith.rgb, clamp(up * 2.0 - 1.0, 0.0, 1.0));
    var amb = hemi;
    let r = reflect(-v, n);
    var env = mix(sky_color(vec3<f32>(r.x, r.y, max(r.z, -0.2))), hemi, roughness);
    // with GTAO, the occlusion of the reflected ray instead of the ambient one
    let spec_ao = specular_occlusion(frag, r, roughness, ao);
    var occl = (0.35 + 0.65 * shadow) * (1.0 - 0.75 * roughness) * spec_ao;
    if (probes_on()) {
        // non classic skies: ambient from the probes' irradiance along the
        // normal, each probe mixed with the sky ambient by its ambiance
        // (sampleProbeAmbient), and pbrIbl radiance from the probes (ambient
        // occlusion only), in one walk of the probes
        let s = sample_probes_pair(world_pos, frag, ProbeLookup(n, 0.0, true), ProbeLookup(r, roughness * probe_max_lod(), false), !legacy, true, hemi);
        if (s.a.w > 0.0) {
            amb = s.a.rgb;
        }
        if (s.b.w > 0.0) {
            env = s.b.rgb;
            occl = spec_ao;
        }
    }
    col += diffuse_color * amb * ao * mat_ao;
    col += diffuse_color * screen_indirect(frag) * mat_ao;
    if (legacy) {
        return col;
    }
    let m = mirror_tap(world_pos, r, 1.0 - clamp(roughness_in, 0.0, 1.0));
    if (m.a > 0.0) {
        env = mix(env, m.rgb, m.a);
        occl = mix(occl, 1.0, m.a);
    }
    col += env * env_brdf(f0, roughness, nv) * occl * mat_ao;
    return col;
}

// Normal mapping without precomputed tangents (Mikkelsen 2010 / Schüler).
// The cotangent frame is the solution divided by det = dot(dp1, dp2perp).
// Schüler drops it because it is positive in GL, where dFdy runs up the
// window; here dpdy runs down the framebuffer, det is negative on every face
// and without its sign both axes come out reversed (bumps lit as dents).
// `uv` is the sampling coordinate of the top-left-origin texture, so the
// derived bitangent points down the image; SL normal maps (materialF.glsl
// getNormal: B = sign * cross(N, T) along +t, GL bottom-left origin) put +Y
// up the image, hence the green channel is negated.
// With the vertices' tangents (`vt`, zero without them, and their
// handedness `vt_sign`), the frame is theirs, smooth across the triangles.
fn perturb_normal(n: vec3<f32>, p: vec3<f32>, uv: vec2<f32>, vt: vec3<f32>, vt_sign: f32, front: bool, map: vec3<f32>) -> vec3<f32> {
    let dp1 = dpdx(p);
    let dp2 = dpdy(p);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    if (dot(vt, vt) > 1e-12) {
        // (after the derivatives, which need uniform control flow)
        // materialF.glsl / pbropaqueF.glsl getNormal: B = sign * cross(N, T)
        // and T left as interpolated (MikkTSpace's pixel shader), the map
        // read as it is (the tangents follow the volume's own s and t, +t up
        // the image), a back face's normal turned once perturbed
        let f = select(-1.0, 1.0, front);
        let vn = n * f;
        let mv = map * 2.0 - vec3<f32>(1.0);
        let rv = normalize(mv.x * vt + mv.y * (vt_sign * cross(vn, vt)) + mv.z * vn) * f;
        return select(n, rv, dot(rv, rv) > 0.5);
    }
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let invmax = sign(dot(dp1, dp2perp)) * inverseSqrt(max(dot(t, t), dot(b, b)) + 1e-12);
    var tbn = mat3x3<f32>(t * invmax, b * invmax, n);
    if ((frame.light_mode.z & TANGENT_UNIT) != 0u) {
        // The Legacy Viewer's frame (LLVolumeFace::createTangents, read by
        // materialF.glsl getNormal): T along dP/du made orthogonal to N and of
        // unit length, B = cross(N, T) times the handedness, whatever the
        // stretch of the coordinates. The cotangent frame above keeps the
        // ratio of the two gradients, so the axis along which the texture is
        // denser had its relief divided by that density ratio (a cloth whose
        // UVs are three times denser one way showed a third of its weave).
        let det_uv = duv1.x * duv2.y - duv2.x * duv1.y;
        if (abs(det_uv) > 1e-20) {
            let dpdu = (dp1 * duv2.y - dp2 * duv1.y) / det_uv;
            let dpdv = (dp2 * duv1.x - dp1 * duv2.x) / det_uv;
            let tp = dpdu - n * dot(n, dpdu);
            if (dot(tp, tp) > 1e-20) {
                let tu = normalize(tp);
                let hand = select(-1.0, 1.0, dot(cross(n, tu), dpdv) >= 0.0);
                tbn = mat3x3<f32>(tu, cross(n, tu) * hand, n);
            }
        }
    }
    let m = (map * 2.0 - vec3<f32>(1.0)) * vec3<f32>(1.0, -1.0, 1.0);
    let res = normalize(tbn * m);
    return select(n, res, dot(res, res) > 0.5);
}

// What calcAmbientOcclusion (aoUtil.glsl) hides on a normal-mapped surface.
// Firestorm runs it on the G-buffer normal: where the normal map tilts the
// normal, the taps on the side it leans to stand above its tangent plane
// (by more than LL's 5 cm) and occlude the pixel, so the relief of a normal
// map is darkened wherever it turns away from the surface. Our pass
// (ssao.wgsl) runs before shading, on the depth alone, and misses it; this
// is that part, for a locally flat surface around the pixel. `n` is the
// normal before the normal map, `nm` after it. LL turns its kernel at random
// (noise map) then blurs: each tap is above the plane over a known share of
// the turn, so the mean stands for both.
fn normal_map_occlusion(n: vec3<f32>, nm: vec3<f32>, p: vec3<f32>, view_z: f32) -> f32 {
    // (misc.x 2: GTAO, whose occlusion is not that kernel's)
    if (frame.misc.x < 0.5 || frame.misc.x > 1.5 || in_reflection_pass()) {
        return 0.0;
    }
    // surface offset of one pixel right and of one pixel down (in the plane
    // of `n`: an interpolated normal is not a tilt)
    let dp1 = dpdx(p);
    let dp2 = dpdy(p);
    let jx = dp1 - n * dot(dp1, n);
    let jy = dp2 - n * dot(dp2, n);
    // height gained above the tilted plane per pixel, in the direction that
    // gains most, and the squared length of that pixel
    let a = dot(jx, nm);
    let b = dot(jy, nm);
    let rise = sqrt(a * a + b * b);
    if (rise < 1e-7) {
        return 0.0;
    }
    let dir = (jx * a + jy * b) / rise;
    let dist2 = max(dot(dir, dir), 1e-12);
    // RenderSSAOScale, RenderSSAOMaxScale and RenderSSAOFactor, as the pass
    // (renderer.rs), and getKern's eight squared distances
    let scale = min(500.0 / max(view_z, 1e-3), 200.0);
    let factor = 0.3;
    var hidden = 0.0;
    for (var i = 1; i <= 8; i++) {
        let r = f32(i) * 0.125;
        let px = scale * r * r;
        let above = acos(clamp(0.05 / (px * rise), 0.0, 1.0)) / PI;
        hidden += above * min(1.0 / (px * px * dist2), 1.0 / factor);
    }
    return min(factor * hidden / 8.0, 1.0);
}

// ------------------------------------------------------ screen-space rays

/// March a world-space ray through the depth buffer; returns the hit color
/// from the scene copy and a confidence (0 = miss).
fn ssr_trace(origin: vec3<f32>, dir: vec3<f32>, steps: i32) -> vec4<f32> {
    var t = 0.15;
    var prev_t = 0.0;
    for (var i = 0; i < steps; i++) {
        let p = origin + dir * t;
        let c = frame.view_proj * vec4<f32>(p, 1.0);
        if (c.w <= 0.01) {
            break;
        }
        let ndc = c.xyz / c.w;
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
            break;
        }
        let d = scene_depth_at(uv);
        // reverse-Z: the scene is in front of the ray point when its depth is larger
        if (d > ndc.z && d > 0.0) {
            let sp = world_from_depth(uv, d);
            let gap = distance(sp, frame.camera_pos.xyz) - distance(p, frame.camera_pos.xyz);
            if (abs(gap) < max(t - prev_t, 0.25) * 1.5) {
                // refine between the previous and current step
                var lo = prev_t;
                var hi = t;
                var huv = uv;
                for (var k = 0; k < 5; k++) {
                    let mid = (lo + hi) * 0.5;
                    let mp = origin + dir * mid;
                    let mc = frame.view_proj * vec4<f32>(mp, 1.0);
                    let mn = mc.xyz / mc.w;
                    let muv = vec2<f32>(mn.x * 0.5 + 0.5, 0.5 - mn.y * 0.5);
                    if (scene_depth_at(muv) > mn.z) {
                        hi = mid;
                        huv = muv;
                    } else {
                        lo = mid;
                    }
                }
                let edge = min(min(huv.x, 1.0 - huv.x), min(huv.y, 1.0 - huv.y));
                let conf = clamp(edge * 10.0, 0.0, 1.0) * (1.0 - f32(i) / f32(steps));
                let col = textureSampleLevel(scene_tex, lin_clamp, huv, 0.0).rgb;
                return vec4<f32>(col, conf);
            }
        }
        prev_t = t;
        t = t * 1.18 + 0.1;
    }
    return vec4<f32>(0.0);
}

// calcLegacyDistanceAttenuation (deferredUtil.glsl): `d` is the distance
// over the light radius, `falloff` the prim light's falloff (0..2).
fn ll_light_atten(d: f32, falloff: f32) -> f32 {
    let f = max(falloff, 0.0);
    let x = 1.0 - clamp((d + f) / (1.0 + f), 0.0, 1.0);
    return x * x * 2.0;
}

// ------------------------------------------------------------ projectors
// A light with a texture lights its frustum only, through the texture:
// spotLightF.glsl and deferredUtil.glsl (clipProjectedLightVars,
// getProjectedLightDiffuseColor, getProjectedLightAmbiance), originally
// LGPL 2.1, Linden Research / Firestorm. Texture coordinates are LL's (t up).

// Last level of the projected texture's chain (LL's proj_lod).
fn projector_levels(slot: u32) -> f32 {
    let loc = tex_loc[slot];
    return f32(textureNumLevels(tex_pages[loc >> 16u])) - 1.0;
}

// The projected texture at `lod`, linear, kept inside the image (the
// atlas sampler repeats; LL clamps).
fn projector_sample(slot: u32, tc: vec2<f32>, lod: f32) -> vec4<f32> {
    let loc = tex_loc[slot];
    let size = vec2<f32>(textureDimensions(tex_pages[loc >> 16u]));
    let half_texel = 0.5 * exp2(ceil(lod)) / size;
    let uv = clamp(vec2<f32>(tc.x, 1.0 - tc.y), half_texel, vec2<f32>(1.0) - half_texel);
    let c = textureSampleLevel(tex_pages[loc >> 16u], tex_sampler, uv, loc & 0xffffu, lod);
    return vec4<f32>(srgb_to_linear(c.rgb), c.a);
}

// getTexture2DLodDiffuse: the image at `blur` of its mip chain (LL's lod /
// proj_lod), fading out at its edges as it blurs. The fade is LL's
// lod / (proj_lod x 0.5), that is twice `blur`: it does not depend on the
// levels the texture has here (a plain white one has a single level and
// still gets the soft edges of its beam).
fn projector_diffuse(slot: u32, tc: vec2<f32>, blur: f32) -> vec4<f32> {
    var ret = projector_sample(slot, tc, blur * projector_levels(slot));
    let dist = vec2<f32>(0.5) - abs(tc - vec2<f32>(0.5));
    let edge = 0.25 * min(2.0 * blur, 1.0);
    if (edge > 0.0) {
        ret *= clamp(min(dist.x, dist.y) / edge, 0.0, 1.0);
    }
    return ret;
}

// texture2DLodSpecular: the projector's image seen in a shiny face. LL's
// extra factor counts the levels left under the one read; ours are counted
// as those of an image at least 32 pixels wide.
fn projector_specular(slot: u32, tc: vec2<f32>, blur: f32) -> vec4<f32> {
    let levels = projector_levels(slot);
    var ret = projector_sample(slot, tc, blur * levels);
    let dist = vec2<f32>(0.5) - abs(tc - vec2<f32>(0.5));
    var d = min(dist.x, dist.y);
    d *= min(1.0, d * max(levels, 5.0) * (1.0 - blur));
    let edge = 0.25 * min(2.0 * blur, 1.0);
    if (edge > 0.0) {
        ret *= clamp(d / edge, 0.0, 1.0);
    }
    return ret;
}

// Light left by the shadow map of projector `i` (1: lit, or no map), as
// spotLightF.glsl with sampleSpotShadow / pcfSpotShadow (shadowUtil.glsl):
// the two projectors that weigh most on screen have a map, in the halves of
// the top row of the shadow atlas's fourth tile. Like Firestorm's, these
// shadows stop with the sun's (the last tenth of its shadow distance).
fn projector_shadow(i: i32, world_pos: vec3<f32>, n: vec3<f32>) -> f32 {
    if (frame.params.z < 0.5 || (i != frame.spot_lights.x && i != frame.spot_lights.y)) {
        return 1.0;
    }
    let k = select(0u, 1u, i == frame.spot_lights.y);
    let far = frame.cascade_splits.z;
    let depth = distance(world_pos, frame.camera_pos.xyz);
    if (depth >= far) {
        return 1.0;
    }
    // RenderSpotShadowOffset: the point is moved 4 cm along its normal
    let lp = frame.spot_vp[k] * vec4<f32>(world_pos + n * 0.04, 1.0);
    if (lp.w <= 0.0) {
        return 1.0;
    }
    let ndc = lp.xyz / lp.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0) {
        return 1.0;
    }
    // a map is a quarter of the atlas wide: its texel in map coordinates
    let texel = 4.0 / f32(textureDimensions(shadow_map).x);
    let origin = vec2<f32>(0.5 + 0.25 * f32(k), 0.5);
    var sum = 0.0;
    // pcfSpotShadow's five taps
    let taps = array<vec2<f32>, 5>(
        vec2<f32>(0.0, 0.0), vec2<f32>(2.0, 1.5), vec2<f32>(1.0, -1.5), vec2<f32>(-1.0, 1.5), vec2<f32>(-2.0, -1.5));
    for (var t = 0; t < 5; t++) {
        let at = clamp(uv + taps[t] * texel, vec2<f32>(0.5 * texel), vec2<f32>(1.0 - 0.5 * texel));
        sum += textureSampleCompareLevel(shadow_map, shadow_sampler, origin + at * 0.25, ndc.z - 0.0002);
    }
    var s = sum * 0.2;
    // the map fades in and out as its projector takes or loses it
    s = clamp(s + select(frame.spot_fade.x, frame.spot_fade.y, k == 1u), 0.0, 1.0);
    return mix(1.0, s, clamp((far - depth) / (far * 0.1), 0.0, 1.0));
}

// What a local light sends to a point.
struct LightAt {
    // distance attenuation; 0: the light does not reach the point
    atten: f32,
    // towards the light (a projector: towards the apex of its frustum)
    l: vec3<f32>,
    // color x intensity, a projector's through its texture
    color: vec3<f32>,
    // a projector's ambiance: its color and its amount
    amb_color: vec3<f32>,
    ambiance: f32,
    projector: bool,
    // a projector's shadow map on the point (already in `color`)
    shadow: f32,
};

// Projector `i` of the frame's list on a point of normal `n`.
fn projector_at(i: i32, world_pos: vec3<f32>, n: vec3<f32>) -> LightAt {
    let lt = frame.lights[i];
    var at: LightAt;
    at.shadow = 1.0;
    at.atten = 0.0;
    at.l = vec3<f32>(0.0, 0.0, 1.0);
    at.color = vec3<f32>(0.0);
    at.amb_color = vec3<f32>(0.0);
    at.ambiance = 0.0;
    at.projector = true;
    let to_l = lt.pos_radius.xyz - world_pos;
    let dist = length(to_l);
    let radius = lt.pos_radius.w;
    if (dist >= radius) {
        return at;
    }
    let atten = ll_light_atten(dist / radius, lt.color_falloff.w);
    let pj = frame.projectors[i];
    // behind the projector's near plane: nothing
    let tc = pj.matrix * vec4<f32>(world_pos, 1.0);
    if (tc.z < 0.0 || atten <= 0.0) {
        return at;
    }
    at.atten = atten;
    at.l = normalize(pj.origin.xyz - world_pos);
    let uv = tc.xy / tc.w;
    if (uv.x <= 0.0 || uv.x >= 1.0 || uv.y <= 0.0 || uv.y >= 1.0) {
        // beside the frustum: within reach (a shiny face may still show
        // the projector's image), lit by nothing
        return at;
    }
    // the texture blurs with the distance from the light, past the focus
    let slot = pj.tex.x;
    let l_dist = -dot(to_l, pj.dir.xyz);
    let c = projector_diffuse(slot, uv, clamp((l_dist - pj.origin.w) / pj.dir.w, 0.0, 1.0));
    // the beam is shadowed, the ambiance is not (spotLightF)
    at.shadow = projector_shadow(i, world_pos, n);
    at.color = lt.color_falloff.rgb * c.rgb * c.a * at.shadow;
    at.ambiance = pj.params.y;
    if (at.ambiance > 0.0) {
        // getTexture2DLodAmbient: the image's mean color (its last level),
        // fading from the middle of the frustum to its corners
        var amb = projector_sample(slot, vec2<f32>(0.5), projector_levels(slot));
        let off = uv - vec2<f32>(0.5);
        amb *= clamp((0.25 - dot(off, off)) / 0.25, 0.0, 1.0);
        at.amb_color = lt.color_falloff.rgb * amb.rgb * amb.a;
    }
    return at;
}

// One local light on a PBR surface (shade_pbr).
fn pbr_local_light(at: LightAt, n: vec3<f32>, v: vec3<f32>, f0: vec3<f32>, a: f32, nv: f32, diffuse_color: vec3<f32>) -> vec3<f32> {
    let lnl = max(dot(n, at.l), 0.0);
    if (at.atten <= 0.0 || lnl <= 0.0) {
        return vec3<f32>(0.0);
    }
    let h = normalize(at.l + v);
    let nh = max(dot(n, h), 0.0);
    let vh = max(dot(v, h), 0.0);
    let f = f_schlick(f0, vh);
    let spec = d_ggx(nh, a) * v_smith(nv, lnl, a) * f;
    let kd = (vec3<f32>(1.0) - f) * diffuse_color / PI;
    return (kd + spec) * lnl * (at.color * at.atten + projector_ambiance(at, lnl, 0.0));
}

// One local light on a classic surface (multiPointLightF, spotLightF); our
// colors carry the 3.25 PBR factor.
fn ll_diffuse_light(at: LightAt, n: vec3<f32>, base: vec3<f32>) -> vec3<f32> {
    if (at.atten <= 0.0) {
        return vec3<f32>(0.0);
    }
    let nl = dot(n, at.l);
    var local = vec3<f32>(0.0);
    if (nl > 0.0) {
        local = at.color / 3.25 * nl * at.atten * base;
    }
    if (at.projector) {
        // spotLightF: the ambiance lights the faces turned away
        let lit = max(nl, 0.0) * at.atten;
        local += base * projector_ambiance(at, max(nl, 0.0), lit) / 3.25 * max(-nl, 0.0);
    }
    return local;
}

// One local light on a PBR surface under a classic sky (multiPointLightF,
// spotLightF: a projector's ambiance is lit like its beam).
fn ll_pbr_light(at: LightAt, n: vec3<f32>, v: vec3<f32>, f0: vec3<f32>, ra: f32, nv: f32, diffuse_color: vec3<f32>) -> vec3<f32> {
    let lnl = max(dot(n, at.l), 0.0);
    if (at.atten <= 0.0 || lnl <= 0.0) {
        return vec3<f32>(0.0);
    }
    let h = normalize(at.l + v);
    let f = f_schlick(f0, max(dot(v, h), 0.0));
    let spec = d_ggx(max(dot(n, h), 0.0), ra) * v_smith(nv, lnl, ra) * f;
    let diff = (vec3<f32>(1.0) - f) * diffuse_color / PI;
    let intensity = at.atten * at.color + projector_ambiance(at, lnl, 0.0);
    return intensity * clamp(lnl * (diff + spec), vec3<f32>(0.0), vec3<f32>(10.0));
}

// getProjectedLightAmbiance: light a projector adds whatever the angle,
// `nl` clamped to 0..1, `lit` the diffuse amount already given.
fn projector_ambiance(at: LightAt, nl: f32, lit: f32) -> vec3<f32> {
    if (at.ambiance <= 0.0) {
        return vec3<f32>(0.0);
    }
    var amb_da = (nl * 0.5 + 0.5) * at.ambiance + at.ambiance + (nl * nl * 0.5 + 0.5) * at.ambiance;
    amb_da = min(amb_da * at.atten, 1.0 - lit);
    return amb_da * at.amb_color;
}

// ------------------------------------------------- classic skies (Firestorm)
// Skies without reflection probe ambiance ("classic" EEP skies): objects are
// lit and hazed as the Second Life viewer does (atmosphericsFuncs.glsl,
// softenLightF.glsl, deferredUtil.glsl pbrBaseLight, hazeF.glsl) and the
// frame is not tone mapped. Values are in SL's own units (1 = white).

// SL lighting (classic or linear) instead of our HDR model: skies whose
// reflection probe ambiance is 0 or absent.
fn is_classic_sky() -> bool {
    return frame.sky_ll.x > 0.5;
}

// Classic mode proper (no reflection_probe_ambiance key at all).
fn ll_classic_mode() -> bool {
    return frame.sky_ll.x < 1.5;
}

// Sunlight for specular terms (linear).
fn ll_sun_linear(a: LLAtmos) -> vec3<f32> {
    if (ll_classic_mode()) {
        return srgb_to_linear(a.sunlit * 1.35);
    }
    return srgb_to_linear(a.sunlit);
}

fn ll_linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let cl = max(c, vec3<f32>(0.0));
    let lo = cl * 12.92;
    let hi = 1.055 * pow(cl, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, cl <= vec3<f32>(0.0031308));
}

struct LLAtmos {
    sunlit: vec3<f32>,   // sRGB-like
    amblit: vec3<f32>,   // sRGB-like, before ambientLighting
    additive: vec3<f32>,
    atten: vec3<f32>,
};

// calcAtmosphericVars: `rel` = point minus camera (world, z up).
fn ll_atmos(rel_in: vec3<f32>) -> LLAtmos {
    var rel_pos = rel_in;
    let max_y = frame.sky_glow.w;
    if (abs(rel_pos.z) > max_y) {
        rel_pos *= max_y / abs(rel_pos.z);
    }
    let rel_pos_norm = normalize(rel_pos + vec3<f32>(0.0, 0.0, 1e-6));
    let rel_pos_len = length(rel_pos);
    let blue_density = frame.sky_blue_density.rgb;
    let haze_density = frame.sky_blue_density.w;
    let density_multiplier = frame.sky_ambient.w;
    let lightnorm = frame.sky_light.xyz;
    let light_dir = frame.sun_dir.xyz;
    // sunlight_color / moonlight_color as LLPipeline::bindDeferredShader sets
    // them: normalized, and the moon without skyV's 0.7
    var sunlight = frame.sky_obj_light.rgb;
    let cloud_shadow = frame.sky_sunlight.w;
    let light_atten = (blue_density + vec3<f32>(haze_density * 0.25)) * (density_multiplier * max_y);
    let combined = max(blue_density + vec3<f32>(haze_density), vec3<f32>(1e-6));
    let blue_weight = blue_density / combined;
    let haze_weight = vec3<f32>(haze_density) / combined;
    sunlight *= exp(-light_atten * (1.0 / max(1e-6, lightnorm.z)));
    let density_dist = rel_pos_len * density_multiplier;
    let atten = exp(-combined * density_dist * frame.sky_ll.y);
    var haze_glow = dot(rel_pos_norm, lightnorm);
    haze_glow *= max(0.0, dot(light_dir, rel_pos_norm));
    haze_glow = max(1.0 - haze_glow, 0.001) * frame.sky_glow.x;
    haze_glow = clamp(pow(haze_glow, frame.sky_glow.z), -100000.0, 100000.0) + 0.25;
    haze_glow *= frame.sky_light.w;
    let amb = frame.sky_ambient.rgb;
    let tmp_ambient = amb + (vec3<f32>(1.0) - amb) * cloud_shadow * 0.5;
    let cs = sunlight * (1.0 - cloud_shadow);
    var additive = frame.sky_blue_horizon.rgb * blue_weight * (cs + tmp_ambient)
        + frame.sky_blue_horizon.w * haze_weight * (cs * haze_glow + tmp_ambient);
    var o: LLAtmos;
    o.sunlit = sunlight;
    o.amblit = pow(max(tmp_ambient, vec3<f32>(0.0)), vec3<f32>(0.9)) * 0.57;
    o.additive = min(additive * (vec3<f32>(1.0) - atten), vec3<f32>(10.0));
    o.atten = atten;
    return o;
}

// ambientLighting: a touch of light opposite the sun.
fn ll_ambient_lighting(n: vec3<f32>) -> f32 {
    let a = min(abs(dot(n, frame.sun_dir.xyz)), 1.0) * 0.5;
    return 1.0 - a * a;
}

// hazeF.glsl applied to a lit color.
fn ll_haze(col: vec3<f32>, a: LLAtmos) -> vec3<f32> {
    return col * a.atten + srgb_to_linear(a.additive * 2.0) * frame.sky_cloud_pd2.w;
}

// adjustIrradiance (SSAO only darkens the ambient): RenderSSAOIrradianceScale
// 0.6, RenderSSAOIrradianceMax 0.18, then ssao_effect_mat from
// RenderSSAOEffect (0.8, 1.0): value x 0.8, saturation x 1, i.e. minus 0.2 x
// the mean of the channels.
fn ll_ssao(irr: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    let ao = screen_ao(frag);
    let occluded = min(irr * 0.6, vec3<f32>(0.18));
    let effect = occluded - vec3<f32>(0.2 * (occluded.r + occluded.g + occluded.b) / 3.0);
    return mix(effect, irr, ao);
}

// softenLightF.glsl final_scale: 1.1 on classic skies, applied to the whole
// sun + sky result of every surface (legacy and PBR, specular, environment
// and emissive included), but not to the local lights, which LL adds in
// another pass (multiPointLightF, its own final_scale 0.9).
fn ll_final_scale() -> f32 {
    return select(1.0, 1.1, ll_classic_mode());
}

// multiPointLightF.glsl final_scale.
fn ll_local_scale() -> f32 {
    return select(1.0, 0.9, ll_classic_mode());
}

// Sun and sky lighting of a classic (Blinn-Phong) surface: softenLightF
// legacy branch before its final_scale. `base` is the linear albedo.
fn ll_classic_sun_ambient(base: vec3<f32>, n: vec3<f32>, shadow: f32, frag: vec2<f32>, a: LLAtmos) -> vec3<f32> {
    let nl_sun = max(dot(n, frame.sun_dir.xyz), 0.0);
    if (ll_classic_mode()) {
        let sunlit = a.sunlit * 1.35;
        let irr = ll_ssao(a.amblit * ll_ambient_lighting(n), frag);
        let sun_contrib = vec3<f32>(min(pow(nl_sun, 1.2), shadow));
        // GTAO's indirect light (Aurora's own: the Legacy Viewer has none)
        return (srgb_to_linear(irr * 0.9 + ll_linear_to_srgb(sun_contrib) * sunlit * 0.7) + screen_indirect(frag)) * base;
    }
    // linear mode: grey sky ambient, linear sunlight, plain lambert
    let amb_lin = srgb_to_linear(a.amblit * ll_ambient_lighting(n));
    let irr = ll_ssao(vec3<f32>(dot(amb_lin, vec3<f32>(0.2126, 0.7152, 0.0722))), frag);
    return (irr + min(nl_sun, shadow) * srgb_to_linear(a.sunlit) + screen_indirect(frag)) * base;
}

// Local lights on a classic surface (multiPointLightF): light color x n.l x
// attenuation x albedo, with its final_scale.
fn ll_local_diffuse(base: vec3<f32>, n: vec3<f32>, world_pos: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    var local = vec3<f32>(0.0);
    var lights = light_walk(world_pos, frag);
    var spots = projector_walk(lights);
    for (var i = next_light(&lights); i >= 0; i = next_light(&lights)) {
        let lt = frame.lights[i];
        let to_l = lt.pos_radius.xyz - world_pos;
        let dist = length(to_l);
        let radius = lt.pos_radius.w;
        if (dist < radius) {
            let nl = dot(n, to_l / max(dist, 1e-4));
            if (nl > 0.0) {
                local += lt.color_falloff.rgb / 3.25 * nl * ll_light_atten(dist / radius, lt.color_falloff.w) * base;
            }
        }
    }
    for (var i = next_light(&spots); i >= 0; i = next_light(&spots)) {
        local += ll_diffuse_light(projector_at(i, world_pos, n), n, base);
    }
    return local * ll_local_scale();
}

// Diffuse lighting of a classic surface without specular or emissive (the
// terrain): sun and sky with the final scale, plus the local lights.
fn ll_classic_diffuse(base: vec3<f32>, n: vec3<f32>, world_pos: vec3<f32>, shadow: f32, frag: vec2<f32>, a: LLAtmos) -> vec3<f32> {
    return ll_classic_sun_ambient(base, n, shadow, frag, a) * ll_final_scale() + ll_local_diffuse(base, n, world_pos, frag);
}

// calcDiffuseSpecular (deferredUtil.glsl): base x (1 - F0) x (1 - metallic).
fn ll_pbr_diffuse_color(base: vec3<f32>, metallic: f32) -> vec3<f32> {
    return base * 0.96 * (1.0 - metallic);
}

// pbrBaseLight classic mode (deferredUtil.glsl): metallic-roughness surfaces
// recombined like Blinn-Phong ones, plus the image based specular (pbrIbl),
// before softenLightF's final_scale. The local lights are separate
// (ll_classic_pbr_local), as LL adds them in another pass.
fn ll_classic_pbr(base: vec3<f32>, metallic: f32, roughness_in: f32, mat_ao: f32, n: vec3<f32>, v: vec3<f32>, world_pos: vec3<f32>, shadow: f32, frag: vec2<f32>, a: LLAtmos) -> vec3<f32> {
    let roughness = clamp(roughness_in, 0.04, 1.0);
    let ra = roughness * roughness;
    let f0 = mix(vec3<f32>(0.04), base, metallic);
    let diffuse_color = ll_pbr_diffuse_color(base, metallic);
    let nv = max(dot(n, v), 1e-4);
    let sunlit = a.sunlit * 1.35;
    let irr = srgb_to_linear(ll_ssao(a.amblit * ll_ambient_lighting(n), frag) * 0.9);
    let l = normalize(frame.sun_dir.xyz);
    let nl = max(dot(n, l), 0.0);
    var final_sun = vec3<f32>(0.0);
    if (nl > 0.0) {
        let h = normalize(l + v);
        let nh = max(dot(n, h), 0.0);
        let vh = max(dot(v, h), 0.0);
        let f = f_schlick(f0, vh);
        let spec = d_ggx(nh, ra) * v_smith(nv, nl, ra) * f;
        let diff = (vec3<f32>(1.0) - f) * diffuse_color / PI;
        let sun_contrib = srgb_to_linear(ll_linear_to_srgb(vec3<f32>(min(pow(nl, 1.2), shadow))) * sunlit * 0.7) * PI;
        final_sun = clamp(sun_contrib * (diff + spec) * shadow, vec3<f32>(0.0), vec3<f32>(10.0));
    }
    var col = srgb_to_linear(ll_linear_to_srgb(irr * diffuse_color) + ll_linear_to_srgb(final_sun) * 1.1);
    // GTAO's indirect light (Aurora's own: the Legacy Viewer has none)
    col += diffuse_color * screen_indirect(frag) * mat_ao;
    // pbrBaseLight adds the image based specular (pbrIbl: radiance x
    // (F0 x brdf.x + brdf.y) x ao) on classic skies too. LL samples its
    // reflection probes; without them the radiance is the sky reflection,
    // blurred toward the hemisphere with roughness, or the mirror.
    let up = n.z * 0.5 + 0.5;
    let hemi = mix(mix(frame.ground_color.rgb, frame.sky_horizon.rgb, clamp(up * 2.0, 0.0, 1.0)),
                   frame.sky_zenith.rgb, clamp(up * 2.0 - 1.0, 0.0, 1.0));
    let r = reflect(-v, n);
    var env = probe_radiance(world_pos, frag, r, roughness, mix(sky_color(vec3<f32>(r.x, r.y, max(r.z, -0.2))), hemi, roughness));
    let m = mirror_tap(world_pos, r, 1.0 - clamp(roughness_in, 0.0, 1.0));
    env = mix(env, m.rgb, m.a);
    // pbrIbl scales the specular by the material's occlusion only (ORM red);
    // SSAO darkens the irradiance alone (adjustIrradiance). The classic
    // ambient (finalAmbient) does not take that occlusion. GTAO adds its
    // specular occlusion (Aurora's own: the Legacy Viewer has no bent
    // normal), 1 otherwise.
    col += env * env_brdf(f0, roughness, nv) * mat_ao * specular_occlusion(frag, r, roughness, 1.0);
    return col;
}

// Local lights on a PBR surface (multiPointLightF -> pbrCalcPointLightOrSpotLight),
// with multiPointLightF's final_scale.
fn ll_classic_pbr_local(base: vec3<f32>, metallic: f32, roughness_in: f32, n: vec3<f32>, v: vec3<f32>, world_pos: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    let roughness = clamp(roughness_in, 0.04, 1.0);
    let ra = roughness * roughness;
    let f0 = mix(vec3<f32>(0.04), base, metallic);
    let diffuse_color = ll_pbr_diffuse_color(base, metallic);
    let nv = max(dot(n, v), 1e-4);
    var col = vec3<f32>(0.0);
    var lights = light_walk(world_pos, frag);
    var spots = projector_walk(lights);
    for (var i = next_light(&lights); i >= 0; i = next_light(&lights)) {
        let lt = frame.lights[i];
        let to_l = lt.pos_radius.xyz - world_pos;
        let dist = length(to_l);
        let radius = lt.pos_radius.w;
        if (dist < radius) {
            let ll = to_l / max(dist, 1e-4);
            let lnl = max(dot(n, ll), 0.0);
            if (lnl > 0.0) {
                let h = normalize(ll + v);
                let f = f_schlick(f0, max(dot(v, h), 0.0));
                let spec = d_ggx(max(dot(n, h), 0.0), ra) * v_smith(nv, lnl, ra) * f;
                let diff = (vec3<f32>(1.0) - f) * diffuse_color / PI;
                let intensity = ll_light_atten(dist / radius, lt.color_falloff.w) * lt.color_falloff.rgb;
                col += intensity * clamp(lnl * (diff + spec), vec3<f32>(0.0), vec3<f32>(10.0));
            }
        }
    }
    for (var i = next_light(&spots); i >= 0; i = next_light(&spots)) {
        col += ll_pbr_light(projector_at(i, world_pos, n), n, v, f0, ra, nv, diffuse_color);
    }
    return col * ll_local_scale();
}

// Local water controls and history. Opaque GTAO depth is never overwritten.
struct WaterFxControls { area: vec4<f32>, previous: vec4<f32>, clock: vec4<f32>, counts: vec4<u32> };
struct WaterFxEvent { position: vec4<f32>, motion: vec4<f32>, params: vec4<f32> };
@group(0) @binding(15) var water_field: texture_2d<f32>;
@group(0) @binding(16) var<storage, read> water_events: array<WaterFxEvent>;
@group(0) @binding(17) var<uniform> water_fx: WaterFxControls;
@group(0) @binding(18) var water_ground: texture_2d<f32>;
@group(0) @binding(19) var water_light: texture_2d<f32>;
fn water_fx_on(bit: u32) -> bool { return (water_fx.counts.z & bit) != 0u; }
fn water_fx_uv(p: vec2<f32>) -> vec2<f32> { return (p - water_fx.area.xy) / max(water_fx.area.z,1.0); }
fn water_fx_bed(p: vec2<f32>) -> vec2<f32> {
    let uv = water_fx_uv(p);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) { return vec2<f32>(frame.params.y - 20.0, frame.params.y); }
    let pos = clamp(uv * 64.0 - 0.5,vec2<f32>(0.0),vec2<f32>(63.0));
    let i = vec2<i32>(floor(pos)); let j = min(i + vec2<i32>(1),vec2<i32>(63)); let f = fract(pos);
    return mix(mix(textureLoad(water_ground,i,0).xy,textureLoad(water_ground,vec2<i32>(j.x,i.y),0).xy,f.x),
        mix(textureLoad(water_ground,vec2<i32>(i.x,j.y),0).xy,textureLoad(water_ground,j,0).xy,f.x),f.y);
}
// Coarse height/velocity/foam above, fine slope/height/foam below. The
// atlas keeps the existing scene bindings and sampler count unchanged.
fn water_atlas_uv(uv:vec2<f32>)->vec2<f32> {
    let pixel=0.5/f32(textureDimensions(water_field).x);
    let clamped=clamp(uv,vec2<f32>(pixel),vec2<f32>(1.0-pixel));
    return clamped*vec2<f32>(1.0,0.5);
}
fn water_wake_sample(p:vec2<f32>,level:f32)->vec4<f32> {
    if(!water_fx_on(16u) || abs(level-water_fx_bed(p).y)>0.2){return vec4<f32>(0.0);}
    let origin=water_fx.clock.zw;
    let uv=(p-origin)/32.0;
    if(any(uv<vec2<f32>(0.0)) || any(uv>vec2<f32>(1.0))){return vec4<f32>(0.0);}
    let border=min(min(uv.x,uv.y),min(1.0-uv.x,1.0-uv.y));
    return textureSampleLevel(water_field,lin_clamp,water_atlas_uv(uv)+vec2<f32>(0.0,0.5),0.0)*smoothstep(0.0,0.08,border);
}
fn water_fx_sample(p: vec2<f32>) -> vec4<f32> {
    let uv = water_fx_uv(p);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) { return vec4<f32>(0.0); }
    return textureSampleLevel(water_field,lin_clamp,water_atlas_uv(uv),0.0);
}
// Cubic B-spline reconstruction with four bilinear taps. Foam contours must
// remain smooth when the interaction grid is much coarser than a screen pixel.
fn water_fx_foam(p:vec2<f32>)->f32 {
    let uv=water_fx_uv(p);
    if(any(uv<vec2<f32>(0.0)) || any(uv>vec2<f32>(1.0))){return 0.0;}
    let size=f32(textureDimensions(water_field).x);
    let cell=uv*size-0.5;let base=floor(cell);let f=fract(cell);
    let f2=f*f;let f3=f2*f;
    let w0=(1.0-3.0*f+3.0*f2-f3)/6.0;
    let w1=(4.0-6.0*f2+3.0*f3)/6.0;
    let w2=(1.0+3.0*f+3.0*f2-3.0*f3)/6.0;
    let w3=f3/6.0;let g0=w0+w1;let g1=w2+w3;
    let a=(base-0.5+w1/g0)/size;let b=(base+1.5+w3/g1)/size;
    return mix(mix(textureSampleLevel(water_field,lin_clamp,water_atlas_uv(a),0.0).z,
        textureSampleLevel(water_field,lin_clamp,water_atlas_uv(vec2<f32>(b.x,a.y)),0.0).z,g1.x),
        mix(textureSampleLevel(water_field,lin_clamp,water_atlas_uv(vec2<f32>(a.x,b.y)),0.0).z,
        textureSampleLevel(water_field,lin_clamp,water_atlas_uv(b),0.0).z,g1.x),g1.y);
}
// Long wavelengths are resolved by the shared mesh; micro-waves stay in normals.
fn water_fx_wave(p: vec2<f32>, level: f32) -> vec3<f32> {
    var offset=vec3<f32>(0.0);
    if(water_fx_on(1u)) {
        offset+=water_swell(p,frame.camera_pos.w,water_fx.counts.w).offset*smoothstep(0.08,2.4,level-water_fx_bed(p).x);
    }
    offset.z+=clamp(water_wake_sample(p+offset.xy,level).z,-0.16,0.16);
    return offset;
}
fn water_fx_normal(p:vec2<f32>,level:f32,wake:vec2<f32>)->vec3<f32> {
    var gradient=vec2<f32>(0.0);
    if(water_fx_on(1u)) {
        gradient=water_swell(p,frame.camera_pos.w,water_fx.counts.w).gradient;
        gradient*=smoothstep(0.08,2.4,level-water_fx_bed(p).x);
    }
    if(water_fx_on(16u)) {
        let texel=water_fx.area.z/f32(textureDimensions(water_field).x);
        gradient+=vec2<f32>(water_fx_sample(p+vec2<f32>(texel,0.0)).x-water_fx_sample(p-vec2<f32>(texel,0.0)).x,
            water_fx_sample(p+vec2<f32>(0.0,texel)).x-water_fx_sample(p-vec2<f32>(0.0,texel)).x)/max(2.0*texel,0.01);
    }
    return vec3<f32>(-gradient-wake,0.0);
}
fn water_hash_u32(value:u32)->f32 {
    var h=value;
    h=(h^(h>>16u))*0x7feb352du;
    h=(h^(h>>15u))*0x846ca68bu;
    return f32((h^(h>>16u))>>8u)/16777216.0;
}
fn water_fx_hash(p: vec2<f32>) -> vec2<f32> {
    // Sine/multiply hashes lose their fractional precision at world-space
    // foam frequencies and produce visible repeating blocks and dotted rows.
    let bits=bitcast<vec2<u32>>(p);
    return vec2<f32>(water_hash_u32(bits.x^(bits.y*0x9e3779b9u)),
        water_hash_u32(bits.y^(bits.x*0x85ebca6bu)^0x68bc21ebu));
}
// A global constant avoids Naga re-emitting a local splat when specializing
// masked/blended pipelines. The alpha specialization regression covers this.
const WATER_CAUSTIC_FILTER:vec2<f32>=vec2<f32>(0.4/256.0,0.4/256.0);
fn water_caustics(pos:vec3<f32>,normal:vec3<f32>)->vec3<f32> {
    if(!water_fx_on(32u) || frame.misc.w>0.5){return vec3<f32>(0.0);}
    if(!camera_underwater() && water_excluded(pos)){return vec3<f32>(0.0);}
    let bed=water_fx_bed(pos.xy);let depth=bed.y-pos.z;
    if(depth<0.03 || depth>18.0 || frame.sky_sun.z<0.05 || frame.sun_dir.w<0.01){return vec3<f32>(0.0);}
    let light=normalize(frame.sun_dir.xyz);
    let ray=refract(-light,vec3<f32>(0.0,0.0,1.0),0.7501875);
    var project=pos.xy-ray.xy*depth/max(-ray.z,0.2);
    project-=water_swell(project,frame.camera_pos.w,water_fx.counts.w).gradient*min(depth,7.0)*0.25;
    let uv=fract(project/16.0);
    var focus=textureSampleLevel(water_light,lin_clamp,uv,0.0).rgb;
    if(water_fx.counts.w>0u){
        focus=(textureSampleLevel(water_light,lin_clamp,fract(uv+WATER_CAUSTIC_FILTER),0.0).rgb+textureSampleLevel(water_light,lin_clamp,fract(uv-WATER_CAUSTIC_FILTER),0.0).rgb
            +textureSampleLevel(water_light,lin_clamp,fract(uv+vec2<f32>(WATER_CAUSTIC_FILTER.x,-WATER_CAUSTIC_FILTER.y)),0.0).rgb+textureSampleLevel(water_light,lin_clamp,fract(uv+vec2<f32>(-WATER_CAUSTIC_FILTER.x,WATER_CAUSTIC_FILTER.y)),0.0).rgb)*0.25;
    }
    var intensity=mix(focus.r,focus.g,smoothstep(0.75,2.5,depth));
    if(depth>2.5){intensity=mix(focus.g,focus.b,smoothstep(2.5,7.0,depth));}
    let energy=max(intensity-0.9,0.0)*0.42;
    let extinction=exp(-depth*(0.13+frame.water_fog.w*0.025));
    let shadow=shadow_factor(pos,normal,distance(pos,frame.camera_pos.xyz));
    return frame.sun_color.rgb*energy*extinction*max(dot(normal,light),0.0)*shadow*frame.sun_dir.w*smoothstep(0.03,0.3,depth);
}

fn water_fx_noise(p: vec2<f32>) -> f32 {
    let i=floor(p);let f=fract(p);let w=f*f*(3.0-2.0*f);
    return mix(mix(water_fx_hash(i).x,water_fx_hash(i+vec2<f32>(1.0,0.0)).x,w.x),
        mix(water_fx_hash(i+vec2<f32>(0.0,1.0)).x,water_fx_hash(i+vec2<f32>(1.0,1.0)).x,w.x),w.y);
}

// Continuous torn coverage and filtered fine grain, without a lattice of holes.
fn water_foam_coverage(p:vec2<f32>,amount:f32,footprint:f32)->f32 {
    if(amount<0.08){return 0.0;}
    let drift=p-vec2<f32>(frame.camera_pos.w*0.09,frame.camera_pos.w*0.035);
    let warp=vec2<f32>(water_fx_noise(drift*1.3),water_fx_noise(drift*1.3+7.4));
    let q=drift+(warp-0.5)*0.7;
    let rotated=vec2<f32>(q.x*0.8-q.y*0.6,q.x*0.6+q.y*0.8);
    let medium=mix(water_fx_noise(rotated*3.7),0.5,smoothstep(0.04,0.18,footprint));
    let fine=mix(water_fx_noise(q*11.3),0.5,smoothstep(0.012,0.07,footprint));
    let structure=water_fx_noise(q*0.9)*0.55+medium*0.3+fine*0.15;
    let coverage=smoothstep(0.08,0.75,amount)*smoothstep(0.28,0.72,structure);
    let grain=mix(0.84,0.5+0.5*water_fx_noise(rotated*38.0),1.0-smoothstep(0.004,0.025,footprint));
    return coverage*grain;
}
