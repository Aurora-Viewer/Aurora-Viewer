// LiveMesh's Fit lab: where a body pokes through a garment (fit_clip.rs).
// `coverage` holds the visible body pixels (fs_fit_body), `garment` the
// nearest outer surface of the garment (fs_fit_garment), `scene` the scene's
// depth, the body's at a covered pixel. Both depths are reversed and
// infinite: view z = P32 / (d + P22).
struct Params {
    clip_color: vec4<f32>,
    touch_color: vec4<f32>,
    // P32, P22, the reach (metres), the touch band (metres)
    depth: vec4<f32>,
    output_srgb: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
};
@group(0) @binding(0) var coverage: texture_2d<f32>;
@group(0) @binding(1) var scene: texture_depth_2d;
@group(0) @binding(2) var garment: texture_depth_2d;
@group(0) @binding(3) var<uniform> params: Params;

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn output_color(linear: vec3<f32>) -> vec3<f32> {
    if (params.output_srgb > 0.5) { return linear; }
    let low = linear * 12.92;
    let high = 1.055 * pow(max(linear, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, linear <= vec3<f32>(0.0031308));
}

fn view_z(d: f32) -> f32 {
    return params.depth.x / max(d + params.depth.y, 1e-12);
}

// The gap between the garment behind and the body seen, in metres, or a
// negative value where there is no body seen or no garment behind it.
fn gap_at(p: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(coverage));
    if (any(p < vec2<i32>(0)) || any(p >= size)) { return -1.0; }
    if (textureLoad(coverage, p, 0).r < 0.5) { return -1.0; }
    let g = textureLoad(garment, p, 0);
    if (g <= 0.0) { return -1.0; }
    return view_z(g) - view_z(textureLoad(scene, p, 0));
}

// Red where the body comes out of the garment, within the reach (farther,
// it is another part of the body in front of it: an arm before the chest);
// amber in the touch band, where body and garment meet. A thin darker edge
// outlines each patch, so a small one reads on any skin or cloth.
@fragment
fn fs_composite_fit(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(pixel.xy);
    let gap = gap_at(p);
    let reach = params.depth.z;
    let touch = params.depth.w;
    if (gap < -touch || gap > reach) { return vec4<f32>(0.0); }
    let fade = 1.0 - smoothstep(reach * 0.7, reach, gap);
    var color = mix(params.clip_color, params.touch_color, 1.0 - smoothstep(touch * 0.5, touch, abs(gap)));
    var edge = 0.0;
    for (var i = 0; i < 4; i++) {
        let o = vec2<i32>(select(-1, 1, (i & 1) == 1), 0);
        let q = select(o, o.yx, i >= 2);
        let other = gap_at(p + q);
        if (other < -touch || other > reach) { edge = 1.0; }
    }
    color = vec4<f32>(color.rgb * (1.0 - 0.45 * edge), color.a);
    return vec4<f32>(output_color(color.rgb), color.a * fade);
}
