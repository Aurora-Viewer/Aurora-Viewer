// Automatic exposure of skies that are not classic, in one dispatch of one
// workgroup: the average luminance of a 256 x 256 grid over the middle of
// the scene (luminanceF.glsl and the mip chain LLPipeline::generateLuminance
// builds), then the exposure it calls for, eased towards from the last one
// (exposureF.glsl). Port of luminanceF.glsl / exposureF.glsl
// (indra/newview/app_settings/shaders/class1/deferred, originally LGPL 2.1).
// The result stays on the GPU: exposure.rs copies it into the post pass's
// uniforms.

struct Params {
    // the inverse of the frame's view-projection (unjittered)
    inv_view_proj: mat4x4<f32>,
    // x seconds since the last update, y RenderDiffuseLuminanceScale,
    // z 1: add the glow, w 1: start from the target (no easing)
    a: vec4<f32>,
    // x RenderDynamicExposureCoefficient (the luminance that calls for the
    // lowest exposure), y lowest exposure, z highest exposure, w easing
    // speed (1/s); w < 0: hold the last exposure (photos)
    b: vec4<f32>,
};

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var glow: texture_2d<f32>;
@group(0) @binding(3) var<uniform> p: Params;
// [0] the exposure, [1] its target, [2] the average luminance (diagnostics)
@group(0) @binding(4) var<storage, read_write> state: array<f32, 4>;

const SIDE: u32 = 16u;
const GRID: f32 = 256.0;

var<workgroup> partial: array<f32, 256>;

fn lum(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// The luminance of one cell of the grid (luminanceF.glsl).
fn cell_luminance(cell: vec2<u32>) -> f32 {
    let dims = textureDimensions(scene);
    // the middle 60 % of the image, nudged towards the ground ("favor
    // ground over sky"; GL's y points up, ours down)
    let uv = (vec2<f32>(cell) + 0.5) / GRID * 0.6 + vec2<f32>(0.2, 0.3);
    let px = min(vec2<u32>(uv * vec2<f32>(dims)), dims - vec2<u32>(1u));
    var c = textureLoad(scene, px, 0).rgb;
    // RenderDiffuseLuminanceScale on everything but the sky, so that a
    // bright sky does not darken the ground too much. The Legacy Viewer
    // reads the G-buffer's sky flags; here a pixel the depth prepass left
    // empty above the horizon is sky (below it: water, the far fog)
    let d = textureLoad(depth, px, 0);
    var sky = false;
    if (d <= 0.0) {
        let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
        let near = p.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
        let far = p.inv_view_proj * vec4<f32>(ndc, 0.5, 1.0);
        let dir = far.xyz / far.w - near.xyz / near.w;
        sky = dir.z >= 0.0;
    }
    if (!sky) {
        c *= p.a.y;
    }
    if (p.a.z > 0.5) {
        let g = textureDimensions(glow);
        c += textureLoad(glow, min(vec2<u32>(uv * vec2<f32>(g)), g - vec2<u32>(1u)), 0).rgb;
    }
    // NaN and negative light count as black
    return select(0.0, max(lum(c), 0.0), lum(c) == lum(c));
}

@compute @workgroup_size(16, 16)
fn cs_exposure(@builtin(local_invocation_id) lid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    if (p.b.w < 0.0) {
        return;
    }
    // each thread sums 16 x 16 cells, interleaved so that neighbouring
    // threads read neighbouring texels
    var sum = 0.0;
    for (var j = 0u; j < SIDE; j++) {
        for (var i = 0u; i < SIDE; i++) {
            sum += cell_luminance(vec2<u32>(i * SIDE + lid.x, j * SIDE + lid.y));
        }
    }
    partial[li] = sum;
    workgroupBarrier();
    for (var n = 128u; n > 0u; n >>= 1u) {
        if (li < n) {
            partial[li] += partial[li + n];
        }
        workgroupBarrier();
    }
    if (li != 0u) {
        return;
    }
    // exposureF.glsl: the brighter the scene up to the coefficient, the
    // closer to the lowest exposure, on a square curve
    let average = partial[0] / (GRID * GRID);
    let max_l = max(p.b.x, 1e-4);
    let l = clamp(average, 0.0, max_l) / max_l;
    let goal = mix(p.b.z, p.b.y, l * l);
    // USE_LAST_EXPOSURE: eased towards with the speed of
    // RenderDynamicExposureSpeedError / SpeedTarget
    let prev = state[0];
    var s = goal;
    if (p.a.w < 0.5 && prev > 0.0 && prev == prev) {
        s = mix(prev, goal, 1.0 - exp(-p.b.w * p.a.x));
    }
    state[0] = s;
    state[1] = goal;
    state[2] = average;
}
