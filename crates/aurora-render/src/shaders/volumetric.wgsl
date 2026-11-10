// Volumetric sun rays: the light the air scatters toward the camera along
// each view ray, marched through the sun's shadow cascades, so that what
// casts a shadow on the ground casts a beam in the air, from any angle.
// Concatenated after common.wgsl; drawn at a reduced resolution, then added by
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
// The settings (renderer.rs VolU): the light the air scatters per metre,
// and the Henyey-Greenstein forward scattering, how tightly the glow
// gathers around the sun (0 even all around, toward 1 a tight glow).
struct VolParams {
    density: f32,
    g: f32,
    // the share of the scene's size drawn (the viewport's)
    res: f32,
    // the even part of the scattering, so that beams show from the side
    // and from behind too, not only toward the sun
    side: f32,
};
@group(3) @binding(0) var<uniform> vol: VolParams;

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
    // the viewport covers `res` of the image
    let uv = in.clip.xy * frame.screen.zw / vol.res;
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
    // the phase without its 1/4pi: an artistic gain, 1 across the light
    // with no forward scattering
    let g2 = vol.g * vol.g;
    let phase = (1.0 - g2) / pow(max(1.0 + g2 - 2.0 * vol.g * cos_t, 1e-4), 1.5) + vol.side;
    let scattered = 1.0 - exp(-len * vol.density);
    return vec4<f32>(frame.sun_color.rgb * (light * lit * phase * scattered), 1.0);
}
