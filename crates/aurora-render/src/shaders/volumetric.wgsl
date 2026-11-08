// Volumetric sun rays: the light the air scatters toward the camera along
// each view ray, marched through the sun's shadow cascades, so that what
// casts a shadow on the ground casts a beam in the air, from any angle.
// Concatenated after common.wgsl; drawn at half resolution, then added by
// the post pass (post.wgsl `rays_tex`).

struct VolOut {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_volumetric(@builtin(vertex_index) vi: u32) -> VolOut {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var o: VolOut;
    o.clip = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

// Steps along each ray; the per-pixel offset turns their banding into noise
// that TAA (or the half-resolution filtering) smooths.
const VOL_STEPS: u32 = 24u;
// How much light the air scatters per metre.
const VOL_DENSITY: f32 = 0.008;
// Henyey-Greenstein forward scattering: brightest looking toward the sun\n// (without its 1/4pi: an artistic gain, 1 looking across the light).
const VOL_G: f32 = 0.6;

// Whether `p` sees the sun: one compare tap in its cascade (no filter, the
// march averages many), 1 past the cascades.
fn vol_lit(p: vec3<f32>) -> f32 {
    let splits = frame.cascade_splits;
    let d = distance(p, frame.camera_pos.xyz);
    if (d > splits.z) {
        return 1.0;
    }
    var cascade = 0u;
    if (d > splits.x) { cascade = 1u; }
    if (d > splits.y) { cascade = 2u; }
    let lp = frame.cascade_vp[cascade] * vec4<f32>(p, 1.0);
    let ndc = lp.xyz / lp.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || ndc.z < 0.0 || ndc.z > 1.0) {
        return 1.0;
    }
    let atlas = vec2<f32>(textureDimensions(shadow_map));
    let texel = 2.0 / atlas.x;
    let tile = vec2<f32>(f32(cascade % 2u), f32(cascade / 2u));
    let tile_scale = vec2<f32>(0.5, atlas.x / (2.0 * atlas.y));
    let t = clamp(uv, vec2<f32>(0.5 * texel), vec2<f32>(1.0 - 0.5 * texel));
    return textureSampleCompareLevel(shadow_map, shadow_sampler, (t + tile) * tile_scale, ndc.z - 1e-4);
}

@fragment
fn fs_volumetric(in: VolOut) -> @location(0) vec4<f32> {
    let light = frame.sun_dir.w;
    if (light <= 0.0 || frame.params.z < 0.5) {
        return vec4<f32>(0.0);
    }
    // the target is half the image
    let uv = in.clip.xy * 2.0 * frame.screen.zw;
    let d = scene_depth_at(uv);
    let far = frame.cascade_splits.z;
    let eye = frame.camera_pos.xyz;
    // the sky: as far as the shadows reach
    let end = world_from_depth(uv, select(1e-7, d, d > 0.0));
    let to_end = end - eye;
    var len = length(to_end);
    let dir = to_end / max(len, 1e-4);
    if (d <= 0.0 || len > far) {
        len = far;
    }
    let dt = len / f32(VOL_STEPS);
    let jitter = fract(52.9829189 * fract(dot(in.clip.xy, vec2<f32>(0.06711056, 0.00583715))));
    var lit = 0.0;
    for (var i = 0u; i < VOL_STEPS; i++) {
        lit += vol_lit(eye + dir * ((f32(i) + jitter) * dt));
    }
    lit /= f32(VOL_STEPS);
    let cos_t = dot(dir, normalize(frame.sun_dir.xyz));
    let g2 = VOL_G * VOL_G;
    let phase = (1.0 - g2) / (pow(max(1.0 + g2 - 2.0 * VOL_G * cos_t, 1e-4), 1.5));
    let scattered = 1.0 - exp(-len * VOL_DENSITY);
    return vec4<f32>(frame.sun_color.rgb * (light * lit * phase * scattered), 1.0);
}
