// Aurora Weather's fog volume (fog.rs), after common.wgsl: a grid of
// froxels on the camera's frustum, its slices spaced exponentially in depth
// (fog.rs `grid`), worked out in two compute passes before scene pass A.
//
// `inject`: per froxel, at a point jittered in depth each frame, the fog's
// density (the ground density, its height above the ground it pools on, the
// noise the wind carries, less under cover) and the light it scatters
// toward the camera: the sky's, the sun's through its shadow map (god rays)
// and, at the top tier, the local lights' (Henyey-Greenstein phase). Blended
// with the last frame's value at the same place (reprojected): the jitter
// averages out into a smooth volume.
//
// `integrate`: front to back along each column, the light gathered and the
// transmittance up to each slice's far side (Hillaire's energy conserving
// step, "Physically Based and Unified Volumetric Rendering in Frostbite",
// SIGGRAPH 2015). The scene reads it (common.wgsl `weather_fog`).
//
// `fs_fog_sky`: in scene pass A after the sky and the clouds, the sky seen
// through the volume and the height fog beyond it, blended `dst * T + L`.

struct FogParams {
    // positions relative to this frame's camera to the last frame's clip space
    prev_vp: mat4x4<f32>,
    // xyz the camera's axis; w the frame's jitter in depth (0..1)
    forward: vec4<f32>,
    // froxels x, y, z; w the frame's number
    grid: vec4<u32>,
    // x the share of this frame in the history (1: none kept), y 1 = the
    // local lights scatter, z 1 for a still (its froxels sampled at their
    // centres), w unused
    look: vec4<f32>,
    // xyz the noise's offset (m: the grid anchor less the drift, and the
    // slow rise), w how much it shapes the fog (0..1)
    noise: vec4<f32>,
    // xy the ground map's corner, z its side (m; 0: none)
    ground: vec4<f32>,
    // the fog's flow grid (fog.rs `Flow`): xyz its corner (m), w a cell's
    // side (0: none, the shelter map alone says what is covered)
    flow: vec4<f32>,
    // its cells x, y, z; w the stamp of the walls drawn last
    flow_dims: vec4<u32>,
}

// A step of the flow: xyz how many cells the grid moved since the last
// frame (the first step of a frame; 0 for the others), w 1 when the grid was
// placed afresh (nothing of the last frame's flow is read: it starts empty).
struct FlowStep {
    shift: vec4<i32>,
}

@group(1) @binding(0) var<uniform> fp: FogParams;
@group(1) @binding(1) var fog_noise: texture_3d<f32>;
@group(1) @binding(2) var fog_repeat: sampler;
@group(1) @binding(3) var fog_ground: texture_2d<f32>;
@group(1) @binding(4) var fog_history: texture_3d<f32>;
@group(1) @binding(5) var fog_scatter_out: texture_storage_3d<rgba16float, write>;
@group(1) @binding(6) var fog_scatter_in: texture_3d<f32>;
@group(1) @binding(7) var fog_integrated_out: texture_storage_3d<rgba16float, write>;
@group(1) @binding(8) var fog_flow: texture_3d<f32>;
@group(1) @binding(10) var fog_solid: texture_3d<u32>;
@group(1) @binding(11) var fog_flow_prev: texture_3d<f32>;
@group(1) @binding(12) var fog_flow_out: texture_storage_3d<rgba16float, write>;
@group(1) @binding(13) var<uniform> fog_step: FlowStep;
// The opaque walls at twice the cells' size (1 if any of the 2 x 2 x 2 cells
// under one has an opaque wall of the last drawing): written by
// `walls_coarse`, read by `fog_light_seen`.
@group(1) @binding(14) var fog_walls_out: texture_storage_3d<r32uint, write>;
@group(1) @binding(15) var fog_walls: texture_3d<u32>;

// How far the fog reaches into what covers it from the open (m): it is 1/e
// of itself that far under a porch or into a room from its doorway.
const FOG_FLOW_REACH: f32 = 3.0;

// The noise's tile (m): the base octave spans it (fog.rs NOISE_PERIOD).
const FOG_NOISE_PERIOD: f32 = 96.0;
// The local lights' scattering, against the sun's: a lamp lights the fog
// all around it, not only toward a camera that looks into it.
const FOG_LIGHT_GAIN: f32 = 2.5;
// The share of the local lights' scattering that goes every way (a halo
// seen from the side), the rest leaning as the sun's.
const FOG_LIGHT_EVEN: f32 = 0.5;

