// The see-through fragments of each pixel over what the opaque faces drew
// (`blend_compose` of object.wgsl comes before).
@group(0) @binding(0) var solid: texture_2d<f32>;

@vertex fn vs_screen(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4(vec2(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0, 0.0, 1.0);
}

@fragment fn fs_compose(@builtin(position) clip: vec4<f32>) -> @location(0) vec4<f32> {
    let under = textureLoad(solid, vec2<i32>(clip.xy), 0).rgb;
    let composed = blend_compose(clip, true);
    if composed.through >= 1.0 { return vec4(under, 1.0); }
    // in light, as the world
    let c = pow(under, vec3(2.2)) * composed.through + composed.light;
    return vec4(pow(max(c, vec3(0.0)), vec3(1.0 / 2.2)), 1.0);
}
