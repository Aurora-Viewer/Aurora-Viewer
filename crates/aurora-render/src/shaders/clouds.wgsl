// Volumetric clouds, a sky-attached layer (clouds.rs). Nubis-style modelling
// (Schneider & Vos, "The Real-time Volumetric Cloudscapes of Horizon Zero
// Dawn", SIGGRAPH 2015): a weather map, a height profile per cloud type and a
// Perlin-Worley base noise eroded by a Worley detail noise; Hillaire-style
// energy conserving integration (Frostbite, SIGGRAPH 2016) with multiple
// scattering octaves (Wrenninge 2013).
//
// Standalone module (its own bindings, no scene shader): the compute entry
// points generate the noise (`gen_*`, a slice per frame), trace the sky
// pixels at half resolution, a quarter of them per frame (`trace`), and
// rebuild the half resolution image from the trace and the history
// (`recon`); the render entry points composite that image into the sky
// (`vs_clouds`, `fs_clouds`, `fs_clouds_g`). The weather map is drawn by the
// scene shader (env.wgsl `fs_cloud_weather`), which holds the cloud texture.
//
// The marching works in a y-up frame (`yup`); the viewer's world is z-up.

struct U {
    inv_vp: mat4x4f,       // clip (ndc, 0.5, 1) to a direction (rotation only, z-up)
    prev_vp: mat4x4f,      // last frame's rotation-only view-projection
    sun: vec4f,            // xyz: direction of the light (z-up)
    sun_col: vec4f,        // linear radiance of the light on a cloud
    amb_col: vec4f,        // linear radiance of the sky light on a cloud
    haze: vec4f,           // rgb: haze colour (linear), w: distance of 1/e (km)
    cam: vec4f,            // xy: camera position in the cloud map's frame (km, z-up horizontal)
    delta: vec4f,          // xyz: camera move since the last frame (km, z-up), w: 1 = reproject it
    drift: vec4f,          // xy: offset of the pattern (km), z: map tile (km), w: extinction scale
    layer: vec4f,          // x: bottom, y: top (km above the camera), z: type (0 St .. 1 Cu)
    sizes: vec4u,          // full w, h, half w, h
    misc: vec4u,           // frame, steps, light steps, history valid
    gen: vec4u,            // x: z of the first noise layer of this frame's slice; a still: yz: its first pixel in the whole image, w: sub-samples
}

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var base_w: texture_storage_3d<rgba8unorm, write>;
@group(0) @binding(2) var detail_w: texture_storage_3d<rgba8unorm, write>;
@group(0) @binding(3) var curl_w: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(5) var base_t: texture_3d<f32>;
@group(0) @binding(6) var detail_t: texture_3d<f32>;
@group(0) @binding(7) var curl_t: texture_2d<f32>;
@group(0) @binding(8) var weather_t: texture_2d<f32>;
@group(0) @binding(9) var samp: sampler;
@group(0) @binding(10) var trace_w: texture_storage_2d<rgba16float, write>;
@group(0) @binding(11) var tdepth_w: texture_storage_2d<rgba16float, write>;
@group(0) @binding(13) var trace_t: texture_2d<f32>;
@group(0) @binding(14) var tdepth_t: texture_2d<f32>;
@group(0) @binding(15) var hist_t: texture_2d<f32>;
@group(0) @binding(16) var hist_w: texture_storage_2d<rgba16float, write>;
@group(0) @binding(19) var depth_t: texture_depth_2d;

const PI: f32 = 3.14159265;
const R_PLANET: f32 = 6371.0;
const FAR_KM: f32 = 45.0;
// The base noise is stretched sideways: a heap is wider than tall (width to
// height 1 to 2 for fair-weather cumulus), and its tile covers 10.5 km across
// but 5 km up (one period of the pattern holds ten of them).
const BASE_KM: f32 = 10.5;
const BASE_Y: f32 = 2.1;
const DETAIL_KM: f32 = 0.5;
const CURL_KM: f32 = 3.0;
const STEP_WARP: f32 = 1.6;
const STEP_MAX_KM: f32 = 0.3;
const SIGMA: f32 = 110.0;    // extinction per km at density 1
// Brightness of the lit side and of the sky light on a cloud against the
// Legacy layer's for the same sky: the thick cloud of a demo sky stays at
// the Legacy layer's mean luminance, and the lit tops reach the white of a
// real cumulus (the bench reference); fitted on the demo skies side by side.
const SUN_GAIN: f32 = 4.5;
const AMB_GAIN: f32 = 0.9;

