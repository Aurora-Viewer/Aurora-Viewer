// Soft sun shadows (soft_shadows.rs): the sun's shadow of every pixel of the
// depth prepass, once per pixel, with a penumbra that widens with the
// distance between the shadow and what casts it (percentage-closer soft
// shadows, Fernando, "Percentage-Closer Soft Shadows", SIGGRAPH 2005), then
// softened by a bilateral blur that takes the noise of the rotated taps
// away. The shading reads the result with one load (`soft_shadow` in
// common.wgsl) where its surface is the one the pixel was made for.
//
// The cascade choice, the normal offset and the receiver bias are those of
// `cascade_shadow` and `shadow_factor` (common.wgsl); the penumbra of that
// filter, `SHADOW_PENUMBRA`, is the least this one gives. The tile addressing
// follows the atlas of any shape (`tile_scale`, projector_shadows.rs).
//
// Entry points: `soft` (reads the depth and the atlas, writes the mask),
// `blur_h` and `blur_v` (the mask through the other target and back).

struct Params {
    cascade_vp: array<mat4x4<f32>, 3>,
    inv_view_proj: mat4x4<f32>,
    camera: vec4<f32>,      // xyz: the camera
    splits: vec4<f32>,      // the three cascades' far distances; w: the frame's noise phase
    light: vec4<f32>,       // x: tan of the light's half angle, y: largest filter radius (m), z: filter taps, w: blocker gathers
    size: vec4<u32>,        // x, y: the target's size
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var shadow_map: texture_depth_2d;
@group(0) @binding(3) var shadow_sampler: sampler_comparison;
@group(0) @binding(4) var point_sampler: sampler;
@group(0) @binding(5) var in_tex: texture_2d<u32>;
@group(0) @binding(6) var out_tex: texture_storage_2d<r32uint, write>;

// Half-width of the filter of before, in metres (common.wgsl, kept equal
// by a test): the least penumbra, so that no shadow is sharper than it was.
const SHADOW_PENUMBRA: f32 = 0.03;

// Points of a disk of radius 1, spread evenly (a Poisson-like sequence):
// every tap count takes each (32 / count)th, the same for every pixel.
const DISK: array<vec2<f32>, 32> = array(
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

// Interleaved gradient noise (Jimenez 2014), shifted by the frame's phase.
fn ign(frag: vec2<u32>, salt: u32) -> f32 {
    let f = vec2<f32>(frag) + f32((u32(p.splits.w) * 5u + salt) % 64u) * vec2<f32>(5.588238, 5.588238);
    return fract(52.9829189 * fract(dot(f, vec2<f32>(0.06711056, 0.00583715))));
}

fn world_at(px: vec2<i32>) -> vec3<f32> {
    let size = vec2<i32>(p.size.xy);
    let c = clamp(px, vec2<i32>(0), size - vec2<i32>(1));
    let depth = textureLoad(depth_tex, c, 0);
    let ndc = vec2<f32>((f32(c.x) + 0.5) / f32(size.x) * 2.0 - 1.0, 1.0 - (f32(c.y) + 0.5) / f32(size.y) * 2.0);
    let w = p.inv_view_proj * vec4<f32>(ndc, max(depth, 1e-7), 1.0);
    return w.xyz / w.w;
}

// The normal of the face at a pixel, from the depth: the smaller of the
// two steps along each axis, so that an edge does not bend it.
fn normal_at(px: vec2<i32>, pos: vec3<f32>) -> vec3<f32> {
    let l = pos - world_at(px - vec2<i32>(1, 0));
    let r = world_at(px + vec2<i32>(1, 0)) - pos;
    let u = pos - world_at(px - vec2<i32>(0, 1));
    let d = world_at(px + vec2<i32>(0, 1)) - pos;
    let dx = select(r, l, dot(l, l) < dot(r, r));
    let dy = select(d, u, dot(u, u) < dot(d, d));
    var n = normalize(cross(dx, dy));
    if (dot(n, p.camera.xyz - pos) < 0.0) {
        n = -n;
    }
    return n;
}

// Uv of a point in a cascade's tile (y down) and its depth along the light.
fn light_space(world_pos: vec3<f32>, cascade: u32) -> vec3<f32> {
    let lp = p.cascade_vp[cascade] * vec4<f32>(world_pos, 1.0);
    let ndc = lp.xyz / lp.w;
    return vec3<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, ndc.z);
}

// Shadow of one cascade (1 lit, 0 shadowed); -1 outside its tile, so that
// the other cascade of a blend can take over (`cascade_shadow`).
fn cascade_soft(world_pos: vec3<f32>, n: vec3<f32>, cascade: u32, frag: vec2<u32>) -> f32 {
    let m = p.cascade_vp[cascade];
    let row_x = vec3<f32>(m[0].x, m[1].x, m[2].x);
    let row_y = vec3<f32>(m[0].y, m[1].y, m[2].y);
    let row_z = vec3<f32>(m[0].z, m[1].z, m[2].z);
    // half extent of the cascade in metres, depth per metre along the light
    let radius = 1.0 / max(length(row_x), 1e-6);
    let depth_per_metre = length(row_z);
    let bias_n = n * (0.02 + 0.04 * f32(cascade));
    let q = light_space(world_pos + bias_n, cascade);
    let uv = q.xy;
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || q.z < 0.0 || q.z > 1.0) {
        return -1.0;
    }
    let dims = vec2<f32>(textureDimensions(shadow_map));
    // texel of a tile, in tile coordinates
    let texel = 2.0 / dims.x;
    let tile = vec2<f32>(f32(cascade % 2u), f32(cascade / 2u));
    let tile_scale = vec2<f32>(0.5, dims.x / (2.0 * dims.y));
    // two texels of the cascade, along the light (cascade_shadow)
    let bias = 2.0 * texel * radius * 2.0 * depth_per_metre;
    // How the depth of the receiver's plane changes across the tile, so that
    // a slanted face does not shadow itself over the wide search and filter.
    // Faded out on grazing faces, whose plane is not well known.
    let to_light = row_z / max(depth_per_metre, 1e-8);
    let facing = dot(n, to_light);
    let slope = 2.0 * radius * depth_per_metre * facing / max(facing * facing, 1e-6);
    var gradient = vec2<f32>(-dot(n, row_x * radius), dot(n, row_y * radius)) * slope;
    gradient *= smoothstep(0.10, 0.20, abs(facing));

