// DLSS Ray Reconstruction's guides (dlss.rs, RenderSettings::rr_guides),
// one pass at the render size, from the scene pass's G-buffer
// (octahedral world normal rg, roughness b, metallic a: object.wgsl
// gbuffer) and its base colour target.
//
// - diffuse albedo: the base colour without the metals';
// - specular albedo: the split-sum environment BRDF of the base colour's
//   F0 (Karis's analytic fit, as NVIDIA's RR guide suggests), at the view
//   angle;
// - normals: world space, roughness in w (RR's packed roughness mode).
//
// Where the renderer drew a blended face opaque to the eye over the opaque
// surface (renderer.rs `RrInputs`: its depth and motion are the face's, its
// normal in the motion's xy marked by NORMAL_RR_FACE on y), the guides are
// that face's too, as NVIDIA's RR guide asks of depth, motion and normal
// (one surface for all three, 3.4.1): its mesh normal, matte, and the
// colour drawn as its albedo, as the impostor cards give theirs
// (object.wgsl fs_impostor_g). Fainter blended faces and the water keep the
// surface behind them (3.4.1, 3.5): only their colour is theirs.
//
// fs_show, a debug pass at the display size: one guide drawn over DLSS's
// image (AURORA_DLSS_RR_SHOW).
struct U {
    // this frame's unjittered clip space to world space
    inv_view_proj: mat4x4<f32>,
    // the camera, world space
    eye: vec4<f32>,
};
@group(0) @binding(0) var depth_tex: texture_depth_2d;
@group(0) @binding(1) var gbuf_tex: texture_2d<f32>;
@group(0) @binding(2) var albedo_tex: texture_2d<f32>;
@group(0) @binding(3) var<uniform> u: U;
// xy: the octahedral normal (object.wgsl depth_normal; plus
// NORMAL_COMPOSITION on x, plus NORMAL_RR_FACE on y on a blended face)
@group(0) @binding(5) var motion_tex: texture_2d<f32>;
// the scene, HDR, at the render size
@group(0) @binding(6) var color_tex: texture_2d<f32>;

// object.wgsl NORMAL_RR_FACE, common.wgsl NORMAL_COMPOSITION
const NORMAL_RR_FACE: f32 = 2.0;
const NORMAL_COMPOSITION: f32 = 2.0;

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex fn vs_main(@builtin(vertex_index) index: u32) -> Out {
    let p = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return Out(vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0), vec2<f32>(p.x, 1.0 - p.y));
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

// Karis, "Physically Based Shading on Mobile" (2014): the split-sum
// environment BRDF's analytic approximation
fn env_brdf_approx(f0: vec3<f32>, roughness: f32, n_dot_v: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + vec3<f32>(ab.y);
}

struct Guides {
    @location(0) diffuse: vec4<f32>,
    @location(1) specular: vec4<f32>,
    @location(2) normals: vec4<f32>,
};

@fragment fn fs_main(in: Out) -> Guides {
    let p = vec2<i32>(in.pos.xy);
    let depth = textureLoad(depth_tex, p, 0);
    // the sky (reversed Z: nothing drawn is 0): no surface
    if (depth <= 0.0) {
        return Guides(vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0, 0.0, 1.0, 1.0));
    }
    var g = textureLoad(gbuf_tex, p, 0);
    var base = textureLoad(albedo_tex, p, 0).rgb;
    let m = textureLoad(motion_tex, p, 0).xy;
    if (m.y >= NORMAL_RR_FACE - 0.5) {
        let e = m - vec2<f32>(select(0.0, NORMAL_COMPOSITION, m.x >= NORMAL_COMPOSITION - 0.5), NORMAL_RR_FACE);
        g = vec4<f32>(e, 1.0, 0.0);
        base = min(textureLoad(color_tex, p, 0).rgb, vec3<f32>(1.0));
    }
    let n = oct_decode(g.xy);
    let roughness = g.z;
    let metallic = g.w;
    let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let wp = u.inv_view_proj * vec4<f32>(ndc, depth, 1.0);
    let v = normalize(u.eye.xyz - wp.xyz / wp.w);
    let n_dot_v = clamp(dot(n, v), 1e-4, 1.0);
    let f0 = mix(vec3<f32>(0.04), base, metallic);
    let specular = env_brdf_approx(f0, roughness, n_dot_v);
    return Guides(
        vec4<f32>(base * (1.0 - metallic), 1.0),
        vec4<f32>(clamp(specular, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0),
        vec4<f32>(n, roughness),
    );
}

// one guide, drawn whole (debug)
@group(0) @binding(4) var show_tex: texture_2d<f32>;
// 0 rgb, 1 xyz as colour (normals), 2 w (roughness)
override show_mode: f32 = 0.0;
// the post pass's exposure and tone map follow: the guide is scaled down
// by this to be seen (AURORA_DLSS_RR_SHOW_SCALE)
override show_scale: f32 = 0.1;

@fragment fn fs_show(in: Out) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(show_tex));
    let p = clamp(vec2<i32>(in.uv * vec2<f32>(size)), vec2<i32>(0), size - vec2<i32>(1));
    let t = textureLoad(show_tex, p, 0);
    var c = t.rgb;
    if (show_mode > 1.5) {
        c = vec3<f32>(t.w);
    } else if (show_mode > 0.5) {
        c = t.xyz * 0.5 + vec3<f32>(0.5);
    }
    return vec4<f32>(c * show_scale, 1.0);
}