fn yup(v: vec3f) -> vec3f { return vec3f(v.x, v.z, v.y); }

// ---------------------------------------------------------------- hashing and periodic noise
fn hash_u(x: u32) -> u32 {
    let v = x * 747796405u + 2891336453u;
    let w = ((v >> ((v >> 28u) + 4u)) ^ v) * 277803737u;
    return (w >> 22u) ^ w;
}
fn hash3(c: vec3i) -> u32 {
    return hash_u(bitcast<u32>(c.x) ^ hash_u(bitcast<u32>(c.y) ^ hash_u(bitcast<u32>(c.z) + 0x9e3779b9u)));
}
fn rand3(c: vec3i) -> vec3f {
    let h = hash3(c);
    return vec3f(f32(h & 1023u), f32((h >> 10u) & 1023u), f32((h >> 20u) & 1023u)) / 1023.0;
}
fn wrap3(c: vec3i, p: i32) -> vec3i { return ((c % p) + p) % p; }
fn remap(v: f32, lo: f32, hi: f32, nlo: f32, nhi: f32) -> f32 { return nlo + (v - lo) * (nhi - nlo) / (hi - lo); }

fn grad3(c: vec3i, f: vec3f) -> f32 {
    let g = rand3(c) * 2.0 - 1.0;
    return dot(normalize(g + vec3f(1e-4)), f);
}
// Periodic gradient noise, about [0, 1].
fn perlin3(p: vec3f, period: i32) -> f32 {
    let i = vec3i(floor(p));
    let f = fract(p);
    let w = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let n000 = grad3(wrap3(i + vec3i(0, 0, 0), period), f - vec3f(0.0, 0.0, 0.0));
    let n100 = grad3(wrap3(i + vec3i(1, 0, 0), period), f - vec3f(1.0, 0.0, 0.0));
    let n010 = grad3(wrap3(i + vec3i(0, 1, 0), period), f - vec3f(0.0, 1.0, 0.0));
    let n110 = grad3(wrap3(i + vec3i(1, 1, 0), period), f - vec3f(1.0, 1.0, 0.0));
    let n001 = grad3(wrap3(i + vec3i(0, 0, 1), period), f - vec3f(0.0, 0.0, 1.0));
    let n101 = grad3(wrap3(i + vec3i(1, 0, 1), period), f - vec3f(1.0, 0.0, 1.0));
    let n011 = grad3(wrap3(i + vec3i(0, 1, 1), period), f - vec3f(0.0, 1.0, 1.0));
    let n111 = grad3(wrap3(i + vec3i(1, 1, 1), period), f - vec3f(1.0, 1.0, 1.0));
    let x0 = mix(mix(n000, n100, w.x), mix(n010, n110, w.x), w.y);
    let x1 = mix(mix(n001, n101, w.x), mix(n011, n111, w.x), w.y);
    return mix(x0, x1, w.z) * 0.6 + 0.5;
}
// Periodic cellular noise: 1 at the feature points.
fn worley3(p: vec3f, period: i32) -> f32 {
    let i = vec3i(floor(p));
    let f = fract(p);
    var d = 1e9;
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let o = vec3i(x, y, z);
                let fp = vec3f(o) + rand3(wrap3(i + o, period)) - f;
                d = min(d, dot(fp, fp));
            }
        }
    }
    return 1.0 - saturate(sqrt(d));
}
fn worley_fbm(p: vec3f, f: i32) -> f32 {
    return worley3(p * f32(f), f) * 0.625 + worley3(p * f32(2 * f), 2 * f) * 0.25 + worley3(p * f32(4 * f), 4 * f) * 0.125;
}