    let angle = ign(frag, 0u) * 6.2831853;
    let rot = mat2x2<f32>(cos(angle), sin(angle), -sin(angle), cos(angle));
    let floor_m = SHADOW_PENUMBRA;
    var filter_m = floor_m;
    if (p.light.x > 0.0) {
        // blocker search: gathers of 2 x 2 texels over a rotated disk
        let search = p.light.y / (2.0 * radius);
        var separation = 0.0;
        var blockers = 0.0;
        let gathers = u32(p.light.w);
        for (var i = 0u; i < gathers; i++) {
            let o = rot * DISK[(i * 32u) / gathers] * search;
            let local = clamp(uv + o, vec2<f32>(texel), vec2<f32>(1.0 - texel));
            let d = textureGather(shadow_map, point_sampler, (local + tile) * tile_scale);
            let expected = q.z + dot(gradient, local - uv) - bias;
            let gap = max(vec4<f32>(expected) - d, vec4<f32>(0.0));
            let hit = select(vec4<f32>(0.0), vec4<f32>(1.0), d < vec4<f32>(expected));
            separation += dot(gap, hit);
            blockers += dot(hit, vec4<f32>(1.0));
        }
        if (blockers > 0.0) {
            // the light's half width seen from the receiver, at the blockers'
            // mean distance, with the filter of before as the least
            let reach = separation / blockers / depth_per_metre * p.light.x;
            filter_m = clamp(sqrt(floor_m * floor_m + reach * reach), floor_m, p.light.y);
        }
    }
    let half_uv = max(filter_m / (2.0 * radius), 0.75 * texel);
    let taps = u32(p.light.z);
    var sum = 0.0;
    for (var i = 0u; i < taps; i++) {
        let o = rot * DISK[(i * 32u) / taps] * half_uv;
        let local = clamp(uv + o, vec2<f32>(0.5 * texel), vec2<f32>(1.0 - 0.5 * texel));
        sum += textureSampleCompareLevel(shadow_map, shadow_sampler, (local + tile) * tile_scale, q.z + dot(gradient, local - uv) - bias);
    }
    return sum / f32(taps);
}

