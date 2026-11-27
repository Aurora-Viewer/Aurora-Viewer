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
    water_air: vec4<f32>,       // x: exterior capture available, y: water rendering type hidden
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
    sky_obj_light: vec4<f32>,   // rgb sun / moon color lighting objects (mSunDiffuse, max component <= 1), w colored sky ambient
    sky_optics: vec4<f32>,      // x moisture level (rainbow), y droplet radius, z ice level (22° halo)
    sky_optics_tex: vec4<u32>,  // x rainbow texture slot, y halo texture slot (0: none)
    // night sky (stars.rs StarFrame): region directions of the catalogue's
    // axes; w: catalogue (-1 none), Milky Way strength, 0
    sky_stars: array<vec4<f32>, 3>,
    sky_star_pass: vec4<f32>,   // this pass's target: width, height, 1/w, 1/h (0: frame.screen)
    studio_aurora: array<vec4<f32>, 4>, // navy/enabled, teal/yaw, violet/time, indigo/white reveal
    anim_clock: vec4<u32>,      // effects clock (rez, portals): x ms (wrapping), y sub-ms fraction (f32 bits); zw the same for texture animations (held while the world is frozen)
    selection_color: vec4<f32>, // selected-face overlay, from the UI palette
    // the poser's hovered part (`fs_part_mask`): bit j of the skeleton's
    // palette joint j, 256 joints in two vectors (word j >> 5)
    part_joints: array<vec4<u32>, 2>,
    // Eight four-bit part classes per word, summed before interpolation.
    part_classes: array<vec4<u32>, 8>,
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
    // shadow maps of the projectors within the configured budget: world to the
    // clip space of each, its light (-1: none), and what it leaves unlit
    // of its shadows (1 - fade)
    spot_vp: array<mat4x4<f32>, 16>,
    spot_lights: array<vec4<i32>, 4>,
    spot_fade: array<vec4<f32>, 4>,
    contact: vec4<f32>,         // x: 1 = the contact shadows' mask is read (main view), y: 1 = blended faces opaque to the eye cast it
    features: vec4<u32>,        // x: FEATURE_* bits, in every pass (main view, reflections, probes); y: the rain's shelter map + 1 (0: none); z: FEATURE_FACE_SELECT_SLOT; w: 1 where the volumetric clouds replace the Legacy layer (main view)
    // Motion vectors (main view; zero in the other passes), without the TAA
    // jitter: this frame's view-projection for positions relative to `eye`,
    // the last frame's for the same positions, and this frame's clip space
    // to the last frame's (a point the depth places, where nothing drew).
    motion_vp: mat4x4<f32>,
    motion_prev_vp: mat4x4<f32>,
    motion_reproj: mat4x4<f32>,
    rain: vec4<f32>,            // rain prototype: x how hard it rains (0..1)
    rain_wet: vec4<f32>,        // wetness memory: corner xy, size z (0: none), w the rain that wetted
    rain_wet_step: vec4<f32>,   // last corner xy, z time since (s, < 0: none), w soak of a new place
    rain_look: vec4<f32>,       // x puddles (0/1), y how far the wet ground is drawn (m), z quality (0 low, 1 medium, 2 high), w snow (0/1)
    rain_wind: vec4<f32>,       // xy how far the wind carried the drops (m), zw the wind now (m/s)
    snow_cover: vec4<f32>,      // x how hard it snows, y the snow's depth on a new place, z flags (1 rain melts it, 2 the puddles may freeze, 4 deep snow, 8 lying snow hidden), w the detail texture's slot (0: none)
    snow_trail: vec4<f32>,      // footprints: corner xy, size z (0: none), w how many stamps
    snow_trail_step: vec4<f32>, // xy my avatar (the footprints' and deep snow's centre), zw unused
    snow_stamps: array<vec4<f32>, 16>, // feet set down: x, y, heading (rad), half length (m)
    lightning: vec4<f32>,       // the lightning's glow: xy the way to it, z its height (sine), w how bright
    aurora: vec4<f32>,          // the aurora: x how bright (0: none), y its heading (rad, clockwise from north), z its foot's height (0..1), w colours (0 green, 1 violet, 2 red)
    photo: vec4<f32>,           // the main view of a photo tile: its part of the photo (left, top, width, height); zero otherwise
    tex_lod: vec4<f32>,         // x: the material textures' mip bias (FSR drawing the world smaller than the window, fsr.rs mip_bias); 0 otherwise
    soft: vec4<f32>,            // x: 1 = the soft shadows' mask is read (main view), y: share of the light the sun's shadows let through (1 - intensity), z: projectors' light size in metres (0: the 5-tap filter), main view only; w: skip Ignore Object in the narrowed pick view
    snow_stamp_z: array<vec4<f32>, 4>, // the soles' heights of snow_stamps (m), four to a vec4
    snow_far: vec4<f32>,        // x how far the snow lying is drawn (m), whatever falls (while it snows and while it melts); yzw unused
    curvature: vec4<f32>,       // planet curvature (apply_curvature): xy the main camera (region m), z 1 / 2R (0: flat), w unused
};

// frame.features.x: "HDR and Emissive" off, the Legacy Viewer's G-buffer has
// no emissive target (RenderEnableEmissiveBuffer, HAS_EMISSIVE): the PBR
// faces it draws deferred (opaque, alpha-masked, terrain) lose their
// emissive; blended and HUD faces keep it (pbralphaF, pbropaqueF IS_HUD)
const FEATURE_NO_EMISSIVE: u32 = 1u;

// frame.features.z: the slot of the face-select pattern's texture (0: the
// procedural stand-in), see FEATURE_FACE_SELECT_SLOT in renderer.rs
const FEATURE_FACE_SELECT_SLOT: u32 = 2u;

fn deferred_emissive_off() -> bool {
    return (frame.features.x & FEATURE_NO_EMISSIVE) != 0u;
}

// ------------------------------------------------------ planet curvature
//
// Aurora's own, off by default (curvature.rs). Regions are flat, so open
// water runs out to eye level; on a planet of radius R it falls away under
// the camera by the sagitta d^2 / 2R of a horizontal distance d, and from
// 10 m up the sea ends ~11 km out, ~0.1 degrees under eye level, with the
// sky below it. (The exact drop R - sqrt(R^2 - d^2) differs by d^4 / 8R^3:
// about a millimetre at 40 km on the Earth.)
//
// The bend is a projection, not a change of the world: only positions
// going to clip space are lowered (world_clip, eye_clip, motion_clips), and
// world_from_depth raises what the depth gives back by the same amount.
// Lighting, shadows, fog, the waterline tests and every other use of a
// world position stay those of the flat region, as the simulator has it;
// the shadow maps, made in that flat world, need no bend of their own. The
// drop depends on x and y alone, so the inverse is exact.
//
// `c`: xy the main camera (the centre of the bend in every pass:
// reflections, mirrors and probes see the planet the camera stands on),
// z 1 / 2R, 0 for a flat world.
fn curvature_drop(p: vec2<f32>, c: vec4<f32>) -> f32 {
    let d = p - c.xy;
    return dot(d, d) * c.z;
}

fn apply_curvature(p: vec3<f32>, c: vec4<f32>) -> vec3<f32> {
    return vec3<f32>(p.xy, p.z - curvature_drop(p.xy, c));
}

fn remove_curvature(p: vec3<f32>, c: vec4<f32>) -> vec3<f32> {
    return vec3<f32>(p.xy, p.z + curvature_drop(p.xy, c));
}

// A position relative to the frame's eye (object vertices), bent: the
// distance is taken from the eye's offsets, so that no region coordinate
// is rounded on the way.
fn curve_eye(ep: vec4<f32>) -> vec4<f32> {
    let d = ep.xy + (frame.eye.xy - frame.curvature.xy);
    return vec4<f32>(ep.x, ep.y, ep.z - dot(d, d) * frame.curvature.z, ep.w);
}

// Clip position of a region position, on the curved planet.
fn world_clip(p: vec3<f32>) -> vec4<f32> {
    return frame.view_proj * vec4<f32>(apply_curvature(p, frame.curvature), 1.0);
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
    identity: vec4<u32>,      // x the OIT tie-break of the face (types.rs `oit_tie`: the object's id and face index hashed, the same on every launch); y, z, w spare
};

const FLAG_FULLBRIGHT: u32 = 1u;
const FLAG_PORTAL_CLIP: u32 = 16384u;
const FLAG_PORTAL_EFFECT: u32 = 32768u;
const FLAG_WORLD_BLEND_PICK: u32 = 2147483648u; // flags.w, above texture animation
const FLAG_OBJECT_MASK_PICK: u32 = 1073741824u; // flags.w, world/worn objects only
const FLAG_IGNORE_OBJECT_PICK: u32 = 134217728u; // flags.w bit 27, Ignore Object click action
// flags.w bits 24..26: layer rank of a rigged face (types.rs flags, object.wgsl eye_clip)
const LAYER_RANK_SHIFT: u32 = 24u;
const LAYER_RANK_MAX: u32 = 7u;
const LAYER_DEPTH_STEP: f32 = 9.5367431640625e-7; // 2^-20
const FLAG_STUDIO_STAGE: u32 = 536870912u; // flags.w bit 29, local scenery, not the avatar reveal
const FLAG_PICK_IGNORES_ALPHA: u32 = 268435456u; // flags.w bit 28, Alt-focus keeps clear texels (emissive mask mode)
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
// Added to the x of the prepass's encoded normal (in 0..1, object.wgsl
// depth_normal) where FSR's history cannot follow the surface, a running
// texture animation: FSR's transparency and composition mask
// (fsr/reactivity.wgsl); GTAO's oct_decode takes it off.
const NORMAL_COMPOSITION: f32 = 2.0;
const FLAG_DOUBLE_SIDED: u32 = 2048u;
// placed by the GPU: model relative to the anchor matrix palettes[flags.y]
// (worn objects); skinned: skeleton placed by its own anchor
const FLAG_ANCHORED: u32 = 4096u;
const FLAG_LIP_SYNC: u32 = 8192u;
// a skeleton's own anchor (skeleton space to world), from its first joint
const SKELETON_ANCHOR: u32 = 208u;
// the weights of a skeleton's live morphs, after its anchors (object.wgsl)
const LIVE_MORPH_WEIGHTS: u32 = 256u;
// the snow on an avatar (x of its first column), and the entries per
// skeleton (types.rs AVATAR_SNOW, PALETTE_STRIDE)
const AVATAR_SNOW: u32 = 260u;
// written by the viewer with this as its last element (types.rs
// AVATAR_SNOW_MARK): anything else in the entry is no snow
const AVATAR_SNOW_MARK: f32 = -2.0;
const PALETTE_STRIDE: u32 = 261u;

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
// The solid surfaces of the depth the occlusion and the contact shadows
// were made from (renderer.rs, "solid depth"): the prepass and the blended
// texels solid to the eye, without those that are in that depth while
// their opacity still rises (blend_screen_share), which would find
// themselves here. Bound where the blended faces are drawn
// (`on_occlusion_surface`); a dummy elsewhere.
@group(0) @binding(29) var solid_depth: texture_depth_2d;
@group(0) @binding(20) var water_interface: texture_depth_2d;
@group(0) @binding(21) var water_air_color: texture_2d<f32>;
@group(0) @binding(22) var water_air_depth: texture_depth_2d;
@group(0) @binding(23) var water_exclusion_mask: texture_2d<f32>;
// GTAO's screen-space indirect light (gtao.rs), when SCREEN_INDIRECT
@group(0) @binding(14) var il_tex: texture_2d<f32>;
// The traced probes' sight of the sky at half resolution (rt_gi.wgsl
// `cs_gi_sky`): in g how open each place is (`screen_open`).
@group(0) @binding(32) var gi_open_tex: texture_2d<f32>;

// Reflection probes (probes.rs, reflectionProbeF.glsl).
// The probes of the frame's list at most (probes::MAX_PROBES).
const MAX_PROBES: u32 = 256u;
// A set of probes: bit i of word i / 32 for probe i of the list
// (probes::ProbeSets; the lights' sets hold 64 and are two words).
struct ProbeMask {
    w: array<u32, 8>,
};
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
    // the box probes, which sample_probes_pair looks through first
    boxes: ProbeMask,
    // the probes that cover each screen column, each screen row and each
    // slice of depth of the main view (probes::tiles): one set per element
    tile_grid: vec4<f32>,       // x, y: columns and rows per pixel
    tile_forward: vec4<f32>,    // camera forward axis (fragment depth)
    tile_mode: vec4<u32>,
    tile_columns: array<ProbeMask, 32>,
    tile_rows: array<ProbeMask, 32>,
    tile_slices: array<ProbeMask, 32>,
    probes: array<Probe, MAX_PROBES>,
};
@group(0) @binding(9) var<storage, read> probe_set: ProbeSet;
@group(0) @binding(10) var probe_tex: texture_cube_array<f32>;
@group(0) @binding(11) var probe_sampler: sampler;
@group(0) @binding(12) var probe_irr_tex: texture_cube_array<f32>;
// the sun's contact shadows (contact_shadows.wgsl): 0 lit, 1 in shadow
@group(0) @binding(13) var contact_tex: texture_2d<f32>;
// What the glass leaves of the traced sun's red, green and blue
// (rt_shadows.wgsl `out_glass`; white without ray tracing): `sun_glass`.
@group(0) @binding(31) var sun_glass_tex: texture_2d<f32>;
// the sun's soft shadows (soft_shadows.wgsl): per pixel, the light let
// through (10 bits) and the distance of the surface it was made for (an f16)
@group(0) @binding(30) var soft_tex: texture_2d<u32>;

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
// The avatars' wet memory (types.rs AVATAR_WET_BLOCKS, AVATAR_WET_JOINTS;
// object.wgsl avatar_wet_memory).
@group(1) @binding(5) var<storage, read> avatar_wet: array<vec4<u32>>;
const AVATAR_WET_JOINTS: u32 = 256u;