// ---------------------------------------------------------------- noise generation, a slice a frame
@compute @workgroup_size(4, 4, 4)
fn gen_base(@builtin(global_invocation_id) gid: vec3u) {
    let n = textureDimensions(base_w);
    let id = vec3u(gid.x, gid.y, gid.z + u.gen.x);
    if any(id >= n) { return; }
    let p = (vec3f(id) + 0.5) / f32(n.x);
    var pf = 0.0;
    var amp = 1.0;
    var tot = 0.0;
    var fr = 4;
    for (var o = 0; o < 4; o++) {
        pf += amp * perlin3(p * f32(fr), fr);
        tot += amp;
        amp *= 0.5;
        fr *= 2;
    }
    pf = saturate((pf / tot - 0.5) * 1.8 + 0.5);
    let w0 = worley_fbm(p, 4);
    let w1 = worley_fbm(p, 8);
    let w2 = worley_fbm(p, 16);
    // Perlin-Worley (Schneider 2015): Perlin dilated by the Worley cells,
    // billowy and connected.
    let pw = saturate(remap(pf, w0 - 1.0, 1.0, 0.0, 1.0));
    textureStore(base_w, id, vec4f(pw, w0, w1, w2));
}

@compute @workgroup_size(4, 4, 4)
fn gen_detail(@builtin(global_invocation_id) id: vec3u) {
    let n = textureDimensions(detail_w);
    if any(id >= n) { return; }
    let p = (vec3f(id) + 0.5) / f32(n.x);
    textureStore(detail_w, id, vec4f(worley_fbm(p, 2), worley_fbm(p, 4), worley_fbm(p, 8), 1.0));
}

@compute @workgroup_size(8, 8)
fn gen_curl(@builtin(global_invocation_id) id: vec3u) {
    let n = textureDimensions(curl_w);
    if any(id.xy >= n) { return; }
    let p = (vec2f(id.xy) + 0.5) / f32(n.x) * 8.0;
    let e = 0.05;
    let dx = perlin3(vec3f(p + vec2f(e, 0.0), 0.5), 8) - perlin3(vec3f(p - vec2f(e, 0.0), 0.5), 8);
    let dy = perlin3(vec3f(p + vec2f(0.0, e), 0.5), 8) - perlin3(vec3f(p - vec2f(0.0, e), 0.5), 8);
    let c = vec2f(dy, -dx) / (2.0 * e);
    textureStore(curl_w, id.xy, vec4f(saturate(c * 0.5 + 0.5), 0.0, 1.0));
}

// ---------------------------------------------------------------- density
fn height_of(p: vec3f) -> f32 { return length(p + vec3f(0.0, R_PLANET, 0.0)) - R_PLANET; }
fn top_km() -> f32 { return u.layer.y; }

// The weather map, laid over the layer: one tile is `u.drift.z` km. `off` is
// the camera's position and the drift of the pattern (km, horizontal).
fn weather_at(p: vec3f, off: vec2f) -> vec4f {
    return textureSampleLevel(weather_t, samp, (p.xz + off) / u.drift.z, 0.0);
}

fn height_grad(hf: f32, t: f32, top: f32) -> f32 {
    // Rounded bottoms and tops for every type: stratus a slab, stratocumulus
    // a lumpy sheet, cumulus a dome whose top varies with `top`.
    let top_end = mix(1.0, mix(0.45, 1.0, top), t);
    let base_ramp = smoothstep(0.0, mix(0.35, 0.08, t), hf);
    let top_fall = 1.0 - smoothstep(mix(0.55, 0.15, t) * top_end, top_end, hf);
    return base_ramp * top_fall;
}

