// Screen-space reflections on glossy surfaces. Concatenated after
// common.wgsl; drawn as a full-screen pass blended over the lit scene.

@group(3) @binding(0) var gbuf_tex: texture_2d<f32>;

struct SsrOut {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_ssr(@builtin(vertex_index) vi: u32) -> SsrOut {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var o: SsrOut;
    o.clip = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

fn oct_decode(e_in: vec2<f32>) -> vec3<f32> {
    let e = e_in * 2.0 - vec2<f32>(1.0);
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>((vec2<f32>(1.0) - abs(n.yx)) * s, n.z);
    }
    return normalize(n);
}

@fragment
fn fs_ssr(in: SsrOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.clip.xy);
    let g = textureLoad(gbuf_tex, px, 0);
    let roughness = g.z;
    let uv = screen_uv(in.clip.xy);
    let d = scene_depth_at(uv);
    if (roughness > 0.5 || d <= 0.0) {
        discard;
    }
    let n = oct_decode(g.xy);
    let p = world_from_depth(uv, d);
    let v = normalize(frame.camera_pos.xyz - p);
    let r = reflect(-v, n);
    let hit = ssr_trace(p + n * 0.03, r, 48);
    if (hit.a <= 0.0) {
        discard;
    }
    let metallic = g.w;
    let nv = max(dot(n, v), 1e-3);
    let f0 = mix(0.04, 1.0, metallic);
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - nv, 5.0);
    let smooth_k = clamp(1.0 - roughness * 2.0, 0.0, 1.0);
    let k = clamp(fresnel * smooth_k * smooth_k * hit.a, 0.0, 1.0);
    // premultiplied: mix the reflection over the lit color
    return vec4<f32>(hit.rgb * k, k);
}
