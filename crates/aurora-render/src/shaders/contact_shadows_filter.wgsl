// Softens the contact shadows' mask (contact_shadows.wgsl) before the
// shading reads it: each pixel's shadow is decided on its own along one ray,
// so the mask's borders are hard steps of whole pixels, and the gap under a
// hovering foot seen from above is a dotted line one pixel wide (issue 988).
// A 5 x 5 tent over the pixels of the same surface: a neighbour counts as
// far as its depth lies on the plane the pixel's own neighbours describe, so
// the shadow never spreads from the ground onto the caster nor from the
// caster onto the ground behind it (no halo).

@group(0) @binding(0) var depth_tex: texture_depth_2d;
@group(0) @binding(1) var mask_tex: texture_2d<f32>;
@group(0) @binding(2) var out_tex: texture_storage_2d<rgba8unorm, write>;

// Reversed depth: the far plane (the sky) at 0.
const FAR_DEPTH: f32 = 0.0;
// How far off the pixel's plane a neighbour may lie, as a share of the
// pixel's depth (relative depth is relative distance), before it stops
// counting: 1 %, 3 cm at 3 m.
const PLANE_TOLERANCE: f32 = 0.01;
const RADIUS: i32 = 2;
// The workgroup's 8 x 8 pixels and RADIUS around them, read once into
// workgroup memory: depth, and the mask (-1 outside the screen).
const TILE: i32 = 8 + 2 * RADIUS;
var<workgroup> tile_depth: array<f32, 144>;
var<workgroup> tile_mask: array<f32, 144>;

fn tile_at(t: vec2<i32>) -> i32 {
    return (t.y + RADIUS) * TILE + t.x + RADIUS;
}

// The smaller of the one-sided slopes: across an edge one side jumps, the
// other follows the surface.
fn slope(before: f32, centre: f32, after: f32) -> f32 {
    let a = centre - before;
    let b = after - centre;
    return select(b, a, abs(a) < abs(b));
}

@compute @workgroup_size(8, 8, 1)
fn main(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
    @builtin(local_invocation_index) index: u32,
) {
    let size = vec2<i32>(textureDimensions(out_tex));
    let origin = vec2<i32>(group.xy) * 8 - RADIUS;
    for (var k = i32(index); k < TILE * TILE; k += 64) {
        let q = origin + vec2<i32>(k % TILE, k / TILE);
        let inside = all(q >= vec2<i32>(0)) && all(q < size);
        let c = clamp(q, vec2<i32>(0), size - 1);
        tile_depth[k] = textureLoad(depth_tex, c, 0);
        tile_mask[k] = select(-1.0, textureLoad(mask_tex, c, 0).r, inside);
    }
    workgroupBarrier();
    let t = vec2<i32>(local.xy);
    let p = origin + RADIUS + t;
    if (any(p >= size)) {
        return;
    }
    let d0 = tile_depth[tile_at(t)];
    if (d0 <= FAR_DEPTH) {
        textureStore(out_tex, p, vec4<f32>(0.0));
        return;
    }
    let gx = slope(tile_depth[tile_at(t - vec2<i32>(1, 0))], d0, tile_depth[tile_at(t + vec2<i32>(1, 0))]);
    let gy = slope(tile_depth[tile_at(t - vec2<i32>(0, 1))], d0, tile_depth[tile_at(t + vec2<i32>(0, 1))]);
    let tolerance = PLANE_TOLERANCE * d0;
    var sum = 0.0;
    var weight = 0.0;
    for (var j = -RADIUS; j <= RADIUS; j++) {
        for (var i = -RADIUS; i <= RADIUS; i++) {
            let at = tile_at(t + vec2<i32>(i, j));
            let mask = tile_mask[at];
            let expected = d0 + gx * f32(i) + gy * f32(j);
            let off_plane = abs(tile_depth[at] - expected);
            let tent = f32((RADIUS + 1 - abs(i)) * (RADIUS + 1 - abs(j)));
            let w = select(0.0, tent * saturate(1.0 - off_plane / tolerance), mask >= 0.0);
            sum += w * mask;
            weight += w;
        }
    }
    // the pixel itself always counts (weight 9 at least)
    textureStore(out_tex, p, vec4<f32>(sum / weight, 0.0, 0.0, 0.0));
}

// The mask as it is (`AURORA_CONTACT_SOFT=0`).
@compute @workgroup_size(8, 8, 1)
fn copy(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (all(p < vec2<i32>(textureDimensions(out_tex)))) {
        textureStore(out_tex, p, textureLoad(mask_tex, p, 0));
    }
}
