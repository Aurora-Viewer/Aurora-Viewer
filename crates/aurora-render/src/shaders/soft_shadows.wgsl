// Soft sun shadows (soft_shadows.rs): the sun's shadow of every pixel of the
// depth prepass, once per pixel, with a penumbra that widens with the
// distance between the shadow and what casts it (percentage-closer soft
// shadows, Fernando, "Percentage-Closer Soft Shadows", SIGGRAPH 2005), then
// softened by a bilateral blur that takes the noise of the rotated taps
// away. The shading reads the result with one load (`soft_shadow` in
// common.wgsl) where its surface is the one the pixel was made for.
//
// The cascade choice, the normal offset and the receiver bias are those of
// `cascade_shadow` and `shadow_factor` (common.wgsl). The tile addressing
// follows the atlas of any shape (`tile_scale`, projector_shadows.rs).
//
// Entry points: `soft` (reads the depth and the atlas, writes the mask),
// `blur_h` and `blur_v` (the mask through the other target and back),
// `mm_build0` and `mm_build` (the atlas's minimum depths).

struct Params {
    cascade_vp: array<mat4x4<f32>, 3>,
    inv_view_proj: mat4x4<f32>,
    camera: vec4<f32>,      // xyz: the camera; w: the angle between two neighbouring pixels' rays
    splits: vec4<f32>,      // the three cascades' far distances; w: the frame's noise phase
    light: vec4<f32>,       // x: tan of the light's half angle, y: largest filter radius (m), z: filter taps, w: blocker gathers
    size: vec4<u32>,        // x, y: the target's size; z, w: its first pixel in the whole photo (0 outside a tile)
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var shadow_map: texture_depth_2d;
@group(0) @binding(3) var shadow_sampler: sampler_comparison;
@group(0) @binding(4) var point_sampler: sampler;
@group(0) @binding(5) var in_tex: texture_2d<u32>;
@group(0) @binding(6) var out_tex: texture_storage_2d<r32uint, write>;
// The shadow atlas's minimum depths over blocks of 4, 8 and 16 texels (levels
// 0 to 2 of `mm`), which the search for casters reads: a block holds the
// nearest depth of its texels, so no thin caster slips between the texels
// of a block.
// In the build passes `mm_in` is the level before and `mm_out` the level made.
@group(0) @binding(7) var mm_in: texture_2d<f32>;
@group(0) @binding(8) var mm_out: texture_storage_2d<r32float, write>;

// The least radius of the filter, in metres (soft_shadows.rs FILTER_FLOOR):
// shadows are sharp where they touch what casts them.
const FILTER_FLOOR: f32 = 0.015;
// Where the penumbra is narrower than the filter of before (3 cm across,
// `SHADOW_PENUMBRA` in common.wgsl), the shadow is that filter: its taps are
// the same for every pixel, so a narrow penumbra shows no grain and no
// pattern that crawls with the camera. The filter of the noisy taps takes
// over by `LEGACY_FADE` times the floor, where its blur hides the noise.
const SHADOW_PENUMBRA: f32 = 0.03;
const LEGACY_FADE: f32 = 3.0;

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

fn depth_at(px: vec2<i32>) -> f32 {
    return textureLoad(depth_tex, clamp(px, vec2<i32>(0), vec2<i32>(p.size.xy) - vec2<i32>(1)), 0);
}

fn world_of(px: vec2<i32>, depth: f32) -> vec3<f32> {
    let size = vec2<f32>(p.size.xy);
    let c = vec2<f32>(clamp(px, vec2<i32>(0), vec2<i32>(p.size.xy) - vec2<i32>(1)));
    let ndc = vec2<f32>((c.x + 0.5) / size.x * 2.0 - 1.0, 1.0 - (c.y + 0.5) / size.y * 2.0);
    let w = p.inv_view_proj * vec4<f32>(ndc, max(depth, 1e-7), 1.0);
    return w.xyz / w.w;
}

// The normal of the face at a pixel, from the depth: along each axis the
// neighbour whose depth is nearer the pixel's, so that an edge does not bend
// it (the depths are compared, only the two chosen neighbours unprojected).
fn normal_at(px: vec2<i32>, depth: f32, pos: vec3<f32>) -> vec3<f32> {
    let l = depth_at(px - vec2<i32>(1, 0));
    let r = depth_at(px + vec2<i32>(1, 0));
    let u = depth_at(px - vec2<i32>(0, 1));
    let d = depth_at(px + vec2<i32>(0, 1));
    let sx = select(1, -1, abs(l - depth) < abs(r - depth));
    let sy = select(1, -1, abs(u - depth) < abs(d - depth));
    let dx = (world_of(px + vec2<i32>(sx, 0), select(r, l, sx < 0)) - pos) * f32(sx);
    let dy = (world_of(px + vec2<i32>(0, sy), select(d, u, sy < 0)) - pos) * f32(sy);
    let c = cross(dx, dy);
    let to_camera = p.camera.xyz - pos;
    // (a degenerate step, on the ring of the target's edge or a flat
    // depth, leaves the view direction)
    var n = select(normalize(to_camera), c * inverseSqrt(max(dot(c, c), 1e-30)), dot(c, c) > 1e-20);
    if (dot(n, to_camera) < 0.0) {
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

// The nearest depths of the 2 x 2 blocks of `level` around an atlas position
// (in texels).
fn blocks_at(at: vec2<f32>, level: u32) -> vec4<f32> {
    let b = vec2<i32>(floor(at / f32(4u << level) - 0.5));
    return vec4<f32>(
        textureLoad(mm_in, b, level).x,
        textureLoad(mm_in, b + vec2<i32>(1, 0), level).x,
        textureLoad(mm_in, b + vec2<i32>(0, 1), level).x,
        textureLoad(mm_in, b + vec2<i32>(1, 1), level).x);
}

// Taps per axis for a filter spanning `span` (tile coordinates): `shadow_taps`
// of common.wgsl.
fn legacy_taps(span: f32, texel: f32) -> u32 {
    return clamp(u32(ceil(span / texel - 1e-3)) + 1u, 4u, 7u);
}

// The filter of `cascade_shadow` (common.wgsl), taps and spacing as they
// are there, on the same receiver bias; `radius`: the cascade's half extent
// in metres, `texel` its texel in tile coordinates.
fn legacy_filter(uv: vec2<f32>, depth: f32, radius: f32, texel: f32, tile: vec2<f32>, tile_scale: vec2<f32>) -> f32 {
    let half_uv = SHADOW_PENUMBRA * 0.5 / radius;
    let spacing = clamp(half_uv * (2.0 / 3.0), 0.5 * texel, 2.0 * texel);
    let span = 3.0 * spacing;
    let taps = legacy_taps(span, texel);
    let step = span / f32(taps - 1u);
    let center = 0.5 * f32(taps - 1u);
    var sum = 0.0;
    for (var y = 0u; y < taps; y++) {
        for (var x = 0u; x < taps; x++) {
            let o = (vec2<f32>(f32(x), f32(y)) - center) * step;
            let t = clamp(uv + o, vec2<f32>(0.5 * texel), vec2<f32>(1.0 - 0.5 * texel));
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, (t + tile) * tile_scale, depth);
        }
    }
    return sum / f32(taps * taps);
}

// Shadow of one cascade (1 lit, 0 shadowed) and the radius of its filter in
// metres; the light is -1 outside its tile, so that the other cascade of a
// blend can take over (`cascade_shadow`).
fn cascade_soft(world_pos: vec3<f32>, n: vec3<f32>, cascade: u32, frag: vec2<u32>) -> vec2<f32> {
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
        return vec2<f32>(-1.0, 0.0);
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
    var filter_m = FILTER_FLOOR;
    // the share of the filter of before (1: all of it, no light size or a
    // penumbra narrower than it)
    var before = 1.0;
    if (p.light.x > 0.0) {
        // Blocker search: gathers over a rotated disk of the minimum depths
        // of blocks of 4, 8 or 16 texels (the larger the wider the disk, so
        // that its gathers keep covering it), and one of the texels at the
        // point itself, which always finds a thin caster whose shadow it is
        // inside. A thin caster cannot slip between the texels of a block,
        // but one far from the point can fall between the eight gathers:
        // it then goes unseen by the search (the filter still sees it).
        // The point's shadow is bounded by the filter taps, a few per cent
        // of the light at the widest softness.
        let search = p.light.y / (2.0 * radius);
        let texel_m = 2.0 * radius * texel;
        let wide = p.light.y / texel_m;
        let level = select(select(2u, 1u, wide <= 48.0), 0u, wide <= 24.0);
        let block = f32(4u << level) * texel;
        var separation = 0.0;
        var blockers = 0.0;
        {
            let local = clamp(uv, vec2<f32>(texel), vec2<f32>(1.0 - texel));
            let d = textureGather(shadow_map, point_sampler, (local + tile) * tile_scale);
            let expected = q.z - bias;
            let gap = max(vec4<f32>(expected) - d, vec4<f32>(0.0));
            let hit = select(vec4<f32>(0.0), vec4<f32>(1.0), d < vec4<f32>(expected));
            separation += dot(gap, hit);
            blockers += dot(hit, vec4<f32>(1.0));
        }
        let gathers = u32(p.light.w);
        for (var i = 0u; i < gathers; i++) {
            let o = rot * DISK[(i * 32u) / gathers] * search;
            let local = clamp(uv + o, vec2<f32>(block), vec2<f32>(1.0 - block));
            let d = blocks_at((local + tile) * tile_scale * dims, level);
            // a block's nearest texel may lie anywhere in it: the plane's
            // depth is taken at the far side of the block
            let expected = q.z + dot(gradient, local - uv) - bias - 0.5 * block * (abs(gradient.x) + abs(gradient.y));
            let gap = max(vec4<f32>(expected) - d, vec4<f32>(0.0));
            let hit = select(vec4<f32>(0.0), vec4<f32>(1.0), d < vec4<f32>(expected));
            separation += dot(gap, hit);
            blockers += dot(hit, vec4<f32>(1.0));
        }
        // no caster within the search: lit, with the floor's filter (the
        // point's own texels and the blocks around it hold none)
        if (blockers == 0.0) {
            return vec2<f32>(1.0, FILTER_FLOOR);
        }
        {
            // the light's half width seen from the receiver, at the blockers'
            // mean distance; the least is the floor (contact hardening)
            let reach = separation / blockers / depth_per_metre * p.light.x;
            filter_m = clamp(reach, FILTER_FLOOR, max(p.light.y, FILTER_FLOOR));
            before = 1.0 - smoothstep(FILTER_FLOOR, LEGACY_FADE * FILTER_FLOOR, reach);
        }
    }
    var light = 0.0;
    if (before > 0.0) {
        light = before * legacy_filter(uv, q.z - bias, radius, texel, tile, tile_scale);
    }
    if (before < 1.0) {
        let half_uv = max(filter_m / (2.0 * radius), 0.75 * texel);
        let taps = u32(p.light.z);
        var sum = 0.0;
        for (var i = 0u; i < taps; i++) {
            let o = rot * DISK[(i * 32u) / taps] * half_uv;
            let local = clamp(uv + o, vec2<f32>(0.5 * texel), vec2<f32>(1.0 - 0.5 * texel));
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, (local + tile) * tile_scale, q.z + dot(gradient, local - uv) - bias);
        }
        light += (1.0 - before) * sum / f32(taps);
    }
    return vec2<f32>(light, filter_m);
}

// The sun's shadow at a point (`shadow_factor`): the cascade by the point's
// distance, the last 15 % of a cascade dithered into the next one (one
// evaluation a pixel, the blur averages the dither) and the fade at the far
// end of the last cascade. With the radius of the filter.
fn soft_factor(world_pos: vec3<f32>, n: vec3<f32>, view_depth: f32, frag: vec2<u32>) -> vec2<f32> {
    let splits = p.splits;
    if (view_depth > splits.z) {
        return vec2<f32>(1.0, 0.0);
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
    if (s.x < 0.0 && t > 0.0) {
        s = cascade_soft(world_pos, n, select(cascade + 1u, cascade, pick != cascade), frag);
    }
    if (s.x < 0.0) {
        return vec2<f32>(1.0, 0.0);
    }
    let fade = clamp((splits.z - view_depth) / (splits.z * 0.1), 0.0, 1.0);
    return vec2<f32>(mix(1.0, s.x, fade), s.y);
}

// The mask's texel: the distance of the surface it was made for from the
// camera as an f16 in the high 16 bits (negative: no surface, the sky), the
// radius of the filter in bits 10 to 15 (0.005 m * 2^(code / 6)), the light
// let through in the low 10. Read by `soft_shadow` in common.wgsl, which keeps
// the same split of the distance and the light.
fn store(px: vec2<u32>, light: f32, distance: f32, radius: f32) {
    let d = pack2x16float(vec2<f32>(select(-1.0, distance, distance >= 0.0), 0.0)) & 0xffffu;
    let code = u32(clamp(round(6.0 * log2(max(radius, 0.005) / 0.005)), 0.0, 63.0));
    let l = u32(clamp(light, 0.0, 1.0) * 1023.0 + 0.5);
    textureStore(out_tex, px, vec4<u32>((d << 16u) | (code << 10u) | l, 0u, 0u, 0u));
}

// (light, distance; distance -1 for no surface)
fn unpack(texel: u32) -> vec2<f32> {
    let d = unpack2x16float(texel >> 16u).x;
    return vec2<f32>(f32(texel & 1023u) / 1023.0, select(d, -1.0, d < 0.0));
}

// The radius of the filter the texel holds, in metres.
fn unpack_radius(texel: u32) -> f32 {
    return 0.005 * exp2(f32((texel >> 10u) & 63u) / 6.0);
}

@compute @workgroup_size(8, 8)
fn soft(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= p.size.x || id.y >= p.size.y) {
        return;
    }
    let px = vec2<i32>(id.xy);
    let depth = textureLoad(depth_tex, px, 0);
    // reversed depth: 0 is the sky, which no shadow reaches
    if (depth <= 0.0) {
        store(id.xy, 1.0, -1.0, 0.0);
        return;
    }
    let pos = world_of(px, depth);
    let view_depth = distance(pos, p.camera.xyz);
    if (view_depth > p.splits.z) {
        store(id.xy, 1.0, view_depth, 0.0);
        return;
    }
    let n = normal_at(px, depth, pos);
    let f = soft_factor(pos, n, view_depth, id.xy + p.size.zw);
    store(id.xy, f.x, view_depth, f.y);
}

// Bilateral blur along one axis, 9 taps, in the world: a neighbour counts by
// its distance in metres from the pixel's surface (the law of cosines over
// the pixel's angle), against a share of the pixel's filter radius. The
// shadow therefore never spreads across an edge, and where the filter is
// under about a pixel nothing is blurred. A floor seen at a grazing angle,
// with its many metres to the pixel, keeps a thin shadow whole.
fn blur(id: vec3<u32>, axis: vec2<i32>) {
    if (id.x >= p.size.x || id.y >= p.size.y) {
        return;
    }
    let px = vec2<i32>(id.xy);
    let texel = textureLoad(in_tex, px, 0).x;
    let c = unpack(texel);
    let radius = unpack_radius(texel);
    if (c.y < 0.0) {
        store(id.xy, c.x, c.y, radius);
        return;
    }
    let size = vec2<i32>(p.size.xy) - vec2<i32>(1);
    let spread = 0.4 * radius;
    // under a quarter of a pixel's width in metres no neighbour counts
    if (spread < 0.25 * c.y * p.camera.w) {
        store(id.xy, c.x, c.y, radius);
        return;
    }
    let sharp = -0.5 / (spread * spread);
    var sum = c.x;
    var weights = 1.0;
    for (var i = -4; i <= 4; i++) {
        if (i == 0) {
            continue;
        }
        let s = unpack(textureLoad(in_tex, clamp(px + axis * i, vec2<i32>(0), size), 0).x);
        let a = f32(i) * p.camera.w;
        let apart = (s.y - c.y) * (s.y - c.y) + s.y * c.y * a * a;
        let w = exp(-0.125 * f32(i * i) + sharp * apart) * select(0.0, 1.0, s.y >= 0.0);
        sum += s.x * w;
        weights += w;
    }
    store(id.xy, sum / weights, c.y, radius);
}

@compute @workgroup_size(8, 8)
fn blur_h(@builtin(global_invocation_id) id: vec3<u32>) { blur(id, vec2<i32>(1, 0)); }

@compute @workgroup_size(8, 8)
fn blur_v(@builtin(global_invocation_id) id: vec3<u32>) { blur(id, vec2<i32>(0, 1)); }

// Whether an atlas texel is in a cascade's tile: the three of the four
// squares of the first two rows, not the projectors' maps (the fourth square
// and the rows below), which the search never reads.
fn in_cascades(texel: vec2<u32>, dims: vec2<u32>) -> bool {
    let tile = dims.x / 2u;
    return texel.y < 2u * tile && (texel.x < tile || texel.y < tile);
}

// Level 0 of the minimum depths: 4 x 4 texels of the atlas a thread.
@compute @workgroup_size(8, 8)
fn mm_build0(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(shadow_map);
    if (id.x * 4u >= dims.x || id.y * 4u >= dims.y || !in_cascades(id.xy * 4u, dims)) {
        return;
    }
    var m = 1.0;
    for (var y = 0u; y < 4u; y += 2u) {
        for (var x = 0u; x < 4u; x += 2u) {
            let d = textureGather(shadow_map, point_sampler, (vec2<f32>(id.xy * 4u + vec2<u32>(x, y)) + 1.0) / vec2<f32>(dims));
            m = min(m, min(min(d.x, d.y), min(d.z, d.w)));
        }
    }
    textureStore(mm_out, id.xy, vec4<f32>(m, 0.0, 0.0, 0.0));
}

// The next level: 2 x 2 blocks of the one before (`mm_in`) a thread.
@compute @workgroup_size(8, 8)
fn mm_build(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(mm_in));
    if (i32(id.x) * 2 >= size.x || i32(id.y) * 2 >= size.y) {
        return;
    }
    let dims = textureDimensions(shadow_map);
    if (!in_cascades(id.xy * 2u * (dims.x / u32(size.x)), dims)) {
        return;
    }
    let at = vec2<i32>(id.xy) * 2;
    let last = size - vec2<i32>(1);
    let m = min(
        min(textureLoad(mm_in, min(at, last), 0).x, textureLoad(mm_in, min(at + vec2<i32>(1, 0), last), 0).x),
        min(textureLoad(mm_in, min(at + vec2<i32>(0, 1), last), 0).x, textureLoad(mm_in, min(at + vec2<i32>(1, 1), last), 0).x));
    textureStore(mm_out, id.xy, vec4<f32>(m, 0.0, 0.0, 0.0));
}