// Bindless textures pooled in pages (textures.rs): a slot's location packs
// the page (high 16 bits) and its layer (low 16 bits).
@group(2) @binding(0) var tex_pages: binding_array<texture_2d_array<f32>>;
@group(2) @binding(1) var tex_sampler: sampler;
struct TextureLocations {
    page_count: u32,
    values: array<u32>,
}
@group(2) @binding(2) var<storage, read> tex_loc: TextureLocations;

// Stale or corrupt locations fall back to page 0, which always holds the
// reserved textures. `TextureTable::build` binds at least that page.
fn texture_page_index(loc: u32) -> u32 {
    let page = loc >> 16u;
    let last_bound_page = max(tex_loc.page_count, 1u) - 1u;
    return select(page, 0u, page > last_bound_page);
}

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
    let loc = tex_loc.values[idx];
    return textureSampleBias(tex_pages[texture_page_index(loc)], tex_sampler, uv, loc & 0xffffu, frame.tex_lod.x);
}

// Without the frame's mip bias: the shadow maps have their own size, and
// their pass binds no frame uniform
fn sample_tex_unbiased(idx: u32, uv: vec2<f32>) -> vec4<f32> {
    let loc = tex_loc.values[idx];
    return textureSample(tex_pages[texture_page_index(loc)], tex_sampler, uv, loc & 0xffffu);
}

fn sample_tex_level(idx: u32, uv: vec2<f32>, lod: f32) -> vec4<f32> {
    let loc = tex_loc.values[idx];
    return textureSampleLevel(tex_pages[texture_page_index(loc)], tex_sampler, uv, loc & 0xffffu, lod);
}

// With the pixel's own uv derivatives (`gx`, `gy`), filtered by the
// anisotropic sampler as they say: where the control flow is not uniform.
fn sample_tex_grad(idx: u32, uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>) -> vec4<f32> {
    let loc = tex_loc.values[idx];
    return textureSampleGrad(tex_pages[texture_page_index(loc)], tex_sampler, uv, loc & 0xffffu, gx, gy);
}

fn in_reflection_pass() -> bool {
    return frame.misc.w > 0.5;
}

/// Reflection passes with a mirrored view: planar ones (water, mirror) and
/// the faces of a probe (probes::face_view).
fn mirrored_pass() -> bool {
    return frame.misc.w > 1.5;
}

// frame.light_mode.z, the capture of a probe that lights its surfaces with
// the probes as LLReflectionMapManager::updateUniforms does: the radiance
// capture with both, the irradiance capture (PROBE_CAPTURE_AMBIANCE too)
// with no irradiance and half the radiance. A closed room then sees its own
// dark surroundings in its captures, not the sky. probes::CaptureLighting.
const PROBE_CAPTURE_LIT: u32 = 0x20000u;
const PROBE_CAPTURE_AMBIANCE: u32 = 0x40000u;
// frame.light_mode.z, any capture of a probe: the sea instead of the sky under
// the rim of the Legacy Viewer's dome (sky_pixel in env.wgsl).
// probes::capture_lighting::DOME.
const PROBE_CAPTURE_DOME: u32 = 0x80000u;

// Probes are sampled in the main view and in the captures of the probes
// themselves (planar reflections and the sky-only default probe see the sky).
fn probes_on() -> bool {
    return probe_set.enabled != 0u && (!in_reflection_pass() || (frame.light_mode.z & PROBE_CAPTURE_LIT) != 0u);
}

// What the pass multiplies the probes' irradiance by (refParams.x).
fn probe_pass_ambiance() -> f32 {
    return select(1.0, 0.0, (frame.light_mode.z & PROBE_CAPTURE_AMBIANCE) != 0u);
}

// What the pass multiplies the probes' radiance by (refParams.y).
fn probe_pass_radiance() -> f32 {
    return select(1.0, 0.5, (frame.light_mode.z & PROBE_CAPTURE_AMBIANCE) != 0u);
}

fn probe_max_lod() -> f32 {
    return probe_set.max_lod;
}

// frame.light_mode.z, AURORA_PROBE_FACE=exact: the probes found from the
// surface's own position, as before (and as the Legacy Viewer).
const PROBE_FACE_EXACT: u32 = 0x800000u;
// How far along its normal a surface is looked up in the probes (m).
const PROBE_FACE_OFFSET: f32 = 0.01;

