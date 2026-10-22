// Port of deferred/rlvF.glsl (LGPL 2.1): sphere blend, separable Gaussian
// blur, variable Gaussian blur, chromatic aberration and pixelation.
struct Params {
    inv_vp: mat4x4<f32>,
    origin: vec4<f32>,
    range: vec4<f32>, // minimum value/distance, maximum value/distance
    effect: vec4<f32>,
    mode: vec4<u32>, // effect, extension bits, blur axis, unused
    screen: vec4<f32>,
};
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var linear_sampler: sampler;
@group(0) @binding(3) var<uniform> p: Params;

struct Vertex { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> Vertex {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Vertex;
    out.position = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(xy.x, 1.0 - xy.y);
    return out;
}
fn color(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(scene, linear_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb;
}
@fragment fn fs_main(in: Vertex) -> @location(0) vec4<f32> {
    let pixel = clamp(vec2<i32>(in.position.xy), vec2<i32>(0), vec2<i32>(textureDimensions(depth)) - 1);
    let z = textureLoad(depth, pixel, 0);
    let h = p.inv_vp * vec4<f32>(in.uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), z, 1.0);
    let world = h.xyz / select(0.000001, h.w, abs(h.w) > 0.000001);
    let distance = length(world - p.origin.xyz);
    let min_d = p.range.y;
    let max_d = max(p.range.w, min_d);
    let t = select(1.0, (distance - min_d) / max(0.000001, max_d - min_d), max_d > min_d);
    var strength = mix(p.range.x, p.range.z, t);
    if distance < min_d { strength = select(0.0, p.range.x, (p.mode.y & 2u) != 0u); }
    if distance > max_d { strength = select(0.0, p.range.z, (p.mode.y & 1u) != 0u); }
    let original = color(in.uv);
    var result = original;
    let direction = select(vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0), p.mode.z == 1u) / p.screen.xy;
    switch p.mode.x {
        case 0u: { result = mix(original, p.effect.rgb, strength); }
        case 1u: {
            let delta = direction * strength;
            result = original * 0.1964825501511404;
            result += (color(in.uv + delta * 1.411764705882353) + color(in.uv - delta * 1.411764705882353)) * 0.2969069646728344;
            result += (color(in.uv + delta * 3.2941176470588234) + color(in.uv - delta * 3.2941176470588234)) * 0.09447039785044732;
            result += (color(in.uv + delta * 5.176470588235294) + color(in.uv - delta * 5.176470588235294)) * 0.010381362401148057;
        }
        case 2u: {
            if strength > 0.0 {
                let sigma = max(strength, 0.0001);
                let step = exp(-0.5 / (sigma * sigma));
                var coefficient = 1.0;
                var next = step;
                var total = 1.0;
                result = original;
                // Bound hostile script workloads; a 256-pixel radius exceeds a
                // useful screen-space Gaussian kernel on supported displays.
                let radius = u32(clamp(floor(p.effect.x * 0.5), 0.0, 256.0));
                for (var i = 1u; i <= radius; i++) {
                    coefficient *= next;
                    next *= step * step;
                    let delta = f32(i) * direction;
                    result += (color(in.uv - delta) + color(in.uv + delta)) * coefficient;
                    total += 2.0 * coefficient;
                }
                result /= total;
            }
        }
        case 3u: {
            let drift = vec3<f32>(color(in.uv + p.effect.xy / p.screen.xy).r, original.g, color(in.uv + p.effect.zw / p.screen.xy).b);
            result = mix(original, drift, strength);
        }
        case 4u: {
            let block = max(vec2<f32>(1.0), round(p.effect.xy * sign(strength))) / p.screen.xy;
            result = color(block * floor(in.uv / block));
        }
        default: {}
    }
    return vec4<f32>(result, 1.0);
}
