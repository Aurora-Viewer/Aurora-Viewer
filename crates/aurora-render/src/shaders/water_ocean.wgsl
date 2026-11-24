// Enhanced water's finer wind-sea maps, on the GPU (water_ocean_gpu.rs):
// the short cascade's waves again on a grid four times finer, and a finest
// cascade of the ripples past them (some 4 cm to 0.43 m), from the initial
// spectrum the CPU lays out (water_fft.rs `FineSpectrum`), evolved to the
// frame's time, brought back by inverse FFTs in workgroup memory, unpacked
// into the maps the surface's normals and foam read (common.wgsl
// `water_fx_fine`), their foam kept from frame to frame, and their mips.
// The fields and their packing are the CPU's (water_fft.rs `field`).

struct OceanParams {
    // time (s, modulo the loop), chop, scale, foam's dt (s)
    time: vec4<f32>,
    // wind toward xy (unit), the cascades' tiles (m)
    wind: vec4<f32>,
    // x the wind's speed, y and z how fast each cascade's crests carry
    // their foam (m/s, water_fft.rs `ride_speed`)
    ride: vec4<f32>,
};

const N: u32 = 256u;
const G: f32 = 9.81;
const TAU: f32 = 6.283185307;
// water_fft.rs OCEAN_LOOP
const LOOP: f32 = 1024.0;
// water_fft.rs FOAM_LINGER, FOAM_STREAK
const LINGER: f32 = 4.5;
const STREAK: f32 = 0.9;
// water_fft.rs FOAM_RIDE
const RIDE: f32 = 1.0;

@group(0) @binding(0) var<uniform> params: OceanParams;
@group(0) @binding(1) var h0: texture_2d_array<f32>;
@group(0) @binding(2) var<storage, read_write> fields: array<vec2<f32>>;
@group(0) @binding(3) var<storage, read_write> spare: array<vec2<f32>>;
@group(0) @binding(4) var<storage, read> foam_before: array<f32>;
@group(0) @binding(5) var<storage, read_write> foam_now: array<f32>;
@group(0) @binding(6) var maps: texture_storage_2d_array<rgba16float, write>;

fn at(cascade: u32, field: u32, x: u32, y: u32) -> u32 {
    return ((cascade * 4u + field) * N + y) * N + x;
}

fn frequency(i: u32) -> f32 {
    return select(f32(i), f32(i) - f32(N), i >= N / 2u);
}

fn cmul(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
}

fn times_i(a: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(-a.y, a.x);
}

// water_fft.rs `omega`: on the loop's lattice
fn omega(len: f32) -> f32 {
    let step = TAU / LOOP;
    return floor(sqrt(G * len) / step) * step;
}

// The four packed fields of each cascade at the frame's time:
// 0 (Dx, Dy), 1 (h, ∂Dx/∂y), 2 (∂h/∂x, ∂h/∂y), 3 (∂Dx/∂x, ∂Dy/∂y).
@compute @workgroup_size(16, 16, 1)
fn cs_spectrum(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = id.z;
    let tile = select(params.wind.z, params.wind.w, c == 1u);
    let k = vec2<f32>(frequency(id.x), frequency(id.y)) * (TAU / tile);
    let len = length(k);
    var out = array<vec2<f32>, 4>(vec2<f32>(0.0), vec2<f32>(0.0), vec2<f32>(0.0), vec2<f32>(0.0));
    if (len > 1e-6) {
        let pair = textureLoad(h0, vec2<i32>(id.xy), i32(c), 0);
        // (the phase in turns first: the time is already within the loop)
        let turns = fract(omega(len) * params.time.x / TAU);
        let phase = turns * TAU;
        let e = vec2<f32>(cos(phase), sin(phase));
        let scale = params.time.z;
        let chop = params.time.y;
        let h = (cmul(pair.xy, e) + cmul(pair.zw, vec2<f32>(e.x, -e.y))) * scale;
        let ih = times_i(h) * (chop / len);
        out[0] = ih * k.x + times_i(ih * k.y);
        out[1] = h + times_i(h * (-k.x * k.y * chop / len));
        out[2] = times_i(h) * k.x + times_i(times_i(h) * k.y);
        out[3] = h * (-k.x * k.x * chop / len) + times_i(h * (-k.y * k.y * chop / len));
    }
    for (var f = 0u; f < 4u; f++) {
        fields[at(c, f, id.x, id.y)] = out[f];
    }
}

var<workgroup> line: array<vec2<f32>, 256>;

fn reverse8(i: u32) -> u32 {
    return reverseBits(i) >> 24u;
}

