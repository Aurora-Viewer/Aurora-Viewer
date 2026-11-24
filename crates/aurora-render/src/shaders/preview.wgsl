struct Uniforms {
    projection: mat4x4<f32>, eye: vec4<f32>, color: vec4<f32>,
    emissive_metallic: vec4<f32>, params: vec4<f32>,
    edge_color: vec4<f32>, ambient: vec4<f32>, surface: vec4<f32>,
    uv: array<mat2x4<f32>,4>,
    shadow: mat4x4<f32>,
    // lit in a studio, the camera's yaw (cosine, sine), the outline's class
    studio: vec4<f32>,
    // the floor: height, radius, whether there is one
    floor: vec4<f32>,
    ground: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var base: texture_2d<f32>;
@group(0) @binding(2) var normal_map: texture_2d<f32>;
@group(0) @binding(3) var mr: texture_2d<f32>;
@group(0) @binding(4) var emissive_map: texture_2d<f32>;
@group(0) @binding(5) var samp: sampler;
@group(0) @binding(6) var<storage,read> bones: array<mat4x4<f32>>;
@group(1) @binding(0) var shadow_map: texture_depth_2d;
@group(1) @binding(1) var shadow_samp: sampler_comparison;
struct Vertex {
    @builtin(position) clip: vec4<f32>, @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>,
}
struct Posed { p: vec3<f32>, n: vec3<f32> }
fn posed(position: vec3<f32>, normal: vec3<f32>, joints: vec4<u32>, weights: vec4<f32>) -> Posed {
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
    return Posed(p, n);
}
@vertex fn vs(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>,
    @location(3) joints: vec4<u32>, @location(4) weights: vec4<f32>) -> Vertex {
    let at = posed(position, normal, joints, weights);
    var out: Vertex; out.clip = u.projection * vec4(at.p, 1.0);
    out.position = at.p; out.normal = at.n; out.uv = uv; return out;
}
// The key light's view of the model, for its shadows.
@vertex fn vs_shadow(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>,
    @location(3) joints: vec4<u32>, @location(4) weights: vec4<f32>) -> Vertex {
    let at = posed(position, normal, joints, weights);
    var out: Vertex; out.clip = u.shadow * vec4(at.p, 1.0);
    out.position = at.p; out.normal = at.n; out.uv = uv; return out;
}
// What a cut-out or a see-through face leaves out.
fn opaque_at(uv: vec2<f32>) -> f32 {
    let coord = vec4(uv,1.0,0.0);
    let at = vec2(dot(u.uv[0][0],coord),dot(u.uv[0][1],coord));
    let alpha = textureSample(base, samp, at).a * u.color.a;
    if u.params.z == 2.0 { return alpha - u.params.y; }
    if u.params.z == 1.0 { return alpha - 0.4; }
    return 1.0;
}
@fragment fn fs_shadow(v: Vertex) {
    if opaque_at(v.uv) < 0.0 { discard; }
}
@fragment fn fs_mask(v: Vertex) -> @location(0) vec4<f32> {
    if opaque_at(v.uv) < 0.0 { discard; }
    return vec4(select(0.0,1.0,u.studio.w==1.0), select(0.0,1.0,u.studio.w==2.0), 0.0, 1.0);
}

// ---- the studio: it turns with the camera, so that what is looked at is lit
// the same from every side.

// A direction in the camera's frame: towards it, to its right, up.
fn in_studio(d: vec3<f32>) -> vec3<f32> {
    return vec3(d.x*u.studio.y + d.y*u.studio.z, -d.x*u.studio.z + d.y*u.studio.y, d.z);
}
// Light from around an axis, seen through a lobe as sharp as `s`: two
// spherical Gaussians convolved, which keeps their energy (a rough surface
// blurs the lamps it reflects).
fn lamp(d: vec3<f32>, axis: vec3<f32>, sharp: f32, color: vec3<f32>, s: f32) -> vec3<f32> {
    let both = sharp * s / (sharp + s);
    return color * (both / sharp) * exp(both * (dot(d, normalize(axis)) - 1.0));
}
// What the studio sends from `d`; the key lamp apart, for its shadows.
fn surroundings(d: vec3<f32>, s: f32) -> vec3<f32> {
    let height = smoothstep(-0.9, 1.0, d.z);
    var light = mix(vec3(0.10,0.105,0.12), vec3(0.50,0.53,0.60), height);
    light += lamp(d, vec3(0.45, 0.85, 0.10), 8.0, vec3(0.95,1.05,1.25), s);
    light += lamp(d, vec3(-0.75,-0.50, 0.45), 22.0, vec3(6.0,6.2,6.8), s);
    light += lamp(d, vec3(-0.75, 0.60, 0.30), 22.0, vec3(4.6,4.4,4.2), s);
    light += lamp(d, vec3(0.05, 0.0, 1.0), 14.0, vec3(1.5), s);
    return light;
}
const KEY = vec3<f32>(0.55, -0.6, 0.62);
fn key_color() -> vec3<f32> { return vec3(1.0, 0.965, 0.92); }
// How much of the key light reaches a point.
fn lit(p: vec3<f32>) -> f32 {
    let at = u.shadow * vec4(p, 1.0);
    let uv = vec2(at.x*0.5+0.5, 0.5-at.y*0.5);
    if any(uv < vec2(0.0)) || any(uv > vec2(1.0)) || at.z > 1.0 { return 1.0; }
    let texel = 1.0/vec2<f32>(textureDimensions(shadow_map));
    var sum = 0.0;
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            sum += textureSampleCompareLevel(shadow_map, shadow_samp, uv + vec2(f32(x),f32(y))*texel*1.4, at.z - 0.0006);
        }
    }
    return sum / 25.0;
}
// Khronos PBR Neutral: the colours of the surface stay what they are, only
// the strongest light is eased into white.
fn neutral(c: vec3<f32>) -> vec3<f32> {
    let start = 0.8 - 0.04; let desaturation = 0.15;
    let x = min(c.r, min(c.g, c.b));
    let offset = select(0.04, x - 6.25*x*x, x < 0.08);
    let color = c - offset;
    let peak = max(color.r, max(color.g, color.b));
    if peak < start { return color; }
    let d = 1.0 - start;
    let eased = 1.0 - d*d/(peak + d - start);
    let g = 1.0 - 1.0/(desaturation*(peak - eased) + 1.0);
    return mix(color*eased/peak, vec3(eased), g);
}
struct Surface { base: vec3<f32>, metallic: f32, roughness: f32, occlusion: f32 }
fn studio_light(m: Surface, n: vec3<f32>, view: vec3<f32>, p: vec3<f32>) -> vec3<f32> {
    let ndv = clamp(dot(n, view), 0.001, 1.0);
    let alpha = m.roughness*m.roughness;
    let a2 = max(alpha*alpha, 0.0004);
    let f0 = mix(vec3(0.04), m.base, m.metallic);
    let albedo = m.base*(1.0-m.metallic);
    let sn = in_studio(n);
    let sr = in_studio(reflect(-view, n));
    let shade = lit(p + n*0.004);
    // the surroundings: soft over the surface, sharper in its reflections
    let sharp = 2.0/a2;
    var around = surroundings(sr, sharp);
    around += lamp(sr, KEY, 26.0, key_color()*5.0, sharp)*mix(0.25, 1.0, shade);
    // the specular part of the split sum (Karis, "Physically based shading on mobile")
    let c0 = vec4(-1.0,-0.0275,-0.572,0.022); let c1 = vec4(1.0,0.0425,1.04,-0.04);
    let r = m.roughness*c0 + c1;
    let a004 = min(r.x*r.x, exp2(-9.28*ndv))*r.x + r.y;
    let ab = vec2(-1.04,1.04)*a004 + r.zw;
    // a reflection of what the surface hides from itself
    let hidden = clamp(1.0 + dot(reflect(-view, n), n), 0.0, 1.0);
    let specular = around*(f0*ab.x + ab.y)*m.occlusion*hidden;
    let diffuse = albedo*surroundings(sn, 2.0)*m.occlusion;
    // the key light
    let l = normalize(KEY);
    let toward = vec3(l.x*u.studio.y - l.y*u.studio.z, l.x*u.studio.z + l.y*u.studio.y, l.z);
    let h = normalize(toward + view);
    let ndl = max(dot(n, toward), 0.0);
    let ndh = max(dot(n, h), 0.0);
    let denom = ndh*ndh*(a2-1.0)+1.0;
    let dist = min(a2/(3.14159265*denom*denom), 60.0);
    let k = (m.roughness+1.0)*(m.roughness+1.0)/8.0;
    let vis = 1.0/max(4.0*(ndl*(1.0-k)+k)*(ndv*(1.0-k)+k), 0.02);
    let fresnel = f0 + (1.0-f0)*pow(1.0-max(dot(view,h),0.0), 5.0);
    let key = key_color()*2.9*ndl*shade*(albedo/3.14159265*(1.0-fresnel) + min(dist*vis*fresnel, vec3(4.0)));
    return diffuse + specular + key;
}
// The ground behind the model, lighter in its middle.
struct Ground { @builtin(position) clip: vec4<f32>, @location(0) at: vec2<f32> }
@vertex fn vs_ground(@builtin(vertex_index) i: u32) -> Ground {
    let p = vec2(f32((i << 1u) & 2u), f32(i & 2u))*2.0 - 1.0;
    return Ground(vec4(p, 0.0, 1.0), p);
}
@fragment fn fs_ground(v: Ground) -> @location(0) vec4<f32> {
    let glow = 1.0 - smoothstep(0.0, 1.35, length(v.at*vec2(0.85,0.9)));
    let c = u.ground.rgb + vec3(0.085,0.092,0.115)*glow*glow;
    // (no steps in the gradient)
    let noise = fract(52.9829189*fract(dot(v.clip.xy, vec2(0.06711056,0.00583715)))) - 0.5;
    return vec4(c + noise/255.0, 1.0);
}
struct Floor { @builtin(position) clip: vec4<f32>, @location(0) position: vec3<f32>, @location(1) at: vec2<f32> }
@vertex fn vs_floor(@builtin(vertex_index) i: u32) -> Floor {
    var corners = array<vec2<f32>,6>(vec2(-1.0,-1.0),vec2(1.0,-1.0),vec2(1.0,1.0),vec2(-1.0,-1.0),vec2(1.0,1.0),vec2(-1.0,1.0));
    let p = vec3(corners[i]*u.floor.y, u.floor.x);
    return Floor(u.projection*vec4(p,1.0), p, corners[i]);
}
@fragment fn fs_floor(v: Floor) -> @location(0) vec4<f32> {
    if u.floor.z < 0.5 { discard; }
    let fade = 1.0 - smoothstep(0.25, 0.75, length(v.at));
    return vec4(0.0, 0.0, 0.0, (1.0 - lit(v.position))*0.38*fade);
}
@fragment fn fs(v: Vertex, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let coord = vec4(v.uv,1.0,0.0);
    let base_uv = vec2(dot(u.uv[0][0],coord),dot(u.uv[0][1],coord));
    let normal_uv = vec2(dot(u.uv[1][0],coord),dot(u.uv[1][1],coord));
    let mr_uv = vec2(dot(u.uv[2][0],coord),dot(u.uv[2][1],coord));
    let emissive_uv = vec2(dot(u.uv[3][0],coord),dot(u.uv[3][1],coord));
    let base_color = textureSample(base, samp, base_uv) * u.color;
    // how much of the pixel a cut-out covers, its edge one pixel wide
    let covered = clamp((base_color.a - u.params.y) / max(fwidth(base_color.a), 0.0001) + 0.5, 0.0, 1.0);
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
    if u.studio.x > 0.5 {
        let m = Surface(base_color.rgb, clamp(metallic,0.0,1.0), clamp(roughness,0.045,1.0), metallic_roughness.r);
        let emitted = textureSampleLevel(emissive_map, samp, emissive_uv, 0.0).rgb * u.emissive_metallic.xyz;
        color = neutral(studio_light(m, n, view, v.position) + emitted);
    } else {
        color = color / (vec3<f32>(1.0) + color);
    }
    if u.params.w > 0.5 {
        let grid = abs(fract(v.uv * 10.0) - vec2<f32>(0.5));
        if max(grid.x, grid.y) > 0.48 { color = u.edge_color.rgb; }
    }
    var opacity = select(1.0, base_color.a, u.params.z == 1.0);
    if u.params.z == 2.0 {
        // several samples a pixel: the coverage smooths the edge
        if u.surface.z > 0.5 { opacity = covered; if covered <= 0.0 { discard; } }
        else if base_color.a < u.params.y { discard; }
    }
    if !front && u.surface.y < 0.5 { discard; }
    let shown = pow(max(color, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    return vec4(shown, opacity);
}
@fragment fn fs_edge(v: Vertex) -> @location(0) vec4<f32> { return u.edge_color; }
