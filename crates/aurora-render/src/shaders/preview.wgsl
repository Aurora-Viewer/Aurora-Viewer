struct Uniforms {
    projection: mat4x4<f32>, eye: vec4<f32>, color: vec4<f32>,
    emissive_metallic: vec4<f32>, params: vec4<f32>,
    edge_color: vec4<f32>, ambient: vec4<f32>, surface: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var base: texture_2d<f32>;
@group(0) @binding(2) var normal_map: texture_2d<f32>;
@group(0) @binding(3) var mr: texture_2d<f32>;
@group(0) @binding(4) var emissive_map: texture_2d<f32>;
@group(0) @binding(5) var samp: sampler;
@group(0) @binding(6) var<storage,read> bones: array<mat4x4<f32>>;
struct Vertex {
    @builtin(position) clip: vec4<f32>, @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>,
}
@vertex fn vs(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>,
    @location(3) joints: vec4<u32>, @location(4) weights: vec4<f32>) -> Vertex {
    var p = position; var n = normal;
    let sum = dot(weights,vec4<f32>(1.0));
    if sum > 0.00001 {
        let j = min(joints,vec4<u32>(arrayLength(&bones)-1u));
        let m = (bones[j.x]*weights.x + bones[j.y]*weights.y + bones[j.z]*weights.z + bones[j.w]*weights.w) * (1.0/sum);
        p = (m * vec4(position,1.0)).xyz; n = normalize((m * vec4(normal,0.0)).xyz);
    }
    var out: Vertex; out.clip = u.projection * vec4(p, 1.0);
    out.position = p; out.normal = n; out.uv = uv; return out;
}
@fragment fn fs(v: Vertex, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let base_color = textureSample(base, samp, v.uv) * u.color;
    var n = normalize(v.normal) * select(-1.0, 1.0, front);
    let dp1 = dpdx(v.position); let dp2 = dpdy(v.position);
    let duv1 = dpdx(v.uv); let duv2 = dpdy(v.uv);
    let tangent = dp1 * duv2.y - dp2 * duv1.y;
    let bitangent = -dp1 * duv2.x + dp2 * duv1.x;
    let norm = max(max(dot(tangent, tangent), dot(bitangent, bitangent)), 0.00000001);
    var nm = textureSample(normal_map, samp, v.uv).xyz * 2.0 - 1.0;
    nm = vec3(nm.xy*u.surface.x,nm.z);
    n = normalize(mat3x3(tangent * inverseSqrt(norm), bitangent * inverseSqrt(norm), n) * nm);
    let view = normalize(u.eye.xyz - v.position);
    let light = normalize(vec3<f32>(0.4, -0.6, 0.8));
    let half_vector = normalize(light + view);
    let metallic_roughness = textureSample(mr, samp, v.uv);
    let metallic = metallic_roughness.b * u.emissive_metallic.w;
    let roughness = max(metallic_roughness.g * u.params.x, 0.045);
    let n_dot_l = max(dot(n, light), 0.0);
    let n_dot_v = max(dot(n, view), 0.001);
    let n_dot_h = max(dot(n, half_vector), 0.0);
    let f0 = mix(vec3<f32>(0.04), base_color.rgb, vec3<f32>(metallic));
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - max(dot(view, half_vector), 0.0), 5.0);
    let alpha = roughness * roughness;
    let a2 = alpha * alpha;
    let denom = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    let distribution = a2 / max(3.14159265 * denom * denom, 0.00001);
    let k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    let geometry = n_dot_l / (n_dot_l * (1.0 - k) + k) * n_dot_v / (n_dot_v * (1.0 - k) + k);
    let specular = distribution * geometry * fresnel / max(4.0 * n_dot_l * n_dot_v, 0.001);
    let diffuse = (vec3<f32>(1.0) - fresnel) * (1.0 - metallic) * base_color.rgb / 3.14159265;
    var color = (diffuse + specular) * n_dot_l * 2.4 + base_color.rgb * u.ambient.rgb * (1.0 - metallic) + f0 * 0.15;
    color += textureSample(emissive_map, samp, v.uv).rgb * u.emissive_metallic.xyz;
    color = color / (vec3<f32>(1.0) + color);
    if u.params.w > 0.5 {
        let grid = abs(fract(v.uv * 10.0) - vec2<f32>(0.5));
        if max(grid.x, grid.y) > 0.48 { color = u.edge_color.rgb; }
    }
    let opacity = select(1.0, base_color.a, u.params.z == 1.0);
    if u.params.z == 2.0 && base_color.a < u.params.y { discard; }
    if !front && u.surface.y < 0.5 { discard; }
    return vec4(pow(max(color, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)), opacity);
}
@fragment fn fs_edge(v: Vertex) -> @location(0) vec4<f32> { return u.edge_color; }