fn density(p: vec3f, w: vec4f, off: vec2f, full: bool) -> f32 {
    let h = height_of(p);
    let hf = (h - u.layer.x) / (top_km() - u.layer.x);
    if hf <= 0.0 || hf >= 1.0 || w.r < 0.01 { return 0.0; }
    let t = u.layer.z;
    let q = vec3f(p.x + off.x, h, p.z + off.y);
    let n = textureSampleLevel(base_t, samp, vec3f(q.x, q.y * BASE_Y, q.z) / BASE_KM, 0.0);
    let low = n.g * 0.625 + n.b * 0.25 + n.a * 0.125;
    var base = saturate(remap(n.r, low - 1.0, 1.0, 0.0, 1.0));
    // A sheet is thicker and thinner from place to place (slow light and
    // dark), a heap's top follows the weather map's detail tap.
    let thick = mix(mix(0.25, 1.0, saturate(n.g * 1.5)), 1.0, t);
    // The base and the top of a heap rise and sink with the cells of the
    // noise (up to 12% of the layer, the heap is squeezed between them): a
    // ray at a grazing angle would otherwise meet the layer's two planes as
    // flat ledges, terraces.
    let lift = 0.12 * t;
    let hv = (hf - lift * saturate(n.b * 1.6 - 0.2)) / (1.0 - lift);
    base *= height_grad(hv / thick, t, w.b);
    base *= mix(mix(0.45, 1.0, saturate(n.g * 1.8)), 1.0, t);
    // Cumulus: coverage shrinks with height, so domes rather than columns.
    let cov = w.r * 0.85 * mix(1.0, mix(1.0, 0.55, hf), t);
    base = saturate(remap(base, 1.0 - cov, 1.0, 0.0, 1.0));
    // A shape that only just passes the cut is a speck, not a cloud: the
    // soft edge starts a little above it.
    base *= smoothstep(0.04, 0.24, base) * mix(0.55, 1.0, cov);
    if !full || base <= 0.0 { return base; }
    let c = textureSampleLevel(curl_t, samp, q.xz / CURL_KM, 0.0).xy * 2.0 - 1.0;
    let qd = q + vec3f(c.x, 0.0, c.y) * 0.12 * (1.0 - hf);
    let d = textureSampleLevel(detail_t, samp, qd / DETAIL_KM, 0.0);
    let df = d.r * 0.625 + d.g * 0.25 + d.b * 0.125;
    // Wispy at the base, billowy towards the top (Schneider 2015): the
    // detail eats the thin edges of the shape and leaves its core.
    let m = mix(df, 1.0 - df, saturate(hf * 4.0));
    let er = mix(0.3, 0.45, t);
    return saturate(remap(base, m * er, 1.0, 0.0, 1.0));
}

// ---------------------------------------------------------------- lighting
fn hg(c: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * pow(1.0 + g2 - 2.0 * g * c, 1.5));
}
// Draine's phase function (1 + alpha c^2 over an HG lobe).
fn draine(c: f32, g: f32, a: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * (1.0 + a * (1.0 + 2.0 * g2) / 3.0)) * (1.0 + a * c * c) / pow(1.0 + g2 - 2.0 * g * c, 1.5);
}
// Approximate Mie phase of cloud droplets (Jendersie & d'Eon, "An
// Approximate Mie Scattering Function for Fog and Cloud Rendering", SIGGRAPH
// 2023 talk, https://research.nvidia.com/labs/rtr/approximate-mie/): a blend
// of a very narrow HG lobe, the glory round the sun that silvers the thin
// edges, and a Draine lobe for the wide scatter. The parameters are the
// paper's fog fit (equations 4 to 7) at a droplet diameter d = 20 micrometres:
//   g_HG = exp(-0.0990567 / (d - 1.67154))                  = 0.9946
//   g_D  = exp(-2.20679 / (d + 3.91029) - 0.428934)         = 0.5938
//   alpha = exp(3.62489 - 8.29288 / (d + 5.52825))          = 27.11
//   w_D  = exp(-0.599085 / (d - 0.641583) - 0.665888)       = 0.4982
const MIE_G_HG: f32 = 0.9946;
const MIE_G_D: f32 = 0.5938;
const MIE_ALPHA: f32 = 27.11;
const MIE_W: f32 = 0.4982;
fn mie(c: f32) -> f32 {
    return mix(hg(c, MIE_G_HG), draine(c, MIE_G_D, MIE_ALPHA), MIE_W);
}

