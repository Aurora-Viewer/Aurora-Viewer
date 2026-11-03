// Resolution-aware contact-shadow softening. Two depth-aware tent passes
// keep the footprint proportional to the full rendered image height, with
// contiguous taps and no cross-surface averaging at depth discontinuities.
@group(0) @binding(0) var depth_tex: texture_depth_2d;
@group(0) @binding(1) var mask_tex: texture_2d<f32>;
@group(0) @binding(2) var out_tex: texture_storage_2d<rgba8unorm, write>;

// Rewritten for the active render height, including full-photo supersampling.
const RADIUS: i32 = 3;
const SUPPORT: f32 = 3.5;
const TILE_SAMPLES: i32 = 8 * (8 + 2 * RADIUS);
const PLANE_TOLERANCE: f32 = 0.01;
var<workgroup> tile_depth: array<f32, TILE_SAMPLES>;
var<workgroup> tile_mask: array<f32, TILE_SAMPLES>;

fn slope(before: f32, centre: f32, after: f32) -> f32 {
    let a = centre - before;
    let b = after - centre;
    return select(b, a, abs(a) < abs(b));
}

fn soften(group: vec3<u32>, local: vec3<u32>, index: u32, horizontal: bool) {
    let size = vec2<i32>(textureDimensions(out_tex));
    let extent = select(vec2<i32>(8, 8 + 2 * RADIUS), vec2<i32>(8 + 2 * RADIUS, 8), horizontal);
    let halo = select(vec2<i32>(0, RADIUS), vec2<i32>(RADIUS, 0), horizontal);
    let axis = select(vec2<i32>(0, 1), vec2<i32>(1, 0), horizontal);
    let origin = vec2<i32>(group.xy) * 8 - halo;
    for (var k = i32(index); k < TILE_SAMPLES; k += 64) {
        let q = origin + vec2<i32>(k % extent.x, k / extent.x);
        let inside = all(q >= vec2<i32>(0)) && all(q < size);
        let c = clamp(q, vec2<i32>(0), size - 1);
        tile_depth[k] = textureLoad(depth_tex, c, 0);
        tile_mask[k] = select(-1.0, textureLoad(mask_tex, c, 0).r, inside);
    }
    workgroupBarrier();
    let t = vec2<i32>(local.xy) + halo;
    let p = vec2<i32>(group.xy) * 8 + vec2<i32>(local.xy);
    if (any(p >= size)) { return; }
    let centre = t.y * extent.x + t.x;
    let step = axis.y * extent.x + axis.x;
    let d0 = tile_depth[centre];
    if (d0 <= 0.0) {
        textureStore(out_tex, p, vec4<f32>(0.0));
        return;
    }
    // The smaller one-sided derivative follows the receiver rather than
    // a neighbouring surface. Depth remains unfiltered in both passes.
    let gradient = slope(tile_depth[centre - step], d0, tile_depth[centre + step]);
    let tolerance = PLANE_TOLERANCE * d0;
    var sum = 0.0;
    var weight = 0.0;
    for (var k = -RADIUS; k <= RADIUS; k++) {
        let at = centre + k * step;
        let mask = tile_mask[at];
        let expected = d0 + gradient * f32(k);
        let off_plane = abs(tile_depth[at] - expected);
        let tent = max(0.0, SUPPORT - abs(f32(k)));
        let w = select(0.0, tent * saturate(1.0 - off_plane / tolerance), mask >= 0.0);
        sum += w * mask;
        weight += w;
    }
    // The centre always has positive weight, even at the viewport border.
    textureStore(out_tex, p, vec4<f32>(sum / weight, textureLoad(mask_tex, p, 0).gba));
}

@compute @workgroup_size(8, 8, 1)
fn horizontal(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_id) local: vec3<u32>, @builtin(local_invocation_index) index: u32) {
    soften(group, local, index, true);
}

@compute @workgroup_size(8, 8, 1)
fn vertical(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_id) local: vec3<u32>, @builtin(local_invocation_index) index: u32) {
    soften(group, local, index, false);
}

// The unfiltered mask (AURORA_CONTACT_SOFT=0).
@compute @workgroup_size(8, 8, 1)
fn copy(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (all(p < vec2<i32>(textureDimensions(out_tex)))) {
        textureStore(out_tex, p, textureLoad(mask_tex, p, 0));
    }
}