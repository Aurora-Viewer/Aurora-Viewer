// The motion vectors shown instead of the image (motion_debug.rs): hue for
// the direction, brightness for the length (about 8 pixels a frame gives
// two thirds of full brightness).

struct Params {
    reproj: mat4x4<f32>,
    screen: vec4<f32>,
};

@group(0) @binding(0) var motion_tex: texture_2d<f32>;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var<uniform> params: Params;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn texel(uv: vec2<f32>, dims: vec2<u32>) -> vec2<i32> {
    return vec2<i32>(min(vec2<u32>(uv * vec2<f32>(dims)), dims - 1u));
}

fn hue(h: f32) -> vec3<f32> {
    let k = vec3<f32>(0.0, 2.0, 1.0) / 3.0;
    return clamp(abs(fract(h + k) * 6.0 - 3.0) - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = pos.xy / params.screen.xy;
    let depth = textureLoad(depth_tex, texel(uv, textureDimensions(depth_tex)), 0);
    var motion: vec2<f32>;
    if (params.screen.z > 0.5 && depth > 0.0) {
        motion = textureLoad(motion_tex, texel(uv, textureDimensions(motion_tex)), 0).zw;
    } else {
        // the camera's motion alone (reversed depth: 0 is the sky)
        let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
        let prev = params.reproj * vec4<f32>(ndc, depth, 1.0);
        motion = select(vec2<f32>(0.0), (ndc - prev.xy / prev.w) * vec2<f32>(0.5, -0.5), abs(prev.w) > 1e-6);
    }
    let pixels = motion * params.screen.xy;
    let len = length(pixels);
    let bright = 1.0 - exp(-len / 8.0);
    let h = atan2(pixels.y, pixels.x) / 6.2831853 + 0.5;
    // a dim grey where nothing moves, to tell it from the sky's fill
    let still = select(0.08, 0.04, depth <= 0.0);
    return vec4<f32>(mix(vec3<f32>(still), hue(h), bright), 1.0);
}