// Exit distance of a ray from the camera (inside the sphere) through the
// sphere of radius `r`.
fn sphere_far(d: vec3f, r: f32) -> f32 {
    let b = R_PLANET * d.y;
    let c = R_PLANET * R_PLANET - r * r;
    return -b + sqrt(b * b - c);
}

struct Res {
    col: vec3f,
    t: f32,
    depth: f32,
}

// `dir` is a y-up direction. Radiance and transmittance of the layer along
// it, and the transmittance-weighted distance of the clouds met.
fn march(dir: vec3f, steps: u32, lsteps: u32, jitter: f32, off: vec2f) -> Res {
    var r: Res;
    r.col = vec3f(0.0);
    r.t = 1.0;
    r.depth = FAR_KM;
    // no clouds below the horizon (as Legacy Viewer SL-11589)
    if dir.y <= 0.0 { return r; }
    let t0 = sphere_far(dir, R_PLANET + u.layer.x);
    let t1 = min(sphere_far(dir, R_PLANET + top_km()), FAR_KM);
    if t0 >= t1 { return r; }
    let len = t1 - t0;
    let sun = yup(u.sun.xyz);
    let ct = dot(dir, sun);
    var wsum = 0.0;
    // Steps grow with the distance, but a grazing ray crosses tens of km of
    // layer: the warp is eased so that no step is longer than STEP_MAX_KM
    // (steps this long sampled a cloud's edge in slabs, terraces). Past what
    // the warp can ease, the count grows with the length, up to 96 on top of
    // `steps`: the cap holds while the layer interval is under 96 x
    // STEP_MAX_KM (28.8 km, not reached with FAR_KM and the layer heights);
    // past that the steps stretch. A caller's own count (stills: 128) is never
    // cut. Mirrored by `march_steps` in clouds.rs (a test pins this line).
    let n = max(steps, min(u32(ceil(len / STEP_MAX_KM)), 96u));
    let warp = clamp(STEP_MAX_KM * f32(n) / len, 1.0, STEP_WARP);
    var prev_t = t0;
    var prev_in = false;
    for (var i = 0u; i < n; i++) {
        let x = (f32(i) + max(jitter, 0.02)) / f32(n);
        var tt = t0 + len * pow(x, warp);
        var ds = len * warp * pow(x, warp - 1.0) / f32(n);
        var p = dir * tt;
        var w = weather_at(p, off);
        var dens = density(p, w, off, true);
        // The step that enters a cloud: the edge is found by bisection
        // between the last empty sample and this one, so that it does not
        // sit on the step grid, and the step only counts from the edge.
        if dens > 0.002 && !prev_in && i > 0u {
            var lo = prev_t;
            var hi = tt;
            for (var k = 0; k < 4; k++) {
                let mid = 0.5 * (lo + hi);
                let pm = dir * mid;
                if density(pm, weather_at(pm, off), off, false) > 0.002 { hi = mid; } else { lo = mid; }
            }
            ds = min(ds, tt - hi + 0.5 * (hi - lo));
            tt = 0.5 * (hi + tt);
            p = dir * tt;
            w = weather_at(p, off);
            dens = density(p, w, off, true);
        }
        prev_t = t0 + len * pow(x, warp);
        prev_in = dens > 0.002;
        if dens > 0.002 {
            let sigma = dens * SIGMA * u.drift.w;
            // Light march: a cone of growing taps on the cheap density, and
            // one far tap.
            var od = 0.0;
            var ls = 0.05;
            var lp = p;
            for (var j = 0u; j < lsteps; j++) {
                lp += sun * ls;
                od += density(lp, weather_at(lp, off), off, false) * ls;
                ls *= 1.7;
            }
            let fp = lp + sun * 1.2;
            od += density(fp, weather_at(fp, off), off, false);
            od *= SIGMA * u.drift.w;
            var s = 0.0;
            var a = 1.0;
            var b = 1.0;
            var cc = 1.0;
            for (var o = 0u; o < 3u; o++) {
                // each octave scatters more evenly (multiple scattering)
                s += a * exp(-od * b) * mix(0.25 / PI, min(mie(ct), 1.2), cc);
                a *= 0.5;
                b *= 0.5;
                cc *= 0.5;
            }
            let powder = 1.0 - 0.6 * exp(-sigma * 0.15);
            let hf = saturate((height_of(p) - u.layer.x) / (top_km() - u.layer.x));
            // The sky light reaches a point through the cloud above it: one
            // tap straight up darkens the undersides and the cores.
            let up = normalize(p + vec3f(0.0, R_PLANET, 0.0));
            let ap = p + up * 0.3;
            let bp = p + up * 0.6;
            let cp = p + up * 1.0;
            let sky_od = (density(ap, weather_at(ap, off), off, false) * 0.3 + density(bp, weather_at(bp, off), off, false) * 0.4
                + density(cp, weather_at(cp, off), off, false) * 0.4) * SIGMA * u.drift.w;
            let sky_vis = mix(0.25, 1.0, exp(-sky_od * 0.012));
            let ls_rad = u.sun_col.rgb * SUN_GAIN * s * 4.0 * PI * powder + u.amb_col.rgb * AMB_GAIN * mix(0.5, 1.0, hf) * sky_vis;
            // A sheet is lit unevenly where it is thick or thin (the base
            // noise's cells, as in `density`): slow light and dark patches.
            let nb = textureSampleLevel(base_t, samp, vec3f(p.x + off.x, height_of(p) * BASE_Y, p.z + off.y) / BASE_KM, 0.0).g;
            let sheet_shade = mix(mix(0.55, 1.0, saturate(nb * 1.8)), 1.0, u.layer.z);
            let ext = exp(-sigma * ds);
            let absorbed = r.t * (1.0 - ext);
            r.col += ls_rad * sheet_shade * absorbed;
            wsum += tt * absorbed;
            r.t *= ext;
            if r.t < 0.01 { break; }
        }
    }
    if r.t < 0.999 {
        r.depth = wsum / (1.0 - r.t);
        // Aerial perspective: far clouds melt into the haze.
        let f = exp(-r.depth / u.haze.w);
        r.col = r.col * f + (1.0 - r.t) * (1.0 - f) * u.haze.rgb;
    }
    return r;
}