// A local light's phase: part even, part Henyey-Greenstein's.
fn fog_light_phase(c: f32, g: f32) -> f32 {
    return mix(fog_phase(c, g), 1.0 / (4.0 * PI), FOG_LIGHT_EVEN);
}

// Depth (m, along the camera's axis) at slice coordinate `w` (0 the near
// end, 1 the far end).
fn fog_slice_depth(w: f32) -> f32 {
    return frame.fog_look[2].x * exp(w / frame.fog_look[3].x);
}

// The world direction through the volume's column at `uv`.
fn fog_column_dir(uv: vec2<f32>) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let p = frame.inv_view_proj_rot * vec4<f32>(ndc, 1.0, 1.0);
    return normalize(p.xyz / p.w);
}



// The terrain or the water under `xy` (fog_ground.rs, its raw heights,
// bilinear between its 4 m cells): far below everything away from the map.
fn fog_ground_raw(xy: vec2<f32>) -> f32 {
    let side = fp.ground.z;
    if (side <= 0.0) { return -1e9; }
    let uv = (xy - fp.ground.xy) / side;
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) { return -1e9; }
    let size = i32(textureDimensions(fog_ground).x);
    let pos = clamp(uv * f32(size) - 0.5, vec2<f32>(0.0), vec2<f32>(f32(size - 1)));
    let i = vec2<i32>(floor(pos));
    let j = min(i + vec2<i32>(1), vec2<i32>(size - 1));
    let f = fract(pos);
    let a = textureLoad(fog_ground, i, 0).x;
    let b = textureLoad(fog_ground, vec2<i32>(j.x, i.y), 0).x;
    let c = textureLoad(fog_ground, vec2<i32>(i.x, j.y), 0).x;
    let d = textureLoad(fog_ground, j, 0).x;
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// The ground the fog pools on under `xy` (fog_ground.rs): its smoothed
// level, or the camera's away from the map.
fn fog_ground_level(xy: vec2<f32>) -> f32 {
    let side = fp.ground.z;
    if (side <= 0.0) { return frame.fog_look[0].z; }
    let uv = (xy - fp.ground.xy) / side;
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) { return frame.fog_look[0].z; }
    let size = i32(textureDimensions(fog_ground).x);
    let pos = clamp(uv * f32(size) - 0.5, vec2<f32>(0.0), vec2<f32>(f32(size - 1)));
    let i = vec2<i32>(floor(pos));
    let j = min(i + vec2<i32>(1), vec2<i32>(size - 1));
    let f = fract(pos);
    let a = textureLoad(fog_ground, i, 0).y;
    let b = textureLoad(fog_ground, vec2<i32>(j.x, i.y), 0).y;
    let c = textureLoad(fog_ground, vec2<i32>(i.x, j.y), 0).y;
    let d = textureLoad(fog_ground, j, 0).y;
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// The sun's shadow at `p`, `dist` metres from the camera: one tap of the
// cascade that holds it (the jitter and the history soften it), 1 past
// the shadows' reach.
fn fog_sun_shadow(p: vec3<f32>, dist: f32) -> f32 {
    if (frame.params.z < 0.5) { return 1.0; }
    let splits = frame.cascade_splits;
    if (dist > splits.z) { return 1.0; }
    var c = 0u;
    if (dist > splits.x) { c = 1u; }
    if (dist > splits.y) { c = 2u; }
    let m = frame.cascade_vp[c];
    let lp = m * vec4<f32>(p, 1.0);
    let ndc = lp.xyz / lp.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || ndc.z < 0.0 || ndc.z > 1.0) { return 1.0; }
    let atlas = vec2<f32>(textureDimensions(shadow_map));
    let texel = 2.0 / atlas.x;
    let tile = vec2<f32>(f32(c % 2u), f32(c / 2u));
    let tile_scale = vec2<f32>(0.5, atlas.x / (2.0 * atlas.y));
    let t = clamp(uv, vec2<f32>(0.5 * texel), vec2<f32>(1.0 - 0.5 * texel));
    // a texel of the cascade along the light, as `cascade_shadow`'s bias
    let radius = 1.0 / max(length(vec3<f32>(m[0].x, m[1].x, m[2].x)), 1e-6);
    let bias = 2.0 * texel * radius * length(vec3<f32>(m[0].z, m[1].z, m[2].z));
    let s = textureSampleCompareLevel(shadow_map, shadow_sampler, (t + tile) * tile_scale, ndc.z - bias);
    return shadow_lift(s);
}

