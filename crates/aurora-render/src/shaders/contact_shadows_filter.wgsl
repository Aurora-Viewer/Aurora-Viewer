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
// X: zero for Aurora original, one for narrow-gap repair.
@group(0) @binding(3) var<uniform> integrity_filter_options: vec4<f32>;

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
var<workgroup> tile_front: array<f32, 144>;
// Whether any pixel of the tile is in contact shadow. Most of the screen has
// none: there the result is lit (a weighted sum of zeros) and the 25 taps are
// skipped, 0.05 ms of the pass's 0.25 at 1920 x 1080 (demo plaza, RTX 4090).
// Off (`Filter::new`, contact_shadows.rs) on the AMD RX 9000 cards, whose
// pass runs without workgroup memory: no atomic there either.
override SKIP_LIT: bool = true;
var<workgroup> any_shadow: atomic<u32>;

fn tile_at(t: vec2<i32>) -> i32 {
    return (t.y + RADIUS) * TILE + t.x + RADIUS;
}

// Bend stores front-surface occlusion in the lower half of B for an
// onscreen ray and its complement for an offscreen ray. View-space tracing
// stores only 0 or 1, both of which decode to zero front-surface occlusion.
fn diagnostic_channels(raw: vec4<f32>) -> vec3<f32> {
    return vec3<f32>(raw.g, select(0.0, 1.0, raw.b > 0.5), raw.a);
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
        let raw = textureLoad(mask_tex, c, 0);
        tile_mask[k] = select(-1.0, raw.r, inside);
        tile_front[k] = min(raw.r, min(raw.b, 1.0 - raw.b) * (255.0 / 127.0));
        if (SKIP_LIT && tile_mask[k] > 0.0) {
            atomicOr(&any_shadow, 1u);
        }
    }
    workgroupBarrier();
    let t = vec2<i32>(local.xy);
    let p = origin + RADIUS + t;
    if (any(p >= size)) {
        return;
    }
    if (SKIP_LIT && atomicLoad(&any_shadow) == 0u) {
        // Keep the diagnostic channels even when the original fast path
        // skips the 5x5 smoothing work. The red shadow mask is still zero.
        let detail = diagnostic_channels(textureLoad(mask_tex, p, 0));
        textureStore(out_tex, p, vec4<f32>(0.0, detail));
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
    var front_sum = 0.0;
    var front_weight = 0.0;
    for (var j = -RADIUS; j <= RADIUS; j++) {
        for (var i = -RADIUS; i <= RADIUS; i++) {
            let at = tile_at(t + vec2<i32>(i, j));
            let mask = tile_mask[at];
            let expected = d0 + gx * f32(i) + gy * f32(j);
            let off_plane = abs(tile_depth[at] - expected);
            let tent = f32((RADIUS + 1 - abs(i)) * (RADIUS + 1 - abs(j)));
            let surface = select(0.0, saturate(1.0 - off_plane / tolerance), mask >= 0.0);
            let w = tent * surface;
            // Filter the resolved component over a one-pixel radius. The wider
            // original tent still removes dotted edges on body/ground contact.
            let narrow = f32((2 - abs(i)) * (2 - abs(j))) * surface;
            let front = tile_front[at];
            front_sum += narrow * front;
            front_weight += narrow;
            sum += w * (mask - front);
            weight += w;
        }
    }
    // Diagnostics always describe the centre ray; spatial filtering only
    // combines the two occlusion components.
    let detail = diagnostic_channels(textureLoad(mask_tex, p, 0));
    // Independent, conservative repair of a narrow gap surrounded by
    // shadow on opposing sides of the SAME fitted depth plane. No growth
    // from a single boundary; no crossing silhouettes or discontinuities.
    var bridge = 0.0;
    let centre_mask = tile_mask[tile_at(t)];
    if (integrity_filter_options.x > 0.5 && centre_mask >= 0.0 && centre_mask < 0.55) {
        for (var axis = 0; axis < 4; axis++) {
            var direction = vec2<i32>(1, 0);
            if (axis == 1) {
                direction = vec2<i32>(0, 1);
            } else if (axis == 2) {
                direction = vec2<i32>(1, 1);
            } else if (axis == 3) {
                direction = vec2<i32>(1, -1);
            }
            for (var distance = 1; distance <= 2; distance++) {
                let offset = direction * distance;
                let ta = tile_at(t - offset);
                let tb = tile_at(t + offset);
                let sa = tile_mask[ta];
                let sb = tile_mask[tb];
                let expected_a = d0 - gx * f32(offset.x) - gy * f32(offset.y);
                let expected_b = d0 + gx * f32(offset.x) + gy * f32(offset.y);
                let compatible = abs(tile_depth[ta] - expected_a) < tolerance
                    && abs(tile_depth[tb] - expected_b) < tolerance;
                if (compatible && sa > 0.72 && sb > 0.72) {
                    bridge = max(bridge, min(sa, sb) * select(0.70, 0.85, distance == 1));
                }
            }
        }
    }
    let filtered = sum * (1.0 / max(weight, 1e-6)) + front_sum / max(front_weight, 1e-6);
    let integrity_mask = saturate(max(filtered, bridge));
    // the pixel itself always counts (weight 9 at least)
    textureStore(out_tex, p, vec4<f32>(integrity_mask, detail));
}

// The mask as it is (`AURORA_CONTACT_SOFT=0`).
@compute @workgroup_size(8, 8, 1)
fn copy(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (all(p < vec2<i32>(textureDimensions(out_tex)))) {
        let raw = textureLoad(mask_tex, p, 0);
        textureStore(out_tex, p, vec4<f32>(raw.r, diagnostic_channels(raw)));
    }
}