// ---------------------------------------------------------------- views
fn view_dir(px: vec2f) -> vec3f {
    let full = vec2f(u.sizes.xy);
    let ndc = vec2f(px.x / full.x * 2.0 - 1.0, 1.0 - px.y / full.y * 2.0);
    let w = u.inv_vp * vec4f(ndc, 0.5, 1.0);
    return normalize(w.xyz / w.w);
}
// The sky is what the depth prepass left at the far plane (reverse-Z: 0).
fn is_sky(px: vec2u) -> bool {
    return textureLoad(depth_t, min(px, u.sizes.xy - 1u), 0) == 0.0;
}
// A half-res pixel takes clouds when its centre is sky or when any pixel of
// the 4x4 full-res block around it is: the blocks on a silhouette (foliage,
// MSAA edges) hold the sky samples the full-res composite still reads.
fn near_sky(hp: vec2u) -> bool {
    if is_sky(vec2u((vec2f(hp) + 0.5) * 2.0)) { return true; }
    let lo = vec2i(hp) * 2 - 1;
    let hi = vec2i(u.sizes.xy) - 1;
    for (var dy = 0; dy < 4; dy++) {
        for (var dx = 0; dx < 4; dx++) {
            let q = clamp(lo + vec2i(dx, dy), vec2i(0), hi);
            if is_sky(vec2u(q)) { return true; }
        }
    }
    return false;
}
// Stills have no half-res composite, but the same silhouette fringe: a pixel
// takes clouds when it or any of its 3x3 neighbours (clamped to the viewport)
// is sky.
fn still_near_sky(px: vec2u) -> bool {
    let hi = vec2i(u.sizes.xy) - 1;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            if is_sky(vec2u(clamp(vec2i(px) + vec2i(dx, dy), vec2i(0), hi))) { return true; }
        }
    }
    return false;
}
fn checker(frame: u32) -> vec2u {
    switch frame % 4u {
        case 0u: { return vec2u(0u, 0u); }
        case 1u: { return vec2u(1u, 1u); }
        case 2u: { return vec2u(1u, 0u); }
        default: { return vec2u(0u, 1u); }
    }
}
fn jitter_of(p: vec2u) -> f32 {
    // Interleaved gradient noise, rotated each frame by the golden ratio.
    let ign = fract(52.9829189 * fract(0.06711056 * f32(p.x) + 0.00583715 * f32(p.y)));
    return fract(ign + f32(u.misc.x) * 0.618034);
}
fn sky_off() -> vec2f { return u.cam.xy + u.drift.xy; }

