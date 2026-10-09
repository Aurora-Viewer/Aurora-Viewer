// Hi-Z visibility test of a bounding sphere, shared by the occlusion of the
// CPU draw lists (occlusion.wgsl) and of the GPU draw lists (cull.wgsl).
// Bounding spheres enclose the whole object (center of its geometry, not its
// origin), and the test projects their whole box: large objects (floors,
// buildings) are only culled when every pixel they could cover is in front.

// Is the sphere possibly visible against the Hi-Z pyramid `hiz` (farthest
// depth per texel, reverse-Z) of a depth of `screen` pixels?
fn hiz_sphere_visible(hiz: texture_2d<f32>, s: vec4<f32>, view_proj: mat4x4<f32>, screen: vec2<f32>, levels: u32) -> bool {
    if (s.w <= 0.0) {
        return true;
    }
    var lo = vec2<f32>(1e9);
    var hi = vec2<f32>(-1e9);
    var nearest = 0.0;
    for (var k = 0u; k < 8u; k++) {
        let o = vec3<f32>(
            select(-s.w, s.w, (k & 1u) != 0u),
            select(-s.w, s.w, (k & 2u) != 0u),
            select(-s.w, s.w, (k & 4u) != 0u),
        );
        let c = view_proj * vec4<f32>(s.xyz + o, 1.0);
        // a corner at or behind the camera plane: keep it
        if (c.w <= 1e-3) {
            return true;
        }
        let ndc = c.xyz / c.w;
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        lo = min(lo, uv);
        hi = max(hi, uv);
        nearest = max(nearest, ndc.z);
    }
    // off screen: left to the frustum culling
    if (hi.x < 0.0 || hi.y < 0.0 || lo.x > 1.0 || lo.y > 1.0) {
        return true;
    }
    // pixels, one pixel of margin (TAA jitter, rasterization rules)
    let pmin = clamp(lo * screen - vec2<f32>(1.0), vec2<f32>(0.0), screen - vec2<f32>(1.0));
    let pmax = clamp(hi * screen + vec2<f32>(1.0), vec2<f32>(0.0), screen - vec2<f32>(1.0));
    let size = max(pmax.x - pmin.x, pmax.y - pmin.y);
    // level whose texels (2^(level+1) pixels) make the box span 2x2 at most
    var level = 0u;
    if (size > 2.0) {
        level = u32(ceil(log2(size))) - 1u;
    }
    level = min(level, levels - 1u);
    let texel = f32(1u << (level + 1u));
    let dims = vec2<i32>(textureDimensions(hiz, i32(level))) - vec2<i32>(1);
    let t0 = min(vec2<i32>(floor(pmin / texel)), dims);
    let t1 = min(vec2<i32>(floor(pmax / texel)), dims);
    var farthest = 1.0;
    for (var y = t0.y; y <= t1.y; y++) {
        for (var x = t0.x; x <= t1.x; x++) {
            farthest = min(farthest, textureLoad(hiz, vec2<i32>(x, y), i32(level)).r);
        }
    }
    // reverse-Z: hidden when even its nearest point is behind the farthest
    // depth over the whole box
    return nearest >= farthest;
}
