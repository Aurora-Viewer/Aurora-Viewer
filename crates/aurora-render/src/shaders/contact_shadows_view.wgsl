// Aurora v1.9: alternate view-space contact ray tracer.
// Runs only when explicitly selected; the original Bend shader is unchanged.
// Reconstructs a camera-view position from the *same* jittered depth prepass
// used by the original contact tracer (infinite reversed Z supported).
struct ViewRayParams {
    proj: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    light_view: vec4<f32>,  // xyz: normalized direction from point toward sun
    tuning: vec4<f32>,      // x: metres reach, y: view-z tolerance, z: budget, w unused
};
@group(0) @binding(0) var<uniform> params: ViewRayParams;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var out_tex: texture_storage_2d<rgba8unorm, write>;

fn project_view(p: vec3<f32>) -> vec4<f32> {
    return params.proj * vec4<f32>(p, 1.0);
}

fn unproject(uv: vec2<f32>, d: f32) -> vec3<f32> {
    let ndc = vec4<f32>(2.0 * uv.x - 1.0, 1.0 - 2.0 * uv.y, d, 1.0);
    let h = params.inv_proj * ndc;
    return h.xyz / max(abs(h.w), 1e-7) * select(-1.0, 1.0, h.w >= 0.0);
}

fn write_result(pixel: vec2<i32>, shadow: f32, hit: f32, offscreen: f32, exhausted: f32) {
    // R: raw occlusion amount. GBA: *raw* outcome, passed through the
    // v1.6 bilateral filter's `detail` diagnostic channels.
    textureStore(out_tex, pixel, vec4<f32>(shadow, hit, offscreen, exhausted));
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<i32>(textureDimensions(depth_tex));
    let pixel = vec2<i32>(gid.xy);
    if (any(pixel >= dims)) { return; }
    let device_depth = textureLoad(depth_tex, pixel, 0);
    // Reversed depth: 0 is sky/far, 1 is near. Do not trace sky.
    if (device_depth <= 1e-7 || device_depth >= 1.0) {
        write_result(pixel, 0.0, 0.0, 0.0, 0.0);
        return;
    }
    let uv0 = (vec2<f32>(pixel) + vec2<f32>(0.5)) / vec2<f32>(dims);
    let origin = unproject(uv0, device_depth);
    let ray_dir = normalize(params.light_view.xyz);
    let reach = max(params.tuning.x, 0.05);
    let thickness = max(params.tuning.y, 0.005);
    let budget = min(128u, max(1u, u32(params.tuning.z)));

    // Estimate projected pixels/metre at the receiver, then advance by
    // 1.25 px where feasible. A fixed budget must still cover the requested
    // physical distance: extremely close views therefore undersample.
    let reference = project_view(origin + ray_dir * 0.05);
    let pixels_per_m = select(
        0.0,
        length((vec2<f32>(reference.x, -reference.y) / max(reference.w, 1e-6) -
               vec2<f32>(2.0 * uv0.x - 1.0, 2.0 * uv0.y - 1.0)) * vec2<f32>(dims) * 0.5) / 0.05,
        reference.w > 1e-5,
    );
    let step_len = max(reach / f32(budget), 1.25 / max(pixels_per_m, 1.0));
    // Skip the first few projected pixels on the *receiver itself*.
    let start_t = max(0.015, 1.5 / max(pixels_per_m, 1.0));
    var visited = false;
    var was_offscreen = false;
    var shadow = 0.0;
    var hit = 0.0;
    var budget_miss = 0.0;

    for (var i = 0u; i < 128u; i += 1u) {
        if (i >= budget) { break; }
        let t = start_t + f32(i) * step_len;
        if (t > reach) { break; }
        let sample_vs = origin + ray_dir * t;
        let clip = project_view(sample_vs);
        if (clip.w < 1e-6) { was_offscreen = true; break; }
        let ndc = clip.xyz / clip.w;
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if (any(uv < vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0)) ||
            ndc.z <= 0.0 || ndc.z >= 1.0) {
            was_offscreen = true;
            break;
        }
        let q = clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - vec2<i32>(1));
        // Nearby samples of the *same* depth texel would merely self-hit.
        if (all(q == pixel)) { continue; }
        let scene_d = textureLoad(depth_tex, q, 0);
        if (scene_d <= 1e-7 || scene_d >= 1.0) { continue; }
        visited = true;
        let scene_vs = unproject((vec2<f32>(q) + vec2<f32>(0.5)) / vec2<f32>(dims), scene_d);
        // Positive gap: the ray has gone *behind* the visible first surface.
        // Use physical view-space z rather than nonlinear hardware depth.
        let gap = -sample_vs.z + scene_vs.z;
        // Reject far/other-layer geometry: a surface in front of the ray
        // by metres is not necessarily an actual ray/mesh intersection.
        let bias = max(0.006, 0.0015 * (-origin.z));
        if (gap > bias && gap < thickness) {
            let fade = clamp((reach - t) / max(reach * 0.35, 1e-4), 0.0, 1.0);
            shadow = fade;
            hit = 1.0;
            break;
        }
    }
    if (hit == 0.0 && !was_offscreen && visited) { budget_miss = 1.0; }
    write_result(pixel, shadow, hit, select(0.0, 1.0, was_offscreen), budget_miss);
}
