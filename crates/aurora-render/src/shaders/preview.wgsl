struct Uniforms {
    projection: mat4x4<f32>, eye: vec4<f32>, color: vec4<f32>,
    emissive_metallic: vec4<f32>, params: vec4<f32>,
    edge_color: vec4<f32>, ambient: vec4<f32>, surface: vec4<f32>,
    uv: array<mat2x4<f32>,4>,
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
        p = (m * vec4(position,1.0)).xyz;
        // Inverse transpose is required for the model's anisotropic
        // normalization and imported joint scales; a rotation-only product
        // applies the normalization scale twice to skinned normals.
        let a=m[0].xyz; let b=m[1].xyz; let c=m[2].xyz;
        let cofactors=mat3x3(cross(b,c),cross(c,a),cross(a,b));
        let determinant=dot(a,cross(b,c));
        if abs(determinant)>0.0000001 {n=normalize((cofactors*normal)/determinant);}
    }
    var out: Vertex; out.clip = u.projection * vec4(p, 1.0);
    out.position = p; out.normal = n; out.uv = uv; return out;
}
@fragment fn fs(v: Vertex, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let coord = vec4(v.uv,1.0,0.0);
    let base_uv = vec2(dot(u.uv[0][0],coord),dot(u.uv[0][1],coord));
    let normal_uv = vec2(dot(u.uv[1][0],coord),dot(u.uv[1][1],coord));
    let mr_uv = vec2(dot(u.uv[2][0],coord),dot(u.uv[2][1],coord));
    let emissive_uv = vec2(dot(u.uv[3][0],coord),dot(u.uv[3][1],coord));
    let base_color = textureSample(base, samp, base_uv) * u.color;
    let facing = select(-1.0, 1.0, front);
    var n = normalize(v.normal) * facing;
    // Cotangent frame from screen derivatives, the construction of
    // perturb_normal (common.wgsl), so that a map looks here as it will in
    // the world. The solution is divided by its determinant, whose sign
    // counts: negative here, where dpdy runs down the target, and without it
    // both axes come out reversed. `v.uv` samples top-left-origin images, so
    // the bitangent points down the image while SL and glTF normal maps put
    // +Y up it (materialF.glsl getNormal: B along +t): green is negated.
    // Back faces negate the whole mapped normal (pbropaqueF.glsl: tnorm *=
    // gl_FrontFacing ? 1.0 : -1.0); n is flipped above, the frame follows.
    // The floor only guards a face without UVs: a larger one would flatten
    // the relief of small models, whose derivatives are tiny.
    let dp1 = dpdx(v.position); let dp2 = dpdy(v.position);
    let duv1 = dpdx(normal_uv); let duv2 = dpdy(normal_uv);
    let dp2perp = cross(dp2, n); let dp1perp = cross(n, dp1);
    let tangent = dp2perp * duv1.x + dp1perp * duv2.x;
    let bitangent = dp2perp * duv1.y + dp1perp * duv2.y;
    let norm = max(max(dot(tangent, tangent), dot(bitangent, bitangent)), 1e-30);
    let frame = facing * sign(dot(dp1, dp2perp)) * inverseSqrt(norm);
    var nm = textureSample(normal_map, samp, normal_uv).xyz * 2.0 - 1.0;
    nm = vec3(nm.x*u.surface.x,-nm.y*u.surface.x,nm.z);
    n = normalize(mat3x3(tangent * frame, bitangent * frame, n) * nm);
    let view = normalize(u.eye.xyz - v.position);
    let light = normalize(vec3<f32>(0.4, -0.6, 0.8));
    let half_vector = normalize(light + view);
    let metallic_roughness = textureSample(mr, samp, mr_uv);
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
    var color = (diffuse + specular) * n_dot_l * 2.4 + (base_color.rgb * u.ambient.rgb * (1.0 - metallic) + f0 * 0.15) * metallic_roughness.r;
    color += textureSample(emissive_map, samp, emissive_uv).rgb * u.emissive_metallic.xyz;
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