// Where a surface at `world_pos`, of normal `n`, finds its probes: 1 cm in
// front of it. A creator sizes a box probe to the inside of a room, its
// faces on those of the walls, the floor and the roof; whether a point of
// such a face is in the box (|lp| <= 1, shouldSampleProbe in
// reflectionProbeF.glsl) then turns on the rounding of its position, pixel
// by pixel, between the box's sharp image of the room and the automatic
// probes'. On a shiny wall it showed as a grain of speckles and streaks,
// as z-fighting does. A point in front of the face is in the box for the
// walls inside it and out of it for a face outside it whose side is on the
// box (the outer face of a box that holds the walls). The Legacy Viewer
// tests the position itself; the offset moves the parallax by 1 cm, which
// does not show (the rounding is about 1 mm at the top of a skybox's
// 4096 m).
fn probe_point(world_pos: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    if ((frame.light_mode.z & PROBE_FACE_EXACT) != 0u) {
        return world_pos;
    }
    return world_pos + n * PROBE_FACE_OFFSET;
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
fn probe_tiles_apply(world_pos: vec3<f32>) -> bool {
    return dot(world_pos - frame.camera_pos.xyz, probe_set.tile_forward.xyz) >= PROBE_TILES_NEAREST;
}

fn probe_tile_cell(world_pos: vec3<f32>, frag: vec2<f32>) -> vec3<u32> {
    return tile_cell(world_pos, frag, probe_set.tile_grid.xy, probe_set.tile_forward.xyz);
}

// Word `k` of a fragment's probes, with k * 32 < count: the first `count` of
// the list and, with `tiles`, those of its sets. The words are read one at a
// time and only the ones the list reaches, with no array of the eight on the
// function's stack: at the default 32 probes a fragment reads one word of
// each table. An integrated GPU's object pass went from 20.0 to 17.1 ms with
// this walk (15.0 ms with the former sets of 64 probes).
fn probe_word(k: u32, count: u32, tiles: bool, cell: vec3<u32>) -> u32 {
    let left = count - k * 32u;
    var held = 0xffffffffu;
    if (left < 32u) {
        held = (1u << left) - 1u;
    }
    if (tiles) {
        held &= probe_set.tile_columns[cell.x].w[k] & probe_set.tile_rows[cell.y].w[k] & probe_set.tile_slices[cell.z].w[k];
    }
    return held;
}

// AURORA_PROBE_WALK=check: whether a probe of the list covers `pos` without
// being in the fragment's sets. That never happens: the image is the one
// the whole list gives.
fn probe_tiles_miss(pos: vec3<f32>, frag: vec2<f32>) -> bool {
    let tiles = probe_tiles_apply(pos);
    let cell = probe_tile_cell(pos, frag);
    let count = min(probe_set.count, MAX_PROBES);
    for (var i = 1u; i < count; i++) {
        if (((probe_word(i >> 5u, count, tiles, cell) >> (i & 31u)) & 1u) != 0u) {
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
    return count > 0u && (probe_word(0u, count, tiles, cell) & 1u) == 0u;
}

fn sample_probes_ex(pos: vec3<f32>, dir: vec3<f32>, lod: f32, irradiance: bool, amblit: vec3<f32>, water: bool, outside: vec4<f32>) -> vec4<f32> {
    let count = min(probe_set.count, MAX_PROBES);
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
            if (i == 0u) {
                refcol = mix(outside.rgb, refcol, outside.a);
            }
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
// tapIrradianceMap); `outside` as `default_probe_outside`'s.
fn probe_tap(i: u32, v: vec3<f32>, lookup: ProbeLookup, amblit: vec3<f32>, outside: vec4<f32>) -> vec3<f32> {
    let slot = i32(probe_set.probes[i].slot.x);
    let scale = probe_set.probes[i].params;
    if (lookup.irradiance) {
        let irr = textureSampleLevel(probe_irr_tex, probe_sampler, v, slot, 0.0).rgb * scale.x;
        return mix(amblit, irr, min(scale.x, 1.0));
    }
    let rad = textureSampleLevel(probe_tex, probe_sampler, v, slot, lookup.lod).rgb * scale.y;
    if (i == 0u) {
        return mix(outside.rgb, rad, outside.a);
    }
    return rad;
}

// What the default probe's reflection becomes at the fragment at `frag`:
// in a the share of it that stays (how open the place is, `screen_open`),
// in rgb what takes the rest (the light the faces around bounce there, the
// traced indirect light's). The default probe sees only the sky, the
// water, the terrain and the clouds (LLReflectionMapManager::
// updateProbeFace), from wherever the camera is, so a shiny face in a
// closed room without a probe of its own reflected the world outside it,
// its sky and its sunlit water; it now reflects the room's own light,
// blurred (the probes keep no direction finer than a face's hemisphere).
// The place's openness and not the sky's share along the normal: out in
// the open a shiny face turned to the ground sees no sky, and the ground
// the probe shows it is right. Aurora's own, with the traced indirect light
// only: the Legacy Viewer's rooms reflect the outside. The probes residents
// place, which see the place, are left as they are; (0, 1) without the
// traced indirect light and in reflection passes.
fn default_probe_outside(frag: vec2<f32>) -> vec4<f32> {
    let open = screen_open(frag);
    if (open >= 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    return vec4<f32>(textureSampleLevel(il_tex, lin_clamp, screen_uv(frag), 0.0).rgb, open);
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
// (`probe_word`; `frag` is the fragment's pixel and `pos` where it is in
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
    let walk = frame.light_mode.w & 0xffu;
    let outside = default_probe_outside(frag);
    if (walk == PROBE_WALK_PER_LOOKUP) {
        out.a = sample_probes_ex(pos, a.dir, a.lod, a.irradiance, amblit, !automatic, outside);
        if (want_b) {
            out.b = sample_probes_ex(pos, b.dir, b.lod, b.irradiance, amblit, !automatic, outside);
        }
        return out;
    }
    if (walk == PROBE_WALK_CHECK && probe_tiles_miss(pos, frag)) {
        out.a = vec4<f32>(4.0, 0.0, 4.0, 1.0);
        out.b = out.a;
        return out;
    }
    // the probes of the fragment's sets or, without the sets, the list
    let count = min(probe_set.count, MAX_PROBES);
    let words = (count + 31u) >> 5u;
    let tiles = walk >= PROBE_WALK_TILES && probe_tiles_apply(pos);
    var cell = vec3<u32>(0u);
    if (tiles) {
        cell = probe_tile_cell(pos, frag);
    }
    // never let automatic probes encroach on box probes
    var sample_automatic = true;
    for (var k = 0u; k < words; k++) {
        var boxes = probe_set.boxes.w[k];
        if (boxes != 0u) {
            boxes &= probe_word(k, count, tiles, cell);
        }
        for (; boxes != 0u; boxes &= boxes - 1u) {
            let n = k * 32u + firstTrailingBit(boxes);
            let lp = (probe_set.probes[n].box_inv * vec4<f32>(pos, 1.0)).xyz;
            if (all(abs(lp) <= vec3<f32>(1.0))) {
                sample_automatic = false;
            }
        }
    }
    var a0 = vec3<f32>(0.0);
    var a1 = vec3<f32>(0.0);
    var b0 = vec3<f32>(0.0);
    var b1 = vec3<f32>(0.0);
    var w0 = 0.0;
    var w1 = 0.0;
    var dw1 = 0.0;
    for (var k = 0u; k < words; k++) {
        for (var held = probe_word(k, count, tiles, cell); held != 0u; held &= held - 1u) {
            let i = k * 32u + firstTrailingBit(held);
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
            let ca = probe_tap(i, va, a, amblit, outside) * w;
            var cb = vec3<f32>(0.0);
            if (want_b) {
                cb = probe_tap(i, vb, b, amblit, outside) * w;
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
    // the point of the flat region the curved one was drawn from (the sky,
    // at depth 0, stays the far point it was)
    if (depth <= 0.0) {
        return w.xyz / w.w;
    }
    return remove_curvature(w.xyz / w.w, frame.curvature);
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
    // The glow's angle is taken from the view direction, not from rel_norm:
    // skyF.glsl / cloudsF.glsl measure it from the dome point raised by 50 m,
    // which on the low dome (about 600-850 m away overhead) puts the hotspot
    // some 3 degrees under the sun or moon. The Legacy Viewer as it renders
    // shows the glow centred on the body (compared on Agni, #2782): a
    // deliberate difference from its source, kept to the glow alone (the
    // haze colour, rainbow and halo still use rel_norm).
    var haze_glow = max(1.0 - dot(d, lightnorm), 0.001);
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

// The login illustration's four curtain waves (ui/aurora_bg.rs), lifted
// onto sky directions so water and material reflections see the same sky.
// Palette colours and time are supplied only by the local studio frame.
fn studio_sky_color(dir: vec3<f32>) -> vec3<f32> {
    let d = normalize(dir);
    let yaw = frame.studio_aurora[1].w;
    let x = -d.x * cos(yaw) - d.y * sin(yaw);
    let y = d.x * sin(yaw) - d.y * cos(yaw);
    let u = atan2(y, x) / 1.5 + 0.5;
    let h = 0.58 - asin(clamp(d.z, -1.0, 1.0)) * 2.8;
    let t = frame.studio_aurora[2].w;
    let navy = frame.studio_aurora[0].rgb;
    let teal = frame.studio_aurora[1].rgb;
    let violet = frame.studio_aurora[2].rgb;
    let indigo = frame.studio_aurora[3].rgb;
    var c = mix(navy * 0.55, mix(navy, indigo, 0.22), clamp(h, 0.0, 1.0));
    let bases = array<f32, 4>(0.30, 0.38, 0.24, 0.46);
    let heights = array<f32, 4>(0.26, 0.20, 0.16, 0.14);
    let speeds = array<f32, 4>(0.050, -0.035, 0.028, -0.020);
    let phases = array<f32, 4>(0.0, 1.7, 3.1, 4.6);
    let tops = array<vec3<f32>, 4>(violet, indigo, teal, mix(violet, vec3<f32>(1.0), 0.2));
    let lows = array<vec3<f32>, 4>(teal, violet, indigo, teal);
    for (var k = 0u; k < 4u; k++) {
        let phase = phases[k];
        let speed = speeds[k];
        let wave = sin(u * 5.3 + t * speed * 6.0 + phase) * 0.045
                 + sin(u * 11.7 - t * speed * 9.0 + phase * 2.0) * 0.018
                 + sin(u * 2.1 + t * 0.03 + phase) * 0.03;
        let base = bases[k] + wave;
        let height = heights[k] * (0.75 + 0.25 * sin(u * 7.0 + phase + t * 0.1));
        let q = (base - h) / height;
        // Continuous version of the illustration's four vertex rows.
        let upper = 0.55 * clamp((1.0 - q) / 0.65, 0.0, 1.0);
        let lower = mix(1.0, 0.55, clamp(q / 0.35, 0.0, 1.0));
        let below = clamp(1.0 + (base - h) / 0.035, 0.0, 1.0);
        let brightness = select(select(upper, lower, q < 0.35), below, q < 0.0);
        let rays = 0.55 + 0.45 * pow(0.5 + 0.5 * sin(u * 38.0 + t * 0.35 + phase * 3.0), 1.5);
        let ends = pow(max(sin(clamp(u, 0.0, 1.0) * 3.14159265), 0.0), 0.6);
        let breathe = 0.8 + 0.2 * sin(t * 0.25 + phase + u * 3.0);
        let tint = mix(tops[k], lows[k], 0.5 + 0.5 * sin(u * 3.0 + phase + t * 0.05));
        c += mix(tint, lows[k], clamp(1.0 - q, 0.0, 1.0))
           * brightness * (0.34 - f32(k) * 0.05) * rays * ends * breathe;
    }
    // Low distant ridgelines, leaving the water and avatar unobstructed.
    for (var k = 0u; k < 2u; k++) {
        let seed = select(0.7, 2.9, k == 1u);
        let n = sin(u * 3.1 + seed) * 0.5 + sin(u * 7.3 + seed * 2.0) * 0.28
              + sin(u * 17.0 + seed * 3.0) * 0.12 + sin(u * 41.0 + seed) * 0.06;
        let ridge = 0.59 - 0.035 * (0.5 + 0.5 * n) + f32(k) * 0.025;
        let shade = select(mix(navy, indigo, 0.16), navy * 0.65, k == 1u);
        c = mix(c, shade, smoothstep(ridge - 0.002, ridge + 0.002, h));
    }
    return srgb_to_linear(c);
}

/// Linear HDR sky radiance for a direction, without clouds (reflections).
fn sky_color(dir: vec3<f32>) -> vec3<f32> {
    if (frame.studio_aurora[0].w > 0.0) {
        return studio_sky_color(dir);
    }
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
    if(frame.water_air.y > 0.5 || in_reflection_pass()) { return false; }
    let depth = frame.cam_up.w-frame.camera_pos.z;
    // Outside the bounded wake height the side is already known. Ordinary
    // above-water materials need no additional texture fetch for this test.
    if(abs(depth)>0.16 || !water_fx_on(16u)) { return depth>0.0; }
    return frame.camera_pos.z < camera_water_level();
}

// LLDrawPoolWaterExclusion: the nearest front-facing exclusion surface
// hides water only when it is in front of the nearest water interface.
fn water_excluded(world_pos:vec3<f32>) -> bool {
    let clip=world_clip(world_pos);
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

// A submerged surface's light `col` with the environment's share dimmed by
// the depth (water_scene_light): the sun, the sky, the ambient and the
// caustics come down from the surface and lose their wavelengths on the
// way. `own` is the share of `col` that shines from where it is, the local
// lights and the emissive: a lamp on a seabed 50 m down lights it in its
// own colour. What the camera's line of sight crosses is the fog's
// (apply_fog), for both. `own` is an estimate on some paths (a legacy
// material's environment mixes it away): never more than `col`, so a pixel
// the depth does not dim keeps `col` exactly.
fn water_scene_lit(col:vec3<f32>,own:vec3<f32>,world_pos:vec3<f32>)->vec3<f32> {
    let kept=clamp(own,vec3<f32>(0.0),max(col,vec3<f32>(0.0)));
    return (col-kept)*water_scene_light(world_pos)+kept;
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

// Whether a surface takes the water's fog in its own shading (apply_fog):
// everything seen from below the water, and from above what lies under it.
// The Legacy Viewer hazes the opaque scene by its own depth before the
// blended faces under the water (LLPipeline::renderGeomPostDeferred, "do
// water haze just before pre water alpha"; doWaterHaze,
// indra/newview/pipeline.cpp), and those faces fog themselves
// (applySkyAndWaterFog, class2/deferred/alphaF.glsl): each layer by its own
// depth, and the surface adds none (class3/environment/waterF.glsl). Aurora's
// surface used to fog its whole copy of the scene by the first depth under
// it: where a faint blended face lay just under the surface, the bed behind
// it lost its fog, and a sheet crossing the water plane showed its triangles.
fn water_fogs_source(world_pos: vec3<f32>) -> bool {
    return frame.water_air.y < 0.5 && (camera_underwater() || world_pos.z < camera_water_level());
}

// Density of that fog: the under-water one from below (and for the enhanced
// absorption, which is tuned on it), the plain one from above, as
// LLSettingsVOWater::getModifiedWaterFogDensity(underwater).
fn water_source_density() -> f32 {
    return select(frame.water_fog.w, frame.water_normal.w, camera_underwater() || water_fx_on(256u));
}

fn apply_fog(col: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    if (water_fogs_source(world_pos)) {
        if(!camera_underwater() && water_excluded(world_pos)){return apply_air_fog(col,world_pos);}
        let wf=water_fog_view(world_pos-frame.camera_pos.xyz,water_source_density());
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
// frame.light_mode.w: il_tex's alpha holds the share of the sky's ambient
// the traced probes let through (rt_gi.wgsl `cs_gi_filter`, `screen_sky`);
// the probe walk is in the bits under it
const TRACED_SKY: u32 = 0x100u;
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
// frame.light_mode.z: the occlusion's depth holds the blended faces opaque
// to the eye (renderer.rs, "occlusion depth"), which then take the
// screen-space terms, as the fainter ones lying on a solid surface do
// (screen_share; AURORA_BLEND_AO)
const BLEND_SCREEN_DEPTH: u32 = 0x10000u;

// frame.light_mode.z: geometric specular anti-aliasing of the objects
// (AURORA_SPECULAR_AA, specular_aa_roughness)
const SPECULAR_AA: u32 = 0x8000u;
// The point lights' share is traced (common.wgsl `local_share`).
const LOCAL_TRACED: u32 = 0x1000000u;
// With it: how many of the frame's first lights the share is of, less one,
// in these bits (Preferences' lamps, 1 to 64, rt_shadows.rs `traced_count`;
// the others are not shadowed), and whether the projectors' beams take it
// (`projector_at`). The bits above LOCAL_TRACED: those below are flags
// (types.rs `the_flags_of_the_light_mode_word_share_no_bit`).
const TRACED_LIGHTS_SHIFT: u32 = 25u;
const TRACED_LIGHTS_BITS: u32 = 0x3Fu;
const PROJECTORS_TRACED: u32 = 0x80000000u;

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
    let atlas = vec2<f32>(textureDimensions(shadow_map));
    let tile_scale = vec2<f32>(0.5, atlas.x / (2.0 * atlas.y));
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
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, (t + tile) * tile_scale, ndc.z - receiver_bias);
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

// How much of the contact shadows' mask the fragment being shaded takes: 1
// on the surface of the depth the mask was made from (the prepass, and the
// blended faces opaque to the eye drawn over it: blend_screen_share), 0 on
// a more transparent blended face, unless it lies on a solid surface: as
// `screen_share`, and for the same reason (the skin under a fading layer
// is in the shadow).
var<private> contact_share: f32 = 1.0;

// Whether the fragment being shaded takes the traced sun's glass with its
// mask (`sun_shadow`): yes on the surface the mask was traced for, no on a
// blended layer that only lies on it (`contact_share` from
// `on_occlusion_surface`). The glass in that surface's mask is mostly the
// layer itself: a sea's foam lying on the sand went dark in its own
// shadow (2026-11-22, grid report of beta6). As the Legacy Viewer, whose
// blended face is lit where its own texel casts in the shadow map, the
// sand under it in the shadow. Another pane's tint over the layer is lost
// with it.
var<private> contact_glass: bool = true;

// The sun's contact shadow at a pixel of the main view (contact_shadows.rs):
// what the shadow map is too coarse to show, taken as the darker of the
// two (sun_shadow). 1 elsewhere: other views, blended faces, off.
fn contact_shadow(frag: vec2<f32>) -> f32 {
    if (frame.contact.x < 0.5 || contact_share <= 0.0 || in_reflection_pass()) {
        return 1.0;
    }
    let p = vec2<i32>(frag);
    let size = vec2<i32>(textureDimensions(contact_tex));
    if (any(p < vec2<i32>(0)) || any(p >= size)) {
        return 1.0;
    }
    return 1.0 - contact_share * textureLoad(contact_tex, p, 0).r;
}

// What arrives of the point lights' and the projectors' red, green and
// blue at a pixel of the main view: the traced lights' tint (rt_lights.wgsl, `.gba` of the traced
// mask). 1 elsewhere: other views, without the traced lights.
fn local_share(frag: vec2<f32>) -> vec3<f32> {
    if (!local_traced(frag)) {
        return vec3<f32>(1.0);
    }
    let p = vec2<i32>(frag);
    let size = vec2<i32>(textureDimensions(contact_tex));
    if (any(p < vec2<i32>(0)) || any(p >= size)) {
        return vec3<f32>(1.0);
    }
    return textureLoad(contact_tex, p, 0).gba;
}

// Whether `local_share` is traced at this pixel, the projectors' beams'
// shadows with it (`projector_at`). On every face of the main view, a
// blended one and the snow included: such a face, which the mask was not
// traced for (`contact_share` 0), takes the share of the solid surface
// behind it. Unshadowed, the hair and veils of an avatar in a closed room
// took a lamp outside through the walls. The limit, chosen by the human
// (2026-11-22): a lit veil in front of a wall in a lamp's shadow is
// darkened with the wall. An Aurora difference: the Legacy Viewer shadows
// no point light, on any face.
fn local_traced(frag: vec2<f32>) -> bool {
    return (frame.light_mode.z & LOCAL_TRACED) != 0u && !in_reflection_pass();
}

// Whether light `i` of the frame is one the traced share is of.
fn light_traced(i: i32) -> bool {
    return u32(i) <= ((frame.light_mode.z >> TRACED_LIGHTS_SHIFT) & TRACED_LIGHTS_BITS);
}

// Point light `i` of the frame with its colour times `share` (local_share),
// if the share is of it.
fn shared_light(i: i32, share: vec3<f32>) -> Light {
    var lt = frame.lights[i];
    let s = select(vec3<f32>(1.0), share, light_traced(i));
    lt.color_falloff = vec4<f32>(lt.color_falloff.rgb * s, lt.color_falloff.w);
    return lt;
}

// The colour the glass gives the sunlight at this pixel, which `sun_shadow`
// reads with the traced shadow and the shading multiplies the sun by
// (`shade_pbr`, the classic sky's `ll_classic_sun_ambient`,
// `ll_classic_pbr`, a legacy face's highlights): 1 where the sun is not
// traced or meets no glass.
var<private> sun_glass: vec3<f32> = vec3<f32>(1.0);

// Whether this shader reads the soft shadows: the sun's mask and the
// projectors' filter. `true` here; while the option is off the scene's
// pipelines are compiled from this source with the line below set to `false`
// (renderer.rs `common_shader_for`), so the soft branches and their
// registers are not in the shaders of a frame that never takes them. The
// line is matched as it stands.
const SOFT_PATH: bool = true;

// The sun's soft shadow at a pixel of the main view (soft_shadows.wgsl): the
// light the mask lets through, or -1 where it does not hold for the surface
// being shaded: it was made for the surface of the depth prepass at that
// pixel, so it is read where the two are at the same distance from the
// camera only (a blended face in front of it, an MSAA sample of another
// surface, a reflection or a probe, which have `soft.x == 0`).
fn soft_shadow(world_pos: vec3<f32>, frag: vec2<f32>) -> f32 {
    // (a blended face outside the occlusion depth keeps the filter of before)
    if (!SOFT_PATH || frame.soft.x < 0.5 || screen_share <= 0.0) {
        return -1.0;
    }
    let p = vec2<i32>(frag);
    let size = vec2<i32>(textureDimensions(soft_tex));
    if (any(p < vec2<i32>(0)) || any(p >= size)) {
        return -1.0;
    }
    // the surface's distance as an f16 in the high 16 bits (negative: the
    // sky), the light in the low 10, as soft_shadows.wgsl `store` writes
    let texel = textureLoad(soft_tex, p, 0).x;
    let stored = unpack2x16float(texel >> 16u).x;
    let d = distance(world_pos, frame.camera_pos.xyz);
    if (stored < 0.0 || abs(stored - d) > max(0.01 * d, 0.02)) {
        return -1.0;
    }
    return f32(texel & 1023u) / 1023.0;
}

// The sun's shadow of an opaque surface of the main view: the ray-traced
// mask where the view traces it (rt_shadows.rs; the contact shadows are
// in it), else the shadow map (its soft mask where that holds) and the
// contact shadows where they are on; then the shadow intensity
// (Preferences, or the photo's).
fn sun_shadow(world_pos: vec3<f32>, n: vec3<f32>, vertex_depth: f32, frag: vec2<f32>) -> f32 {
    // the traced mask holds where the surface is the prepass's
    if (frame.contact.z > 0.5 && frame.params.z > 0.5 && contact_share > 0.0 && !in_reflection_pass()) {
        let p = vec2<i32>(frag);
        let size = vec2<i32>(textureDimensions(sun_glass_tex));
        if (contact_glass && all(p >= vec2<i32>(0)) && all(p < size)) {
            sun_glass = textureLoad(sun_glass_tex, p, 0).rgb;
        }
        let traced = contact_shadow(frag);
        // (as below: a classic sky lifts it in `classic_shadowed`)
        return select(traced, shadow_lift(traced), frame.sky_ll.x < 0.5);
    }
    var s = soft_shadow(world_pos, frag);
    if (s < 0.0) {
        s = shadow_factor(world_pos, n, vertex_depth);
    }
    if (frame.params.z < 0.5) {
        return s;
    }
    s = min(s, contact_shadow(frag));
    // (a classic sky's sun is min(N.L, shadow), where the lift would hide
    // under N.L: `classic_shadowed` and `ll_classic_pbr` lift it there)
    if (frame.sky_ll.x < 0.5) {
        s = shadow_lift(s);
    }
    return s;
}

// The shadow with the shadow intensity: the share of the light a shadow lets
// through, `frame.soft.y` (1 - intensity, 0 at full intensity: the shadow is
// then returned as it is, to the bit).
fn shadow_lift(s: f32) -> f32 {
    return s + (1.0 - s) * frame.soft.y;
}

// The Legacy `min(N.L, shadow)` of a classic sky's sun, with the shadow
// intensity: what the shadow takes of the sun's light, `nl - min(nl, shadow)`,
// is scaled by the intensity (1 - frame.soft.y), 0 lifts it away.
fn classic_shadowed(nl: f32, shadow: f32) -> f32 {
    let lit = min(nl, shadow);
    return lit + (nl - lit) * frame.soft.y;
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
    // A planar reflection (the mirror) reads the main view's cascades, fitted
    // around the real camera: the pixel's distance to that camera, its own
    // camera mirrored back through the clip plane, picks the cascade
    var view_depth = vertex_depth;
    let planar = in_reflection_pass() && dot(frame.clip_plane.xyz, frame.clip_plane.xyz) > 0.5;
    if (splits.w > 0.5 || planar) {
        var eye = frame.camera_pos.xyz;
        if (planar) {
            eye -= 2.0 * (dot(frame.clip_plane.xyz, eye) + frame.clip_plane.w) * frame.clip_plane.xyz;
        }
        view_depth = distance(world_pos, eye);
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
    // the mirror also sees behind the main camera, where the near cascades
    // do not reach: a coarser one that does
    if (planar) {
        for (var k = cascade + 1u; k < 3u && s < 0.0; k++) {
            s = cascade_shadow(world_pos, n, k);
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
// A blended face opaque to the eye is in that depth (BLEND_SCREEN_DEPTH)
// and takes them like an opaque one: a head whose halves differ in alpha
// mode no longer shows a line down the middle (#882). 1 on an opaque
// surface, 0 on a translucent face, blend_screen_share between; and on a
// blended face not yet solid, how much it lies on a solid surface
// (`on_occlusion_surface`) where that is more. A soft layer worn on a body
// (a tattoo, a blush, a second skin) fades out over the skin: taking none,
// over a skin that takes them, it lit the skin back up as far as the line
// where it enters that depth and stopped there, a hard edge in every
// crease with the occlusion on.
var<private> screen_share: f32 = 1.0;

// How far a faint blended fragment may be from the surface in the
// occlusion's depth and still take its screen-space terms whole (m), the
// larger of this and a hundredth of its distance from the camera; none from
// twice as far.
const SCREEN_LAYER_REACH: f32 = 0.03;

// How much a fragment `here` from the camera lies on the surface seen
// `there` in its pixel (m): 1 within the reach, falling smoothly to 0 at
// twice it, so that a layer leaving its surface shows no line either. A
// pane in front of a room is far from the wall behind it and takes none of
// the wall's occlusion, as before (#556).
fn layer_on_surface(here: f32, there: f32) -> f32 {
    let reach = max(SCREEN_LAYER_REACH, 0.01 * here);
    return 1.0 - smoothstep(reach, 2.0 * reach, abs(there - here));
}

// `layer_on_surface` of the fragment at `world_pos`, against the solid
// surface in its pixel (`solid_depth`).
fn on_occlusion_surface(frag: vec2<f32>, world_pos: vec3<f32>) -> f32 {
    let uv = screen_uv(frag);
    let size = vec2<i32>(textureDimensions(solid_depth));
    let px = clamp(vec2<i32>(uv * vec2<f32>(size)), vec2<i32>(0), size - vec2<i32>(1));
    let depth = textureLoad(solid_depth, px, 0);
    // (reversed depth: 0 is the sky)
    if (depth <= 0.0) {
        return 0.0;
    }
    // from the camera, by the view's rotation alone: region coordinates
    // lose their millimetres far from the region's origin
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let seen = frame.inv_view_proj_rot * vec4<f32>(ndc, depth, 1.0);
    return layer_on_surface(distance(world_pos, frame.camera_pos.xyz), length(seen.xyz / seen.w));
}

// The opacities from which a blended texel is opaque to the eye, and
// solid (the constants of these names in renderer.rs).
const SCREEN_BLEND_ALPHA: f32 = 0.88;
const SCREEN_SOLID_ALPHA: f32 = 0.95;

// How much of the screen-space terms (ambient occlusion, indirect light,
// contact shadows) a blended fragment of opacity `alpha` takes. They are
// made from a depth that holds a blended texel from an opacity of 0.88
// (SCREEN_BLEND_ALPHA), where the Legacy Viewer's shadows stop treating a
// face as semi-transparent (shadowAlphaMaskF.glsl): none under it, where
// they are those of what lies behind the face, all from 0.95
// (SCREEN_SOLID_ALPHA), and a ramp between. A step drew a ragged line along the texels of
// a glass texture whose opacity crosses it.
fn blend_screen_share(alpha: f32) -> f32 {
    return smoothstep(SCREEN_BLEND_ALPHA, SCREEN_SOLID_ALPHA, alpha);
}

// The share of the screen-space terms the fragment being shaded takes (see
// screen_share).
fn screen_terms() -> f32 {
    if ((frame.light_mode.z & BLEND_SCREEN_TERMS) != 0u) {
        return 1.0;
    }
    return screen_share;
}

fn screen_ao(frag: vec2<f32>) -> f32 {
    let share = screen_terms();
    if (frame.misc.x < 0.5 || in_reflection_pass() || share <= 0.0) {
        return 1.0;
    }
    let ao = max(textureSampleLevel(ao_tex, lin_clamp, screen_uv(frag), 0.0).r - surface_occlusion, 0.0);
    return mix(1.0, ao, share);
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
    let share = screen_terms();
    if ((frame.light_mode.z & SCREEN_INDIRECT) == 0u || in_reflection_pass() || share <= 0.0) {
        return vec3<f32>(0.0);
    }
    return textureSampleLevel(il_tex, lin_clamp, screen_uv(frag), 0.0).rgb * (share * screen_ao(frag));
}

// `screen_sky` and `screen_open` in a reflection pass, which has no screen
// of the traced light: how open the place the pass is drawn from is, for
// all its faces (rt_gi.wgsl `gi_points`: texel 0 a probe's capture, 1 the
// mirror). 1 in the passes that do not set TRACED_SKY (the water's
// reflection, the impostors, the HUD) and without the traced probes.
fn reflection_open() -> f32 {
    if ((frame.light_mode.w & TRACED_SKY) == 0u) {
        return 1.0;
    }
    let last = i32(textureDimensions(gi_open_tex).x) - 1;
    let i = min(select(1, 0, (frame.light_mode.z & PROBE_CAPTURE_DOME) != 0u), last);
    return clamp(textureLoad(gi_open_tex, vec2<i32>(i, 0), 0).r, 0.0, 1.0);
}

// The share of the sky's ambient that reaches the fragment at `frag`: what
// the traced global illumination's probes see of the sky there (gi.rs; a
// room, the shade of a roof or a tree), 1 without them and in reflection
// passes (the HUD is fullbright). A face the indirect light's depth does
// not hold (a blended one, the snow) takes the sky of what lies behind
// it, most often the same room. The light the surfaces around bounce stays the
// traced indirect light's (`screen_indirect`), so that none is counted
// twice. An Aurora difference, with the traced indirect light only: the
// Legacy Viewer's sky ambient reaches every place alike, darkened by its
// screen-space occlusion alone.
fn screen_sky(frag: vec2<f32>) -> f32 {
    if (in_reflection_pass()) {
        return reflection_open();
    }
    if ((frame.light_mode.w & TRACED_SKY) == 0u || (frame.light_mode.z & SCREEN_INDIRECT) == 0u) {
        return 1.0;
    }
    return clamp(textureSampleLevel(il_tex, lin_clamp, screen_uv(frag), 0.0).a, 0.0, 1.0);
}

// How open the place of the fragment at `frag` is, whatever its normal
// (gi_update.wgsl `gi_openness`): 1 out in the open, 0 in a closed room;
// 1 without the traced probes; in reflection passes `reflection_open`.
fn screen_open(frag: vec2<f32>) -> f32 {
    if (in_reflection_pass()) {
        return reflection_open();
    }
    if ((frame.light_mode.w & TRACED_SKY) == 0u || (frame.light_mode.z & SCREEN_INDIRECT) == 0u) {
        return 1.0;
    }
    return clamp(textureSampleLevel(gi_open_tex, lin_clamp, screen_uv(frag), 0.0).g, 0.0, 1.0);
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
// The local lights' share of the last shade_pbr (its point lights and
// projectors), for water_scene_lit. Returned aside rather than worked out
// again: a second walk of the lights costs as much as the first.
var<private> shade_pbr_local: vec3<f32> = vec3<f32>(0.0);

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
        col += diffuse_color * frame.sun_color.rgb * sun_glass * (nl * shadow / 3.0);
    } else if (nl > 0.0) {
        let h = normalize(l + v);
        let nh = max(dot(n, h), 0.0);
        let vh = max(dot(v, h), 0.0);
        let f = f_schlick(f0, vh);
        let spec = d_ggx(nh, a) * v_smith(nv, nl, a) * f;
        let kd = (vec3<f32>(1.0) - f) * diffuse_color / PI;
        col += (kd + spec) * frame.sun_color.rgb * sun_glass * nl * shadow;
    }
    // local lights
    var local = vec3<f32>(0.0);
    var lights = light_walk(world_pos, frag);
    var spots = projector_walk(lights);
    let share = local_share(frag);
    for (var i = next_light(&lights); i >= 0; i = next_light(&lights)) {
        let lt = shared_light(i, share);
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
                local += (kd + spec) * lt.color_falloff.rgb * lnl * atten;
            }
        }
    }
    for (var i = next_light(&spots); i >= 0; i = next_light(&spots)) {
        local += pbr_local_light(projector_at(i, world_pos, n, frag), n, v, f0, a, nv, diffuse_color);
    }
    col += local;
    shade_pbr_local = local;
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
    // the sky's reflection, where no reflection probe gives the place's,
    // takes the traced probes' sight of the sky as the ambient does
    var env_of_sky = true;
    if (probes_on()) {
        // non classic skies: ambient from the probes' irradiance along the
        // normal, each probe mixed with the sky ambient by its ambiance
        // (sampleProbeAmbient), and pbrIbl radiance from the probes (ambient
        // occlusion only), in one walk of the probes
        let s = sample_probes_pair(probe_point(world_pos, n), frag, ProbeLookup(n, 0.0, true), ProbeLookup(r, roughness * probe_max_lod(), false), !legacy, true, hemi);
        if (s.a.w > 0.0) {
            amb = s.a.rgb * probe_pass_ambiance();
        }
        if (s.b.w > 0.0) {
            env = s.b.rgb * probe_pass_radiance();
            occl = spec_ao;
            env_of_sky = false;
        }
    }
    let sky = screen_sky(frag);
    if (env_of_sky) {
        occl *= sky;
    }
    col += diffuse_color * amb * ao * mat_ao * sky;
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
        let c = world_clip(p);
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
                    let mc = world_clip(mp);
                    let mn = mc.xyz / mc.w;
                    let muv = vec2<f32>(mn.x * 0.5 + 0.5, 0.5 - mn.y * 0.5);
                    if (scene_depth_at(muv) > mn.z) {
                        hi = mid;
                        huv = muv;
                    } else {
                        lo = mid;
                    }
                }
                let conf = ssr_edge_fade(huv) * (1.0 - f32(i) / f32(steps));
                let col = textureSampleLevel(scene_tex, lin_clamp, huv, 0.0).rgb;
                return vec4<f32>(col, conf);
            }
        }
        prev_t = t;
        t = t * 1.18 + 0.1;
    }
    return vec4<f32>(0.0);
}

// Pixels of a photo tile's inner edges over which a reflection's hit fades:
// past them the ray finds nothing, the scene beyond is not drawn. Within
// the guard band (at least 128 pixels, `snapshot::TILE_GUARD`), so the
// pixels the tile keeps read their hits whole.
const SSR_TILE_FADE: f32 = 64.0;

// How much of a reflection's hit at `uv` is kept: it fades over the last
// tenth of the image towards its edges. A photo tile fades at the photo's
// edges, as the photo in one pass does, and at its inner edges only over
// `SSR_TILE_FADE`: fading over a tenth of the tile took the reflections
// out along every seam.
fn ssr_edge_fade(uv: vec2<f32>) -> f32 {
    if (frame.photo.z <= 0.0) {
        let edge = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
        return clamp(edge * 10.0, 0.0, 1.0);
    }
    let p = frame.photo.xy + uv * frame.photo.zw;
    let edge = min(min(p.x, 1.0 - p.x), min(p.y, 1.0 - p.y));
    // in this tile's pixels from each side; a side on the photo's edge
    // fades with the photo
    let px = uv * frame.screen.xy;
    let to_end = frame.screen.xy - px;
    var inner = 1.0;
    if (frame.photo.x > 1e-6) {
        inner = min(inner, px.x / SSR_TILE_FADE);
    }
    if (frame.photo.y > 1e-6) {
        inner = min(inner, px.y / SSR_TILE_FADE);
    }
    if (frame.photo.x + frame.photo.z < 1.0 - 1e-6) {
        inner = min(inner, to_end.x / SSR_TILE_FADE);
    }
    if (frame.photo.y + frame.photo.w < 1.0 - 1e-6) {
        inner = min(inner, to_end.y / SSR_TILE_FADE);
    }
    return clamp(edge * 10.0, 0.0, 1.0) * clamp(inner, 0.0, 1.0);
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
    let loc = tex_loc.values[slot];
    return f32(textureNumLevels(tex_pages[texture_page_index(loc)])) - 1.0;
}

// The projected texture at `lod`, linear, kept inside the image (the
// atlas sampler repeats; LL clamps).
fn projector_sample(slot: u32, tc: vec2<f32>, lod: f32) -> vec4<f32> {
    let loc = tex_loc.values[slot];
    let size = vec2<f32>(textureDimensions(tex_pages[texture_page_index(loc)]));
    let half_texel = 0.5 * exp2(ceil(lod)) / size;
    let uv = clamp(vec2<f32>(tc.x, 1.0 - tc.y), half_texel, vec2<f32>(1.0) - half_texel);
    let c = textureSampleLevel(tex_pages[texture_page_index(loc)], tex_sampler, uv, loc & 0xffffu, lod);
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

// Points of a disk of radius 1, spread evenly (soft_shadows.wgsl DISK).
const SOFT_DISK: array<vec2<f32>, 32> = array(
    vec2<f32>(0.176776695, 0.000000000), vec2<f32>(-0.176776695, 0.000000000),
    vec2<f32>(-0.225772188, 0.206825818), vec2<f32>(0.225772188, -0.206825818),
    vec2<f32>(0.034558052, -0.393771179), vec2<f32>(-0.034558052, 0.393771179),
    vec2<f32>(0.284571220, 0.371172764), vec2<f32>(-0.284571220, -0.371172764),
    vec2<f32>(-0.522223187, -0.092373929), vec2<f32>(0.522223187, 0.092373929),
    vec2<f32>(0.494695392, -0.314684715), vec2<f32>(-0.494695392, 0.314684715),
    vec2<f32>(-0.165465927, 0.615525001), vec2<f32>(0.165465927, -0.615525001),
    vec2<f32>(-0.315561468, -0.607594404), vec2<f32>(0.315561468, 0.607594404),
    vec2<f32>(0.684642162, 0.250030219), vec2<f32>(-0.684642162, -0.250030219),
    vec2<f32>(-0.712256086, 0.294008958), vec2<f32>(0.712256086, -0.294008958),
    vec2<f32>(0.343354499, -0.733728620), vec2<f32>(-0.343354499, 0.733728620),
    vec2<f32>(0.253730241, 0.808931990), vec2<f32>(-0.253730241, -0.808931990),
    vec2<f32>(-0.764745892, -0.443185876), vec2<f32>(0.764745892, 0.443185876),
    vec2<f32>(0.897133984, -0.197232390), vec2<f32>(-0.897133984, 0.197232390),
    vec2<f32>(-0.547506905, 0.778772232), vec2<f32>(0.547506905, -0.778772232),
    vec2<f32>(-0.126486773, -0.976089697), vec2<f32>(0.126486773, 0.976089697)
);

// Soft shadow of projector map `k` on the point `q` (1 lit, 0 shadowed),
// main view only (frame.soft.z, the light's radius in metres): percentage-
// closer soft shadows with the map of a perspective projector. A search
// of the map around the point finds the casters and their mean distance
// from the projector, the light's size gives the penumbra by similar
// triangles, and a filter of 12 taps of that width takes the light; 1 when
// the search finds no caster.
// `uv`, `receiver`: the point in the map and its depth; `z`: its distance
// along the projector's axis; `origin`, `half`, `dims`, `texel`: the map in
// the atlas, in pixels (projector_shadow).
//
// The stored depth is a + b / z (a, b from the matrix), so a caster's
// distance is b / (stored - a): gaps and penumbrae are in metres, never an
// average of stored depths. The depth of the receiver's plane across the
// map (from two steps along it) keeps a slanted face from shadowing itself
// over the wide filter.
fn projector_soft(k: u32, q: vec3<f32>, n: vec3<f32>, uv: vec2<f32>, receiver: f32, z: f32,
    origin: vec2<f32>, half: f32, dims: vec2<f32>, texel: f32) -> f32 {
    let m = frame.spot_vp[k];
    let row_x = vec3<f32>(m[0].x, m[1].x, m[2].x);
    let row_y = vec3<f32>(m[0].y, m[1].y, m[2].y);
    let row_z = vec3<f32>(m[0].z, m[1].z, m[2].z);
    let row_w = vec3<f32>(m[0].w, m[1].w, m[2].w);
    let a = dot(row_z, row_w) / max(dot(row_w, row_w), 1e-12);
    let b = m[3].z - a * m[3].w;
    // map units across one metre at one metre from the apex, each way
    let per_metre = 0.5 * vec2<f32>(length(row_x), length(row_y));
    // the receiver's plane: depth change per map unit, from two steps of 5 cm
    let tangent = normalize(cross(n, select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.z) > 0.9)));
    let bitangent = cross(n, tangent);
    var step_uv = array<vec2<f32>, 2>(vec2<f32>(0.0), vec2<f32>(0.0));
    var step_z = vec2<f32>(0.0);
    for (var i = 0u; i < 2u; i++) {
        let lp = m * vec4<f32>(q + select(bitangent, tangent, i == 0u) * 0.05, 1.0);
        let ndc = lp.xyz / max(lp.w, 1e-6);
        step_uv[i] = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5) - uv;
        step_z[i] = ndc.z - receiver;
    }
    let det = step_uv[0].x * step_uv[1].y - step_uv[0].y * step_uv[1].x;
    var gradient = vec2<f32>(0.0);
    if (abs(det) > 1e-9 * texel) {
        gradient = vec2<f32>(
            step_uv[1].y * step_z.x - step_uv[0].y * step_z.y,
            step_uv[0].x * step_z.y - step_uv[1].x * step_z.x) / det;
    }
    let light = frame.soft.z;
    // the widest half width of the filter at the receiver: 25 light radii,
    // between 25 cm and 1 m (the sun search radius, soft_shadows.rs)
    let cap = clamp(25.0 * light, 0.25, 1.0);
    let search = min(per_metre * cap / z, vec2<f32>(0.1));
    var ratio = 0.0;
    var blockers = 0.0;
    for (var i = 0u; i < 9u; i++) {
        var o = vec2<f32>(0.0);
        if (i > 0u) {
            o = SOFT_DISK[((i - 1u) * 32u) / 8u] * search;
        }
        let local = clamp(uv + o, vec2<f32>(0.5 * texel), vec2<f32>(1.0 - 0.5 * texel));
        let at = vec2<i32>(floor(origin + local * half));
        let stored = textureLoad(shadow_map, at, 0);
        let centre = (vec2<f32>(at) + 0.5 - origin) / half;
        if (stored < 1.0 && stored < receiver + dot(gradient, centre - uv) - 0.0002) {
            let caster = b / (stored - a);
            if (caster > 0.0 && caster < z) {
                ratio += (z - caster) / caster;
                blockers += 1.0;
            }
        }
    }
    // no caster within the search: lit
    if (blockers == 0.0) {
        return 1.0;
    }
    let width = min(light * ratio / blockers, cap);
    let radius = max(per_metre * width / z, vec2<f32>(1.5 * texel));
    var sum = 0.0;
    for (var i = 0u; i < 12u; i++) {
        let local = clamp(uv + SOFT_DISK[(i * 32u) / 12u] * radius, vec2<f32>(0.5 * texel), vec2<f32>(1.0 - 0.5 * texel));
        sum += textureSampleCompareLevel(shadow_map, shadow_sampler, (origin + local * half) / dims,
            receiver + dot(gradient, local - uv) - 0.0002);
    }
    return sum / 12.0;
}

// Light left by the shadow map of projector `i` (1: lit, or no map), as
// spotLightF.glsl with sampleSpotShadow / pcfSpotShadow (shadowUtil.glsl):
// the configured budget extends the Legacy Viewer's two-map limit without
// reducing each map's resolution (projector_shadows.rs). These shadows stop
// with the sun's (the last tenth of its shadow distance).
fn projector_shadow(i: i32, world_pos: vec3<f32>, n: vec3<f32>) -> f32 {
    let slot = frame.projectors[i].tex.y;
    if (frame.params.z < 0.5 || slot == 0u) {
        return 1.0;
    }
    let k = slot - 1u;
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
    let dims = vec2<f32>(textureDimensions(shadow_map));
    let size = dims.x * 0.5;
    let half = size * 0.5;
    var origin: vec2<f32>;
    if (k < 4u) {
        origin = vec2<f32>(size + f32(k % 2u) * half, size + f32(k / 2u) * half);
    } else {
        origin = vec2<f32>(f32((k - 4u) % 4u) * half, 2.0 * size + f32((k - 4u) / 4u) * half);
    }
    var s = -1.0;
    if (SOFT_PATH && frame.soft.z > 0.0) {
        s = projector_soft(k, world_pos + n * 0.04, n, uv, ndc.z, lp.w, origin, half, dims, texel);
    }
    if (s < 0.0) {
        var sum = 0.0;
        // pcfSpotShadow's five taps
        let taps = array<vec2<f32>, 5>(
            vec2<f32>(0.0, 0.0), vec2<f32>(2.0, 1.5), vec2<f32>(1.0, -1.5), vec2<f32>(-1.0, 1.5), vec2<f32>(-2.0, -1.5));
        for (var t = 0; t < 5; t++) {
            let at = clamp(uv + taps[t] * texel, vec2<f32>(0.5 * texel), vec2<f32>(1.0 - 0.5 * texel));
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, (origin + at * half) / dims, ndc.z - 0.0002);
        }
        s = sum * 0.2;
    }
    // the map fades in and out as its projector takes or loses it
    s = clamp(s + frame.spot_fade[k / 4u][k % 4u], 0.0, 1.0);
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

// Projector `i` of the frame's list on a point of normal `n`, seen at
// pixel `frag`.
fn projector_at(i: i32, world_pos: vec3<f32>, n: vec3<f32>, frag: vec2<f32>) -> LightAt {
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
    // the beam is shadowed, the ambiance is not (spotLightF): by the traced
    // lights' tint where it is traced (rt_lights.wgsl, which draws the
    // projectors with the point lights), by its shadow map elsewhere
    var beam = vec3<f32>(1.0);
    if (local_traced(frag) && (frame.light_mode.z & PROJECTORS_TRACED) != 0u && light_traced(i)) {
        beam = local_share(frag);
        at.shadow = dot(beam, vec3<f32>(0.2126, 0.7152, 0.0722));
    } else {
        at.shadow = projector_shadow(i, world_pos, n);
        beam = vec3<f32>(at.shadow);
    }
    at.color = lt.color_falloff.rgb * c.rgb * c.a * beam;
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
    return ll_atmos_core(rel_in, vec3<f32>(0.0, 0.0, 1.0), frame.sky_light.xyz, frame.sun_dir.xyz);
}

// calcAtmosphericVars as hazeF.glsl runs it over a surface: the point in
// eye space (rebuilt from the depth), `lightnorm` in world GL axes
// (LLEnvironment::toLightNorm: x north, y up, z east) and `light_dir` in eye
// space. The haze glow's first factor thus mixes eye and world axes: its
// hotspot is not along the sun for rays down to the water, where the
// world-space ll_atmos put a second sun on the sea (the Legacy Viewer shows
// none). `rel_world` = point minus camera (world, z up).
fn ll_atmos_eye(rel_world: vec3<f32>) -> LLAtmos {
    let forward = normalize((frame.inv_view_proj_rot * vec4<f32>(0.0, 0.0, 0.5, 1.0)).xyz);
    let up = normalize(frame.cam_up.xyz - forward * dot(frame.cam_up.xyz, forward));
    let right = cross(forward, up);
    let ln = frame.sky_light.xyz;
    return ll_atmos_core(
        ll_eye_axes(rel_world, right, up, forward),
        vec3<f32>(0.0, 1.0, 0.0),
        vec3<f32>(ln.y, ln.z, ln.x),
        ll_eye_axes(frame.sun_dir.xyz, right, up, forward),
    );
}

// A world vector in eye axes (x right, y up, z backward).
fn ll_eye_axes(v: vec3<f32>, right: vec3<f32>, up: vec3<f32>, forward: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(v, right), dot(v, up), -dot(v, forward));
}

// The body of calcAtmosphericVars: `rel_in` the point in the frame of
// `up_axis` (the altitude limit and the direction's nudge), `glow_norm` and
// `glow_light` the two vectors of the haze glow.
fn ll_atmos_core(rel_in: vec3<f32>, up_axis: vec3<f32>, glow_norm: vec3<f32>, glow_light: vec3<f32>) -> LLAtmos {
    var rel_pos = rel_in;
    let max_y = frame.sky_glow.w;
    let rel_up = dot(rel_pos, up_axis);
    if (abs(rel_up) > max_y) {
        rel_pos *= max_y / abs(rel_up);
    }
    let rel_pos_norm = normalize(rel_pos + up_axis * 1e-6);
    let rel_pos_len = length(rel_pos);
    let blue_density = frame.sky_blue_density.rgb;
    let haze_density = frame.sky_blue_density.w;
    let density_multiplier = frame.sky_ambient.w;
    let lightnorm = frame.sky_light.xyz;
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
    var haze_glow = dot(rel_pos_norm, glow_norm);
    haze_glow *= max(0.0, dot(glow_light, rel_pos_norm));
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
    if (colored_sky_ambient()) {
        o.amblit = max(tmp_ambient, vec3<f32>(0.0)) * 0.7;
    }
    o.additive = min(additive * (vec3<f32>(1.0) - atten), vec3<f32>(10.0));
    o.atten = atten;
    return o;
}

// The experimental colored sky ambient: the first PBR release lit with the
// sky's ambient colour x RenderSkyAmbientScale 0.7, neither softened (pow
// 0.9, x 0.57) nor turned grey in linear mode (atmosphericsFuncs.glsl
// calcAtmosphericVarsLinear, settings.xml, tag
// Second_Life_Release#843866d-DRTVWR-559). It had no classic branch: a
// classic sky keeps its colour either way and only takes the 0.7 curve.
fn colored_sky_ambient() -> bool {
    return frame.sky_obj_light.w > 0.5;
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
    // (and the traced probes' sight of the sky, `screen_sky`)
    return mix(effect, irr, ao) * screen_sky(frag);
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
        let sunlit = a.sunlit * 1.35 * sun_glass;
        let irr = ll_ssao(a.amblit * ll_ambient_lighting(n), frag);
        let sun_contrib = vec3<f32>(classic_shadowed(pow(nl_sun, 1.2), shadow));
        // GTAO's indirect light (Aurora's own: the Legacy Viewer has none)
        return (srgb_to_linear(irr * 0.9 + ll_linear_to_srgb(sun_contrib) * sunlit * 0.7) + screen_indirect(frag)) * base;
    }
    // linear mode: grey sky ambient, linear sunlight, plain lambert
    let amb_lin = srgb_to_linear(a.amblit * ll_ambient_lighting(n));
    let amb_grey = vec3<f32>(dot(amb_lin, vec3<f32>(0.2126, 0.7152, 0.0722)));
    let irr = ll_ssao(select(amb_grey, amb_lin, colored_sky_ambient()), frag);
    return (irr + classic_shadowed(nl_sun, shadow) * srgb_to_linear(a.sunlit) * sun_glass + screen_indirect(frag)) * base;
}

// Local lights on a classic surface (multiPointLightF): light color x n.l x
// attenuation x albedo, with its final_scale.
fn ll_local_diffuse(base: vec3<f32>, n: vec3<f32>, world_pos: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    var local = vec3<f32>(0.0);
    var lights = light_walk(world_pos, frag);
    var spots = projector_walk(lights);
    let share = local_share(frag);
    for (var i = next_light(&lights); i >= 0; i = next_light(&lights)) {
        let lt = shared_light(i, share);
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
        local += ll_diffuse_light(projector_at(i, world_pos, n, frag), n, base);
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
        let sun_contrib = srgb_to_linear(ll_linear_to_srgb(vec3<f32>(classic_shadowed(pow(nl, 1.2), shadow))) * sunlit * 0.7) * PI;
        final_sun = clamp(sun_contrib * (diff + spec) * sun_glass * shadow_lift(shadow), vec3<f32>(0.0), vec3<f32>(10.0));
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
    // (the sky's reflection, where no reflection probe gives the place's,
    // takes the traced probes' sight of the sky, as `shade_pbr`'s)
    let sky_env = mix(sky_color(vec3<f32>(r.x, r.y, max(r.z, -0.2))), hemi, roughness) * screen_sky(frag);
    var env = probe_radiance(probe_point(world_pos, n), frag, r, roughness, sky_env);
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
    let share = local_share(frag);
    for (var i = next_light(&lights); i >= 0; i = next_light(&lights)) {
        let lt = shared_light(i, share);
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
        col += ll_pbr_light(projector_at(i, world_pos, n, frag), n, v, f0, ra, nv, diffuse_color);
    }
    return col * ll_local_scale();
}

// Local water controls and history. Opaque GTAO depth is never overwritten.
struct WaterFxControls { area: vec4<f32>, previous: vec4<f32>, clock: vec4<f32>, counts: vec4<u32>, sea_area: vec4<f32>, sea: WaterSea, fine: vec4<f32> };
struct WaterFxEvent { position: vec4<f32>, motion: vec4<f32>, params: vec4<f32> };
@group(0) @binding(15) var water_field: texture_2d<f32>;
@group(0) @binding(16) var<storage, read> water_events: array<WaterFxEvent>;
@group(0) @binding(17) var<uniform> water_fx: WaterFxControls;
@group(0) @binding(18) var water_ground: texture_2d<f32>;
@group(0) @binding(19) var water_light: texture_2d<f32>;
// the wind sea's finer maps (water_ocean_gpu.rs): per cascade its
// displacement and slopes, then the foam; `fine` in water_fx says whether
// they hold this frame's sea and their tiles
@group(0) @binding(33) var water_fine: texture_2d_array<f32>;
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
// One texel of the sea grid (water_effects.rs `WaterGround::sea`, 128 a
// side under the 64 bathymetry rows): x the wave depth, y the shelter
// packed (`pack_shelter`: calm in 1/255, clearance in centimetres), z the
// main swell's refraction (its travel's offset from the plane wave, m), w
// its swell reach and chop packed. Decoded before any mix:
// [0] depth, calm, travel, swell reach; [1].x clearance, .y chop.
fn water_fx_sea_texel(c: vec2<i32>) -> array<vec4<f32>,2> {
    let t=textureLoad(water_ground,c+vec2<i32>(0,64),0);
    // the previous grid, all of it, fading out (WaterGround::sea_prev, the
    // sea's shore.x the fade): its refraction offset, swell reach and chop,
    // its depth and its shelter (a prim entering or leaving the shelter
    // calmed or roughened a patch of sea at once, a pop)
    let old=textureLoad(water_ground,c+vec2<i32>(0,192),0);
    let blend=water_fx.sea.shore.x;
    let high=floor(t.y/256.0);
    let old_high=floor(old.w/256.0);
    let calm=mix((old.w-old_high*256.0)/255.0,(t.y-high*256.0)/255.0,blend);
    let clear=mix(old_high/100.0,high/100.0,blend);
    let now=water_fx_exposure(t.w);let was=water_fx_exposure(old.y);
    let exposure=mix(was,now,blend);
    return array<vec4<f32>,2>(vec4<f32>(mix(old.z,t.x,blend),calm,mix(old.x,t.z,blend),exposure.x),vec4<f32>(clear,exposure.y,0.0,0.0));
}
// A cell's swell reach and chop (`pack_exposure`: 1/255 each).
fn water_fx_exposure(packed: f32) -> vec2<f32> {
    let high=floor(packed/256.0);
    return vec2<f32>(high/255.0,(packed-high*256.0)/255.0);
}
struct WaterSeaCell { depth: f32, shelter: vec2<f32>, travel: f32, shadow: f32, chop: f32 };
// The sea grid at `p`, bilinear; outside it, its nearest edge fading into
// open sea over 120 m, the swell's travel going on as a plane wave from the
// edge's (`WaterGround::sea_cell`, EDGE_FADE).
fn water_fx_sea_cell(p: vec2<f32>) -> WaterSeaCell {
    let lo=water_fx.sea_area.xy;let extent=max(water_fx.sea_area.z,1.0);
    let inside=clamp(p,lo,lo+vec2<f32>(extent));
    let pos=clamp((inside-lo)/extent*128.0-0.5,vec2<f32>(0.0),vec2<f32>(127.0));
    let i=vec2<i32>(floor(pos));let j=min(i+vec2<i32>(1),vec2<i32>(127));let f=fract(pos);
    let t00=water_fx_sea_texel(i);let t10=water_fx_sea_texel(vec2<i32>(j.x,i.y));
    let t01=water_fx_sea_texel(vec2<i32>(i.x,j.y));let t11=water_fx_sea_texel(j);
    let a=mix(mix(t00[0],t10[0],f.x),mix(t01[0],t11[0],f.x),f.y);
    let b=mix(mix(t00[1],t10[1],f.x),mix(t01[1],t11[1],f.x),f.y);
    let clear=b.x;
    let out=smoothstep(0.0,120.0,distance(p,inside));
    var cell:WaterSeaCell;
    cell.depth=mix(a.x,20.0,out);
    cell.shelter=mix(vec2<f32>(a.y,clear),vec2<f32>(1.0,6.0),out);
    // the plane wave along the swell now, and the grid's refraction offset,
    // its edge's going on past the grid (faded out there, tens of metres of
    // the swell's delay were squeezed into the fade: a line of short waves
    // all round the grid, some 256 m out)
    cell.travel=dot(p-lo,water_fx.sea.trains[0].xy)+a.z;
    cell.shadow=mix(a.w,1.0,out);
    cell.chop=mix(b.y,1.0,out);
    return cell;
}
// The spectrum's wind sea at `p` (water_fft.rs `OceanMaps::sample`): per
// cascade, its maps in the bathymetry texture from row 320 (displacement
// xyz and ∂Dx/∂y, then the slopes and ∂Dx/∂x, ∂Dy/∂y, then the lingering
// foam), bilinear and wrapping over the cascade's tile. Where a pixel spans
// `footprint` m, a cascade's slopes and folds fade before they alias; the
// long chop's foam does not (a smooth field: kept, the whitecaps vanished
// toward the horizon and at grazing angles, where pixels are long).
// (`fine`: the surface's own reading, whose short cascade's slope comes
// from the GPU's finer maps when they are on: water_fx_fine)
// Each cascade is taken by its share where the sea is read as `at`
// (water_ocean_weights): the open sea's two, or the lakes' chop over a
// short fetch (the third cascade, `sea.lake`).
fn water_fx_ocean(p: vec2<f32>, footprint: f32, fine: bool, at: WaterSeaAt) -> WaterOceanAt {
    var o=WATER_OCEAN_NONE;
    if(water_fx.sea.shore.w<=0.0){return o;}
    let shares=water_ocean_weights(at.chop);
    for(var c=0;c<3;c++){
        let w=select(shares.x,shares.y,c==2);
        if(w<=0.0 || (c==2 && water_fx.sea.lake.x<=0.0)){continue;}
        let tile=select(select(water_fx.sea.shore.y,water_fx.sea.shore.z,c==1),water_fx.sea.lake.x,c==2);
        let pos=fract(p/tile)*128.0;
        let base=floor(pos);let f=pos-base;
        let i=vec2<i32>(base)%vec2<i32>(128);let j=(i+vec2<i32>(1))%vec2<i32>(128);
        let row=320+c*384;
        let d=mix(mix(textureLoad(water_ground,vec2<i32>(i.x,row+i.y),0),textureLoad(water_ground,vec2<i32>(j.x,row+i.y),0),f.x),
            mix(textureLoad(water_ground,vec2<i32>(i.x,row+j.y),0),textureLoad(water_ground,vec2<i32>(j.x,row+j.y),0),f.x),f.y);
        let s=mix(mix(textureLoad(water_ground,vec2<i32>(i.x,row+128+i.y),0),textureLoad(water_ground,vec2<i32>(j.x,row+128+i.y),0),f.x),
            mix(textureLoad(water_ground,vec2<i32>(i.x,row+128+j.y),0),textureLoad(water_ground,vec2<i32>(j.x,row+128+j.y),0),f.x),f.y);
        // (the long chop's shortest waves are 10 m, the ripples' and the
        // lakes' chop's ~2 m on the whole: a pixel past a fraction of them
        // aliases them)
        let k=select(2.5,0.63,c==0);
        let att=(1.0-smoothstep(0.5,2.0,k*footprint))*w;
        o.offset+=d.xyz*w;
        if(!(fine && c==1 && water_fine_on())){o.slope+=s.xy*att;}
        o.jacobian+=vec3<f32>(s.z,d.w,s.w)*att;
        let foam=mix(mix(textureLoad(water_ground,vec2<i32>(i.x,row+256+i.y),0).x,textureLoad(water_ground,vec2<i32>(j.x,row+256+i.y),0).x,f.x),
            mix(textureLoad(water_ground,vec2<i32>(i.x,row+256+j.y),0).x,textureLoad(water_ground,vec2<i32>(j.x,row+256+j.y),0).x,f.x),f.y);
        o.foam=max(o.foam,foam*select(att,w,c==0));
    }
    return o;
}
// One map of a cascade (`row`: its first row) at `q` in tiles, bilinear and
// wrapping.
fn water_fx_ocean_tap(q:vec2<f32>,row:i32)->vec4<f32> {
    let pos=fract(q)*128.0;
    let base=floor(pos);let f=pos-base;
    let i=vec2<i32>(base)%vec2<i32>(128);let j=(i+vec2<i32>(1))%vec2<i32>(128);
    return mix(mix(textureLoad(water_ground,vec2<i32>(i.x,row+i.y),0),textureLoad(water_ground,vec2<i32>(j.x,row+i.y),0),f.x),
        mix(textureLoad(water_ground,vec2<i32>(i.x,row+j.y),0),textureLoad(water_ground,vec2<i32>(j.x,row+j.y),0),f.x),f.y);
}
// Enhanced water's surface height at `p` over the region water at `level`
// (as vs_water draws it: the waves at the place they moved here from, one
// step back by their sway; flat away from the region's water, as
// particle_water_level). For what is cut at the surface itself.
fn water_surface_z(p:vec2<f32>,level:f32)->f32 {
    if(!water_fx_on(1u)){return level;}
    let bed=water_fx_bed(p);
    let region=(1.0-smoothstep(0.25,0.5,abs(level-bed.y)))*smoothstep(0.08,2.4,level-bed.x);
    if(region<=0.0){return level;}
    let at=water_fx_sea_at(p);
    let q=p-water_sea(p,water_fx.clock.x,water_fx.counts.w,water_fx.sea,at,water_fx_ocean(p,0.0,false,at),0.0).offset.xy;
    let at_q=water_fx_sea_at(q);
    return level+water_sea(q,water_fx.clock.x,water_fx.counts.w,water_fx.sea,at_q,water_fx_ocean(q,0.0,false,at_q),0.0).offset.z*region;
}
// The GPU's finer maps of the wind sea at `p` (water_ocean_gpu.rs: the
// short cascade's waves four times finer than the CPU's, and the finest
// ripples, some 4 cm to 0.43 m), filtered over the pixel's footprint
// (`gx`, `gy`: the world position's screen derivatives) through their mips
// and the material sampler (repeating, anisotropic). Each cascade is read
// through its own horizontal displacement (`chop`, as the surface takes
// the maps here), so the ripples' crests pinch sharp and their troughs
// open wide: the jagged facets that break the sun into glints. `slope`
// both cascades' slopes (the short one's replaces the CPU's, read with it
// in water_fx_ocean), `foam` the short cascade's lingering foam, `kept`
// how much of the finest ripples the pixel still resolves, `fold` how much
// the short cascade alone pinches the surface together here (1 - its
// displacement's Jacobian, filtered over the pixel: the crests of the small
// wind waves, steep however small they are, where the sea's whole fold
// takes the long swell with it and seldom shows them).
struct WaterFine { slope: vec2<f32>, foam: f32, kept: f32, fold: f32 };
fn water_fine_on() -> bool { return water_fx.fine.x > 0.5; }
// (`p` where the water lies before the waves move it: water_pixel's
// `flow`. The short cascade is in the surface's shape already, read there
// as it is; the finest, not in it, through its own displacement.)
// (`open`: the open sea's share where the water is read, water_ocean_weights:
// the short cascade is the open sea's, and over a short fetch gives way to
// the lakes' chop, read with the CPU's maps; the finest ripples, the
// wind's, stay on any water)
fn water_fx_fine(p:vec2<f32>,gx:vec2<f32>,gy:vec2<f32>,chop:f32,open:f32)->WaterFine {
    var o=WaterFine(vec2<f32>(0.0),0.0,0.0,0.0);
    if(!water_fine_on()){return o;}
    let short=chop*open;
    for(var c=0;c<2;c++){
        let tile=select(water_fx.fine.y,water_fx.fine.z,c==1);
        let uv=p/tile;let du=gx/tile;let dv=gy/tile;
        var at=uv;
        if(c==1){at=uv-textureSampleGrad(water_fine,tex_sampler,uv,2,du,dv).xy*chop/tile;}
        let s=textureSampleGrad(water_fine,tex_sampler,at,2*c+1,du,dv);
        o.slope+=s.xy*select(short,chop,c==1);
        if(c==0){
            // its Jacobian (∂Dx/∂x, ∂Dy/∂y in the slopes' zw, ∂Dx/∂y in the
            // displacement's w), as water_ocean.wgsl's foam takes it; the
            // finest ripples are capillary and do not break white
            let dxy=textureSampleGrad(water_fine,tex_sampler,at,0,du,dv).w*short;
            o.fold=1.0-((1.0+s.z*short)*(1.0+s.w*short)-dxy*dxy);
        }
    }
    o.foam=textureSampleGrad(water_fine,tex_sampler,p/water_fx.fine.y,4,gx/water_fx.fine.y,gy/water_fx.fine.y).x*short;
    o.kept=1.0-smoothstep(0.02,0.1,max(length(gx),length(gy)));
    return o;
}
// Where the sea is read at `p` (`WaterGround::sea_point`): the wave depth
// and the swell's travel with their gradients two cells either side, the
// shelter, and the main swell's phase, wave vector and shadow there. What
// every sampler of the sea reads.
fn water_fx_sea_at(p: vec2<f32>) -> WaterSeaAt {
    let c=water_fx_sea_cell(p);
    // (two cells either side: a smoother way for the swell)
    let h=max(water_fx.sea_area.z/64.0,0.25);
    let e=water_fx_sea_cell(p+vec2<f32>(h,0.0));let w=water_fx_sea_cell(p-vec2<f32>(h,0.0));
    let n=water_fx_sea_cell(p+vec2<f32>(0.0,h));let s=water_fx_sea_cell(p-vec2<f32>(0.0,h));
    let k0=water_fx.sea.trains[0].z;let d0=water_fx.sea.trains[0].xy;
    var at:WaterSeaAt;
    at.depth=c.depth;
    at.slope=vec2<f32>(e.depth-w.depth,n.depth-s.depth)/(2.0*h);
    at.shelter=clamp(c.shelter.x,0.0,1.0);
    at.cap=water_sea_cap(water_fx.sea,c.shelter.y);
    // the travel's gradient: the way the swell goes, and how much shorter
    // it is than in open water (1 to 6 times)
    let grad=vec2<f32>(e.travel-w.travel,n.travel-s.travel)/(2.0*h);
    let len=length(grad);
    at.wave=select(d0,grad/max(len,1e-4)*clamp(len,1.0,6.0),len>0.2)*k0;
    at.phase=k0*(c.travel+dot(water_fx.sea_area.xy-water_fx.sea.frame.xy,d0));
    at.shadow=c.shadow;
    at.chop=c.chop;
    // the real bed under the fine bathymetry round the camera, the sea
    // grid's depth past it (faded over its last 12 %: the swash once
    // switched on and off at a shore as the camera moved)
    let bed=water_fx_bed(p);
    let uv=water_fx_uv(p);
    let inside=smoothstep(0.0,0.12,min(min(uv.x,uv.y),min(1.0-uv.x,1.0-uv.y)));
    at.bed=mix(c.depth,bed.y-bed.x,inside);
    return at;
}
// Coarse height/velocity/foam above, fine slope/height/foam below. The
// atlas keeps the existing scene bindings and sampler count unchanged.
fn water_atlas_uv(uv:vec2<f32>)->vec2<f32> {
    let pixel=0.5/f32(textureDimensions(water_field).x);
    let clamped=clamp(uv,vec2<f32>(pixel),vec2<f32>(1.0-pixel));
    return clamped*vec2<f32>(1.0,0.5);
}
fn water_wake_sample(p:vec2<f32>,level:f32)->vec4<f32> {
    if(!water_fx_on(16u)){return vec4<f32>(0.0);}
    return water_wake_at(p,level,water_fx_bed(p).y);
}
// The same, for a caller that already read the water's own level at `p`
// (water_fx_bed(p).y).
fn water_wake_at(p:vec2<f32>,level:f32,native_level:f32)->vec4<f32> {
    if(!water_fx_on(16u) || abs(level-native_level)>0.2){return vec4<f32>(0.0);}
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
    var held=1.0;
    if(water_fx_on(1u)) {
        let at=water_fx_sea_at(p);
        // The swell's geometry flattens 280-310 m out: past 320 m a water
        // patch is one quad (scene/water.rs `draws`, WAVE_GRID_REACH), and a
        // displaced grid meeting it left a wall of water along the patch's
        // edge. The far water keeps the swell in its normals. (Flattened
        // from 130 m, the sea turned to a flat sheet at a visible line.)
        let reach=1.0-smoothstep(280.0,310.0,distance(p,frame.camera_pos.xy));
        offset+=water_sea(p,frame.camera_pos.w,water_fx.counts.w,water_fx.sea,at,water_fx_ocean(p,0.0,false,at),0.0).offset*reach;
        held=at.cap;
    }
    // (the wake's ripples stay under a floor at the water too)
    offset.z+=clamp(water_wake_sample(p+offset.xy,level).z,-0.16,0.16)*held;
    return offset;
}
// `swell` is the caller's: the gradient of water_sea at `p` (shore handled),
// which the surface reads once for its normal, its wake and its foam.
// `ground` the bed under it: (the wind's sea meets the shore itself, in
// water_sea: shoaling, breaking, swash; no fade of its slope here)
fn water_fx_normal(p:vec2<f32>,level:f32,wake:vec2<f32>,swell:vec2<f32>,ground:f32)->vec3<f32> {
    var gradient=vec2<f32>(0.0);
    if(water_fx_on(1u)) {
        gradient=swell;
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
// The second reading of the caustics map: turned 37 degrees and 1.618 times
// larger (an irrational ratio), so its 16 m tile never lines up with the
// first one's.
const WATER_CAUSTIC_TURN:mat2x2<f32>=mat2x2<f32>(0.7986355,0.6018150,-0.6018150,0.7986355);
const WATER_CAUSTIC_TILE2:f32=16.0*1.618034;
// The map's focus at `uv` (in tiles) for a point `depth` under the water:
// its three layers (0.75, 2.5 and 7 m, water_caustics.wgsl) mixed by depth.
fn water_caustic_tap(uv:vec2<f32>,depth:f32)->f32 {
    let focus=textureSampleLevel(water_light,lin_clamp,fract(uv),0.0).rgb;
    if(depth>2.5){return mix(focus.g,focus.b,smoothstep(2.5,7.0,depth));}
    return mix(focus.r,focus.g,smoothstep(0.75,2.5,depth));
}
// The first reading's focus `a` broken up by the second one's `b`: the
// lines of one stay where the other is bright too, so the network has no
// period, and the gain keeps their brightness over the threshold of
// water_caustic_light about where it was.
fn water_caustic_combine(a:vec3<f32>,b:f32)->vec3<f32> {
    return mix(a,min(a,vec3<f32>(b))*1.5,0.7);
}
// The light the water focuses on a submerged point, before the sun's shadow.
// A surface that already looked its shadow up multiplies by it (the terrain);
// water_caustics looks it up for the others.
// The map is one 16 m tile of a few sine waves (water_micro): stamped as it
// is, its interference reads as a grid. It is read twice, the second time
// turned and larger (WATER_CAUSTIC_TURN), through a slow warp of the
// lookup, and red and blue a little apart along the sun's refracted ray
// (the water's dispersion, more with the depth).
// They follow the sea above (water_sea.rs `caustic_focus`, `caustic_drift`):
// on glassy water, broad soft light wandering slowly, in patches where a
// breath of wind ripples it; under a breeze, the sharp net, drifting
// downwind; in a gale, a quick, torn, dimmer light that the swell sways.
// Behind a reef or in a short fetch, the water is as calm as it is there.
fn water_caustic_light(pos:vec3<f32>,normal:vec3<f32>)->vec3<f32> {
    if(!water_fx_on(32u) || frame.misc.w>0.5){return vec3<f32>(0.0);}
    if(!camera_underwater() && water_excluded(pos)){return vec3<f32>(0.0);}
    let bed=water_fx_bed(pos.xy);let depth=bed.y-pos.z;
    if(depth<0.03 || depth>18.0 || frame.sky_sun.z<0.05 || frame.sun_dir.w<0.01){return vec3<f32>(0.0);}
    let light=normalize(frame.sun_dir.xyz);
    let ray=refract(-light,vec3<f32>(0.0,0.0,1.0),0.7501875);
    var project=pos.xy-ray.xy*depth/max(-ray.z,0.2);
    let sea=water_fx.sea;
    let at=water_fx_sea_at(project);
    // (the spectrum's chop left out: the micro waves' map draws the short
    // caustics, and its texture reads would cost every lit pixel underwater)
    project-=water_sea(project,frame.camera_pos.w,water_fx.counts.w,sea,at,WATER_OCEAN_NONE,0.0).gradient*min(depth,7.0)*0.25;
    // how rough the water is above: the wind's, as much as the fetch and
    // what shelters it leave
    let rough=clamp(sea.frame.w*clamp(at.chop,0.0,1.0)*clamp(at.shelter,0.0,1.0),0.0,1.0);
    // the ripples' drift downwind (m), and a slow wander of its own
    let time=frame.camera_pos.w;
    let drift=sea.look.zw+vec2<f32>(time*0.011,-time*0.007);
    // about 11 m across, torn further as the sea roughens
    let warp_at=(project-drift)*0.09;
    project+=(vec2<f32>(water_fx_noise(warp_at),water_fx_noise(warp_at+vec2<f32>(4.7,3.1)+vec2<f32>(-drift.y,drift.x)*0.03))-0.5)*(1.8+3.2*rough);
    let uv=project/16.0;
    let second=water_caustic_tap((WATER_CAUSTIC_TURN*(project-drift*0.35))/WATER_CAUSTIC_TILE2,depth);
    var focus:vec3<f32>;
    if(water_fx.counts.w==0u){focus=vec3<f32>(water_caustic_tap(uv,depth));}
    else{
        // about 9 mm per metre of depth between red and blue, along the
        // sun's direction (less under a high sun)
        let spread=ray.xy/max(length(ray.xy),0.35)*(min(depth,7.0)*0.009/16.0);
        focus=vec3<f32>(water_caustic_tap(uv+spread,depth),
            (water_caustic_tap(uv+WATER_CAUSTIC_FILTER,depth)+water_caustic_tap(uv-WATER_CAUSTIC_FILTER,depth))*0.5,
            water_caustic_tap(uv-spread,depth));
    }
    // a rough sea's light is less focused (whitecaps, bubbles, a torn
    // surface): a lower threshold, a lower gain
    let threshold=0.9-0.08*rough;
    var energy=max(water_caustic_combine(focus,second)-vec3<f32>(threshold),vec3<f32>(0.0))*(0.42*(1.0-0.2*rough));
    // patches some 30 m across drifting downwind, where gusts ripple the
    // water more than beside them: plain on calm water, less in a gale
    let gusty=smoothstep(0.15,0.85,water_fx_noise((project-drift*1.6)*0.034+vec2<f32>(9.2,1.7)));
    energy*=mix(1.0,0.3+1.4*gusty,0.65-0.35*rough);
    let extinction=exp(-depth*(0.13+frame.water_fog.w*0.025));
    return frame.sun_color.rgb*energy*extinction*max(dot(normal,light),0.0)*frame.sun_dir.w*smoothstep(0.03,0.3,depth);
}
fn water_caustics(pos:vec3<f32>,normal:vec3<f32>)->vec3<f32> {
    let focused=water_caustic_light(pos,normal);
    // no focused light here, or a face turned away from the sun: nothing for
    // the shadow to dim, it is not looked up
    if(max(focused.r,max(focused.g,focused.b))<=0.0){return vec3<f32>(0.0);}
    return focused*shadow_lift(shadow_factor(pos,normal,distance(pos,frame.camera_pos.xyz)));
}

fn water_fx_noise(p: vec2<f32>) -> f32 {
    let i=floor(p);let f=fract(p);let w=f*f*(3.0-2.0*f);
    return mix(mix(water_fx_hash(i).x,water_fx_hash(i+vec2<f32>(1.0,0.0)).x,w.x),
        mix(water_fx_hash(i+vec2<f32>(0.0,1.0)).x,water_fx_hash(i+vec2<f32>(1.0,1.0)).x,w.x),w.y);
}

// The swash's foam (0..1): a flat, marbled lace over the whole swash, as
// a breaking wave leaves it. Dense as the swash rushes up the beach, it
// thins into a net of lines as it drains back, and it rides with the
// water: up the beach with the front, dragged back seaward with it. The
// water's surface and the backwash's sheet on the sand draw the same lace
// at the same place, so as the water sinks under the sand its foam goes on
// pulling back and fades there. `at` the sea read here, `theta` the swash's
// phase (water_swash_shift added), `reach` how high it runs.
fn water_swash_foam(p: vec2<f32>, at: WaterSeaAt, theta: f32, reach: f32) -> f32 {
    let fall=length(at.slope);
    let beach=water_swash_beach(at);
    if(beach<0.01 || reach<0.02){return 0.0;}
    let down=at.slope/fall;
    let rise=water_swash_rise(theta).x;
    let s=fract(-theta/6.2831853);
    // how much: building as it runs up, thinning to a lace as it drains
    let u=clamp((s-0.3)/0.7,0.0,1.0);
    let amount=select(0.15+0.85*smoothstep(0.0,0.3,s),1.0-0.85*u*u,s>0.3)*smoothstep(0.03,0.2,reach);
    // where the water carried it: the lace slides seaward as the front
    // falls back, landward as it runs up
    let slid=min(reach*(1.0-rise)/max(fall,0.03),20.0);
    let q=p-down*slid;
    // the foam texture's lace, when there is one (water_foam_lace; the
    // footprint from the distance, at about a 1080p view's)
    if(water_foam_textured()){
        let lace=water_foam_lace(q,distance(p,frame.camera_pos.xy)*0.0012+0.002);
        return water_foam_lace_cover(amount,lace)*clamp(amount*1.3,0.0,1.0)*beach;
    }
    // a marbled network: warped noise folded into ridges
    let warp=vec2<f32>(water_fx_noise(q*0.35),water_fx_noise(q*0.35+vec2<f32>(5.2,1.3)))-0.5;
    let r=q+warp*3.0;
    let n=water_fx_noise(r*0.9)*0.65+water_fx_noise(r*2.3+vec2<f32>(3.7,8.1))*0.35;
    let ridge=1.0-abs(n-0.5)*2.0;
    // much foam fills the cells (a sheet with holes), little leaves lines
    let edge=1.0-amount*0.85;
    return smoothstep(edge,edge+0.15,ridge)*clamp(amount*1.3,0.0,1.0)*beach;
}
// Continuous torn coverage and filtered fine grain, without a lattice of holes.
// The pattern at a point (x the torn structure, y the grain) does not depend
// on how much foam there is: the surface works it out once for the foam of
// its field and the foam of its wakes, and only where one of them shows.
fn water_foam_pattern(p:vec2<f32>,footprint:f32)->vec2<f32> {
    let drift=p-vec2<f32>(frame.camera_pos.w*0.09,frame.camera_pos.w*0.035);
    let warp=vec2<f32>(water_fx_noise(drift*1.3),water_fx_noise(drift*1.3+7.4));
    let q=drift+(warp-0.5)*0.7;
    let rotated=vec2<f32>(q.x*0.8-q.y*0.6,q.x*0.6+q.y*0.8);
    let medium=mix(water_fx_noise(rotated*3.7),0.5,smoothstep(0.04,0.18,footprint));
    let fine=mix(water_fx_noise(q*11.3),0.5,smoothstep(0.012,0.07,footprint));
    let structure=water_fx_noise(q*0.9)*0.55+medium*0.3+fine*0.15;
    let grain=mix(0.84,0.5+0.5*water_fx_noise(rotated*38.0),1.0-smoothstep(0.004,0.025,footprint));
    return vec2<f32>(smoothstep(0.28,0.72,structure),grain);
}
fn water_foam_coverage(amount:f32,pattern:vec2<f32>)->f32 {
    if(amount<0.08){return 0.0;}
    return smoothstep(0.08,0.75,amount)*pattern.x*pattern.y;
}
// The foam texture (scene/water_foam.rs: "Foam 003", a lace of sea foam
// and its thickness), when the viewer has it: its bindless slot rides in
// `water_fx.fine.w`.
fn water_foam_textured() -> bool { return water_fx.fine.w > 0.5; }
const WATER_FOAM_TURN:mat2x2<f32>=mat2x2<f32>(0.7986355,0.6018150,-0.6018150,0.7986355);
// The lace at `p` for a pixel spanning `footprint` m: read twice, on tiles
// of 3.1 m and 5.3 m turned 37° against each other (so neither repeats
// where it shows) through a slow warp, drifting with the wind; mipmapped
// by the footprint. x the lace, y its thickness, z how much of it the
// pixel still resolves (past some 20 cm a pixel holds whole cells, and the
// mips' averages are not a lace to cut).
fn water_foam_lace(p:vec2<f32>,footprint:f32)->vec3<f32> {
    let slot=u32(water_fx.fine.w);
    // (`p` where the water lies before the waves move it: the lace rides
    // with it; drifting only slowly downwind, as the surface's water does)
    let drift=p-water_fx.sea.wind.xy*frame.camera_pos.w*(0.02+0.004*water_fx.sea.wind.z);
    let warp=vec2<f32>(water_fx_noise(drift*0.21),water_fx_noise(drift*0.21+vec2<f32>(5.2,1.3)))-0.5;
    let q=drift+warp*1.4;
    let a=sample_tex_level(slot,q/3.1,log2(max(footprint*1024.0/3.1,1.0)));
    let b=sample_tex_level(slot,(WATER_FOAM_TURN*q)/5.3,log2(max(footprint*1024.0/5.3,1.0)));
    return vec3<f32>(max(a.r,b.r*0.85),max(a.a,b.a*0.85),1.0-smoothstep(0.04,0.25,footprint));
}
// The same on the water's surface, where the pixel's derivatives of `p`
// are known (`gx`, `gy`, m per pixel across and down the screen): each
// tile read through the anisotropic sampler along them. One mip for a
// pixel's mean size kept the lace sharp along the view across a sloping
// sea, where a pixel spans many of its cells: its holes and edges then
// stepped from pixel to pixel, and crawled as the waves moved.
fn water_foam_lace_grad(p:vec2<f32>,gx:vec2<f32>,gy:vec2<f32>)->vec3<f32> {
    let slot=u32(water_fx.fine.w);
    let drift=p-water_fx.sea.wind.xy*frame.camera_pos.w*(0.02+0.004*water_fx.sea.wind.z);
    let warp=vec2<f32>(water_fx_noise(drift*0.21),water_fx_noise(drift*0.21+vec2<f32>(5.2,1.3)))-0.5;
    let q=drift+warp*1.4;
    let a=sample_tex_grad(slot,q/3.1,gx/3.1,gy/3.1);
    let b=sample_tex_grad(slot,(WATER_FOAM_TURN*q)/5.3,(WATER_FOAM_TURN*gx)/5.3,(WATER_FOAM_TURN*gy)/5.3);
    let footprint=sqrt(max(length(gx)*length(gy),1e-10));
    return vec3<f32>(max(a.r,b.r*0.85),max(a.a,b.a*0.85),1.0-smoothstep(0.04,0.25,footprint));
}
// How much of the lace `amount` of foam shows (0..1): thick foam fills its
// cells, thin foam leaves only its strands, and fades as it thins (foam
// left behind by a crest dissolves as it lingers, water_fft.rs `linger`,
// instead of holding its strands and then vanishing); far off, its
// average. Under it a soft white veil in the foam's own shape, from which
// the lace stands out where the foam is dense (Sea of Thieves blurs its
// foam mask for it: Ang et al., SIGGRAPH 2018); our amount is already a
// smooth field, read here as it is.
fn water_foam_lace_cover(amount:f32,lace:vec3<f32>)->f32 {
    if(amount<0.04){return 0.0;}
    let veil=smoothstep(0.04,0.7,amount)*0.24;
    // (soft edges, and the far average about the near lace's: a harder cut
    // drew paper cut-outs, a fuller average greyed a gale's whole sea)
    let edge=1.0-clamp(amount,0.0,1.0)*0.85;
    let body=smoothstep(edge-0.1,edge+0.24,lace.y*0.7+lace.x*0.4);
    // (faint foam shows as fewer strands, not a fainter sheet: drawn half
    // see-through, the thin foam a turned-up Foam slider brings out behind
    // the crests read as a grey fog)
    let near=body*mix(0.55,1.0,lace.x)*smoothstep(0.04,0.15,amount);
    return max(mix(clamp(amount,0.0,1.0)*0.45,near,lace.z),veil);
}