// ---------------------------------------------------------------- main-view trace (sky pixels, half res, a quarter a frame)
@compute @workgroup_size(8, 8)
fn trace(@builtin(global_invocation_id) id: vec3u) {
    let ts = textureDimensions(trace_w);
    if any(id.xy >= ts) { return; }
    let hp = id.xy * 2u + checker(u.misc.x);
    let fpx = (vec2f(hp) + 0.5) * 2.0;
    if any(hp >= u.sizes.zw) || !near_sky(hp) {
        textureStore(trace_w, id.xy, vec4f(0.0, 0.0, 0.0, 1.0));
        textureStore(tdepth_w, id.xy, vec4f(FAR_KM));
        return;
    }
    if u.layer.w > 1.5 {
        // AURORA_CLOUDS_DEBUG=slice: vertical slices of the density, three
        // bands of the picture (100 px a km, the horizontal axis too, so the
        // aspect of a heap can be measured on the capture); red: density
        // above 0.02, green: above 0.15.
        let band = floor((fpx.y - 60.0) / 100.0);
        let hh = (160.0 + 100.0 * band - fpx.y) / 100.0;
        var o = vec3f(0.0);
        if band >= 0.0 && band < 3.0 && hh >= 0.0 && hh < top_km() - u.layer.x {
            let x = fpx.x / 100.0;
            let z = 2.0 + 5.0 * band;
            let ycorr = u.layer.x + hh - (x * x + z * z) / (2.0 * R_PLANET);
            let p = vec3f(x, ycorr, z);
            let d = density(p, weather_at(p, sky_off()), sky_off(), true);
            o = vec3f(select(0.0, 1.0, d > 0.02), select(0.0, 1.0, d > 0.15), 0.0);
        }
        textureStore(trace_w, id.xy, vec4f(o, 0.0));
        textureStore(tdepth_w, id.xy, vec4f(10.0));
        return;
    }
    if u.layer.w > 0.5 {
        // AURORA_CLOUDS_DEBUG=weather: the coverage under the ray's entry
        // into the layer, as light on the sky
        let d = yup(view_dir(fpx));
        let p = d * (u.layer.x / max(d.y, 0.02));
        let w = weather_at(p, sky_off());
        textureStore(trace_w, id.xy, vec4f(vec3f(w.r), 0.2));
        textureStore(tdepth_w, id.xy, vec4f(10.0));
        return;
    }
    let r = march(yup(view_dir(fpx)), u.misc.y, u.misc.z, jitter_of(hp), sky_off());
    textureStore(trace_w, id.xy, vec4f(r.col, r.t));
    textureStore(tdepth_w, id.xy, vec4f(r.depth));
}

// ---------------------------------------------------------------- still trace (a photo, a tile, a cube face)
// One shot, no history: every pixel of the frame, `u.gen.w` sub-samples whose
// jitters are spread over the step grid. The jitter is keyed on the pixel's
// place in the whole image (`u.gen.yz` is the frame's first pixel in it), so
// that the tiles of a photo agree along their borders; the direction follows
// from the tile's own projection, which is the whole photo's.
fn jitter_still(p: vec2u, s: u32, n: u32) -> f32 {
    let ign = fract(52.9829189 * fract(0.06711056 * f32(p.x) + 0.00583715 * f32(p.y)));
    return fract(ign + f32(s) / f32(n));
}

