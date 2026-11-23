// Shared part of the traced global illumination (gi.rs): the volume's
// uniforms, the octahedral map of a probe's tiles, the rays' directions and
// the addresses of a probe. gi.rs prepends the `GI_*` constants, from its
// own, so that the shaders and the CPU agree; gi_update.wgsl and
// gi_sample.wgsl follow this file. Every function here has a CPU twin in
// gi.rs that the tests compare.

struct GiCascade {
    // the cell of the grid's first probe (xyz)
    origin: vec4<i32>,
    // the same last frame: the probes whose cell was outside it are reset
    old_origin: vec4<i32>,
    // x: spacing (m), y: 1 / spacing, z: the depth's longest distance (m)
    spacing: vec4<f32>,
    // this frame's probes: x first, y count, z index of the first in the
    // frame's update list (its rays start at z * GI_RAYS)
    plan: vec4<u32>,
}

struct GiUniforms {
    cascades: array<GiCascade, GI_CASCADES>,
    // x, y, z: probes per axis; w: probes per cascade
    dims: vec4<u32>,
    // x, y: 1 / the irradiance atlas's size; z, w: the depth atlas's
    atlas: vec4<f32>,
    // x: probes updated this frame, y: 1 when the volume is in use, z:
    // which probe reads trace their visibility ray (AURORA_RT_GI_VIS,
    // gi_sample.wgsl `GI_VIS_*`), w: gi_sample.wgsl `GI_COARSE`
    // (AURORA_RT_GI_DIAG=coarse)
    frame: vec4<u32>,
    // this frame's rotation of the rays (columns)
    rotation: array<vec4<f32>, 3>,
}

const GI_PI: f32 = 3.14159265358979;
// Bit of a probe's state: its tiles hold light gathered at its current cell.
const GI_VALID: u32 = 0x80000000u;
// Bit of a probe's state: its last fixed rays found it inside a wall.
const GI_INSIDE: u32 = 0x40000000u;
const GI_AGE: u32 = 0x00ffffffu;
const GI_NONE: u32 = 0xffffffffu;

// Octahedral map (Cigolle et al., "A Survey of Efficient Representations
// for Independent Unit Vectors", JCGT 2014): a direction to a point of the
// square [-1, 1]².
fn gi_sign_not_zero(v: vec2<f32>) -> vec2<f32> {
    return select(vec2<f32>(-1.0), vec2<f32>(1.0), v >= vec2<f32>(0.0));
}

fn gi_oct_encode(n: vec3<f32>) -> vec2<f32> {
    let p = n.xy / (abs(n.x) + abs(n.y) + abs(n.z));
    if (n.z <= 0.0) {
        return (vec2<f32>(1.0) - abs(p.yx)) * gi_sign_not_zero(p);
    }
    return p;
}

fn gi_oct_decode(e: vec2<f32>) -> vec3<f32> {
    var v = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (v.z < 0.0) {
        let xy = (vec2<f32>(1.0) - abs(v.yx)) * gi_sign_not_zero(v.xy);
        v = vec3<f32>(xy, v.z);
    }
    return normalize(v);
}

// The direction of the centre of interior texel `t` of a tile with
// `interior` texels a side.
fn gi_texel_dir(t: vec2<u32>, interior: u32) -> vec3<f32> {
    let e = (vec2<f32>(t) + vec2<f32>(0.5)) / f32(interior) * 2.0 - vec2<f32>(1.0);
    return gi_oct_decode(e);
}

// Point `i` of a spherical Fibonacci set of `n`.
fn gi_fibonacci(i: u32, n: u32) -> vec3<f32> {
    let golden = 1.61803398874989;
    let fi = f32(i);
    let phi = 2.0 * GI_PI * fract(fi / golden);
    let cos_theta = 1.0 - (2.0 * fi + 1.0) / f32(n);
    let sin_theta = sqrt(max(1.0 - cos_theta * cos_theta, 0.0));
    return vec3<f32>(cos(phi) * sin_theta, sin(phi) * sin_theta, cos_theta);
}

// Ray `i` of a probe's update: the GI_FIXED_RAYS first always the same
// (the relocation's), the others a set of their own turned by the frame's
// rotation, so that the texels see other directions each frame.
fn gi_ray_dir(i: u32) -> vec3<f32> {
    if (i < GI_FIXED_RAYS) {
        return gi_fibonacci(i, GI_FIXED_RAYS);
    }
    let r = mat3x3<f32>(gi.rotation[0].xyz, gi.rotation[1].xyz, gi.rotation[2].xyz);
    return normalize(r * gi_fibonacci(i - GI_FIXED_RAYS, GI_RAYS - GI_FIXED_RAYS));
}

// A ray's w (gi.rs BACK_SCALE): no hit, a hit on a face's back.
fn gi_is_miss(w: f32) -> bool {
    return w < GI_MISS * 0.5;
}

fn gi_is_back(w: f32) -> bool {
    return w < 0.0 && !gi_is_miss(w);
}

// The slot (x, y, z) of probe `i` of a cascade, and back.
fn gi_slot(i: u32) -> vec3<u32> {
    let d = gi.dims;
    return vec3<u32>(i % d.x, (i / d.x) % d.y, i / (d.x * d.y));
}

fn gi_slot_index(s: vec3<u32>) -> u32 {
    let d = gi.dims;
    return s.x + s.y * d.x + s.z * d.x * d.y;
}

fn gi_wrap(a: vec3<i32>) -> vec3<u32> {
    let d = vec3<i32>(gi.dims.xyz);
    return vec3<u32>(((a % d) + d) % d);
}

// The world cell slot `s` holds while the grid starts at `origin`: the one
// cell of the grid that is congruent to it (the grid scrolls).
fn gi_cell_of_slot(s: vec3<u32>, origin: vec3<i32>) -> vec3<i32> {
    return origin + vec3<i32>(gi_wrap(vec3<i32>(s) - origin));
}

// Cascade and probe of entry `k` of the frame's update list (x GI_NONE
// past its end).
fn gi_update_probe(k: u32) -> vec2<u32> {
    if (k >= gi.frame.x) {
        return vec2<u32>(GI_NONE, 0u);
    }
    for (var c = 0u; c < GI_CASCADES; c++) {
        let p = gi.cascades[c].plan;
        if (k >= p.z && k < p.z + p.y) {
            return vec2<u32>(c, (p.x + k - p.z) % gi.dims.w);
        }
    }
    return vec2<u32>(GI_NONE, 0u);
}

// The position of probe `i` of cascade `c`: its cell's corner, moved by
// its relocation (`gi_offsets`, which the module that includes this file
// declares).
fn gi_probe_pos(c: u32, i: u32) -> vec3<f32> {
    let k = gi.cascades[c];
    let cell = vec3<f32>(gi_cell_of_slot(gi_slot(i), k.origin.xyz)) * k.spacing.x;
    return cell + gi_offsets[c * gi.dims.w + i].xyz;
}

// The atlas texel of the top-left corner (border included) of the tile of
// slot `s` of cascade `c`, tiles of `tile` texels a side.
fn gi_tile_origin(c: u32, s: vec3<u32>, tile: u32) -> vec2<u32> {
    let d = gi.dims;
    return vec2<u32>(s.x + s.z * d.x, s.y + c * d.y) * tile;
}