// How open the air at `p` is to the outside, 0..1: the flow grid's (the
// fog flowing in from the open sky, around walls, fading under cover), and
// past it the shelter map's (open sky or not), blended over its last
// metres.
fn fog_open(p: vec3<f32>) -> f32 {
    let sky = 1.0 - fog_sheltered(p);
    let cell = fp.flow.w;
    if (cell <= 0.0) { return sky; }
    let size = vec3<f32>(fp.flow_dims.xyz) * cell;
    // (half a cell up: a floor's cell is solid, and read at the point the
    // fog lying on it took half of that; a ceiling lets it a quarter of a
    // metre further down)
    let c = (p + vec3<f32>(0.0, 0.0, cell * 0.5) - fp.flow.xyz) / size;
    if (any(c <= vec3<f32>(0.0)) || any(c >= vec3<f32>(1.0))) { return sky; }
    let flow = textureSampleLevel(fog_flow, lin_clamp, c, 0.0).x;
    // within 2 m of the grid's sides, toward the shelter map's
    let edge = min(min(c, vec3<f32>(1.0) - c) * size, vec3<f32>(2.0));
    return mix(sky, flow, min(min(edge.x, edge.y), edge.z) * 0.5);
}

// The fog's density at `p` (1/m): its ground density, thinning with the
// height above the ground it pools on, shaped by the noise the wind
// carries (more ragged toward its top), as much as the air there is open.
// `footprint` is the froxel's size there (m): each octave of the noise is
// read at the level as coarse as it (fog.rs `noise_levels`), so that far
// froxels, metres wide, see its wisps averaged and not one point of them
// that slides as the camera turns.
fn fog_density(p: vec3<f32>, open: f32, footprint: f32) -> f32 {
    let base = fog_ground_level(p.xy);
    let share = fog_height_share(p.z, base);
    let q = p + fp.noise.xyz;
    let texels = f32(textureDimensions(fog_noise).x);
    let big_level = max(log2(footprint * texels / FOG_NOISE_PERIOD), 0.0);
    let big = textureSampleLevel(fog_noise, fog_repeat, q / FOG_NOISE_PERIOD, big_level).x;
    // (a quarter of the tile: both octaves repeat on the tile, so a wrap of
    // the anchor, the drift or the rise by a tile shows nothing)
    let small = textureSampleLevel(fog_noise, fog_repeat, q / (FOG_NOISE_PERIOD * 0.25) + vec3<f32>(0.37, 0.11, 0.71), big_level + 2.0).x;
    let n = big * 0.7 + small * 0.3;
    // the noise erodes the fog more where it is thin: wisps at its top,
    // a thick layer at its foot
    let erosion = mix(0.15, 0.55, 1.0 - share);
    let shape = clamp((n - erosion) / (1.0 - erosion) * 1.8, 0.0, 2.0);
    let d = frame.fog_look[0].x * share * mix(1.0, shape, fp.noise.w);
    return d * open;
}

// The walls stop a local light's light in the fog: the way from `p` to the
// light at `l`, walked through the coarse walls (`fog_walls`, 1 m cells) in
// steps shorter than one, up to a metre short of the light (its own prim, a
// lamp's shade, is a wall of the grid too). 0 when an opaque wall stands
// between them; 1 past the grid (no walls known there) or with none.
const FOG_WALL_STEP: f32 = 0.9;
const FOG_WALL_STEPS: i32 = 12;
const FOG_LIGHT_CLEAR: f32 = 1.0;

fn fog_light_seen(p: vec3<f32>, l: vec3<f32>) -> f32 {
    let cell = fp.flow.w * 2.0;
    if (cell <= 0.0) { return 1.0; }
    let to = l - p;
    let d = length(to);
    let reach = d - FOG_LIGHT_CLEAR;
    if (reach <= FOG_WALL_STEP * 0.5) { return 1.0; }
    let dims = vec3<i32>(fp.flow_dims.xyz / 2u);
    let step = to / d * FOG_WALL_STEP;
    var q = p + step * 0.5;
    let n = min(i32(ceil(reach / FOG_WALL_STEP)), FOG_WALL_STEPS);
    for (var k = 0; k < n; k++) {
        let c = vec3<i32>(floor((q - fp.flow.xyz) / cell));
        if (all(c >= vec3<i32>(0)) && all(c < dims) && textureLoad(fog_walls, c, 0).x != 0u) {
            return 0.0;
        }
        q += step;
    }
    return 1.0;
}

