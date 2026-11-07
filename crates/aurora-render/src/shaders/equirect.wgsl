// The 360° snapshot's equirectangular image from its six cube faces
// (equirect.rs). One thread per output pixel: its direction in the z-up
// world, the cube sampled four times across the pixel (the hardware
// filters across the faces' edges; the mean smooths the faces finer than
// the panorama, High and Maximum), the mean written.

const PI: f32 = 3.14159265358979;
const TAU: f32 = 6.28318530717959;

struct Params {
    width: u32,
    height: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var cube: texture_cube<f32>;
@group(0) @binding(1) var cube_sampler: sampler;
@group(0) @binding(2) var out: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(3) var<uniform> params: Params;

// The panorama's centre column looks north (+Y), east (+X) is a quarter
// of the width to its right, the top row is the zenith (+Z).
fn direction(x: f32, y: f32) -> vec3<f32> {
    let heading = (x / f32(params.width) - 0.5) * TAU;
    let pitch = (0.5 - y / f32(params.height)) * PI;
    let c = cos(pitch);
    return vec3<f32>(sin(heading) * c, cos(heading) * c, sin(pitch));
}

@compute @workgroup_size(8, 8, 1)
fn cs_equirect(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    var sum = vec3<f32>(0.0);
    for (var j = 0u; j < 2u; j++) {
        for (var i = 0u; i < 2u; i++) {
            let d = direction(f32(id.x) + 0.25 + 0.5 * f32(i), f32(id.y) + 0.25 + 0.5 * f32(j));
            // the faces were drawn by ordinary cameras, which a cube map's
            // faces mirror: the cube is read with x reversed (equirect.rs)
            sum += textureSampleLevel(cube, cube_sampler, vec3<f32>(-d.x, d.y, d.z), 0.0).rgb;
        }
    }
    textureStore(out, vec2<i32>(id.xy), vec4<f32>(sum * 0.25, 1.0));
}