// The sun's shadow at a point (`shadow_factor`): the cascade by the point's
// distance, the last 15 % of a cascade dithered into the next one (one
// evaluation a pixel, the blur averages the dither) and the fade at the far
// end of the last cascade.
fn soft_factor(world_pos: vec3<f32>, n: vec3<f32>, view_depth: f32, frag: vec2<u32>) -> f32 {
    let splits = p.splits;
    if (view_depth > splits.z) {
        return 1.0;
    }
    var cascade = 0u;
    if (view_depth > splits.x) { cascade = 1u; }
    if (view_depth > splits.y) { cascade = 2u; }
    var pick = cascade;
    var t = 0.0;
    if (cascade < 2u) {
        let start = select(0.0, splits.x, cascade == 1u);
        let end = select(splits.x, splits.y, cascade == 1u);
        let band = (end - start) * 0.15;
        t = clamp((view_depth - (end - band)) / max(band, 1e-4), 0.0, 1.0);
        if (t > ign(frag, 7u)) {
            pick = cascade + 1u;
        }
    }
    var s = cascade_soft(world_pos, n, pick, frag);
    // in the band, a point outside one tile takes the other (cascade_shadow)
    if (s < 0.0 && t > 0.0) {
        s = cascade_soft(world_pos, n, select(cascade + 1u, cascade, pick != cascade), frag);
    }
    if (s < 0.0) {
        return 1.0;
    }
    let fade = clamp((splits.z - view_depth) / (splits.z * 0.1), 0.0, 1.0);
    return mix(1.0, s, fade);
}

// The mask's texel: the light let through in the low 12 bits, the distance of
// the surface it was made for from the camera in the high 20 (4 mm steps up
// to 4 km; all ones: no surface, the sky). Read by `soft_shadow` in
// common.wgsl, which keeps the same split.
const NO_SURFACE: u32 = 0xfffffu;

fn store(px: vec2<u32>, light: f32, distance: f32) {
    var d = NO_SURFACE;
    if (distance >= 0.0) {
        d = min(u32(distance * 256.0), NO_SURFACE - 1u);
    }
    let l = u32(clamp(light, 0.0, 1.0) * 4095.0 + 0.5);
    textureStore(out_tex, px, vec4<u32>((d << 12u) | l, 0u, 0u, 0u));
}

// (light, distance; distance -1 for no surface)
fn unpack(texel: u32) -> vec2<f32> {
    let d = texel >> 12u;
    return vec2<f32>(f32(texel & 4095u) / 4095.0, select(f32(d) / 256.0, -1.0, d == NO_SURFACE));
}

@compute @workgroup_size(8, 8)
fn soft(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= p.size.x || id.y >= p.size.y) {
        return;
    }
    let px = vec2<i32>(id.xy);
    // reversed depth: 0 is the sky, which no shadow reaches
    if (textureLoad(depth_tex, px, 0) <= 0.0) {
        store(id.xy, 1.0, -1.0);
        return;
    }
    let pos = world_at(px);
    let view_depth = distance(pos, p.camera.xyz);
    if (view_depth > p.splits.z) {
        store(id.xy, 1.0, view_depth);
        return;
    }
    let n = normal_at(px, pos);
    store(id.xy, soft_factor(pos, n, view_depth, id.xy), view_depth);
}

// Bilateral blur along one axis, 9 taps: a neighbour counts when it is
// the same surface, at a distance from the camera within a few per cent of
// the pixel's (more the farther it is along the axis, for a floor seen at a
// grazing angle), so the shadow never bleeds across an edge.
fn blur(id: vec3<u32>, axis: vec2<i32>) {
    if (id.x >= p.size.x || id.y >= p.size.y) {
        return;
    }
    let px = vec2<i32>(id.xy);
    let c = unpack(textureLoad(in_tex, px, 0).x);
    if (c.y < 0.0) {
        store(id.xy, c.x, c.y);
        return;
    }
    let size = vec2<i32>(p.size.xy) - vec2<i32>(1);
    var sum = c.x;
    var weights = 1.0;
    for (var i = -4; i <= 4; i++) {
        if (i == 0) {
            continue;
        }
        let s = unpack(textureLoad(in_tex, clamp(px + axis * i, vec2<i32>(0), size), 0).x);
        let tolerance = (0.005 + 0.01 * f32(abs(i))) * c.y;
        let w = exp(-f32(i * i) / 8.0) * select(0.0, 1.0, s.y >= 0.0 && abs(s.y - c.y) < tolerance);
        sum += s.x * w;
        weights += w;
    }
    store(id.xy, sum / weights, c.y);
}

@compute @workgroup_size(8, 8)
fn blur_h(@builtin(global_invocation_id) id: vec3<u32>) { blur(id, vec2<i32>(1, 0)); }

@compute @workgroup_size(8, 8)
fn blur_v(@builtin(global_invocation_id) id: vec3<u32>) { blur(id, vec2<i32>(0, 1)); }