// The coarse walls of the last drawing (`fog_light_seen`): a cell is a wall
// if one of its 2 x 2 x 2 fine cells holds an opaque face of it.
@compute @workgroup_size(4, 4, 4)
fn walls_coarse(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id >= fp.flow_dims.xyz / 2u)) { return; }
    let walls = fp.flow_dims.w * 2u + 1u;
    var any_wall = 0u;
    for (var k = 0u; k < 8u; k++) {
        let c = vec3<i32>(id * 2u + vec3<u32>(k & 1u, (k >> 1u) & 1u, k >> 2u));
        if (textureLoad(fog_solid, c, 0).x == walls) { any_wall = 1u; }
    }
    textureStore(fog_walls_out, id, vec4<u32>(any_wall, 0u, 0u, 0u));
}

// The local lights' light scattered toward the camera at `p`, the camera
// looking along `dir`; `frag` its column in pixels (light_tiles.rs sets).
fn fog_local_light(p: vec3<f32>, dir: vec3<f32>, frag: vec2<f32>, g: f32) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    let walk = light_walk(p, frag);
    var w = walk;
    loop {
        let i = next_light(&w);
        if (i < 0) { break; }
        let lt = frame.lights[i];
        let to = lt.pos_radius.xyz - p;
        let d = length(to);
        if (d >= lt.pos_radius.w || d < 1e-3) { continue; }
        let atten = ll_light_atten(d / lt.pos_radius.w, lt.color_falloff.w);
        let lit = lt.color_falloff.rgb * atten * fog_light_phase(dot(to / d, dir), g);
        // (too faint to show: no walk through the walls)
        if (max(max(lit.r, lit.g), lit.b) < 1e-3) { continue; }
        sum += lit * fog_light_seen(p, lt.pos_radius.xyz);
    }
    // the projectors light their frustum only, softly at its edges (their
    // texture is not read: the beam's colour is the light's)
    var pw = projector_walk(walk);
    loop {
        let i = next_light(&pw);
        if (i < 0) { break; }
        let lt = frame.lights[i];
        let to = lt.pos_radius.xyz - p;
        let d = length(to);
        if (d >= lt.pos_radius.w || d < 1e-3) { continue; }
        let tc = frame.projectors[i].matrix * vec4<f32>(p, 1.0);
        if (tc.z < 0.0 || tc.w <= 0.0) { continue; }
        let uv = tc.xy / tc.w;
        let edge = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
        if (edge <= 0.0) { continue; }
        let atten = ll_light_atten(d / lt.pos_radius.w, lt.color_falloff.w) * smoothstep(0.0, 0.15, edge);
        let lit = lt.color_falloff.rgb * atten * fog_light_phase(dot(to / d, dir), g);
        if (max(max(lit.r, lit.g), lit.b) < 1e-3) { continue; }
        sum += lit * fog_light_seen(p, lt.pos_radius.xyz);
    }
    return sum * FOG_LIGHT_GAIN;
}

