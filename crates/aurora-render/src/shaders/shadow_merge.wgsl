// A cascade's share of the map of the cascade before (shadow_merge.rs).
// The cascades look along the same light: what the finer one drew inside
// its square is, for the coarser one, the same casters at the same places.
// fs_extract reads the finer tile at the centre of each texel of the
// coarser cascade — the point a draw into that cascade would have sampled —
// and writes its depth, in the coarser cascade's range, to a scratch map;
// fs_merge then lays the scratch over the coarser tile, keeping the nearest.

struct MergeParams {
    // first texel filled in the coarser tile (x, y), texels filled (w, h)
    part: vec4<u32>,
    // atlas origin of the finer tile (x, y), of the coarser tile (z, w)
    tiles: vec4<u32>,
    // clip position in the finer cascade from the one in the coarser:
    // scale x, y, offset x, y
    to_fine: vec4<f32>,
    // depth in the coarser cascade from the finer one: scale, offset; what
    // the slope adds per unit of depth between two finer texels; the most
    // it adds
    depth: vec4<f32>,
    // side of a tile in texels
    size: vec4<f32>,
}

@group(0) @binding(0) var<uniform> merge: MergeParams;
// fs_extract: the shadow atlas; fs_merge: the scratch map
@group(0) @binding(1) var source: texture_depth_2d;

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

// Depth of a texel of the finer tile (its edge texel beyond the tile).
fn finer(t: vec2<i32>) -> f32 {
    let last = i32(merge.size.x) - 1;
    let c = clamp(t, vec2<i32>(0), vec2<i32>(last));
    return textureLoad(source, c + vec2<i32>(merge.tiles.xy), 0);
}

// Drawn into the scratch map, one texel per texel of the part filled.
@fragment
fn fs_extract(@builtin(position) pos: vec4<f32>) -> @builtin(frag_depth) f32 {
    let size = merge.size.x;
    // centre of the coarser texel, in its cascade's clip space
    let tk = floor(pos.xy) + vec2<f32>(merge.part.xy) + 0.5;
    let coarse = vec2<f32>(tk.x / size * 2.0 - 1.0, 1.0 - tk.y / size * 2.0);
    let fine = coarse * merge.to_fine.xy + merge.to_fine.zw;
    let at = vec2<f32>((fine.x * 0.5 + 0.5) * size, (0.5 - fine.y * 0.5) * size);
    if (at.x < 0.0 || at.y < 0.0 || at.x >= size || at.y >= size) {
        return 1.0;
    }
    let t = vec2<i32>(floor(at));
    let d = finer(t);
    if (d >= 1.0) {
        return 1.0;
    }
    // The pipelines move a depth back with the slope of its triangle over a
    // texel, and a coarser texel is wider: the slope is read from the map,
    // on the side of the texel that stays on the same surface.
    let sx = min(abs(finer(t + vec2<i32>(1, 0)) - d), abs(d - finer(t - vec2<i32>(1, 0))));
    let sy = min(abs(finer(t + vec2<i32>(0, 1)) - d), abs(d - finer(t - vec2<i32>(0, 1))));
    let slope = min(max(sx, sy) * merge.depth.z, merge.depth.w);
    return clamp(d * merge.depth.x + merge.depth.y + slope, 0.0, 1.0);
}

// Drawn in the coarser tile's pass, over the part filled.
@fragment
fn fs_merge(@builtin(position) pos: vec4<f32>) -> @builtin(frag_depth) f32 {
    let t = vec2<i32>(floor(pos.xy)) - vec2<i32>(merge.tiles.zw) - vec2<i32>(merge.part.xy);
    let d = textureLoad(source, t, 0);
    if (d >= 1.0) {
        discard;
    }
    return d;
}
