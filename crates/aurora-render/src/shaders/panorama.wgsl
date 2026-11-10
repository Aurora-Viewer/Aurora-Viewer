// The panorama viewer (panorama.rs): a full-screen triangle that looks into
// a 2:1 equirectangular image, and the 2x2 box blit that builds its mips.
//
// The seam: the texture coordinate jumps from 1 to 0 where the view crosses
// the longitude +-PI, so the implicit derivatives of `uv` there are huge and
// the hardware would pick the smallest mip for a one-pixel column. The
// gradients are taken from the (smooth) direction instead and handed to
// `textureSampleGrad`.

const PI: f32 = 3.14159265358979;
const TAU: f32 = 6.28318530717959;

struct View {
    // the view's forward axis, and the right and up axes already scaled by
    // tan(fov / 2) (and the aspect for right): ndc in [-1, 1] picks a ray
    forward: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var pano: texture_2d<f32>;
@group(0) @binding(2) var pano_sampler: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> VOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VOut;
    o.ndc = p * 2.0 - 1.0;
    o.pos = vec4<f32>(o.ndc, 0.0, 1.0);
    return o;
}

@fragment
fn fs_pano(in: VOut) -> @location(0) vec4<f32> {
    let d = normalize(view.forward.xyz + view.right.xyz * in.ndc.x + view.up.xyz * in.ndc.y);
    let ddx = dpdx(d);
    let ddy = dpdy(d);
    let lon = atan2(d.x, d.y);
    let lat = asin(clamp(d.z, -1.0, 1.0));
    let uv = vec2<f32>(lon / TAU + 0.5, 0.5 - lat / PI);
    // d(lon) = (y dx - x dy) / (x^2 + y^2), d(lat) = dz / sqrt(1 - z^2)
    let r2 = max(d.x * d.x + d.y * d.y, 1e-6);
    let k = 1.0 / sqrt(max(1.0 - d.z * d.z, 1e-6));
    let gx = vec2<f32>(clamp((d.y * ddx.x - d.x * ddx.y) / r2 / TAU, -0.5, 0.5), -ddx.z * k / PI);
    let gy = vec2<f32>(clamp((d.y * ddy.x - d.x * ddy.y) / r2 / TAU, -0.5, 0.5), -ddy.z * k / PI);
    return vec4<f32>(textureSampleGrad(pano, pano_sampler, uv, gx, gy).rgb, 1.0);
}

// One level of the mip chain from the level above it (a view of that single
// level): the target pixel's centre sits between four texels.
@fragment
fn fs_blit(in: VOut) -> @location(0) vec4<f32> {
    let uv = vec2<f32>(in.ndc.x * 0.5 + 0.5, 0.5 - in.ndc.y * 0.5);
    return textureSampleLevel(pano, pano_sampler, uv, 0.0);
}