@compute @workgroup_size(8, 8, 1)
fn inject(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id >= fp.grid.xyz)) { return; }
    let n = vec3<f32>(fp.grid.xyz);
    // the froxel's point this frame: jittered across it and through its
    // depth (each frame elsewhere in it, the history averaging them), so
    // that the fog it holds is the whole froxel's and stays put in the world
    // as the froxels sweep across it when the camera turns
    let cell = vec2<f32>(id.xy) + vec2<f32>(f32(id.z) * 7.0, f32(id.z) * 3.0);
    let still = fp.look.z > 0.5;
    let jitter = select(fract(fp.forward.w + fog_ign(cell)), 0.5, still);
    let frame_n = f32(fp.grid.w);
    let across = select(
        fract(vec2<f32>(frame_n * 0.7548777, frame_n * 0.5698403) + vec2<f32>(fog_ign(cell + 17.0), fog_ign(cell + 43.0))) - 0.5,
        vec2<f32>(0.0),
        still,
    );
    let uv = (vec2<f32>(id.xy) + 0.5 + across) / n.xy;
    let dir = fog_column_dir(uv);
    let fwd = fp.forward.xyz;
    let depth = fog_slice_depth((f32(id.z) + jitter) / n.z);
    let dist = depth / max(dot(dir, fwd), 1e-3);
    let cam = frame.camera_pos.xyz;
    let p = cam + dir * dist;
    // the froxel's size there: as wide as a column, as deep as its slice
    let centre_dir = fog_column_dir((vec2<f32>(id.xy) + 0.5) / n.xy);
    let wide = length(fog_column_dir((vec2<f32>(id.xy) + vec2<f32>(1.5, 0.5)) / n.xy) - centre_dir) * dist;
    let deep = fog_slice_depth((f32(id.z) + 1.0) / n.z) - fog_slice_depth(f32(id.z) / n.z);
    let footprint = max(wide, deep);

    // a point under the ground takes the fog just above it: what lies under
    // the ground is never seen, and counted as none it thinned the froxels
    // the ground cuts through, more or less as it cut each slice, in bands
    // across the ground at the slices' distances
    let ground = fog_ground_raw(p.xy);
    let at = vec3<f32>(p.xy, max(p.z, ground + 0.25));
    let open = fog_open(at);
    let sigma = fog_density(at, open, footprint);
    var scatter = vec3<f32>(0.0);
    if (sigma > 1e-7) {
        let g = frame.fog_look[1].w;
        let sky = fog_sky_light() * mix(FOG_SHELTERED_SKY, 1.0, open);
        let sun = fog_sun(p - cam) * fog_phase(dot(dir, frame.sun_dir.xyz), g) * fog_sun_shadow(p, dist);
        var light = sky + sun;
        if (fp.look.y > 0.5) {
            light += fog_local_light(p, dir, uv * frame.screen.xy, g);
        }
        scatter = sigma * frame.fog_look[1].rgb * light;
    }
    var now = vec4<f32>(scatter, sigma);

    // the last frame's value where this froxel's centre was: the jitter of
    // the frames before averaged in. The centre, not the jittered point: a
    // history read at the jittered depth blended the slices around it again
    // each frame, and the fog drifted darker than a fresh frame of it (15 %
    // at 50 m), so that turning the camera, which keeps less of it, changed
    // its brightness
    if (fp.look.x < 1.0) {
        let centre = fog_slice_depth((f32(id.z) + 0.5) / n.z) / max(dot(centre_dir, fwd), 1e-3);
        let prev = fp.prev_vp * vec4<f32>(centre_dir * centre, 1.0);
        if (prev.w > 1e-3) {
            let puv = prev.xy / prev.w * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
            let near = frame.fog_look[2].x;
            let pw = log(max(prev.w, near) / near) * frame.fog_look[3].x;
            if (all(puv >= vec2<f32>(0.0)) && all(puv <= vec2<f32>(1.0)) && pw <= 1.0) {
                let h = textureSampleLevel(fog_history, lin_clamp, vec3<f32>(puv, pw), 0.0);
                // sealed air takes nothing of the history: a froxel deep
                // enough to straddle a wall last frame carried the fog
                // outside into the room as the camera moved
                let keep = select(fp.look.x, 1.0, open < 0.05);
                now = mix(h, now, keep);
            }
        }
    }
    textureStore(fog_scatter_out, id, now);
}

@compute @workgroup_size(8, 8, 1)
fn integrate(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= fp.grid.xy)) { return; }
    let n = vec3<f32>(fp.grid.xyz);
    let uv = (vec2<f32>(id.xy) + 0.5) / n.xy;
    let stretch = 1.0 / max(dot(fog_column_dir(uv), fp.forward.xyz), 1e-3);
    var light = vec3<f32>(0.0);
    var through = 1.0;
    var d0 = fog_slice_depth(0.0);
    for (var k = 0u; k < fp.grid.z; k++) {
        let d1 = fog_slice_depth(f32(k + 1u) / n.z);
        let len = (d1 - d0) * stretch;
        d0 = d1;
        let s = textureLoad(fog_scatter_in, vec3<i32>(vec2<i32>(id.xy), i32(k)), 0);
        let sigma = max(s.a, 1e-7);
        let t = exp(-sigma * len);
        light += through * (s.rgb - s.rgb * t) / sigma;
        through *= t;
        textureStore(fog_integrated_out, vec3<i32>(vec2<i32>(id.xy), i32(k)), vec4<f32>(light, through));
    }
}

