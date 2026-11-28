// The Director's shutter (shutter.rs), Aurora's own: the sub-frames of an
// offline film frame added up in linear light, then the average written
// back into the frame's picture before it is read back.

@group(0) @binding(0) var src: texture_2d<f32>;

struct Shutter {
    // x: each sub-frame's share (1 / count), y: 1 when the picture's
    // texels are sRGB values read and written as they are (a Unorm format)
    params: vec4<f32>,
};
@group(0) @binding(1) var<uniform> shutter: Shutter;

struct Out {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_shutter(@builtin(vertex_index) vi: u32) -> Out {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var o: Out;
    o.clip = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let l = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let lo = l * 12.92;
    let hi = 1.055 * pow(l, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, l <= vec3<f32>(0.0031308));
}

// A sub-frame's share, added to the sum (the pipeline blends One + One).
@fragment
fn fs_add(in: Out) -> @location(0) vec4<f32> {
    let c = textureLoad(src, vec2<i32>(in.clip.xy), 0);
    var rgb = c.rgb;
    if (shutter.params.y > 0.5) {
        rgb = to_linear(rgb);
    }
    return vec4<f32>(rgb, c.a) * shutter.params.x;
}

// The sum, the average of the sub-frames, written as the picture.
@fragment
fn fs_resolve(in: Out) -> @location(0) vec4<f32> {
    let c = textureLoad(src, vec2<i32>(in.clip.xy), 0);
    var rgb = c.rgb;
    if (shutter.params.y > 0.5) {
        rgb = to_srgb(rgb);
    }
    return vec4<f32>(rgb, clamp(c.a, 0.0, 1.0));
}
