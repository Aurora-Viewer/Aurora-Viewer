// DLSS's inputs (dlss.rs), one pass at the render size.
//
// Motion vectors, one per render pixel, in uv: where the pixel was last
// frame minus where it is, without the jitter. The same source and fallback
// as FSR's (fsr/prepare_inputs.wgsl, load_input_motion_vector), without its
// 3x3 dilation: DLSS dilates itself.
//
// The scene's colour made safe as FSR's is (fsr/common.wgsl safe_color,
// #1380): DLSS keeps what it is given in its history.
//
// fs_alpha, a pass of its own at the display size after DLSS drew: the
// scene's alpha (the glow amount, a cut-out photo's coverage) of this frame,
// taken at the nearest render pixel as FSR does (fsr/accumulate.wgsl).
struct U {
    // this frame's unjittered clip space to last frame's (main.motion_reproj)
    reproj: mat4x4<f32>,
    // where the jitter moved the image, in uv
    jitter_uv: vec2<f32>,
    motion_on: f32,
    pad: f32,
};
@group(0) @binding(0) var depth_tex: texture_depth_2d;
// zw: the pixel's screen motion since the last frame in uv (object.wgsl
// depth_normal); 0 where the prepass drew nothing
@group(0) @binding(1) var motion_tex: texture_2d<f32>;
@group(0) @binding(2) var<uniform> u: U;
// the scene, HDR, at the render size
@group(0) @binding(3) var color_tex: texture_2d<f32>;

// the largest finite value of a 16-bit float (fsr/common.wgsl FP16_MAX)
const FP16_MAX: f32 = 65504.0;

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex fn vs_main(@builtin(vertex_index) index: u32) -> Out {
    let p = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return Out(vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0), vec2<f32>(p.x, 1.0 - p.y));
}

struct Inputs {
    @location(0) motion: vec4<f32>,
    @location(1) color: vec4<f32>,
};

@fragment fn fs_main(@builtin(position) pos: vec4<f32>) -> Inputs {
    let c = textureLoad(color_tex, vec2<i32>(pos.xy), 0);
    // an infinite value held at the format's largest, a NaN black
    let rgb = select(clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(FP16_MAX)), vec3<f32>(0.0), c.rgb != c.rgb);
    return Inputs(motion_at(pos), vec4<f32>(rgb, c.a));
}

@fragment fn fs_alpha(in: Out) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(color_tex));
    let p = clamp(vec2<i32>(in.uv * vec2<f32>(size)), vec2<i32>(0), size - vec2<i32>(1));
    return vec4<f32>(0.0, 0.0, 0.0, textureLoad(color_tex, p, 0).a);
}

fn motion_at(pos: vec4<f32>) -> vec4<f32> {
    let p = vec2<i32>(pos.xy);
    let motion = textureLoad(motion_tex, p, 0).zw;
    let depth = textureLoad(depth_tex, p, 0);
    if (u.motion_on > 0.5 && depth > 0.0 && any(motion != vec2<f32>(0.0))) {
        return vec4<f32>(-motion, 0.0, 0.0);
    }
    // the camera alone moved the pixel, through its depth
    let uv = pos.xy / vec2<f32>(textureDimensions(depth_tex));
    let cur_uv = uv - u.jitter_uv;
    let ndc = vec2<f32>(cur_uv.x * 2.0 - 1.0, 1.0 - cur_uv.y * 2.0);
    let pp = u.reproj * vec4<f32>(ndc, max(depth, 1e-7), 1.0);
    if (pp.w <= 0.0) {
        return vec4<f32>(0.0);
    }
    let prev_uv = vec2<f32>(pp.x / pp.w * 0.5 + 0.5, 0.5 - pp.y / pp.w * 0.5);
    return vec4<f32>(prev_uv - cur_uv, 0.0, 0.0);
}