// ------------------------------------------------------- the fog's flow
// One step of the fog flowing through the grid around the camera (fog.rs
// `Flow`): a cell open to the sky is full of fog; a solid one (the walls
// drawn with this stamp, the ground, the water) holds none; any other takes
// the most of its six neighbours', less what a cell's length takes
// (FOG_FLOW_REACH). Repeated a few times a frame and kept between frames,
// the fog reaches under porches and through doorways, fading inward, and
// never into a closed room. A neighbour past the grid's sides is the open
// air there.
fn flow_prev(c: vec3<i32>) -> f32 {
    let dims = vec3<i32>(fp.flow_dims.xyz);
    let at = c + fog_step.shift.xyz;
    if (at.z < 0 || at.z >= dims.z) { return 0.0; }
    if (any(at.xy < vec2<i32>(0)) || any(at.xy >= dims.xy)) { return 1.0; }
    if (fog_step.shift.w != 0) { return 0.0; }
    return textureLoad(fog_flow_prev, at, 0).x;
}

@compute @workgroup_size(4, 4, 4)
fn flow_step(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id >= fp.flow_dims.xyz)) { return; }
    let c = vec3<i32>(id);
    let p = fp.flow.xyz + (vec3<f32>(id) + 0.5) * fp.flow.w;
    var v = 0.0;
    let solid = (textureLoad(fog_solid, c, 0).x >> 1u) == fp.flow_dims.w;
    // (a metre of slack: the map's cells are 4 m, a slope between them is
    // not a plane, and the air just over the ground is where fog lies)
    let under = p.z < fog_ground_raw(p.xy) - 1.0;
    if (!solid && !under) {
        if (fog_sheltered(p) < 0.5) {
            v = 1.0;
        } else {
            var best = flow_prev(c + vec3<i32>(1, 0, 0));
            best = max(best, flow_prev(c - vec3<i32>(1, 0, 0)));
            best = max(best, flow_prev(c + vec3<i32>(0, 1, 0)));
            best = max(best, flow_prev(c - vec3<i32>(0, 1, 0)));
            best = max(best, flow_prev(c + vec3<i32>(0, 0, 1)));
            best = max(best, flow_prev(c - vec3<i32>(0, 0, 1)));
            v = best * exp(-fp.flow.w / FOG_FLOW_REACH);
        }
    }
    textureStore(fog_flow_out, id, vec4<f32>(v, 0.0, 0.0, 0.0));
}

// ------------------------------------------------------- the sky's fog

// How far the sky is taken to be behind the fog (m): far enough that a
// level view into a thick layer is lost in it, near enough that looking
// up the high share leaves the sky.
const FOG_SKY_REACH: f32 = 4000.0;

@vertex
fn vs_fog_sky(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

// `dst * a + rgb` for the sky pixel at `pos`.
fn fog_sky(pos: vec4<f32>) -> vec4<f32> {
    let uv = pos.xy * frame.screen.zw;
    let dir = fog_column_dir(uv);
    let cam = frame.camera_pos.xyz;
    if (fog_mode() == FOG_MODE_HEIGHT) {
        let end = cam + dir * FOG_SKY_REACH;
        let t = exp(-fog_optical_depth(cam.z, end.z, FOG_SKY_REACH, frame.fog_look[0].z));
        return vec4<f32>(fog_light(dir, 0.0) * (1.0 - t), t);
    }
    let slices = frame.fog_look[2].z;
    let v = textureSampleLevel(fog_volume, lin_clamp, vec3<f32>(uv, 1.0 - 0.5 / slices), 0.0);
    let reach = frame.fog_look[2].y / max(dot(dir, frame.fog_look[3].yzw), 1e-3);
    let start = cam + dir * reach;
    let len = max(FOG_SKY_REACH - reach, 0.0);
    let end = start + dir * len;
    let t = exp(-fog_optical_depth(start.z, end.z, len, frame.fog_look[0].z));
    return vec4<f32>(v.rgb + v.a * fog_light(dir, 0.0) * (1.0 - t), v.a * t);
}

@fragment
fn fs_fog_sky(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return fog_sky(pos);
}

struct FogSkyG {
    @location(0) color: vec4<f32>,
    @location(1) gbuf: vec4<f32>,
}

// With the G-buffer of the SSR: its target is not written (empty mask).
@fragment
fn fs_fog_sky_g(@builtin(position) pos: vec4<f32>) -> FogSkyG {
    var o: FogSkyG;
    o.color = fog_sky(pos);
    o.gbuf = vec4<f32>(0.0);
    return o;
}