// One inverse FFT of 256 in workgroup memory (water_fft.rs inverse_fft:
// e^{+i}, no 1/n), 128 threads a butterfly each per stage.
fn line_fft(t: u32) {
    for (var len = 2u; len <= N; len = len << 1u) {
        let half = len >> 1u;
        let j = t % half;
        let i0 = (t / half) * len + j;
        let i1 = i0 + half;
        let angle = TAU * f32(j) / f32(len);
        let w = vec2<f32>(cos(angle), sin(angle));
        let a = line[i0];
        let b = cmul(line[i1], w);
        line[i0] = a + b;
        line[i1] = a - b;
        workgroupBarrier();
    }
}

// Rows: `fields` into `spare`. One workgroup per row of each field.
@compute @workgroup_size(128, 1, 1)
fn cs_rows(@builtin(local_invocation_id) local: vec3<u32>, @builtin(workgroup_id) group: vec3<u32>) {
    let t = local.x;
    let y = group.x;
    let slot = group.y;
    let base = (slot * N + y) * N;
    line[reverse8(t)] = fields[base + t];
    line[reverse8(t + 128u)] = fields[base + t + 128u];
    workgroupBarrier();
    line_fft(t);
    spare[base + t] = line[t];
    spare[base + t + 128u] = line[t + 128u];
}

// Columns: `spare` back into `fields`.
@compute @workgroup_size(128, 1, 1)
fn cs_columns(@builtin(local_invocation_id) local: vec3<u32>, @builtin(workgroup_id) group: vec3<u32>) {
    let t = local.x;
    let x = group.x;
    let slot = group.y;
    let base = slot * N * N + x;
    line[reverse8(t)] = spare[base + t * N];
    line[reverse8(t + 128u)] = spare[base + (t + 128u) * N];
    workgroupBarrier();
    line_fft(t);
    fields[base + t * N] = line[t];
    fields[base + (t + 128u) * N] = line[t + 128u];
}

// (`c` 0 and 1 the cascades' foam left behind, 2 and 3 their riding foam)
fn foam_at(c: u32, x: f32, y: f32) -> f32 {
    let b = floor(vec2<f32>(x, y));
    let f = vec2<f32>(x, y) - b;
    let n = i32(N);
    let i0 = (vec2<i32>(b) % n + n) % n;
    let i1 = (i0 + 1) % n;
    let o = c * N * N;
    let v00 = foam_before[o + u32(i0.y) * N + u32(i0.x)];
    let v10 = foam_before[o + u32(i0.y) * N + u32(i1.x)];
    let v01 = foam_before[o + u32(i1.y) * N + u32(i0.x)];
    let v11 = foam_before[o + u32(i1.y) * N + u32(i1.x)];
    return mix(mix(v00, v10, f.x), mix(v01, v11, f.x), f.y);
}

// The maps (layers: per cascade its displacement then its slopes, then
// both cascades' foam), and the foam the folds leave (water_fft.rs
// `Ocean::linger`: held as it fades, spread along the wind, and a share
// riding on with the crest for a moment).
@compute @workgroup_size(16, 16, 1)
fn cs_unpack(@builtin(global_invocation_id) id: vec3<u32>) {
    let dt = params.time.w;
    let fade = exp(-dt / LINGER);
    let toward = params.wind.xy;
    var foam = vec2<f32>(0.0);
    for (var c = 0u; c < 2u; c++) {
        let f0 = fields[at(c, 0u, id.x, id.y)];
        let f1 = fields[at(c, 1u, id.x, id.y)];
        let f2 = fields[at(c, 2u, id.x, id.y)];
        let f3 = fields[at(c, 3u, id.x, id.y)];
        let disp = vec4<f32>(f0, f1);
        let slope = vec4<f32>(f2, f3);
        textureStore(maps, vec2<i32>(id.xy), i32(2u * c), disp);
        textureStore(maps, vec2<i32>(id.xy), i32(2u * c + 1u), slope);
        let tile = select(params.wind.z, params.wind.w, c == 1u);
        let texel = tile / f32(N);
        let k = min(STREAK * dt / (texel * texel), 0.45);
        let p = vec2<f32>(id.xy);
        let here = foam_before[c * N * N + id.y * N + id.x];
        let spread = here + k * (foam_at(c, p.x + toward.x, p.y + toward.y) + foam_at(c, p.x - toward.x, p.y - toward.y) - 2.0 * here);
        let fold = 1.0 - ((1.0 + slope.z) * (1.0 + slope.w) - disp.w * disp.w);
        let kept = max(max(fold, spread * fade), 0.0);
        foam_now[c * N * N + id.y * N + id.x] = kept;
        let shift = toward * select(params.ride.y, params.ride.z, c == 1u) * dt / texel;
        let riding = max(max(fold, foam_at(c + 2u, p.x - shift.x, p.y - shift.y) * exp(-dt / RIDE)), 0.0);
        foam_now[(c + 2u) * N * N + id.y * N + id.x] = riding;
        foam[c] = max(kept, riding);
    }
    textureStore(maps, vec2<i32>(id.xy), 4, vec4<f32>(foam, 0.0, 0.0));
}