@compute @workgroup_size(8, 8)
fn trace_still(@builtin(global_invocation_id) id: vec3u) {
    if any(id.xy >= u.sizes.xy) { return; }
    if !still_near_sky(id.xy) {
        textureStore(hist_w, id.xy, vec4f(0.0, 0.0, 0.0, 1.0));
        return;
    }
    let dir = yup(view_dir(vec2f(id.xy) + 0.5));
    let g = id.xy + u.gen.yz;
    let n = max(u.gen.w, 1u);
    var col = vec3f(0.0);
    var t = 0.0;
    for (var s = 0u; s < n; s++) {
        let r = march(dir, u.misc.y, u.misc.z, jitter_still(g, s, n), sky_off());
        col += r.col;
        t += r.t;
    }
    textureStore(hist_w, id.xy, vec4f(col / f32(n), t / f32(n)));
}

// ---------------------------------------------------------------- temporal reconstruction (half res)
@compute @workgroup_size(8, 8)
fn recon(@builtin(global_invocation_id) id: vec3u) {
    let hp = id.xy;
    if any(hp >= u.sizes.zw) { return; }
    let fpx = (vec2f(hp) + 0.5) * 2.0;
    if !near_sky(hp) {
        textureStore(hist_w, hp, vec4f(0.0, 0.0, 0.0, 1.0));
        return;
    }
    let c = vec2i(hp / 2u);
    let ts = vec2i(textureDimensions(trace_t)) - 1;
    var mn = vec4f(1e9);
    var mx = vec4f(-1e9);
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let s = textureLoad(trace_t, clamp(c + vec2i(dx, dy), vec2i(0), ts), 0);
            mn = min(mn, s);
            mx = max(mx, s);
        }
    }
    let fresh = textureLoad(trace_t, c, 0);
    let depth = textureLoad(tdepth_t, c, 0).r;
    let is_fresh = all(hp % 2u == checker(u.misc.x));
    // Reprojection: the rotation, and the camera's horizontal move (the
    // pattern is anchored to the world); time comes through the clamp.
    let dir = view_dir(fpx);
    var pd = dir;
    if u.delta.w > 0.5 {
        pd = normalize(dir * depth + u.delta.xyz);
    }
    let clip = u.prev_vp * vec4f(pd, 1.0);
    let ndc = clip.xy / clip.w;
    let puv = vec2f(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let valid = u.misc.w == 1u && clip.w > 0.0 && all(puv >= vec2f(0.0)) && all(puv <= vec2f(1.0));
    var o = fresh;
    if valid {
        let h = clamp(textureSampleLevel(hist_t, samp, puv, 0.0), mn, mx);
        if is_fresh {
            o = mix(h, fresh, 0.25);
        } else {
            o = h;
        }
    }
    textureStore(hist_w, hp, o);
}

// ---------------------------------------------------------------- composite into the sky pass
// Drawn right after the sky, with the blend `dst * T + L`: the haze, the
// stars, the moon and the sun already in the target are seen through the
// clouds' transmittance.
@vertex
fn vs_clouds(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4f {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    return vec4f(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

fn clouds_pixel(pos: vec4f) -> vec4f {
    let uv = pos.xy / vec2f(u.sizes.xy);
    return textureSampleLevel(hist_t, samp, uv, 0.0);
}

@fragment
fn fs_clouds(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    return clouds_pixel(pos);
}

struct GOut {
    @location(0) color: vec4f,
    @location(1) gbuf: vec4f,
}

// With the G-buffer of the SSR: its target is not written (empty mask).
@fragment
fn fs_clouds_g(@builtin(position) pos: vec4f) -> GOut {
    var o: GOut;
    o.color = clouds_pixel(pos);
    o.gbuf = vec4f(0.0);
    return o;
}
