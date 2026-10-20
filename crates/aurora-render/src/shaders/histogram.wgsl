// The luminance histogram of the photo viewfinder, in one dispatch of one
// workgroup: a 128 x 72 grid over the whole scene image (resolved, before
// the tone mapping), each cell's luminance times the exposure, encoded for
// display (x / (1 + x), then the sRGB gamma) and counted in 64 bins. The
// counts are read back without waiting (histogram.rs).

struct Params {
    // x the exposure the frame is shown with, yzw unused
    a: vec4<f32>,
};

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var<uniform> p: Params;
@group(0) @binding(2) var<storage, read_write> bins: array<u32, 64>;

const COLS: u32 = 128u;
const ROWS: u32 = 72u;

var<workgroup> counts: array<atomic<u32>, 64>;

@compute @workgroup_size(16, 16)
fn cs_histogram(@builtin(local_invocation_id) lid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    if (li < 64u) {
        atomicStore(&counts[li], 0u);
    }
    workgroupBarrier();
    let dims = textureDimensions(scene);
    // each thread takes 8 x 4.5 cells, interleaved
    for (var j = lid.y; j < ROWS; j += 16u) {
        for (var i = lid.x; i < COLS; i += 16u) {
            let uv = (vec2<f32>(f32(i), f32(j)) + 0.5) / vec2<f32>(f32(COLS), f32(ROWS));
            let px = min(vec2<u32>(uv * vec2<f32>(dims)), dims - vec2<u32>(1u));
            let c = textureLoad(scene, px, 0).rgb;
            var l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)) * p.a.x;
            // NaN and negative light count as black
            l = select(0.0, max(l, 0.0), l == l);
            let v = pow(l / (1.0 + l), 1.0 / 2.2);
            let bin = min(u32(v * 64.0), 63u);
            atomicAdd(&counts[bin], 1u);
        }
    }
    workgroupBarrier();
    if (li < 64u) {
        bins[li] = atomicLoad(&counts[li]);
    }
}
