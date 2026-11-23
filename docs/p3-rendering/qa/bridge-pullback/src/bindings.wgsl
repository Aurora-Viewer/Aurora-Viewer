struct Frame {
    cascade_vp: array<mat4x4<f32>,3>, detail_vp: mat4x4<f32>, detail_params:vec4<f32>,
    cascade_splits:vec4<f32>, light_mode:vec4<u32>, render_controls:vec4<u32>,
    camera_pos:vec4<f32>, params:vec4<f32>, misc:vec4<f32>, clip_plane:vec4<f32>, portrait_vp:mat4x4<f32>, portrait_depth:vec4<f32>
};
struct Probe { position:vec4<f32>, normal:vec4<f32> };
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(0) @binding(29) var detail_shadow_map: texture_depth_2d;
@group(0) @binding(30) var portrait_shadow_map:texture_depth_2d;
@group(0) @binding(3) var<storage,read> probes:array<Probe>;
@group(0) @binding(4) var<storage,read_write> results:array<vec4<f32>>;
