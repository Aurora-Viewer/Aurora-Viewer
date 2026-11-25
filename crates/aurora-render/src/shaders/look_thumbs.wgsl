// The photo filters' catalogue still (look_thumbs.rs): a part of the frame's
// image before its looks, shrunk to the pictures' size, with its depth.

struct Source {
    // the part of the frame, 0..1: left, top, right, bottom
    rect: vec4<f32>,
    // the still's size in pixels
    size: vec4<f32>,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(0) @binding(2) var filtered: sampler;
@group(0) @binding(3) var<uniform> src: Source;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(xy.x, 1.0 - xy.y);
    return out;
}

struct FsOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@fragment
fn fs(in: VsOut) -> FsOut {
    let uv = mix(src.rect.xy, src.rect.zw, in.uv);
    // a still pixel covers several of the frame's: 4 x 4 filtered taps over
    // its footprint, so that it is not a sparkling sample of one
    let footprint = (src.rect.zw - src.rect.xy) / src.size.xy;
    var sum = vec3<f32>(0.0);
    for (var j = 0; j < 4; j++) {
        for (var i = 0; i < 4; i++) {
            let o = (vec2<f32>(f32(i), f32(j)) - 1.5) / 4.0;
            sum += textureSampleLevel(scene, filtered, uv + o * footprint, 0.0).rgb;
        }
    }
    // the depth at the middle of the footprint (the depth looks read it as
    // they read the frame's)
    let dims = vec2<f32>(textureDimensions(scene_depth));
    let px = clamp(vec2<i32>(uv * dims), vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
    var out: FsOut;
    out.color = vec4<f32>(sum / 16.0, 1.0);
    out.depth = textureLoad(scene_depth, px, 0);
    return out;
}
